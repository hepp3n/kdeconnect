use std::sync::Arc;
use tokio::{
    io::{AsyncRead, AsyncWriteExt},
    sync::{RwLock, mpsc},
};
use tracing::{debug, info, warn};

use crate::{
    GLOBAL_CONFIG,
    device::Device,
    event::{ConnectionEvent, CoreEvent},
    filetransfer::TransferAdapter,
    plugins::{
        self,
        battery::Battery,
        clipboard::Clipboard,
        connectivity_report::ConnectivityReport,
        mpris::{Mpris, MprisRequest},
    },
    protocol::{PacketPayloadTransferInfo, PacketType, ProtocolPacket},
    transport::prepare_listener_for_payload,
};

pub trait Plugin: Sync + Send {
    fn id(&self) -> &'static str;
}

#[derive(Clone)]
pub struct PluginRegistry {
    plugins: Arc<RwLock<Vec<Arc<dyn Plugin>>>>,
}

impl PluginRegistry {
    pub fn new() -> Self {
        Self {
            plugins: Arc::new(RwLock::new(Vec::new())),
        }
    }

    pub async fn register(&self, plugin: Arc<dyn Plugin>) {
        let mut plugins = self.plugins.write().await;
        info!("Registering plugin: {}", plugin.id());
        plugins.push(plugin);
    }

    pub async fn dispatch(
        &self,
        device: Device,
        packet: ProtocolPacket,
        core_tx: mpsc::UnboundedSender<CoreEvent>,
        tx: mpsc::UnboundedSender<ConnectionEvent>,
        mpris_tx: mpsc::UnboundedSender<ConnectionEvent>,
    ) {
        let body = packet.body.clone();
        let core_tx = core_tx.clone();
        let connection_tx = tx.clone();
        let mpris_connection_tx = mpris_tx.clone();
        let payload_info = packet.payload_transfer_info;

        match packet.packet_type {
            PacketType::Identity => {
                debug!("Skipping identity packet");
            }
            PacketType::Pair => {
                debug!("Skipping pair packet");
            }
            PacketType::Battery => {
                if let Ok(battery) = serde_json::from_value::<Battery>(body) {
                    battery.received_packet(connection_tx).await;
                }
            }
            PacketType::BatteryRequest => {
                debug!("BatteryRequest received — not implemented, ignoring");
            }
            PacketType::SmsMessages => {
                debug!("Received SmsMessages packet");
                if let Ok(sms_messages) =
                    serde_json::from_value::<plugins::sms::SmsMessages>(body.clone())
                {
                    info!(
                        "Received SMS messages packet with {} messages",
                        sms_messages.messages.len()
                    );
                    sms_messages.received_packet(connection_tx).await;
                } else {
                    warn!("Failed to parse SMS messages packet: {:?}", body);
                }
            }
            PacketType::ContactsResponseUidsTimestamps => {
                debug!("Received ContactsResponseUidsTimestamps");
                if let Some(uids_val) = body.get("uids").and_then(|v| v.as_array()) {
                    let uids: Vec<String> = uids_val
                        .iter()
                        .filter_map(|v| v.as_str().map(|s| s.to_string()))
                        .collect();
                    if !uids.is_empty() {
                        debug!("Requesting vcards for {} UIDs", uids.len());
                        let packet = ProtocolPacket::new(
                            PacketType::ContactsRequestVcardsByUid,
                            serde_json::json!({ "uids": uids }),
                        );
                        let _ = core_tx.send(CoreEvent::SendPacket {
                            device: device.device_id.clone(),
                            packet,
                        });
                    }
                }
            }
            PacketType::ContactsResponseVcards => {
                debug!("Received ContactsResponseVcards");
                let mut contacts: std::collections::HashMap<String, String> =
                    std::collections::HashMap::new();
                if let Some(uids_val) = body.get("uids").and_then(|v| v.as_array()) {
                    for uid_val in uids_val {
                        if let Some(uid) = uid_val.as_str() {
                            if let Some(vcard_str) = body.get(uid).and_then(|v| v.as_str()) {
                                let (name_opt, phones) = parse_vcard(vcard_str);
                                if let Some(name) = name_opt {
                                    for phone in phones {
                                        contacts.insert(phone, name.clone());
                                    }
                                }
                            }
                        }
                    }
                }
                debug!("Parsed {} phone->name contact entries", contacts.len());
                if !contacts.is_empty() {
                    let _ = connection_tx.send(ConnectionEvent::ContactsReceived(contacts));
                }
            }
            PacketType::Clipboard => {
                if let Ok(clipboard) = serde_json::from_value::<Clipboard>(body) {
                    clipboard.received_packet(connection_tx).await;
                }
            }
            PacketType::ConnectivityReport => {
                if let Ok(connectivity_rep) = serde_json::from_value::<ConnectivityReport>(body) {
                    connectivity_rep.received_packet(connection_tx).await;
                }
            }
            PacketType::ClipboardConnect => {
                if let Ok(clipboard) = serde_json::from_value::<Clipboard>(body)
                    && let Some(timestamp) = clipboard.timestamp
                {
                    if timestamp > 0 {
                        info!("Clipboard sync requested with timestamp: {}", timestamp);
                        clipboard.received_packet(connection_tx).await;
                    } else {
                        info!("Clipboard sync requested without timestamp. Ignoring");
                    }
                }
            }
            PacketType::MousePadKeyboardState => {
                if let Ok(keyboard_state) =
                    serde_json::from_value::<plugins::mousepad::KeyboardState>(body)
                {
                    debug!("{:?}", keyboard_state);
                }
            }
            PacketType::Mpris => {
                if let Ok(mpris_packet) = serde_json::from_value::<Mpris>(body) {
                    info!("Received MPRIS packet: {:?}", mpris_packet);
                    let mpris_event =
                        ConnectionEvent::Mpris((device.device_id.clone(), mpris_packet));
                    let _ = connection_tx.send(mpris_event.clone());
                    let _ = mpris_connection_tx.send(mpris_event);
                }
            }
            PacketType::MprisRequest => {
                if let Ok(mpris_request) = serde_json::from_value::<MprisRequest>(body) {
                    mpris_request.received_packet(&device, core_tx).await;
                }
            }
            PacketType::Notification => {
                debug!("Received notification packet");
                info!("Notification body: {:?}", body);
                if let Ok(notification) =
                    serde_json::from_value::<plugins::notification::Notification>(body)
                {
                    notification.received_packet(&device, core_tx).await;
                }
            }
            PacketType::Ping => {
                if let Ok(ping) = serde_json::from_value::<plugins::ping::Ping>(body) {
                    ping.received_packet(&device, core_tx).await;
                }
            }
            PacketType::RunCommandRequest => {
                if let Ok(run_command_request) =
                    serde_json::from_value::<plugins::run_command::RunCommandRequest>(body)
                {
                    run_command_request
                        .received_packet(&device, connection_tx, core_tx)
                        .await;
                }
            }
            PacketType::ShareRequest => {
                if let Ok(share_request) =
                    serde_json::from_value::<plugins::share::ShareRequest>(body)
                    && let Some(payload_info) = payload_info
                {
                    // Spawn so the event loop is not blocked during the
                    // notification dialog wait + network payload download.
                    tokio::spawn(async move {
                        if let Err(e) = share_request.receive_share(&device, &payload_info).await {
                            warn!("[share] receive_share failed: {}", e);
                        }
                    });
                }
            }
            _ => {
                debug!(
                    "No plugin found to handle packet type: {:?}",
                    packet.packet_type
                );
            }
        }
    }

    /// Send a packet that carries a binary payload (file / album art).
    ///
    /// The packet is enqueued immediately so the phone knows which port to
    /// connect to. The actual TLS accept + byte copy is spawned as a
    /// background task so the event loop is never blocked.
    pub async fn send_payload(
        &self,
        packet: ProtocolPacket,
        device_writer: &mpsc::UnboundedSender<ProtocolPacket>,
        mut payload: TransferAdapter<impl AsyncRead + Sync + Send + Unpin + 'static>,
        payload_size: u64,
    ) {
        info!("preparing payload transfer");

        let free_listener = match prepare_listener_for_payload().await {
            Ok(l) => l,
            Err(e) => {
                warn!("cannot find free port: {}", e);
                return;
            }
        };

        let addr = match free_listener.local_addr() {
            Ok(a) => a,
            Err(e) => {
                warn!("cannot get local addr for payload listener: {}", e);
                return;
            }
        };

        debug!("payload listener bound on {}", addr);
        let payload_transfer_info = Some(PacketPayloadTransferInfo { port: addr.port() });
        let body = packet.body.clone();

        // Enqueue the packet with the port info NOW — the phone needs this to
        // know where to connect. This is non-blocking (channel send).
        match packet.packet_type {
            PacketType::Mpris => {
                if let Ok(mpris) = serde_json::from_value::<plugins::mpris::Mpris>(body) {
                    debug!("got mpris packet, sending info.");
                    let _ = mpris
                        .send_art(device_writer, payload_size, payload_transfer_info)
                        .await;
                }
            }
            PacketType::ShareRequest => {
                if let Ok(share_request) =
                    serde_json::from_value::<plugins::share::ShareRequest>(body)
                {
                    debug!("got share request packet, sending info.");
                    let _ = share_request
                        .send_file(device_writer, payload_size, payload_transfer_info)
                        .await;
                }
            }
            _ => {
                warn!(
                    "[payload] No plugin found to handle packet type: {:?}",
                    packet.packet_type
                );
                return;
            }
        }

        // Spawn the accept + copy so the event loop stays responsive for the
        // entire duration of the file transfer.
        let server_config = GLOBAL_CONFIG.get().unwrap().key_store.server_config.clone();
        tokio::spawn(async move {
            let (incoming, peer_addr) = match free_listener.accept().await {
                Ok(res) => res,
                Err(e) => {
                    warn!("[payload] accepting connection failed: {}", e);
                    return;
                }
            };
            debug!("[payload] incoming connection from {}", peer_addr);

            let mut stream = match tokio_rustls::TlsAcceptor::from(server_config)
                .accept(incoming)
                .await
            {
                Ok(s) => s,
                Err(e) => {
                    warn!("[payload] TLS handshake failed: {}", e);
                    return;
                }
            };

            debug!("[payload] TLS accepted, copying payload");
            let _ = tokio::io::copy(&mut payload, &mut stream).await;
            let _ = stream.flush().await;
            let _ = stream.shutdown().await;
            info!("[payload] successfully sent payload to {}", peer_addr);
        });
    }

    pub async fn send(
        &self,
        device: Device,
        packet: ProtocolPacket,
        core_tx: mpsc::UnboundedSender<CoreEvent>,
    ) {
        let body = packet.body.clone();
        let core_event = core_tx.clone();

        match packet.packet_type {
            PacketType::Ping => {
                if let Ok(ping) = serde_json::from_value::<plugins::ping::Ping>(body) {
                    ping.send_packet(&device, core_event).await;
                }
            }
            PacketType::MprisRequest => {
                if let Ok(mpris_request) = serde_json::from_value::<MprisRequest>(body) {
                    mpris_request.send_packet(&device, core_event).await;
                }
            }
            _ => {
                warn!(
                    "No plugin found to handle packet type: {:?}",
                    packet.packet_type
                );
            }
        }
    }

    pub async fn list_plugins(&self) -> Vec<String> {
        let plugins = self.plugins.read().await;
        plugins.iter().map(|p| p.id().to_string()).collect()
    }
}

fn parse_vcard(content: &str) -> (Option<String>, Vec<String>) {
    let mut name: Option<String> = None;
    let mut phones: Vec<String> = Vec::new();
    for line in content.lines() {
        let line = line.trim();
        if line.starts_with("FN:") {
            name = Some(line[3..].trim().to_string());
        } else if name.is_none() && line.starts_with("N:") {
            let parts: Vec<&str> = line[2..].split(';').collect();
            if parts.len() >= 2 {
                let full = format!("{} {}", parts[1].trim(), parts[0].trim());
                let full = full.trim().to_string();
                if !full.is_empty() {
                    name = Some(full);
                }
            }
        } else if line.starts_with("TEL") {
            if let Some(pos) = line.rfind(':') {
                let phone = line[pos + 1..].trim().to_string();
                if !phone.is_empty() {
                    phones.push(phone);
                }
            }
        }
    }
    (name, phones)
}
