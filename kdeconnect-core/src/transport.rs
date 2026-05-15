use std::{
    collections::HashMap,
    net::{IpAddr, Ipv4Addr, SocketAddr, SocketAddrV4},
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};

use rustls::pki_types::ServerName;
use socket2::TcpKeepalive;
use tokio::{
    io::{
        AsyncBufReadExt as _, AsyncRead, AsyncReadExt as _, AsyncWrite, AsyncWriteExt, BufReader,
        split,
    },
    net::{TcpListener, TcpStream, UdpSocket},
    sync::{Mutex, mpsc, watch},
    time::MissedTickBehavior,
};
use tokio_rustls::{TlsAcceptor, TlsConnector};
use tracing::{debug, error, info, warn};

use crate::{
    GLOBAL_CONFIG,
    device::{Device, DeviceId, PairState},
    plugin_config,
    protocol::{Identity, PacketType, ProtocolPacket},
};

pub const DEFAULT_DISCOVERY_INTERVAL: Duration = Duration::from_secs(60);

/// Application-level keepalive interval — sends a bare newline to keep the
/// TCP connection from going idle long enough to trigger OS keepalive probes
/// (which Android Doze often ignores, causing spurious disconnects).
/// Bare newlines are silently skipped by all KDE Connect readers and never
/// surfaced to users.
const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(15);
const TLS_SHUTDOWN_TIMEOUT: Duration = Duration::from_millis(250);
const MAX_IDENTITY_PACKET_SIZE: usize = 8192;
const MIN_DISCOVERY_CONNECTION_INTERVAL: Duration = Duration::from_millis(500);
const IDENTITY_BURST_COUNT: usize = 2;
const IDENTITY_BURST_DELAY: Duration = Duration::from_millis(250);

pub const DEFAULT_LISTEN_PORT: u16 = 1716;
pub const DEFAULT_LISTEN_ADDR: SocketAddr = SocketAddr::V4(SocketAddrV4::new(
    Ipv4Addr::UNSPECIFIED,
    DEFAULT_LISTEN_PORT,
));
pub const BROADCAST_ADDR: SocketAddr =
    SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::BROADCAST, DEFAULT_LISTEN_PORT));

/// Monotonically increasing counter — each accepted/initiated connection gets
/// a unique ID so that `Disconnected` events can be matched to the exact
/// connection that generated them, preventing stale disconnects from
/// incorrectly wiping a newer live connection out of `writer_map`.
static CONN_COUNTER: AtomicU64 = AtomicU64::new(0);

#[derive(Default)]
pub(crate) struct ConnectionRateLimiter {
    by_device: Mutex<HashMap<DeviceId, Instant>>,
}

impl ConnectionRateLimiter {
    pub(crate) async fn allow_device_connection(&self, id: &DeviceId) -> bool {
        let now = Instant::now();
        let mut guard = self.by_device.lock().await;
        if let Some(last_seen) = guard.get(id)
            && now.duration_since(*last_seen) < MIN_DISCOVERY_CONNECTION_INTERVAL
        {
            return false;
        }
        guard.insert(id.clone(), now);
        true
    }
}

#[derive(Debug)]
pub enum TransportEvent {
    IncomingPacket {
        addr: SocketAddr,
        id: DeviceId,
        raw: String,
        conn_id: u64,
    },
    NewConnection {
        addr: SocketAddr,
        id: DeviceId,
        name: String,
        device_type: String,
        incoming_capabilities: Vec<String>,
        outgoing_capabilities: Vec<String>,
        /// Protocol version announced by the peer in its identity packet.
        protocol_version: usize,
        /// Pairing timestamp from the peer's identity (used for clock-sync validation).
        pairing_timestamp: u64,
        /// DER-encoded TLS certificate presented by the peer.
        peer_certificate: Vec<u8>,
        write_tx: mpsc::UnboundedSender<ProtocolPacket>,
        shutdown_tx: watch::Sender<bool>,
        /// Unique ID for this connection instance.
        conn_id: u64,
    },
    /// A paired device presented a different TLS certificate than the one
    /// pinned at pairing time.
    PairTrustFailed { id: DeviceId },
    /// A packet had already been accepted into the write queue, but the socket
    /// failed while the writer task was sending it.
    PacketSendFailed {
        id: DeviceId,
        packet_type: PacketType,
        conn_id: u64,
    },
    /// Emitted when the reader loop ends (peer closed / broken pipe).
    /// `conn_id` must match the stored value for this device before core
    /// removes the writer_map entry; a mismatch means a newer connection
    /// has already replaced this one.
    Disconnected { id: DeviceId, conn_id: u64 },
}

/// Enable TCP keepalive so the OS detects a dead connection within ~25s
/// (10s idle + 3 × 5s probes) rather than waiting indefinitely.
fn apply_keepalive(stream: &TcpStream) {
    let keepalive = TcpKeepalive::new()
        .with_time(Duration::from_secs(10))
        .with_interval(Duration::from_secs(5))
        .with_retries(3);
    let sock_ref = socket2::SockRef::from(stream);
    if let Err(e) = sock_ref.set_tcp_keepalive(&keepalive) {
        warn!("Failed to set TCP keepalive: {}", e);
    }
}

fn is_private_kdeconnect_addr(addr: IpAddr) -> bool {
    match addr {
        IpAddr::V4(addr) => {
            let octets = addr.octets();
            let is_cgnat = octets[0] == 100 && (octets[1] & 0b1100_0000) == 0b0100_0000;
            addr.is_private() || addr.is_link_local() || addr.is_loopback() || is_cgnat
        }
        IpAddr::V6(addr) => {
            addr.is_unique_local() || addr.is_unicast_link_local() || addr.is_loopback()
        }
    }
}

async fn read_line_bounded<R: AsyncRead + Unpin>(
    reader: &mut R,
    max_len: usize,
) -> anyhow::Result<String> {
    let mut raw = Vec::new();
    let mut byte = [0u8; 1];
    loop {
        match reader.read(&mut byte).await? {
            0 => return Err(anyhow::anyhow!("EOF reading identity")),
            1 => {
                raw.push(byte[0]);
                if byte[0] == b'\n' {
                    break;
                }
                if raw.len() > max_len {
                    return Err(anyhow::anyhow!("identity line too long"));
                }
            }
            _ => unreachable!(),
        }
    }
    Ok(String::from_utf8_lossy(&raw).into_owned())
}

fn validate_identity_target(
    peer_identity: &Identity,
    local_identity: &Identity,
) -> anyhow::Result<()> {
    if let Some(target_device_id) = peer_identity.target_device_id.as_deref()
        && target_device_id != local_identity.device_id
    {
        return Err(anyhow::anyhow!(
            "received connection request for different target device {}",
            target_device_id
        ));
    }

    if let Some(target_protocol_version) = peer_identity.target_protocol_version
        && target_protocol_version != local_identity.protocol_version
    {
        return Err(anyhow::anyhow!(
            "received connection request for protocol {}, local protocol is {}",
            target_protocol_version,
            local_identity.protocol_version
        ));
    }

    Ok(())
}

fn targeted_identity(
    base: &Identity,
    target_device_id: &str,
    target_protocol_version: usize,
) -> Identity {
    let mut identity = base.clone();
    identity.target_device_id = Some(target_device_id.to_string());
    identity.target_protocol_version = Some(target_protocol_version);
    identity
}

async fn write_identity_packet<W: AsyncWrite + Unpin>(
    writer: &mut W,
    identity: &Identity,
) -> anyhow::Result<()> {
    let raw =
        ProtocolPacket::new(PacketType::Identity, serde_json::to_value(identity)?).as_raw()?;
    writer.write_all(&raw).await?;
    writer.flush().await?;
    Ok(())
}

async fn exchange_secure_identity<S>(
    stream: &mut S,
    expected_device_id: &str,
    expected_protocol_version: usize,
    local_identity: &Identity,
) -> anyhow::Result<Identity>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    write_identity_packet(stream, local_identity).await?;

    let line = tokio::time::timeout(
        Duration::from_secs(5),
        read_line_bounded(stream, MAX_IDENTITY_PACKET_SIZE),
    )
    .await??;
    let packet = serde_json::from_str::<ProtocolPacket>(&line)?;
    if !matches!(packet.packet_type, PacketType::Identity) {
        return Err(anyhow::anyhow!(
            "expected secure identity packet, received {:?}",
            packet.packet_type
        ));
    }

    let identity = serde_json::from_value::<Identity>(packet.body)?;
    crate::device::validate_device_id(&identity.device_id)?;
    if identity.device_id != expected_device_id {
        return Err(anyhow::anyhow!(
            "device ID changed during TLS handshake: {} -> {}",
            expected_device_id,
            identity.device_id
        ));
    }
    if identity.protocol_version != expected_protocol_version {
        return Err(anyhow::anyhow!(
            "protocol version changed during TLS handshake: {} -> {}",
            expected_protocol_version,
            identity.protocol_version
        ));
    }

    Ok(identity)
}

pub struct TcpTransport {
    listen_addr: SocketAddr,
    event_tx: mpsc::UnboundedSender<TransportEvent>,
    identity: Arc<Identity>,
    client_config: Arc<rustls::ClientConfig>,
    server_config: Arc<rustls::ServerConfig>,
    rate_limiter: Arc<ConnectionRateLimiter>,
}

impl TcpTransport {
    pub fn new(
        event_tx: &mpsc::UnboundedSender<TransportEvent>,
        rate_limiter: Arc<ConnectionRateLimiter>,
    ) -> Self {
        let config = GLOBAL_CONFIG.get().unwrap();
        let listen_addr = config.listen_addr;
        let event_tx = event_tx.clone();
        let identity = Arc::new(config.identity.clone());
        let client_config = config.key_store.client_config.clone();
        let server_config = config.key_store.server_config.clone();

        Self {
            listen_addr,
            event_tx,
            identity,
            client_config,
            server_config,
            rate_limiter,
        }
    }

    pub async fn listen(&self) -> anyhow::Result<()> {
        use socket2::{Domain, Protocol, Socket, Type};

        // SO_REUSEADDR prevents "address already in use" when the service
        // restarts before the OS has released the port from the previous instance.
        let socket = Socket::new(Domain::IPV4, Type::STREAM, Some(Protocol::TCP))?;
        socket.set_reuse_address(true)?;
        socket.set_nonblocking(true)?;
        socket.bind(&self.listen_addr.into())?;
        socket.listen(128)?;
        let listener = TcpListener::from_std(std::net::TcpListener::from(socket))?;
        info!("TCP listener bound to {}", self.listen_addr);

        loop {
            match listener.accept().await {
                Ok((stream, peer)) => {
                    info!(peer = ?peer, "[tcp] new connection");
                    apply_keepalive(&stream);

                    let event_tx = self.event_tx.clone();
                    let identity = self.identity.clone();
                    let client_config = self.client_config.clone();
                    let server_config = self.server_config.clone();
                    let rate_limiter = self.rate_limiter.clone();

                    tokio::spawn(async move {
                        if let Err(e) = handle_incoming_tcp(
                            stream,
                            peer,
                            event_tx,
                            identity,
                            client_config,
                            server_config,
                            rate_limiter,
                        )
                        .await
                        {
                            warn!(peer = ?peer, "[tcp] connection handler failed: {}", e);
                        }
                    });
                }
                Err(e) => {
                    warn!("[tcp] accept error: {}", e);
                }
            }
        }
    }
}

async fn handle_incoming_tcp(
    mut stream: TcpStream,
    peer: SocketAddr,
    event_tx: mpsc::UnboundedSender<TransportEvent>,
    identity: Arc<Identity>,
    client_config: Arc<rustls::ClientConfig>,
    server_config: Arc<rustls::ServerConfig>,
    rate_limiter: Arc<ConnectionRateLimiter>,
) -> anyhow::Result<()> {
    if !is_private_kdeconnect_addr(peer.ip()) {
        return Err(anyhow::anyhow!(
            "discarding TCP connection from non-local address {}",
            peer.ip()
        ));
    }

    // Step 1: read phone's pre-TLS identity.
    // Read byte-by-byte to avoid BufReader consuming TLS ClientHello
    // bytes into its internal buffer, which would cause "tls handshake eof".
    debug!(peer = ?peer, "[tcp] step 1: reading pre-TLS identity");
    let buffer = tokio::time::timeout(
        Duration::from_secs(5),
        read_line_bounded(&mut stream, MAX_IDENTITY_PACKET_SIZE),
    )
    .await??;
    debug!(peer = ?peer, "[tcp] step 1: read {} bytes", buffer.len());

    let packet = serde_json::from_str::<ProtocolPacket>(&buffer)?;
    if !matches!(packet.packet_type, PacketType::Identity) {
        return Err(anyhow::anyhow!(
            "expected pre-TLS identity packet, received {:?}",
            packet.packet_type
        ));
    }
    let mut peer_identity = match serde_json::from_value::<Identity>(packet.body) {
        Ok(i) => i,
        Err(e) => {
            // Complete the TLS handshake gracefully so the phone doesn't see
            // an abrupt TCP drop and retry aggressively causing connection churn.
            tokio::spawn(async move {
                if let Ok(mut tls_stream) = TlsAcceptor::from(server_config).accept(stream).await {
                    let _ = tls_stream.shutdown().await;
                }
            });
            return Err(e.into());
        }
    };
    validate_identity_target(&peer_identity, &identity)?;

    let name = peer_identity.device_name.clone();
    let id = peer_identity.device_id.clone();
    let peer_protocol_version = peer_identity.protocol_version;
    info!(peer = ?peer, device_id = ?id, device_name = name, "[tcp] identified peer");

    if identity.device_id == peer_identity.device_id {
        return Err(anyhow::anyhow!("skipping the same device"));
    }

    // Validate device ID and sanitize name early, before TLS.
    crate::device::validate_device_id(&id)?;
    let device_id = DeviceId(id.clone());
    if !rate_limiter.allow_device_connection(&device_id).await {
        debug!(
            peer = ?peer,
            device_id = ?device_id,
            "[tcp] duplicate connection attempt suppressed"
        );
        return Ok(());
    }
    let name = crate::device::sanitize_device_name(&name);

    // Step 2: TLS handshake. KDE Connect's LAN protocol makes the
    // side accepting the TCP connection act as the TLS client.
    debug!(peer = ?peer, "[tcp] step 2: starting TLS connect (we are client)");
    let server_name = ServerName::try_from(id.as_str())?.to_owned();
    let mut tls_stream = tokio::time::timeout(
        Duration::from_secs(10),
        TlsConnector::from(client_config).connect(server_name, stream),
    )
    .await??;

    info!(peer = ?peer, device_id = ?id, "[tcp] step 2: TLS established");
    let peer_certificate = peer_certificate_der(tls_stream.get_ref().1.peer_certificates());
    if paired_certificate_mismatch(
        &DeviceId(id.clone()),
        &name,
        peer_identity.device_type.to_string(),
        peer_identity.incoming_capabilities.clone(),
        peer_identity.outgoing_capabilities.clone(),
        peer,
        &peer_certificate,
    )
    .await
    {
        let _ = event_tx.send(TransportEvent::PairTrustFailed {
            id: DeviceId(id.clone()),
        });
        let _ = tls_stream.shutdown().await;
        return Err(anyhow::anyhow!(
            "paired device presented a different TLS certificate"
        ));
    }

    if peer_protocol_version >= 8 {
        // Protocol v8 requires replacing the cleartext identity with the
        // identity exchanged inside TLS before trusting capabilities.
        debug!(peer = ?peer, "[tcp] step 3: exchanging secure identity");
        let filtered = filtered_identity_for_device(&id).await;
        peer_identity =
            exchange_secure_identity(&mut tls_stream, &id, peer_protocol_version, &filtered)
                .await?;
        debug!(peer = ?peer, "[tcp] step 3: secure identity exchanged");
    }

    let name = crate::device::sanitize_device_name(&peer_identity.device_name);

    let (reader, writer) = split(tls_stream);

    let (write_tx, write_rx) = mpsc::unbounded_channel::<ProtocolPacket>();
    let write_rx = Arc::new(Mutex::new(write_rx));
    let (shutdown_tx, shutdown_rx) = watch::channel(false);

    let conn_id = CONN_COUNTER.fetch_add(1, Ordering::Relaxed);

    if let Err(e) = event_tx.send(TransportEvent::NewConnection {
        addr: peer,
        id: DeviceId(peer_identity.device_id),
        name: name.clone(),
        device_type: peer_identity.device_type.to_string(),
        incoming_capabilities: peer_identity.incoming_capabilities,
        outgoing_capabilities: peer_identity.outgoing_capabilities,
        protocol_version: peer_identity.protocol_version,
        pairing_timestamp: 0,
        peer_certificate,
        write_tx,
        shutdown_tx,
        conn_id,
    }) {
        return Err(anyhow::anyhow!("transport event channel closed: {}", e));
    }

    tokio::spawn(handle_connection(
        event_tx.clone(),
        reader,
        writer,
        write_rx,
        peer,
        DeviceId(id.clone()),
        conn_id,
        shutdown_rx,
    ));

    Ok(())
}

pub struct UdpTransport {
    socket: Arc<UdpSocket>,
    discovery_interval: Duration,
    #[allow(dead_code)]
    event_tx: mpsc::UnboundedSender<TransportEvent>,
    identity: Arc<Identity>,
    server_config: Arc<rustls::ServerConfig>,
    broadcast_handle: Arc<Mutex<Option<tokio::task::JoinHandle<()>>>>,
    rate_limiter: Arc<ConnectionRateLimiter>,
}

impl UdpTransport {
    pub async fn new(
        event_tx: &mpsc::UnboundedSender<TransportEvent>,
        rate_limiter: Arc<ConnectionRateLimiter>,
    ) -> Self {
        let config = GLOBAL_CONFIG.get().unwrap();

        let socket = {
            let mut attempts = 0u32;
            loop {
                match UdpSocket::bind(config.listen_addr).await {
                    Ok(s) => break s,
                    Err(e) => {
                        attempts += 1;
                        if attempts >= 10 {
                            // Port still held — another instance is almost certainly
                            // running. Exit cleanly so the caller (the real owner) is
                            // not disrupted, rather than panicking into the journal.
                            tracing::error!(
                                "UDP port {} still in use after {} attempts — \
                                 another instance may be running, exiting: {}",
                                config.listen_addr.port(),
                                attempts,
                                e
                            );
                            std::process::exit(1);
                        }
                        tracing::warn!(
                            "UDP bind failed (attempt {}), retrying in 1s: {}",
                            attempts,
                            e
                        );
                        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                    }
                }
            }
        };
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
            broadcast_handle: Arc::new(Mutex::new(None)),
            rate_limiter,
        }
    }

    pub async fn send_identity(&self) -> anyhow::Result<()> {
        if std::env::var("KDECONNECT_DISABLE_UDP_BROADCAST").is_ok() {
            warn!("UDP broadcast disabled by environment variable");
            return Ok(());
        }

        // Cancel any previously-spawned broadcast task so we never accumulate
        // orphaned broadcast loops.
        {
            let mut guard = self.broadcast_handle.lock().await;
            if let Some(handle) = guard.take() {
                handle.abort();
            }
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

        let handle = tokio::spawn(async move {
            // Send a short burst immediately for explicit scans. UDP broadcast is
            // intentionally lossy on many Wi-Fi networks, so a single packet can
            // be missed even when discovery should work.
            for _ in 0..IDENTITY_BURST_COUNT {
                match udp_socket.send_to(packet.as_slice(), BROADCAST_ADDR).await {
                    Ok(size) => {
                        debug!(addr = ?BROADCAST_ADDR, packet.size = size, "Sending udp broadcast")
                    }
                    Err(e) => warn!(addr = ?BROADCAST_ADDR, "Failed to send UDP packet: {}", e),
                }
                tokio::time::sleep(IDENTITY_BURST_DELAY).await;
            }

            let mut interval = tokio::time::interval(Duration::from_secs(interval.as_secs()));
            interval.set_missed_tick_behavior(MissedTickBehavior::Skip);

            loop {
                interval.tick().await;
                match udp_socket.send_to(packet.as_slice(), BROADCAST_ADDR).await {
                    Ok(size) => {
                        debug!(addr = ?BROADCAST_ADDR, packet.size = size, "Sending udp broadcast")
                    }
                    Err(e) => warn!(addr = ?BROADCAST_ADDR, "Failed to send UDP packet: {}", e),
                }
            }
        });

        *self.broadcast_handle.lock().await = Some(handle);

        Ok(())
    }

    pub async fn listen(&self) -> anyhow::Result<()> {
        let mut buf = vec![0u8; MAX_IDENTITY_PACKET_SIZE];

        loop {
            match self.socket.recv_from(&mut buf).await {
                Ok((len, peer)) => {
                    if !is_private_kdeconnect_addr(peer.ip()) {
                        debug!(
                            peer = ?peer,
                            "[udp] discarding identity from non-local address"
                        );
                        continue;
                    }

                    let raw = &buf[..len];
                    let packet = match ProtocolPacket::from_raw(raw) {
                        Ok(p) => p,
                        Err(e) => {
                            warn!("[udp] Failed to parse UDP packet: {}", e);
                            continue;
                        }
                    };
                    if !matches!(packet.packet_type, PacketType::Identity) {
                        debug!(
                            "[udp] ignoring non-identity packet {:?}",
                            packet.packet_type
                        );
                        continue;
                    }

                    let peer_identity = match serde_json::from_value::<Identity>(packet.body) {
                        Ok(i) => i,
                        Err(e) => {
                            warn!("[udp] failed to parse identity body: {}", e);
                            continue;
                        }
                    };

                    if self.identity.device_id == peer_identity.device_id {
                        continue;
                    }

                    let event_tx = self.event_tx.clone();
                    let this_identity = self.identity.clone();
                    let server_config = self.server_config.clone();
                    let rate_limiter = self.rate_limiter.clone();

                    tokio::spawn(async move {
                        if let Err(e) = handle_discovered_device(
                            peer,
                            peer_identity,
                            event_tx,
                            this_identity,
                            server_config,
                            rate_limiter,
                        )
                        .await
                        {
                            warn!(peer = ?peer, "[udp] failed to handle discovery: {}", e);
                        }
                    });
                }
                Err(e) => {
                    warn!("[udp] recv_from error: {}", e);
                }
            }
        }
    }
}

async fn handle_discovered_device(
    mut peer: SocketAddr,
    mut peer_identity: Identity,
    event_tx: mpsc::UnboundedSender<TransportEvent>,
    this_identity: Arc<Identity>,
    server_config: Arc<rustls::ServerConfig>,
    rate_limiter: Arc<ConnectionRateLimiter>,
) -> anyhow::Result<()> {
    if !is_private_kdeconnect_addr(peer.ip()) {
        return Err(anyhow::anyhow!(
            "discarding UDP discovery from non-local address {}",
            peer.ip()
        ));
    }

    let id = DeviceId(peer_identity.device_id.clone());
    let name = peer_identity.device_name.clone();
    let peer_protocol_version = peer_identity.protocol_version;

    // Validate device ID and sanitize name early.
    crate::device::validate_device_id(&id.0)?;
    if !rate_limiter.allow_device_connection(&id).await {
        debug!(
            peer = ?peer,
            device_id = ?id,
            "[udp] duplicate discovery connection suppressed"
        );
        return Ok(());
    }
    let name = crate::device::sanitize_device_name(&name);

    if let Some(new_port) = peer_identity.tcp_port {
        peer.set_port(new_port);
        info!(peer = ?peer, device_id = ?id, device_name = name, "Device supports TCP");
    }

    let mut stream =
        tokio::time::timeout(Duration::from_secs(5), TcpStream::connect(peer)).await??;
    apply_keepalive(&stream);

    // Send identity pre-TLS — phone reads this before initiating TLS.
    let pre_tls_identity = targeted_identity(&this_identity, &id.0, peer_protocol_version);
    let pre_tls_identity = ProtocolPacket::new(
        PacketType::Identity,
        serde_json::to_value(&pre_tls_identity)?,
    )
    .as_raw()?;
    stream.write_all(pre_tls_identity.as_slice()).await?;
    stream.flush().await?;

    let acceptor = TlsAcceptor::from(server_config);

    let mut tls_stream =
        tokio::time::timeout(Duration::from_secs(10), acceptor.accept(stream)).await??;

    info!(peer = ?peer, device_id = ?id, device_name = name, "[udp] Established TLS connection");
    let peer_certificate = peer_certificate_der(tls_stream.get_ref().1.peer_certificates());
    if paired_certificate_mismatch(
        &id,
        &name,
        peer_identity.device_type.to_string(),
        peer_identity.incoming_capabilities.clone(),
        peer_identity.outgoing_capabilities.clone(),
        peer,
        &peer_certificate,
    )
    .await
    {
        let _ = event_tx.send(TransportEvent::PairTrustFailed { id });
        let _ = tls_stream.shutdown().await;
        return Err(anyhow::anyhow!(
            "paired device presented a different TLS certificate"
        ));
    }

    if peer_protocol_version >= 8 {
        let filtered = filtered_identity_for_device(&id.0).await;
        peer_identity =
            exchange_secure_identity(&mut tls_stream, &id.0, peer_protocol_version, &filtered)
                .await?;
    }

    let name = crate::device::sanitize_device_name(&peer_identity.device_name);

    let (reader, writer) = split(tls_stream);

    let (write_tx, write_rx) = mpsc::unbounded_channel::<ProtocolPacket>();
    let write_rx = Arc::new(Mutex::new(write_rx));
    let (shutdown_tx, shutdown_rx) = watch::channel(false);

    let conn_id = CONN_COUNTER.fetch_add(1, Ordering::Relaxed);

    if let Err(e) = event_tx.send(TransportEvent::NewConnection {
        addr: peer,
        id: DeviceId(peer_identity.device_id),
        name: name.clone(),
        device_type: peer_identity.device_type.to_string(),
        incoming_capabilities: peer_identity.incoming_capabilities,
        outgoing_capabilities: peer_identity.outgoing_capabilities,
        protocol_version: peer_identity.protocol_version,
        pairing_timestamp: 0,
        peer_certificate,
        write_tx,
        shutdown_tx,
        conn_id,
    }) {
        return Err(anyhow::anyhow!("transport event channel closed: {}", e));
    }

    tokio::spawn(handle_connection(
        event_tx.clone(),
        reader,
        writer,
        write_rx,
        peer,
        id.clone(),
        conn_id,
        shutdown_rx,
    ));

    Ok(())
}

fn peer_certificate_der(certs: Option<&[rustls::pki_types::CertificateDer<'static>]>) -> Vec<u8> {
    certs
        .and_then(|certs| certs.first())
        .map(|cert| cert.as_ref().to_vec())
        .unwrap_or_default()
}

async fn paired_certificate_mismatch(
    id: &DeviceId,
    name: &str,
    device_type: String,
    incoming_capabilities: Vec<String>,
    outgoing_capabilities: Vec<String>,
    addr: SocketAddr,
    peer_certificate: &[u8],
) -> bool {
    if peer_certificate.is_empty() {
        return false;
    }

    let Ok(device) = Device::new(
        id.0.clone(),
        name.to_string(),
        device_type,
        incoming_capabilities,
        outgoing_capabilities,
        addr,
    )
    .await
    else {
        return false;
    };

    let mismatch = device.pair_state == PairState::Paired
        && !device.remote_certificate.is_empty()
        && device.remote_certificate != peer_certificate;

    if mismatch {
        let _ = device.update_pair_state(PairState::NotPaired).await;
    }

    mismatch
}

async fn handle_connection<R, W>(
    event_tx: mpsc::UnboundedSender<TransportEvent>,
    reader: R,
    mut writer: W,
    write_rx: Arc<Mutex<mpsc::UnboundedReceiver<ProtocolPacket>>>,
    peer: SocketAddr,
    id: DeviceId,
    conn_id: u64,
    shutdown_rx: watch::Receiver<bool>,
) where
    R: tokio::io::AsyncRead + Unpin + Send + 'static,
    W: tokio::io::AsyncWrite + Unpin + Send + 'static,
{
    let mut reader = BufReader::new(reader);
    let mut buffer = String::new();

    // Reader task — forwards packets and emits Disconnected when the connection ends.
    let event_tx_reader = event_tx.clone();
    let id_reader = id.clone();
    let mut shutdown_rx_reader = shutdown_rx.clone();
    tokio::spawn(async move {
        loop {
            let read_result = tokio::select! {
                _ = shutdown_rx_reader.changed() => {
                    debug!(peer = ?peer, "[reader loop] shutting down superseded connection");
                    break;
                }
                result = reader.read_line(&mut buffer) => result,
            };

            match read_result {
                Ok(0) => {
                    debug!(peer = ?peer, "[reader loop] connection closed");
                    break;
                }
                Ok(_) => {
                    let trimmed = buffer.trim();
                    if trimmed.is_empty() {
                        buffer.clear();
                        continue;
                    }

                    // we should not print raw packets since they might expose
                    // sensitive data eg. from sms
                    // eprintln!("[reader] raw bytes from {}: {:?}", peer, trimmed);

                    if let Err(e) = event_tx_reader.send(TransportEvent::IncomingPacket {
                        addr: peer,
                        id: id_reader.clone(),
                        raw: trimmed.to_string(),
                        conn_id,
                    }) {
                        error!(peer = ?peer, "[reader loop] transport event channel closed: {}", e);
                        break;
                    }
                }
                Err(e) => {
                    if matches!(
                        e.kind(),
                        std::io::ErrorKind::ConnectionReset
                            | std::io::ErrorKind::UnexpectedEof
                            | std::io::ErrorKind::BrokenPipe
                    ) {
                        debug!(peer = ?peer, "[reader loop] connection ended: {}", e);
                    } else {
                        error!(peer = ?peer, "[reader loop] error reading: {}", e);
                    }
                    break;
                }
            }
            buffer.clear();
        }
        debug!(peer = ?peer, "reader loop ended");
        // Notify core the connection is dead. conn_id lets core distinguish this
        // disconnect from a stale event belonging to a previous connection.
        let _ = event_tx_reader.send(TransportEvent::Disconnected {
            id: id_reader,
            conn_id,
        });
    });

    // Writer task — drains the write channel and sends periodic bare-newline
    // keepalives to prevent the TCP connection from going idle.
    let event_tx_writer = event_tx.clone();
    let id_writer = id.clone();
    let mut shutdown_rx_writer = shutdown_rx;
    tokio::spawn(async move {
        let mut close_gracefully = true;
        loop {
            let msg = tokio::select! {
                _ = shutdown_rx_writer.changed() => {
                    debug!(peer = ?peer, "writer shutting down superseded connection");
                    close_gracefully = false;
                    break;
                }
                msg = async {
                    let mut rx = write_rx.lock().await;
                    rx.recv().await
                } => msg,
                _ = tokio::time::sleep(HEARTBEAT_INTERVAL) => {
                    if let Err(e) = writer.write_all(b"\n").await {
                        error!(peer = ?peer, "Error writing heartbeat: {}", e);
                        close_gracefully = false;
                        break;
                    }
                    if let Err(e) = writer.flush().await {
                        error!(peer = ?peer, "Error flushing heartbeat: {}", e);
                        close_gracefully = false;
                        break;
                    }
                    continue;
                }
            };
            match msg {
                Some(msg) => {
                    debug!(peer = ?peer, packet_type = ?msg.packet_type, "writing");
                    let packet_type = msg.packet_type.clone();

                    if let Err(e) = writer.write_all(&msg.as_raw().unwrap()).await {
                        error!(peer = ?peer, "Error writing: {}", e);
                        close_gracefully = false;
                        let _ = event_tx_writer.send(TransportEvent::PacketSendFailed {
                            id: id_writer.clone(),
                            packet_type,
                            conn_id,
                        });
                        break;
                    }
                    if let Err(e) = writer.flush().await {
                        error!(peer = ?peer, "Error flushing: {}", e);
                        close_gracefully = false;
                        let _ = event_tx_writer.send(TransportEvent::PacketSendFailed {
                            id: id_writer.clone(),
                            packet_type,
                            conn_id,
                        });
                        break;
                    }
                }
                None => break,
            }
        }
        if close_gracefully {
            let _ = tokio::time::timeout(TLS_SHUTDOWN_TIMEOUT, writer.shutdown()).await;
        }
        let _ = event_tx_writer.send(TransportEvent::Disconnected {
            id: id_writer,
            conn_id,
        });
        info!(peer = ?peer, "writer task ended");
    });
}

/// Build a filtered identity for a specific device, removing capabilities
/// for plugins that have been disabled for that device. Called at handshake
/// time so the phone receives accurate capabilities on every connection.
async fn filtered_identity_for_device(device_id: &str) -> Identity {
    let base = &GLOBAL_CONFIG.get().unwrap().identity;
    let disabled = plugin_config::load_disabled_plugins(device_id).await;

    if disabled.is_empty() {
        return base.clone();
    }

    // Map plugin IDs to the capability strings they own.
    // (incoming_caps, outgoing_caps)
    let cap_map: &[(&str, &[&str], &[&str])] = &[
        (
            "battery",
            &["kdeconnect.battery", "kdeconnect.battery.request"],
            &["kdeconnect.battery", "kdeconnect.battery.request"],
        ),
        (
            "clipboard",
            &["kdeconnect.clipboard", "kdeconnect.clipboard.connect"],
            &["kdeconnect.clipboard", "kdeconnect.clipboard.connect"],
        ),
        (
            "connectivity_report",
            &["kdeconnect.connectivity_report"],
            &["kdeconnect.connectivity_report.request"],
        ),
        (
            "contacts",
            &[
                "kdeconnect.contacts.response_uids_timestamps",
                "kdeconnect.contacts.response_vcards",
            ],
            &[
                "kdeconnect.contacts.request_all_uids_timestamps",
                "kdeconnect.contacts.request_vcards_by_uid",
            ],
        ),
        (
            "findmyphone",
            &["kdeconnect.findmyphone.request"],
            &["kdeconnect.findmyphone.request"],
        ),
        (
            "mousepad",
            &[
                "kdeconnect.mousepad.echo",
                "kdeconnect.mousepad.keyboardstate",
                "kdeconnect.mousepad.request",
            ],
            &[
                "kdeconnect.mousepad.echo",
                "kdeconnect.mousepad.keyboardstate",
                "kdeconnect.mousepad.request",
            ],
        ),
        (
            "mpris",
            &["kdeconnect.mpris", "kdeconnect.mpris.request"],
            &["kdeconnect.mpris", "kdeconnect.mpris.request"],
        ),
        (
            "notification",
            &["kdeconnect.notification", "kdeconnect.notification.request"],
            &[
                "kdeconnect.notification",
                "kdeconnect.notification.action",
                "kdeconnect.notification.reply",
                "kdeconnect.notification.request",
            ],
        ),
        ("ping", &["kdeconnect.ping"], &["kdeconnect.ping"]),
        ("presenter", &["kdeconnect.presenter"], &[]),
        (
            "runcommand",
            &["kdeconnect.runcommand", "kdeconnect.runcommand.request"],
            &["kdeconnect.runcommand", "kdeconnect.runcommand.request"],
        ),
        (
            "share",
            &["kdeconnect.share.request"],
            &["kdeconnect.share.request"],
        ),
        ("sftp", &["kdeconnect.sftp"], &["kdeconnect.sftp.request"]),
        (
            "sms",
            &["kdeconnect.sms.messages"],
            &[
                "kdeconnect.sms.request",
                "kdeconnect.sms.request_conversations",
                "kdeconnect.sms.request_conversation",
            ],
        ),
        (
            "systemvolume",
            &["kdeconnect.systemvolume.request"],
            &["kdeconnect.systemvolume"],
        ),
        (
            "telephony",
            &["kdeconnect.telephony"],
            &["kdeconnect.telephony.request_mute"],
        ),
    ];

    let mut remove_inc: std::collections::HashSet<&str> = std::collections::HashSet::new();
    let mut remove_out: std::collections::HashSet<&str> = std::collections::HashSet::new();
    for (plugin_id, inc, out) in cap_map {
        if disabled.contains(*plugin_id) {
            remove_inc.extend(inc.iter().copied());
            remove_out.extend(out.iter().copied());
        }
    }

    Identity {
        device_id: base.device_id.clone(),
        device_name: base.device_name.clone(),
        device_type: base.device_type,
        protocol_version: base.protocol_version,
        tcp_port: base.tcp_port,
        target_device_id: None,
        target_protocol_version: None,
        incoming_capabilities: base
            .incoming_capabilities
            .iter()
            .filter(|c| !remove_inc.contains(c.as_str()))
            .cloned()
            .collect(),
        outgoing_capabilities: base
            .outgoing_capabilities
            .iter()
            .filter(|c| !remove_out.contains(c.as_str()))
            .cloned()
            .collect(),
    }
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

    if let Some(parent) = temp_file.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }

    let mut save_path = tokio::fs::File::create(temp_file).await?;
    tokio::io::copy(&mut stream, &mut save_path).await?;
    save_path.flush().await?;
    stream.shutdown().await?;

    info!("successfully received payload");

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{ConnectionRateLimiter, is_private_kdeconnect_addr};
    use crate::device::DeviceId;
    use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

    #[test]
    fn kdeconnect_lan_filter_allows_private_link_local_loopback_and_cgnat() {
        assert!(is_private_kdeconnect_addr(IpAddr::V4(Ipv4Addr::new(
            192, 168, 1, 2
        ))));
        assert!(is_private_kdeconnect_addr(IpAddr::V4(Ipv4Addr::new(
            169, 254, 1, 2
        ))));
        assert!(is_private_kdeconnect_addr(IpAddr::V4(Ipv4Addr::LOCALHOST)));
        assert!(is_private_kdeconnect_addr(IpAddr::V4(Ipv4Addr::new(
            100, 64, 1, 2
        ))));
        assert!(is_private_kdeconnect_addr(IpAddr::V6(Ipv6Addr::LOCALHOST)));
        assert!(is_private_kdeconnect_addr(IpAddr::V6(
            "fc00::1".parse().unwrap()
        )));
    }

    #[test]
    fn kdeconnect_lan_filter_rejects_public_internet_addresses() {
        assert!(!is_private_kdeconnect_addr(IpAddr::V4(Ipv4Addr::new(
            8, 8, 8, 8
        ))));
        assert!(!is_private_kdeconnect_addr(IpAddr::V6(
            "2001:4860:4860::8888".parse().unwrap()
        )));
    }

    #[tokio::test]
    async fn rate_limiter_suppresses_immediate_duplicate_device_connections() {
        let limiter = ConnectionRateLimiter::default();
        let id = DeviceId("duplicate-device-000000000000000".to_string());

        assert!(limiter.allow_device_connection(&id).await);
        assert!(!limiter.allow_device_connection(&id).await);
    }
}
