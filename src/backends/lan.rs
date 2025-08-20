use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;
use std::{env, io};

use serde_json as json;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt as _, BufReader};
use tokio::sync::{Mutex, mpsc};
use tokio::time::MissedTickBehavior;
use tokio::{net, task};
use tokio_native_tls::{self, native_tls};
use tracing::{debug, info, warn};

use crate::device::{self, DeviceId, Linked, NewClient};
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
    tpc_server: Option<Arc<net::TcpListener>>,
    u_socket: Option<Arc<net::UdpSocket>>,
    disabled: bool,
    test_mode: bool,
    device_tx: Option<mpsc::UnboundedSender<NewClient>>,
    connected_clients: Arc<Mutex<Vec<NewClient>>>,
    client_action: Option<Arc<Mutex<mpsc::UnboundedReceiver<ClientAction>>>>,
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
            connected_clients: Arc::new(Mutex::new(Vec::new())),
            client_action: None,
        }
    }

    pub fn is_disabled(&self) -> bool {
        self.disabled
    }

    pub(crate) async fn on_start(
        &mut self,
        client_tx: Arc<Mutex<mpsc::UnboundedReceiver<ClientAction>>>,
        device_tx: mpsc::UnboundedSender<NewClient>,
    ) {
        if self.disabled {
            return;
        }

        self.device_tx = Some(device_tx);
        self.client_action = Some(client_tx);

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

            let this_identity = json::from_value::<Identity>(self.network_packet.body.clone())
                .expect("Failed to parse identity");

            if let Ok(packet) = json::from_str::<Packet>(&buffer) {
                let identity =
                    json::from_value::<Identity>(packet.body).expect("Failed to parse identity");

                if identity.device_id == this_identity.device_id {
                    debug!("[TCP] Dont respond to the same device");
                    continue;
                }

                if let Some(port) = identity.tcp_port {
                    addr.set_port(port);
                }

                let data =
                    make_packet_str!(this_identity).expect("Failed to serialize identity packet");

                let ret = async {
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

                    debug!(
                        "New Device Found: {} - {}",
                        identity.device_id, identity.device_name
                    );

                    let (reader, writer) = tokio::io::split(tls_stream);

                    let device_id = DeviceId {
                        id: identity.device_id.clone(),
                        name: identity.device_name.clone(),
                    };

                    if !self.connected_clients.lock().await.is_empty() {
                        self.connected_clients
                            .lock()
                            .await
                            .retain(|c| c.linked.id.id != identity.device_id);
                    }
                    let device = create_device(
                        Arc::new(device_id.clone()),
                        Arc::new(Mutex::new(reader)),
                        Arc::new(Mutex::new(writer)),
                    );

                    let new_client = NewClient {
                        linked: Linked {
                            id: device_id.clone(),
                            connection_type: device::ConnectionType::Server,
                        },
                        device: device.clone(),
                    };

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
            let this_identity = json::from_value::<Identity>(self.network_packet.body.clone())
                .expect("Failed to parse identity");

            if let Ok(packet) = json::from_slice::<Packet>(&buf[..size]) {
                let identity =
                    json::from_value::<Identity>(packet.body).expect("Failed to parse identity");

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

                        debug!(
                            "New Device Found: {} - {}",
                            identity.device_id, identity.device_name
                        );

                        let (reader, writer) = tokio::io::split(tls_stream);

                        let device_id = DeviceId {
                            id: identity.device_id.clone(),
                            name: identity.device_name.clone(),
                        };

                        if !self.connected_clients.lock().await.is_empty() {
                            self.connected_clients
                                .lock()
                                .await
                                .retain(|c| c.linked.id.id != identity.device_id);
                        }
                        let device = create_device(
                            Arc::new(device_id.clone()),
                            Arc::new(Mutex::new(reader)),
                            Arc::new(Mutex::new(writer)),
                        );

                        let new_client = NewClient {
                            linked: Linked {
                                id: device_id.clone(),
                                connection_type: device::ConnectionType::Client,
                            },
                            device: device.clone(),
                        };

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

        let this_identity = json::from_value::<Identity>(self.network_packet.body.clone())
            .expect("Failed to parse identity");

        let data =
            Arc::new(make_packet_str!(this_identity).expect("Failed to serialize identity packet"));

        let task_socket = Arc::clone(socket);
        let task_ident = Arc::clone(&data);

        task::spawn(async move {
            tokio::time::sleep(Duration::from_secs(1)).await;
            let mut interval = tokio::time::interval(Duration::from_secs(30));
            interval.set_missed_tick_behavior(MissedTickBehavior::Skip);

            loop {
                match task_socket
                    .send_to(task_ident.as_bytes(), BROADCAST_ADDR)
                    .await
                {
                    Ok(size) => debug!("Sent {} bytes to {}", size, BROADCAST_ADDR),
                    Err(e) => warn!("Failed to send UDP packet: {}", e),
                }

                interval.tick().await;
            }
        });

        if let Some(client_action) = self.client_action.as_ref() {
            let mut action = client_action.lock().await;

            while let Some(action) = action.recv().await {
                match action {
                    ClientAction::Broadcast => {
                        match socket.send_to(data.as_bytes(), BROADCAST_ADDR).await {
                            Ok(size) => debug!("Sent {} bytes to {}", size, BROADCAST_ADDR),
                            Err(e) => warn!("Failed to send UDP packet: {}", e),
                        }
                    }
                }
            }
        }
    }
}
