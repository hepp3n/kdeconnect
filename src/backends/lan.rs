use std::sync::Arc;
use std::time::Duration;
use std::{env, io};

use serde_json as json;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt as _, BufReader};
use tokio::net::UdpSocket;
use tokio::sync::{Mutex, mpsc};
use tokio::time::MissedTickBehavior;
use tokio::{net, task};
use tokio_native_tls::{self, native_tls};
use tracing::{debug, error, info, warn};

use crate::device::{Device, DeviceId};
use crate::make_packet_str;
use crate::packet::{Identity, Packet, PacketType};
use crate::{ClientAction, make_packet};
use crate::{
    backends::{BROADCAST_ADDR, LOCALHOST, UNSPECIFIED_ADDR},
    device::create_device,
    ssl::{read_certificate, read_keypair},
};

pub(crate) struct LanLinkProvider {
    network_packet: Packet,
    tpc_server: Option<net::TcpListener>,
    u_socket: Option<Arc<net::UdpSocket>>,
    disabled: bool,
    test_mode: bool,
    device_tx: Option<mpsc::UnboundedSender<Device>>,
    connected_clients: Vec<Device>,
}

impl LanLinkProvider {
    pub fn new(np: Packet) -> Self {
        Self {
            network_packet: np,
            tpc_server: None,
            u_socket: None,
            disabled: false,
            test_mode: false,
            device_tx: None,
            connected_clients: Vec::new(),
        }
    }

    pub fn is_disabled(&self) -> bool {
        self.disabled
    }

    pub(crate) async fn on_start(
        &mut self,
        client_tx: Arc<Mutex<mpsc::UnboundedReceiver<ClientAction>>>,
        device_tx: mpsc::UnboundedSender<Device>,
    ) {
        self.device_tx = Some(device_tx);

        debug!("Starting LAN Link Provider");

        let bind_addr = match self.test_mode {
            true => LOCALHOST,
            false => UNSPECIFIED_ADDR,
        };

        self.tpc_server = Some(
            net::TcpListener::bind(bind_addr)
                .await
                .expect("Failed to bind TCP listener"),
        );

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
                task::spawn(send_identity_udp(
                    socket.clone(),
                    self.network_packet.clone(),
                    client_tx,
                ));
            }
            None => {
                eprintln!("Cannot broadcast UDP socket.");
            }
        };

        while !self.disabled {
            match self.u_socket.as_ref() {
                Some(udp) => {
                    let mut buf = [0; 8192];

                    if let Ok((size, mut addr)) = udp.recv_from(&mut buf).await {
                        let this_identity =
                            json::from_value::<Identity>(self.network_packet.body.clone())
                                .expect("Failed to parse identity");

                        if let Ok(packet) = json::from_slice::<Packet>(&buf[..size]) {
                            let identity = json::from_value::<Identity>(packet.body)
                                .expect("Failed to parse identity");

                            if identity.device_id == this_identity.device_id {
                                debug!("[UDP] Dont respond to the same device");
                                continue;
                            }

                            let ret = async {
                                if let Some(port) = identity.tcp_port {
                                    addr.set_port(port);
                                }

                                if let Ok(mut sock) = net::TcpStream::connect(addr).await {
                                    let data = make_packet_str!(this_identity)
                                        .expect("Failed to serialize identity packet");

                                    sock.write_all(data.as_bytes())
                                        .await
                                        .expect("[TCP] Failed to write data to socket");

                                    let cert = native_tls::Identity::from_pkcs8(
                                        &read_certificate(),
                                        &read_keypair(),
                                    )
                                    .unwrap();

                                    let tls_acceptor = tokio_native_tls::TlsAcceptor::from(
                                        native_tls::TlsAcceptor::builder(cert)
                                            .build()
                                            .expect("Failed to create TLS acceptor"),
                                    );

                                    let mut tls_stream =
                                        tls_acceptor.accept(sock).await.expect("accept error");

                                    tls_stream
                                        .write_all(data.as_bytes())
                                        .await
                                        .expect("[TCP] Failed to write data to TLS stream");

                                    debug!(
                                        "New Device Found: {} - {}",
                                        identity.device_id, identity.device_name
                                    );

                                    let (reader, writer) = tokio::io::split(tls_stream);

                                    let device_id = DeviceId {
                                        id: identity.device_id.clone(),
                                        name: identity.device_name.clone(),
                                    };

                                    if !self.connected_clients.is_empty() {
                                        self.connected_clients
                                            .retain(|c| c.id.id != identity.device_id);
                                    }
                                    let device = create_device(
                                        device_id.clone(),
                                        Arc::new(Mutex::new(reader)),
                                        Arc::new(Mutex::new(writer)),
                                    )
                                    .await;

                                    self.connected_clients.push(device.clone());

                                    if let Some(tx) = &self.device_tx {
                                        tx.send(device).unwrap_or_else(|e| {
                                            warn!("Failed to send device to channel: {}", e);
                                        });
                                    } else {
                                        warn!("Device channel is not set, cannot send device");
                                    }

                                    info!("Current clients: {}", self.connected_clients.len());
                                } else {
                                    return Err(io::Error::new(
                                        io::ErrorKind::ConnectionRefused,
                                        "Failed to connect to TCP server",
                                    ));
                                };

                                Ok(()) as Result<(), io::Error>
                            };

                            if let Err(e) = ret.await {
                                warn!("Failed to handle UDP packet from {}: {}", addr, e);
                            };
                        };
                    }
                }
                None => {
                    error!("UDP socket is not initialized, cannot start server");
                }
            }
            match self.tpc_server.as_ref() {
                Some(tcp) => {
                    if let Ok((mut socket, mut addr)) = tcp.accept().await {
                        let mut buffer = String::new();
                        let (reader, _writer) = socket.split();
                        let mut reader = BufReader::new(reader);

                        reader
                            .read_line(&mut buffer)
                            .await
                            .expect("Failed to read line");

                        let this_identity =
                            json::from_value::<Identity>(self.network_packet.body.clone())
                                .expect("Failed to parse identity");

                        if let Ok(packet) = json::from_str::<Packet>(&buffer) {
                            let identity = json::from_value::<Identity>(packet.body)
                                .expect("Failed to parse identity");

                            if identity.device_id == this_identity.device_id {
                                debug!("[TCP] Dont respond to the same device");

                                continue;
                            }

                            let ret = async {
                                if let Some(port) = identity.tcp_port {
                                    addr.set_port(port);
                                }

                                let data = make_packet_str!(this_identity)
                                    .expect("Failed to serialize identity packet");

                                let mut connector = native_tls::TlsConnector::builder();
                                connector.identity(
                                    native_tls::Identity::from_pkcs8(
                                        &read_certificate(),
                                        &read_keypair(),
                                    )
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

                                debug!(
                                    "New Device Found: {} - {}",
                                    identity.device_id, identity.device_name
                                );

                                let (reader, writer) = tokio::io::split(tls_stream);

                                let device_id = DeviceId {
                                    id: identity.device_id.clone(),
                                    name: identity.device_name.clone(),
                                };

                                if !self.connected_clients.is_empty() {
                                    self.connected_clients
                                        .retain(|c| c.id.id != identity.device_id);
                                }

                                let device = create_device(
                                    device_id.clone(),
                                    Arc::new(Mutex::new(reader)),
                                    Arc::new(Mutex::new(writer)),
                                )
                                .await;

                                self.connected_clients.push(device.clone());

                                if let Some(tx) = &self.device_tx {
                                    tx.send(device).unwrap_or_else(|e| {
                                        warn!("Failed to send device to channel: {}", e);
                                    });
                                } else {
                                    warn!("Device channel is not set, cannot send device");
                                }

                                info!("Current clients: {}", self.connected_clients.len());

                                Ok(()) as Result<(), io::Error>
                            };

                            if let Err(e) = ret.await {
                                warn!("Failed to handle TCP packet from {}: {}", addr, e);
                            };
                        };
                    }
                }
                None => {
                    error!("TCP server is not initialized.");
                }
            };
        }
    }
}

async fn send_identity_udp(
    socket: Arc<UdpSocket>,
    np: Packet,
    client_tx: Arc<Mutex<mpsc::UnboundedReceiver<ClientAction>>>,
) {
    if env::var("KDECONNECT_DISABLE_UDP_BROADCAST").is_ok() {
        warn!("UDP broadcast is disabled by environment variable KDECONNECT_DISABLE_UDP_BROADCAST");
        return; // Skip broadcasting in test mode
    }

    debug!("Broadcasting UDP identity packet");

    let this_identity =
        json::from_value::<Identity>(np.body.clone()).expect("Failed to parse identity");

    let data =
        Arc::new(make_packet_str!(this_identity).expect("Failed to serialize identity packet"));

    let task_data = Arc::clone(&data);
    let udp_socket = Arc::clone(&socket);

    task::spawn(async move {
        tokio::time::sleep(Duration::from_secs(1)).await;
        let mut interval = tokio::time::interval(Duration::from_secs(30));
        interval.set_missed_tick_behavior(MissedTickBehavior::Skip);

        loop {
            match udp_socket
                .send_to(task_data.as_bytes(), BROADCAST_ADDR)
                .await
            {
                Ok(size) => debug!("Sent {} bytes to {}", size, BROADCAST_ADDR),
                Err(e) => warn!("Failed to send UDP packet: {}", e),
            }

            interval.tick().await;
        }
    });

    let client_data = Arc::clone(&data);

    while let Some(action) = client_tx.lock().await.recv().await {
        match action {
            ClientAction::Broadcast => {
                match socket.send_to(client_data.as_bytes(), BROADCAST_ADDR).await {
                    Ok(size) => debug!("Sent {} bytes to {}", size, BROADCAST_ADDR),
                    Err(e) => warn!("Failed to send UDP packet: {}", e),
                }
            }
        }
    }
}
