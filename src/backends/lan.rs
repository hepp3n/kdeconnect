use std::env;
use std::net::SocketAddr;
use std::sync::Arc;

use serde_json as json;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt as _, BufReader};
use tokio::sync::{Mutex, mpsc};
use tokio::time::sleep;
use tokio::{join, net, task};
use tokio_native_tls::{self, native_tls};
use tracing::{debug, info, warn};

use crate::device;
use crate::packet::Identity;
use crate::{
    backends::{BROADCAST_ADDR, DEFAULT_PORT, LOCALHOST, UNSPECIFIED_ADDR},
    config::CONFIG,
    device::create_device,
    packet::IdentityPacket,
    ssl::{read_certificate, read_keypair},
};

pub(crate) struct LanLinkProvider {
    identity: Identity,
    server: Option<Arc<net::TcpListener>>,
    u_socket: Option<Arc<net::UdpSocket>>,
    disabled: bool,
    test_mode: bool,
}

impl LanLinkProvider {
    pub fn new() -> Self {
        let device_uuid = CONFIG.device_uuid.clone();
        let device_name = CONFIG.device_name.clone();

        let identity = Identity::new(device_uuid, device_name);

        Self {
            identity,
            server: None,
            u_socket: None,
            disabled: false,
            test_mode: false,
        }
    }

    pub fn is_disabled(&self) -> bool {
        self.disabled
    }

    pub fn enable(&mut self) {
        self.disabled = false;
    }

    pub fn disable(&mut self) {
        self.disabled = true;
    }

    pub(crate) async fn on_start(
        &mut self,
        device_tx: mpsc::UnboundedSender<(String, device::Device)>,
    ) {
        // This function can be used to start the server and socket if needed.
        // For now, it does nothing as the server and socket are initialized lazily.
        if self.disabled {
            return;
        }

        debug!("Starting LAN Link Provider");

        let bind_addr = match self.test_mode {
            true => LOCALHOST,
            false => UNSPECIFIED_ADDR,
        };

        self.u_socket = Some(Arc::new(
            net::UdpSocket::bind(bind_addr)
                .await
                .expect("Failed to bind UDP socket"),
        ));

        match self.u_socket.as_ref() {
            Some(socket) => {
                if let Err(e) = socket.set_broadcast(true) {
                    eprintln!("Failed to set broadcast on UDP socket: {}", e);
                }
                debug!("UDP socket bound to {}", bind_addr);
            }
            None => {
                eprintln!("Cannot broadcast UDP socket.");
            }
        };

        let arc_ident = Arc::new(self.identity.clone());

        let cloned_ident = Arc::clone(&arc_ident);
        let cloned_socket = Arc::clone(self.u_socket.as_ref().unwrap());

        let broadcaster = task::spawn(async move {
            loop {
                Self::broadcast_udp_identity_packet(cloned_socket.clone(), cloned_ident.clone())
                    .await;

                sleep(std::time::Duration::from_secs(15)).await;
            }
        });

        let cloned_socket = Arc::clone(self.u_socket.as_ref().unwrap());
        let cloned_ident = Arc::clone(&arc_ident);
        let tx = device_tx.clone();

        let receiver = task::spawn(async move {
            // Start listening for UDP packets in a separate task
            Self::udp_broadcast_received(cloned_socket.clone(), cloned_ident.clone(), tx).await;
        });

        self.server = Some(Arc::new(
            net::TcpListener::bind(bind_addr)
                .await
                .expect("Failed to bind TCP listener"),
        ));

        let tcp = match self.server.as_ref() {
            Some(listener) => Arc::clone(listener),
            None => {
                warn!("Cannot bind TCP server on {}", bind_addr);
                return;
            }
        };

        let server = task::spawn(async move {
            while let Ok((mut socket, mut addr)) = tcp.accept().await {
                let mut buffer = String::new();
                let (reader, mut writer) = socket.split();
                let mut reader = BufReader::new(reader);

                reader
                    .read_line(&mut buffer)
                    .await
                    .expect("Failed to read line");

                if let Ok(packet) = json::from_str::<IdentityPacket>(&buffer) {
                    let identity = packet.body;

                    if identity.device_id == arc_ident.device_id {
                        debug!("[TCP] Dont respond to the same device");
                        continue;
                    }

                    if let Some(port) = identity.tcp_port {
                        addr.set_port(port);
                    }

                    let packet = arc_ident.create_packet(Some(DEFAULT_PORT));
                    let data = json::to_string(&packet).expect("Creating packet") + "\n";

                    let mut connector = native_tls::TlsConnector::builder();
                    connector.identity(
                        native_tls::Identity::from_pkcs8(&read_certificate(), &read_keypair())
                            .expect("Failed to create TLS identity"),
                    );
                    connector.danger_accept_invalid_certs(true);
                    connector.use_sni(true);
                    let connector = tokio_native_tls::TlsConnector::from(
                        connector.build().expect("Failed to create TLS connector"),
                    );

                    let mut tls_stream = connector
                        .connect(&identity.device_id, socket)
                        .await
                        .unwrap();

                    tls_stream
                        .write_all(data.as_bytes())
                        .await
                        .expect("[TCP] Failed to write data to TLS stream");

                    debug!(
                        "New Device Found: {} - {}",
                        identity.device_id, identity.device_name
                    );

                    let device = create_device(Some(Arc::new(Mutex::new(tls_stream))));

                    device_tx
                        .send((identity.device_id, device))
                        .unwrap_or_else(|e| {
                            warn!("Failed to send device to channel: {}", e);
                        });

                    debug!("Device linked.");
                }
            }
        });

        let _ = join!(broadcaster, receiver, server);
    }

    async fn broadcast_udp_identity_packet(socket: Arc<net::UdpSocket>, identity: Arc<Identity>) {
        if env::var("KDECONNECT_DISABLE_UDP_BROADCAST").is_ok() {
            warn!(
                "UDP broadcast is disabled by environment variable KDECONNECT_DISABLE_UDP_BROADCAST"
            );
            return; // Skip broadcasting in test mode
        }

        debug!("Broadcasting UDP identity packet");

        match socket
            .send_to(identity.to_string(None).as_bytes(), BROADCAST_ADDR)
            .await
        {
            Ok(size) => debug!("Sent {} bytes to {}", size, BROADCAST_ADDR),
            Err(e) => warn!("Failed to send UDP packet: {}", e),
        }
    }

    async fn send_udp_identity_packet(
        identity: Identity,
        socket: &net::UdpSocket,
        addr: SocketAddr,
    ) {
        if env::var("KDECONNECT_DISABLE_UDP_BROADCAST").is_ok() {
            warn!(
                "UDP broadcast is disabled by environment variable KDECONNECT_DISABLE_UDP_BROADCAST"
            );
            return; // Skip broadcasting in test mode
        }

        debug!("Sending UDP identity packet to {:#?}", addr);

        match socket
            .send_to(identity.to_string(None).as_bytes(), addr)
            .await
        {
            Ok(size) => debug!("Sent {} bytes to {}", size, addr),
            Err(e) => warn!("Failed to send UDP packet: {}", e),
        }
    }

    async fn udp_broadcast_received(
        socket: Arc<net::UdpSocket>,
        this_device: Arc<Identity>,
        device_tx: mpsc::UnboundedSender<(String, device::Device)>,
    ) {
        let mut buf = [0; 8192];

        while let Ok((size, mut addr)) = socket.recv_from(&mut buf).await {
            if let Ok(packet) = json::from_slice::<IdentityPacket>(&buf[..size]) {
                let identity = packet.body;

                if identity.device_id == this_device.device_id {
                    debug!("[UDP] Dont respond to the same device");
                    continue;
                }

                if let Some(port) = identity.tcp_port {
                    addr.set_port(port);
                }

                let packet = this_device.create_packet(Some(DEFAULT_PORT));
                let data = json::to_string(&packet).expect("Creating packet") + "\n";

                let mut sock = net::TcpStream::connect(addr)
                    .await
                    .expect("[TCP] Failed to connect to device");

                sock.write_all(data.as_bytes())
                    .await
                    .expect("[TCP] Failed to write data to socket");

                let cert =
                    native_tls::Identity::from_pkcs8(&read_certificate(), &read_keypair()).unwrap();

                let tls_acceptor = tokio_native_tls::TlsAcceptor::from(
                    native_tls::TlsAcceptor::builder(cert)
                        .build()
                        .expect("Failed to create TLS acceptor"),
                );

                let mut tls_stream = tls_acceptor.accept(sock).await.expect("accept error");
                tls_stream
                    .write_all(data.as_bytes())
                    .await
                    .expect("[TCP] Failed to write data to TLS stream");

                debug!(
                    "New Device Found: {} - {}",
                    identity.device_id, identity.device_name
                );

                let device = create_device(Some(Arc::new(Mutex::new(tls_stream))));

                device_tx
                    .send((identity.device_id, device))
                    .unwrap_or_else(|e| {
                        warn!("Failed to send device to channel: {}", e);
                    });

                debug!("Device linked.");
            };
        }
    }
}
