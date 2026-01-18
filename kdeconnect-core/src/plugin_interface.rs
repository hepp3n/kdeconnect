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

/// All plugins must implement this trait.
/// Plugins are loaded into the core and can react to incoming packets.
pub trait Plugin: Sync + Send {
    /// Unique identifier of the plugin.
    fn id(&self) -> &'static str;
}

/// A thread-safe registry that holds all loaded plugins and dispatches packets to them.
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

    /// Registers a new plugin (usually called during initialization).
    pub async fn register(&self, plugin: Arc<dyn Plugin>) {
        let mut plugins = self.plugins.write().await;
        info!("Registering plugin: {}", plugin.id());
        plugins.push(plugin);
    }

    /// Dispatches an incoming packet to the appropriate plugin based on its type.
    pub async fn dispatch(
        &self,
        device: Device,
        packet: ProtocolPacket,
        core_tx: mpsc::UnboundedSender<CoreEvent>,
        tx: mpsc::UnboundedSender<ConnectionEvent>,
    ) {
        let body = packet.body.clone();
        let core_tx = core_tx.clone();
        let connection_tx = tx.clone();
        let payload_info = packet.payload_transfer_info;

        match packet.packet_type {
            // if it's indentity we can skip
            PacketType::Identity => {
                debug!("Skipping identity packet");
            }
            // if it's pair we can also skip because we handle it elsewhere
            PacketType::Pair => {
                debug!("Skipping pair packet");
            }
            PacketType::Battery => {
                if let Ok(battery) = serde_json::from_value::<Battery>(body) {
                    battery.received_packet(connection_tx).await;
                }
            }
            PacketType::BatteryRequest => todo!(),
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
                    println!("{:?}", keyboard_state);
                }
            }
            PacketType::Mpris => {
                if let Ok(mpris_packet) = serde_json::from_value::<Mpris>(body) {
                    info!("Received MPRIS packet: {:?}", mpris_packet);
                    let _ = connection_tx.send(ConnectionEvent::Mpris((
                        device.device_id.clone(),
                        mpris_packet,
                    )));
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
                    && let Some(payload_info) = payload_info.as_ref()
                {
                    let _ = share_request.receive_share(&device, payload_info).await;
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

    pub async fn send_payload(
        &self,
        packet: ProtocolPacket,
        device_writer: &mpsc::UnboundedSender<ProtocolPacket>,
        mut payload: impl AsyncRead + Sync + Send + Unpin,
        payload_size: i64,
    ) {
        info!("preparing payload transfer");

        let free_listener = match prepare_listener_for_payload().await {
            Ok(l) => l,
            Err(e) => {
                warn!("cannot find free port: {}", e);
                return;
            }
        };

        let body = packet.body.clone();

        if let Ok(addr) = free_listener.local_addr() {
            debug!("trying to match packet for addr: {}", addr);
            let payload_transfer_info = Some(PacketPayloadTransferInfo { port: addr.port() });

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
                }
            }
        };

        let config = GLOBAL_CONFIG.get().unwrap();
        let server_config = config.key_store.server_config.clone();
        debug!("server config created.");

        let (incoming, peer_addr) = match free_listener.accept().await {
            Ok(res) => res,
            Err(e) => {
                warn!("accepting connection failed: {}", e);
                return;
            }
        };
        debug!("incoming connection from {}", peer_addr);

        let mut stream = match tokio_rustls::TlsAcceptor::from(server_config)
            .accept(incoming)
            .await
        {
            Ok(stream) => stream,
            Err(e) => {
                warn!("TLS handshake failed during payload transfer: {}", e);
                return;
            }
        };

        debug!("accepted");

        let _ = tokio::io::copy(&mut payload, &mut stream).await;
        let _ = stream.flush().await;
        let _ = stream.shutdown().await;

        info!("successfully sent payload");
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
