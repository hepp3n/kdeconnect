use std::{
    net::{Ipv4Addr, SocketAddr, SocketAddrV4},
    sync::Arc,
    time::Duration,
};

use rustls::pki_types::{CertificateDer, PrivateKeyDer, ServerName};
use tokio::{
    io::{AsyncBufReadExt as _, AsyncWriteExt as _, BufReader, split},
    net::{TcpListener, TcpStream, UdpSocket},
    sync::{Mutex, mpsc},
    time::MissedTickBehavior,
};
use tokio_rustls::{TlsAcceptor, TlsConnector, client, server};
use tracing::{debug, error, info, warn};

use crate::{
    GLOBAL_CONFIG,
    device::DeviceId,
    protocol::{Identity, PacketType, ProtocolPacket},
};

pub const DEFAULT_DISCOVERY_INTERVAL: Duration = Duration::from_secs(60);

pub const DEFAULT_LISTEN_PORT: u16 = 1716;
pub const DEFAULT_LISTEN_ADDR: SocketAddr = SocketAddr::V4(SocketAddrV4::new(
    Ipv4Addr::UNSPECIFIED,
    DEFAULT_LISTEN_PORT,
));
pub const BROADCAST_ADDR: SocketAddr =
    SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::BROADCAST, DEFAULT_LISTEN_PORT));

#[derive(Debug)]
pub enum TransportEvent {
    IncomingPacket {
        addr: SocketAddr,
        id: DeviceId,
        raw: String,
    },
    NewConnection {
        addr: SocketAddr,
        id: DeviceId,
        name: String,
        write_tx: mpsc::UnboundedSender<ProtocolPacket>,
    },
}

#[async_trait::async_trait]
pub trait Transport: Send + Sync {
    async fn listen(&self) -> anyhow::Result<()>;
    async fn connect(&self, peer: SocketAddr, id: DeviceId) -> anyhow::Result<()>;
    async fn stop(&self) -> anyhow::Result<()>;
}

pub struct TcpTransport {
    listen_addr: SocketAddr,
    event_tx: mpsc::UnboundedSender<TransportEvent>,
    write_tx: mpsc::UnboundedSender<ProtocolPacket>,
    write_rx: Arc<Mutex<mpsc::UnboundedReceiver<ProtocolPacket>>>,
    identity: Arc<Identity>,
    cert: CertificateDer<'static>,
}

impl TcpTransport {
    pub fn new(event_tx: &mpsc::UnboundedSender<TransportEvent>) -> Self {
        let config = GLOBAL_CONFIG.get().unwrap();
        let (write_tx, write_rx) = mpsc::unbounded_channel::<ProtocolPacket>();
        let write_rx = Arc::new(Mutex::new(write_rx));
        let cert = config.key_store.get_certificateder().clone();

        Self {
            listen_addr: config.listen_addr,
            event_tx: event_tx.clone(),
            write_tx,
            write_rx,
            identity: Arc::new(config.identity.clone()),
            cert,
        }
    }
}

#[async_trait::async_trait]
impl Transport for TcpTransport {
    async fn listen(&self) -> anyhow::Result<()> {
        let listener = TcpListener::bind(self.listen_addr).await?;
        let event_tx = self.event_tx.clone();
        let write_tx = self.write_tx.clone();

        let identity = self.identity.clone();

        loop {
            match listener.accept().await {
                Ok((mut stream, peer)) => {
                    info!("[tcp] new connection from {}", peer);

                    let mut buffer = String::new();
                    let (reader, _writer) = stream.split();
                    let mut reader = BufReader::new(reader);

                    reader
                        .read_line(&mut buffer)
                        .await
                        .expect("Failed to read identity line");

                    let identity = identity.clone();

                    if let Ok(packet) = serde_json::from_str::<ProtocolPacket>(&buffer)
                        && let Ok(peer_identity) = serde_json::from_value::<Identity>(packet.body)
                    {
                        let id = DeviceId(peer_identity.device_id.clone());
                        let name = peer_identity.device_name.clone();

                        if identity.device_id == peer_identity.device_id {
                            warn!("skipping the same device");
                            continue;
                        }

                        // notify about new connection (gives write_tx to allow sending)
                        if let Err(e) = event_tx.send(TransportEvent::NewConnection {
                            addr: peer,
                            id: DeviceId(peer_identity.device_id),
                            name,
                            write_tx: write_tx.clone(),
                        }) {
                            error!("[tcp] transport event channel closed: {}", e);
                        }

                        let _ = self.connect(peer, id).await;
                    }
                }
                Err(e) => {
                    eprintln!("Failed to accept connection: {}", e);
                }
            }
        }
    }

    async fn connect(&self, peer: SocketAddr, id: DeviceId) -> anyhow::Result<()> {
        let event_tx = self.event_tx.clone();
        let write_rx = self.write_rx.clone();
        let identity = self.identity.clone();

        let mut root_cert_store = rustls::RootCertStore::empty();
        root_cert_store.add(self.cert.clone())?;

        let config = rustls::ClientConfig::builder()
            .with_root_certificates(root_cert_store)
            .with_no_client_auth();

        let connector = TlsConnector::from(Arc::new(config));

        let stream = TcpStream::connect(&peer).await?;

        let domain = ServerName::try_from(id.to_string().as_str())?.to_owned();

        let mut tls_stream = connector.connect(domain, stream).await?;

        info!("[tcp] Established TLS connection with device {}", id);

        tls_stream
            .write_all(
                ProtocolPacket::new(
                    PacketType::Identity,
                    serde_json::to_value(&*identity).unwrap(),
                )
                .as_raw()
                .expect("Failed to serialize identity packet")
                .as_slice(),
            )
            .await
            .expect("Failed to send identity packet");

        let (reader, writer) = tokio::io::split(tls_stream);

        let _ = tokio::spawn(rw_client(event_tx, reader, writer, write_rx, peer, id)).await;

        Ok(())
    }

    async fn stop(&self) -> anyhow::Result<()> {
        Ok(())
    }
}

pub struct UdpTransport {
    listen_addr: SocketAddr,
    discovery_interval: Duration,
    event_tx: mpsc::UnboundedSender<TransportEvent>,
    write_tx: mpsc::UnboundedSender<ProtocolPacket>,
    write_rx: Arc<Mutex<mpsc::UnboundedReceiver<ProtocolPacket>>>,
    identity: Arc<Identity>,
    cert: CertificateDer<'static>,
    keypair: PrivateKeyDer<'static>,
}

impl UdpTransport {
    pub fn new(event_tx: &mpsc::UnboundedSender<TransportEvent>) -> Self {
        let config = GLOBAL_CONFIG.get().unwrap();
        let (write_tx, write_rx) = mpsc::unbounded_channel::<ProtocolPacket>();
        let write_rx = Arc::new(Mutex::new(write_rx));
        let cert = config.key_store.get_certificateder().clone();
        let keypair = config.key_store.get_keys().clone_key();

        Self {
            listen_addr: config.listen_addr,
            discovery_interval: config.discovery_interval,
            event_tx: event_tx.clone(),
            write_tx,
            write_rx,
            identity: Arc::new(config.identity.clone()),
            cert,
            keypair,
        }
    }

    pub async fn send_identity(&self, socket: Arc<UdpSocket>) -> anyhow::Result<()> {
        if std::env::var("KDECONNECT_DISABLE_UDP_BROADCAST").is_ok() {
            warn!(
                "UDP broadcast is disabled by environment variable KDECONNECT_DISABLE_UDP_BROADCAST"
            );
            return Ok(()); // Skip broadcasting in test mode
        }

        debug!("Broadcasting UDP identity packet");
        let interval = self.discovery_interval;
        let udp_socket = socket.clone();

        let packet = ProtocolPacket::new(
            PacketType::Identity,
            serde_json::to_value(&*self.identity).unwrap(),
        )
        .as_raw()
        .expect("Failed to serialize identity packet");

        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_secs(1)).await;
            let mut interval = tokio::time::interval(Duration::from_secs(interval.as_secs()));
            interval.set_missed_tick_behavior(MissedTickBehavior::Skip);

            loop {
                match udp_socket.send_to(packet.as_slice(), BROADCAST_ADDR).await {
                    Ok(size) => debug!("Sent {} bytes to {}", size, BROADCAST_ADDR),
                    Err(e) => warn!("Failed to send UDP packet: {}", e),
                }

                interval.tick().await;
            }
        });

        Ok(())
    }
}

#[async_trait::async_trait]
impl Transport for UdpTransport {
    async fn listen(&self) -> anyhow::Result<()> {
        let socket = UdpSocket::bind(self.listen_addr)
            .await
            .expect("failed to bind to socket address");
        let _ = socket.set_broadcast(true);

        let socket = Arc::new(socket);

        let event_tx = self.event_tx.clone();
        let write_tx = self.write_tx.clone();
        let this_identity = self.identity.clone();

        let broadcaster = socket.clone();
        let _ = self.send_identity(broadcaster).await;

        loop {
            let this_identity = this_identity.clone();

            let mut buf = vec![0u8; 8192];

            match socket.recv_from(&mut buf).await {
                Ok((len, mut peer)) => {
                    let raw = &buf[..len];
                    let packet = ProtocolPacket::from_raw(raw).expect("Failed to parse UDP packet");

                    if let Ok(peer_identity) = serde_json::from_value::<Identity>(packet.body) {
                        if this_identity.device_id == peer_identity.device_id {
                            warn!("skipping the same device");
                            continue;
                        }

                        let id = DeviceId(peer_identity.device_id.clone());
                        let name = peer_identity.device_name.clone();

                        if let Some(new_port) = peer_identity.tcp_port {
                            peer.set_port(new_port);
                            info!("Device {} supports TCP at {}", id, peer);
                        }

                        // notify about new connection (gives write_tx to allow sending)
                        if let Err(e) = event_tx.send(TransportEvent::NewConnection {
                            addr: peer,
                            id: DeviceId(peer_identity.device_id),
                            name,
                            write_tx: write_tx.clone(),
                        }) {
                            error!("[udp] transport event channel closed: {}", e);
                        }

                        let _ = self.connect(peer, id).await;
                    }
                }
                Err(e) => {
                    eprintln!("Failed to receive UDP packet: {}", e);
                }
            }
        }
    }

    async fn connect(&self, peer: SocketAddr, id: DeviceId) -> anyhow::Result<()> {
        let event_tx = self.event_tx.clone();
        let write_rx = self.write_rx.clone();

        if let Ok(mut socket) = TcpStream::connect(peer).await {
            let identity_packet = ProtocolPacket::new(
                PacketType::Identity,
                serde_json::to_value(&*self.identity).unwrap(),
            );

            socket
                .write_all(
                    &identity_packet
                        .as_raw()
                        .expect("Failed to serialize identity packet"),
                )
                .await
                .expect("Failed to send identity packet");

            let config = rustls::ServerConfig::builder()
                .with_no_client_auth()
                .with_single_cert(vec![self.cert.clone()], self.keypair.clone_key())?;

            let acceptor = TlsAcceptor::from(Arc::new(config));

            let mut tls_stream = acceptor
                .accept(socket)
                .await
                .expect("Faimed to accept TLS connection");

            info!("[udp] Established TLS connection with device {} ", id);

            tls_stream
                .write_all(
                    &identity_packet
                        .as_raw()
                        .expect("Failed to serialize identity packet"),
                )
                .await
                .expect("Failed to send identity packet over TLS");

            info!("[udp] Sent identity to device {} at {}", id, peer);

            let (reader, writer) = split(tls_stream);

            let _ = tokio::spawn(rw_server(event_tx, reader, writer, write_rx, peer, id)).await;
        }
        Ok(())
    }

    async fn stop(&self) -> anyhow::Result<()> {
        Ok(())
    }
}

/// Helper to run per-connection read loop.
async fn rw_client(
    event_tx: mpsc::UnboundedSender<TransportEvent>,
    reader: tokio::io::ReadHalf<client::TlsStream<TcpStream>>,
    mut writer: tokio::io::WriteHalf<client::TlsStream<TcpStream>>,
    write_rx: Arc<Mutex<mpsc::UnboundedReceiver<ProtocolPacket>>>,
    peer: SocketAddr,
    id: DeviceId,
) {
    let mut reader = BufReader::new(reader);
    let mut buffer = String::new();

    tokio::spawn(async move {
        loop {
            match reader.read_line(&mut buffer).await {
                Ok(0) => {
                    // connection closed
                    warn!("[reader loop] connection to {} closed", peer);
                    break;
                }
                Ok(_len) => {
                    if buffer.trim().is_empty() {
                        warn!("[reader loop] peer {} sent empty message", peer);
                        continue;
                    }

                    // forward raw packet string
                    if let Err(e) = event_tx.send(TransportEvent::IncomingPacket {
                        addr: peer,
                        id: id.clone(),
                        raw: buffer.trim().to_string(),
                    }) {
                        error!("[reader loop] transport event channel closed: {}", e);
                        break;
                    }
                }
                Err(e) => {
                    error!("[reader loop] error reading from {}, {}", peer, e);
                    break;
                }
            }

            buffer.clear();
        }
        warn!("reader loop for {} ended", peer);
    });

    tokio::spawn(async move {
        while let Some(msg) = write_rx.lock().await.recv().await {
            if let Err(e) = writer.write_all(&msg.as_raw().unwrap()).await {
                error!("Error writing to {}: {}", peer, e);
                break;
            }
        }
        info!("writer task for {} ended", peer);
    });
}

/// Helper to create writer channel and writer task that will forward strings to socket
async fn rw_server(
    event_tx: mpsc::UnboundedSender<TransportEvent>,
    reader: tokio::io::ReadHalf<server::TlsStream<TcpStream>>,
    mut writer: tokio::io::WriteHalf<server::TlsStream<TcpStream>>,
    write_rx: Arc<Mutex<mpsc::UnboundedReceiver<ProtocolPacket>>>,
    peer: SocketAddr,
    id: DeviceId,
) {
    let mut reader = BufReader::new(reader);
    let mut buffer = String::new();

    tokio::spawn(async move {
        loop {
            match reader.read_line(&mut buffer).await {
                Ok(0) => {
                    // connection closed
                    warn!("[reader loop] connection to {} closed", peer);
                    break;
                }
                Ok(_len) => {
                    if buffer.trim().is_empty() {
                        warn!("[reader loop] peer {} sent empty message", peer);
                        continue;
                    }

                    // forward raw packet string
                    if let Err(e) = event_tx.send(TransportEvent::IncomingPacket {
                        addr: peer,
                        id: id.clone(),
                        raw: buffer.trim().to_string(),
                    }) {
                        error!("[reader loop] transport event channel closed: {}", e);
                        break;
                    }
                }
                Err(e) => {
                    error!("[reader loop] error reading from {}, {}", peer, e);
                    break;
                }
            }

            buffer.clear();
        }
        warn!("reader loop for {} ended", peer);
    });

    tokio::spawn(async move {
        while let Some(msg) = write_rx.lock().await.recv().await {
            if let Err(e) = writer.write_all(&msg.as_raw().unwrap()).await {
                error!("Error writing to {}: {}", peer, e);
                break;
            }
        }
        info!("writer task for {} ended", peer);
    });
}
