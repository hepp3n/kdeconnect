use std::{
    net::{Ipv4Addr, SocketAddr, SocketAddrV4},
    path::PathBuf,
    sync::Arc,
    time::Duration,
};

use rustls::pki_types::ServerName;
use socket2::TcpKeepalive;
use tokio::{
    io::{AsyncBufReadExt as _, AsyncWriteExt, BufReader, split},
    net::{TcpListener, TcpStream, UdpSocket},
    sync::{Mutex, mpsc},
    time::MissedTickBehavior,
};
use tokio_rustls::TlsAcceptor;
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
    /// Emitted when the reader loop ends (peer closed / broken pipe).
    /// Core removes the dead write_tx so the next NewConnection can replace it.
    Disconnected { id: DeviceId },
}

/// Enable TCP keepalive so the OS detects a dead connection within ~60s
/// (30s idle + 3 × 10s probes) rather than waiting indefinitely.
fn apply_keepalive(stream: &TcpStream) {
    let keepalive = TcpKeepalive::new()
        .with_time(Duration::from_secs(30))
        .with_interval(Duration::from_secs(10))
        .with_retries(3);
    let sock_ref = socket2::SockRef::from(stream);
    if let Err(e) = sock_ref.set_tcp_keepalive(&keepalive) {
        warn!("Failed to set TCP keepalive: {}", e);
    }
}

pub struct TcpTransport {
    listen_addr: SocketAddr,
    event_tx: mpsc::UnboundedSender<TransportEvent>,
    identity: Arc<Identity>,
    server_config: Arc<rustls::ServerConfig>,
}

impl TcpTransport {
    pub fn new(event_tx: &mpsc::UnboundedSender<TransportEvent>) -> Self {
        let config = GLOBAL_CONFIG.get().unwrap();
        let listen_addr = config.listen_addr;
        let event_tx = event_tx.clone();
        let identity = Arc::new(config.identity.clone());
        let server_config = config.key_store.server_config.clone();

        Self {
            listen_addr,
            event_tx,
            identity,
            server_config,
        }
    }

    pub async fn listen(&self) -> anyhow::Result<()> {
        let listener = TcpListener::bind(self.listen_addr).await?;

        loop {
            let event_tx = self.event_tx.clone();
            let identity = self.identity.clone();

            match listener.accept().await {
                Ok((mut stream, peer)) => {
                    info!(peer = ?peer, "[tcp] new connection");
                    apply_keepalive(&stream);

                    // Read phone's identity (sent pre-TLS, plaintext)
                    let mut buffer = String::new();
                    {
                        let mut reader = BufReader::new(&mut stream);
                        reader
                            .read_line(&mut buffer)
                            .await
                            .expect("Failed to read identity line");
                    }

                    let identity = identity.clone();

                    if let Ok(packet) = serde_json::from_str::<ProtocolPacket>(&buffer)
                        && let Ok(peer_identity) = serde_json::from_value::<Identity>(packet.body)
                    {
                        let name = peer_identity.device_name.clone();
                        let id = peer_identity.device_id.clone();

                        if identity.device_id == peer_identity.device_id {
                            warn!(peer = ?peer, device_id = ?id, "skipping the same device");
                            continue;
                        }

                        // Send our identity back (pre-TLS, plaintext) — phone expects this
                        let our_identity = ProtocolPacket::new(
                            PacketType::Identity,
                            serde_json::to_value(&*self.identity).unwrap(),
                        )
                        .as_raw()
                        .expect("Failed to serialize identity packet");
                        let _ = stream.write_all(our_identity.as_slice()).await;
                        let _ = stream.flush().await;

                        // Phone connected to us, so we are the TLS server
                        let mut stream = match TlsAcceptor::from(self.server_config.clone())
                            .accept(stream)
                            .await
                        {
                            Ok(s) => s,
                            Err(e) => {
                                warn!(peer = ?peer, "[tcp] TLS accept failed: {}", e);
                                continue;
                            }
                        };

                        // Send our identity again post-TLS for plugin negotiation
                        let post_tls_identity = ProtocolPacket::new(
                            PacketType::Identity,
                            serde_json::to_value(&*self.identity).unwrap(),
                        )
                        .as_raw()
                        .expect("Failed to serialize identity packet");
                        let _ = stream.write_all(post_tls_identity.as_slice()).await;
                        let _ = stream.flush().await;

                        let (reader, writer) = split(stream);

                        let (write_tx, write_rx) = mpsc::unbounded_channel::<ProtocolPacket>();
                        let write_rx = Arc::new(Mutex::new(write_rx));

                        // Do NOT .await the spawn — blocks the accept loop until connection closes.
                        tokio::spawn(handle_connection(
                            event_tx.clone(),
                            reader,
                            writer,
                            write_rx,
                            peer,
                            DeviceId(id.clone()),
                        ));

                        if let Err(e) = event_tx.send(TransportEvent::NewConnection {
                            addr: peer,
                            id: DeviceId(peer_identity.device_id),
                            name: name.clone(),
                            write_tx,
                        }) {
                            error!(peer = ?peer, "[tcp] transport event channel closed: {}", e);
                        }
                    }
                }
                Err(e) => {
                    warn!("[tcp] accept error: {}", e);
                }
            }
        }
    }
}

pub struct UdpTransport {
    socket: Arc<UdpSocket>,
    discovery_interval: Duration,
    #[allow(dead_code)]
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
            warn!("UDP broadcast disabled by environment variable");
            return Ok(());
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
                    Ok(size) => {
                        debug!(addr = ?BROADCAST_ADDR, packet.size = size, "Sending udp broadcast")
                    }
                    Err(e) => warn!(addr = ?BROADCAST_ADDR, "Failed to send UDP packet: {}", e),
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
                    let packet = match ProtocolPacket::from_raw(raw) {
                        Ok(p) => p,
                        Err(e) => {
                            warn!("[udp] Failed to parse UDP packet: {}", e);
                            continue;
                        }
                    };

                    if let Ok(peer_identity) = serde_json::from_value::<Identity>(packet.body) {
                        if this_identity.device_id == peer_identity.device_id {
                            warn!("[udp] skipping the same device");
                            continue;
                        }

                        let id = DeviceId(peer_identity.device_id.clone());
                        let name = peer_identity.device_name.clone();

                        if let Some(new_port) = peer_identity.tcp_port {
                            peer.set_port(new_port);
                            info!(peer = ?peer, device_id = ?id, device_name = name, "Device supports TCP");
                        }

                        let mut stream = match TcpStream::connect(peer).await {
                            Ok(s) => {
                                apply_keepalive(&s);
                                s
                            }
                            Err(e) => {
                                warn!(peer = ?peer, "[udp] TCP connect failed: {}", e);
                                continue;
                            }
                        };

                        // Send identity pre-TLS — phone reads this before initiating TLS.
                        let pre_tls_identity = ProtocolPacket::new(
                            PacketType::Identity,
                            serde_json::to_value(&*self.identity).unwrap(),
                        )
                        .as_raw()
                        .expect("Failed to serialize identity packet");

                        let _ = stream.write_all(pre_tls_identity.as_slice()).await;
                        let _ = stream.flush().await;

                        let acceptor = TlsAcceptor::from(self.server_config.clone());

                        let mut tls_stream = match acceptor.accept(stream).await {
                            Ok(s) => s,
                            Err(e) => {
                                warn!(peer = ?peer, "[udp] TLS handshake failed: {}", e);
                                continue;
                            }
                        };

                        info!(peer = ?peer, device_id = ?id, device_name = name, "[udp] Established TLS connection");

                        // Send identity post-TLS — phone uses this for plugin negotiation.
                        let post_tls_identity = ProtocolPacket::new(
                            PacketType::Identity,
                            serde_json::to_value(&*self.identity).unwrap(),
                        )
                        .as_raw()
                        .expect("Failed to serialize identity packet");

                        let _ = tls_stream.write_all(post_tls_identity.as_slice()).await;
                        let _ = tls_stream.flush().await;

                        let (reader, writer) = split(tls_stream);

                        let (write_tx, write_rx) = mpsc::unbounded_channel::<ProtocolPacket>();
                        let write_rx = Arc::new(Mutex::new(write_rx));

                        // Do NOT .await the spawn — blocks the UDP listen loop.
                        tokio::spawn(handle_connection(
                            event_tx.clone(),
                            reader,
                            writer,
                            write_rx,
                            peer,
                            id.clone(),
                        ));

                        if let Err(e) = event_tx.send(TransportEvent::NewConnection {
                            addr: peer,
                            id: DeviceId(peer_identity.device_id),
                            name: name.clone(),
                            write_tx,
                        }) {
                            error!(peer = ?peer, device_id = ?id, "[udp] transport event channel closed: {}", e);
                        }
                    }
                }
                Err(e) => {
                    warn!("[udp] recv_from error: {}", e);
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

    // Reader task — forwards packets and emits Disconnected when the connection ends.
    let event_tx_reader = event_tx.clone();
    let id_reader = id.clone();
    tokio::spawn(async move {
        loop {
            match reader.read_line(&mut buffer).await {
                Ok(0) => {
                    warn!(peer = ?peer, "[reader loop] connection closed");
                    break;
                }
                Ok(_) => {
                    let trimmed = buffer.trim();
                    if trimmed.is_empty() {
                        buffer.clear();
                        continue;
                    }

                    eprintln!("[reader] raw bytes from {}: {:?}", peer, trimmed);

                    if let Err(e) = event_tx_reader.send(TransportEvent::IncomingPacket {
                        addr: peer,
                        id: id_reader.clone(),
                        raw: trimmed.to_string(),
                    }) {
                        error!(peer = ?peer, "[reader loop] transport event channel closed: {}", e);
                        break;
                    }
                }
                Err(e) => {
                    error!(peer = ?peer, "[reader loop] error reading: {}", e);
                    break;
                }
            }
            buffer.clear();
        }
        warn!(peer = ?peer, "reader loop ended");
        // Notify core the connection is dead so the writer_map entry can be cleared.
        let _ = event_tx_reader.send(TransportEvent::Disconnected { id: id_reader });
    });

    // Writer task — drains the write channel and sends packets to the peer.
    tokio::spawn(async move {
        while let Some(msg) = write_rx.lock().await.recv().await {
            debug!(peer = ?peer, packet_type = ?msg.packet_type, "writing");

            if let Err(e) = writer.write_all(&msg.as_raw().unwrap()).await {
                error!(peer = ?peer, "Error writing: {}", e);
                break;
            }
            if let Err(e) = writer.flush().await {
                error!(peer = ?peer, "Error flushing: {}", e);
                break;
            }
        }
        let _ = writer.shutdown().await;
        info!(peer = ?peer, "writer task ended");
    });
}

pub(crate) async fn prepare_listener_for_payload() -> Result<TcpListener, String> {
    for port in 1739..1769 {
        if let Ok(listener) =
            TcpListener::bind(SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, port)).await
        {
            return Ok(listener);
        }
    }
    Err("no free port for payload, failed.".to_string())
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
