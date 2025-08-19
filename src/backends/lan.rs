use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;
use std::{env, io};

use serde_json as json;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt as _, BufReader};
use tokio::net;
use tokio::sync::{Mutex, mpsc};
use tokio::time::MissedTickBehavior;
use tokio_native_tls::{self, native_tls};
use tracing::{debug, info, warn};

use crate::device::{self, ConnectionType, NewClient};
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
    tpc_server: Option<Arc<net::TcpListener>>,
    u_socket: Option<Arc<net::UdpSocket>>,
    disabled: bool,
    test_mode: bool,
    device_tx: Option<mpsc::UnboundedSender<NewClient>>,
    connected_clients: Arc<Mutex<Vec<NewClient>>>,
}

impl LanLinkProvider {
    pub fn new() -> Self {
        let device_uuid = CONFIG.device_uuid.clone();
        let device_name = CONFIG.device_name.clone();

        let identity = Identity::new(device_uuid, device_name);

        Self {
            identity,
            tpc_server: None,
            u_socket: None,
            disabled: false,
            test_mode: false,
            device_tx: None,
            connected_clients: Arc::new(Mutex::new(Vec::new())),
        }
    }

    pub fn is_disabled(&self) -> bool {
        self.disabled
    }

    pub(crate) async fn on_start(&mut self, device_tx: mpsc::UnboundedSender<NewClient>) {
        if self.disabled {
            return;
        }

        self.device_tx = Some(device_tx);

        debug!("Starting LAN Link Provider");

        let bind_addr = match self.test_mode {
            true => LOCALHOST,
            false => UNSPECIFIED_ADDR,
        };

        self.tpc_server = Some(Arc::new(
            net::TcpListener::bind(bind_addr)
                .await
                .expect("Failed to bind TCP listener"),
        ));

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

        tokio::select! {
            _ = self.send_identity_udp() => (),
            _ = self.server(bind_addr) => (),
            _ = self.client() => (),
        };
    }

    async fn server(&self, bind_addr: SocketAddr) -> io::Result<()> {
        let tcp = match self.tpc_server.as_ref() {
            Some(listener) => Arc::clone(listener),
            None => {
                warn!("Cannot bind TCP server on {}", bind_addr);
                return Ok(());
            }
        };

        while let Ok((mut socket, mut addr)) = tcp.accept().await {
            let mut buffer = String::new();
            let (reader, _writer) = socket.split();
            let mut reader = BufReader::new(reader);

            reader
                .read_line(&mut buffer)
                .await
                .expect("Failed to read line");

            if let Ok(packet) = json::from_str::<IdentityPacket>(&buffer) {
                let identity = packet.body;

                if identity.device_id == self.identity.device_id {
                    debug!("[TCP] Dont respond to the same device");
                    continue;
                }

                if self.connected_clients.lock().await.iter().any(|client| {
                    client.0 == identity.device_id && client.1 == ConnectionType::Server
                }) {
                    debug!("[TCP] Device already connected: {}", identity.device_id);
                    continue;
                }

                if let Some(port) = identity.tcp_port {
                    addr.set_port(port);
                }

                let ret = async {
                    let packet = self.identity.create_packet(Some(DEFAULT_PORT));
                    let data = json::to_string(&packet).expect("Creating packet") + "\n";

                    let mut connector = native_tls::TlsConnector::builder();
                    connector.identity(
                        native_tls::Identity::from_pkcs8(&read_certificate(), &read_keypair())
                            .expect("Failed to create TLS identity"),
                    );
                    connector.danger_accept_invalid_certs(true);
                    connector.use_sni(false);
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

                    tls_stream
                        .flush()
                        .await
                        .expect("[TCP] Failed to flush TLS stream");

                    debug!(
                        "New Device Found: {} - {}",
                        identity.device_id, identity.device_name
                    );

                    let (reader, writer) = tokio::io::split(tls_stream);

                    let device =
                        create_device(Arc::new(Mutex::new(reader)), Arc::new(Mutex::new(writer)));

                    let new_client = NewClient(
                        identity.device_id.clone(),
                        device::ConnectionType::Server,
                        device.clone(),
                    );

                    self.connected_clients.lock().await.push(new_client.clone());

                    if let Some(tx) = &self.device_tx {
                        tx.send(new_client).unwrap_or_else(|e| {
                            warn!("Failed to send device to channel: {}", e);
                        });
                    } else {
                        warn!("Device channel is not set, cannot send device");
                    }

                    info!(
                        "Current clients: {}",
                        self.connected_clients.lock().await.len()
                    );

                    debug!("Linked as client....");

                    Ok(()) as Result<(), io::Error>
                };

                if let Err(e) = ret.await {
                    warn!("Failed to handle TCP packet from {}: {}", addr, e);
                };
            }
        }
        Ok(())
    }

    async fn client(&self) -> io::Result<()> {
        let mut buf = [0; 8192];

        let Some(socket) = self.u_socket.as_ref() else {
            warn!("UDP socket is not initialized, cannot start server");
            return Ok(());
        };

        while let Ok((size, mut addr)) = socket.recv_from(&mut buf).await {
            if let Ok(packet) = json::from_slice::<IdentityPacket>(&buf[..size]) {
                let identity = packet.body;

                if identity.device_id == self.identity.device_id {
                    debug!("[UDP] Dont respond to the same device");
                    continue;
                }

                if self.connected_clients.lock().await.iter().any(|client| {
                    client.0 == identity.device_id && client.1 == ConnectionType::Client
                }) {
                    debug!("[TCP] Device already connected: {}", identity.device_id);
                    continue;
                }

                let ret = async {
                    if let Some(port) = identity.tcp_port {
                        addr.set_port(port);
                    }

                    let packet = self.identity.create_packet(Some(DEFAULT_PORT));
                    let data = json::to_string(&packet).expect("Creating packet") + "\n";

                    let mut sock = net::TcpStream::connect(addr)
                        .await
                        .expect("[TCP] Failed to connect to device");

                    sock.write_all(data.as_bytes())
                        .await
                        .expect("[TCP] Failed to write data to socket");

                    let cert =
                        native_tls::Identity::from_pkcs8(&read_certificate(), &read_keypair())
                            .unwrap();

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

                    tls_stream
                        .flush()
                        .await
                        .expect("[TCP] Failed to flush TLS stream");

                    debug!(
                        "New Device Found: {} - {}",
                        identity.device_id, identity.device_name
                    );

                    let (reader, writer) = tokio::io::split(tls_stream);

                    let device =
                        create_device(Arc::new(Mutex::new(reader)), Arc::new(Mutex::new(writer)));

                    let new_client = NewClient(
                        identity.device_id.clone(),
                        device::ConnectionType::Client,
                        device.clone(),
                    );

                    self.connected_clients.lock().await.push(new_client.clone());

                    if let Some(tx) = &self.device_tx {
                        tx.send(new_client).unwrap_or_else(|e| {
                            warn!("Failed to send device to channel: {}", e);
                        });
                    } else {
                        warn!("Device channel is not set, cannot send device");
                    }

                    info!(
                        "Current clients: {}",
                        self.connected_clients.lock().await.len()
                    );

                    debug!("New server linked....");

                    Ok(()) as Result<(), io::Error>
                };

                if let Err(e) = ret.await {
                    warn!("Failed to handle UDP packet from {}: {}", addr, e);
                };
            };
        }
        Ok(())
    }

    async fn send_identity_udp(&self) {
        if env::var("KDECONNECT_DISABLE_UDP_BROADCAST").is_ok() {
            warn!(
                "UDP broadcast is disabled by environment variable KDECONNECT_DISABLE_UDP_BROADCAST"
            );
            return; // Skip broadcasting in test mode
        }

        debug!("Broadcasting UDP identity packet");

        let Some(socket) = self.u_socket.as_ref() else {
            warn!("UDP socket is not initialized, cannot start server");
            return;
        };

        tokio::time::sleep(Duration::from_secs(1)).await;
        let mut interval = tokio::time::interval(Duration::from_secs(30));
        interval.set_missed_tick_behavior(MissedTickBehavior::Skip);

        loop {
            match socket
                .send_to(
                    self.identity.to_string(Some(DEFAULT_PORT)).as_bytes(),
                    BROADCAST_ADDR,
                )
                .await
            {
                Ok(size) => debug!("Sent {} bytes to {}", size, BROADCAST_ADDR),
                Err(e) => warn!("Failed to send UDP packet: {}", e),
            }

            interval.tick().await;
        }
    }
}
