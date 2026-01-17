use std::{
    net::{Ipv4Addr, SocketAddr, SocketAddrV4},
    path::PathBuf,
    sync::Arc,
    time::Duration,
};

use rustls::pki_types::ServerName;
use tokio::{
    io::{AsyncBufReadExt as _, AsyncWriteExt, BufReader, split},
    net::{TcpListener, TcpStream, UdpSocket},
    sync::{Mutex, mpsc},
    time::MissedTickBehavior,
};
use tokio_rustls::{TlsAcceptor, TlsConnector};
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

pub struct TcpTransport {
    listen_addr: SocketAddr,
    event_tx: mpsc::UnboundedSender<TransportEvent>,
    identity: Arc<Identity>,
    client_config: Arc<rustls::ClientConfig>,
}

impl TcpTransport {
    pub fn new(event_tx: &mpsc::UnboundedSender<TransportEvent>) -> Self {
        let config = GLOBAL_CONFIG.get().unwrap();
        let listen_addr = config.listen_addr;
        let event_tx = event_tx.clone();
        let identity = Arc::new(config.identity.clone());
        let client_config = config.key_store.client_config.clone();

        Self {
            listen_addr,
            event_tx,
            identity,
            client_config,
        }
    }

    pub async fn listen(&self) -> anyhow::Result<()> {
        let listener = TcpListener::bind(self.listen_addr).await?;

        loop {
            let event_tx = self.event_tx.clone();
            let identity = self.identity.clone();

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
                        let name = peer_identity.device_name.clone();
                        let id = peer_identity.device_id.clone();

                        if identity.device_id == peer_identity.device_id {
                            warn!("skipping the same device");
                            continue;
                        }

                        let mut stream = TlsConnector::from(self.client_config.clone())
                            .connect(id.clone().try_into()?, stream)
                            .await?;

                        let packet = ProtocolPacket::new(
                            PacketType::Identity,
                            serde_json::to_value(&*self.identity).unwrap(),
                        )
                        .as_raw()
                        .expect("Failed to serialize identity packet");

                        let _ = stream.write_all(packet.as_slice()).await;

                        let (reader, writer) = split(stream);

                        let (write_tx, write_rx) = mpsc::unbounded_channel::<ProtocolPacket>();
                        let write_rx = Arc::new(Mutex::new(write_rx));

                        let _ = tokio::spawn(handle_connection(
                            event_tx.clone(),
                            reader,
                            writer,
                            write_rx,
                            peer,
                            DeviceId(id),
                        ))
                        .await;

                        // notify about new connection (gives write_tx to allow sending)
                        if let Err(e) = event_tx.send(TransportEvent::NewConnection {
                            addr: peer,
                            id: DeviceId(peer_identity.device_id),
                            name,
                            write_tx,
                        }) {
                            error!("[tcp] transport event channel closed: {}", e);
                        }
                    }
                }
                Err(e) => {
                    eprintln!("Failed to accept connection: {}", e);
                }
            }
        }
    }
}

pub struct UdpTransport {
    socket: Arc<UdpSocket>,
    discovery_interval: Duration,
    event_tx: mpsc::UnboundedSender<TransportEvent>,
    identity: Arc<Identity>,
    server_config: Arc<rustls::ServerConfig>,
}

impl UdpTransport {
    pub async fn new(event_tx: &mpsc::UnboundedSender<TransportEvent>) -> Self {
        let config = GLOBAL_CONFIG.get().unwrap();

        let socket = UdpSocket::bind(config.listen_addr)
            .await
            .expect("failed to bind to socket address");
        let _ = socket.set_broadcast(true);
        let socket = Arc::new(socket);

        let discovery_interval = config.discovery_interval;
        let event_tx = event_tx.clone();
        let identity = Arc::new(config.identity.clone());
        let server_config = config.key_store.server_config.clone();

        Self {
            socket,
            discovery_interval,
            event_tx,
            identity,
            server_config,
        }
    }

    pub async fn send_identity(&self) -> anyhow::Result<()> {
        if std::env::var("KDECONNECT_DISABLE_UDP_BROADCAST").is_ok() {
            warn!(
                "UDP broadcast is disabled by environment variable KDECONNECT_DISABLE_UDP_BROADCAST"
            );
            return Ok(()); // Skip broadcasting in test mode
        }

        debug!("Broadcasting UDP identity packet");
        let interval = self.discovery_interval;
        let udp_socket = self.socket.clone();

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

    pub async fn listen(&self) -> anyhow::Result<()> {
        let event_tx = self.event_tx.clone();
        let this_identity = self.identity.clone();

        loop {
            let this_identity = this_identity.clone();

            let mut buf = vec![0u8; 8192];

            match self.socket.recv_from(&mut buf).await {
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

                        let mut stream = TcpStream::connect(peer).await?;

                        let packet = ProtocolPacket::new(
                            PacketType::Identity,
                            serde_json::to_value(&*self.identity).unwrap(),
                        )
                        .as_raw()
                        .expect("Failed to serialize identity packet");

                        let _ = stream.write_all(packet.as_slice()).await;

                        let acceptor = TlsAcceptor::from(self.server_config.clone());

                        let mut tls_stream = acceptor
                            .accept(stream)
                            .await
                            .expect("Failed to accept TLS connection");

                        info!("[udp] Established TLS connection with device {} ", id);

                        let packet = ProtocolPacket::new(
                            PacketType::Identity,
                            serde_json::to_value(&*self.identity).unwrap(),
                        )
                        .as_raw()
                        .expect("Failed to serialize identity packet");

                        let _ = tls_stream.write_all(packet.as_slice()).await;

                        let (reader, writer) = split(tls_stream);

                        let (write_tx, write_rx) = mpsc::unbounded_channel::<ProtocolPacket>();
                        let write_rx = Arc::new(Mutex::new(write_rx));

                        let _ = tokio::spawn(handle_connection(
                            event_tx.clone(),
                            reader,
                            writer,
                            write_rx,
                            peer,
                            id,
                        ))
                        .await;

                        // notify about new connection (gives write_tx to allow sending)
                        if let Err(e) = event_tx.send(TransportEvent::NewConnection {
                            addr: peer,
                            id: DeviceId(peer_identity.device_id),
                            name,
                            write_tx,
                        }) {
                            error!("[udp] transport event channel closed: {}", e);
                        }
                    }
                }
                Err(e) => {
                    eprintln!("Failed to receive UDP packet: {}", e);
                }
            }
        }
    }
}

async fn handle_connection<R, W>(
    event_tx: mpsc::UnboundedSender<TransportEvent>,
    reader: R,
    mut writer: W,
    write_rx: Arc<Mutex<mpsc::UnboundedReceiver<ProtocolPacket>>>,
    peer: SocketAddr,
    id: DeviceId,
) where
    R: tokio::io::AsyncRead + Unpin + Send + 'static,
    W: tokio::io::AsyncWrite + Unpin + Send + 'static,
{
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
            debug!("writing {}", msg.packet_type);

            if let Err(e) = writer.write_all(&msg.as_raw().unwrap()).await {
                error!("Error writing to {}: {}", peer, e);
                break;
            }
        }
        let _ = writer.shutdown().await;
        info!("writer task for {} ended", peer);
    });
}

pub(crate) async fn prepare_listener_for_payload() -> Result<TcpListener, String> {
    let mut free_listener: Option<TcpListener> = None;
    let mut free_port: Option<u16> = None;

    for port in 1739..1769 {
        if let Ok(listener) =
            TcpListener::bind(SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, port)).await
        {
            free_listener = Some(listener);
            free_port = Some(port);

            break;
        }
    }

    if let Some(free_listener) = free_listener
        && let Some(_free_port) = free_port
    {
        Ok(free_listener)
    } else {
        Err("no free port for payload, failed.".to_string())
    }
}

pub(crate) async fn receive_payload(
    domain: &DeviceId,
    addr: &SocketAddr,
    temp_file: &PathBuf,
) -> anyhow::Result<()> {
    let config = GLOBAL_CONFIG.get().unwrap();
    let client_config = config.key_store.client_config.clone();
    debug!("client config created.");

    let stream = TcpStream::connect(&addr).await?;

    let domain = ServerName::try_from(domain.0.as_str())?.to_owned();

    let mut stream = tokio_rustls::TlsConnector::from(client_config)
        .connect(domain, stream)
        .await?;

    debug!("connected");

    if let Ok(mut save_path) = tokio::fs::File::create(&temp_file).await {
        let _ = tokio::io::copy(&mut stream, &mut save_path).await;
        let _ = stream.flush().await;
        let _ = stream.shutdown().await;
    }

    info!("successfully received payload");

    Ok(())
}
