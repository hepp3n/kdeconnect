use rustls::crypto::aws_lc_rs::default_provider;
use std::sync::Arc;
use tokio::{
    io::{AsyncRead, AsyncWriteExt},
    sync::{RwLock, mpsc},
};
use tracing::{debug, info, warn};

use crate::{
    GLOBAL_CONFIG,
    crypto::NoCertificateVerification,
    device::Device,
    event::{ConnectionEvent, CoreEvent},
    plugins::{
        self,
        battery::Battery,
        clipboard::Clipboard,
        mpris::{Mpris, MprisPlayer, MprisRequest},
    },
    protocol::{DeviceFile, DevicePayload, PacketPayloadTransferInfo, PacketType, ProtocolPacket},
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
            PacketType::ConnectivityReportRequest => {
                debug!("Received connectivity report request packet");
                // Handle connectivity report request if needed
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
            PacketType::Mpris => {
                if let Ok(mpris_packet) = serde_json::from_value::<Mpris>(body) {
                    info!("Received MPRIS packet: {:?}", mpris_packet);
                }
            }
            PacketType::MprisRequest => {
                if let Ok(mpris_request) = serde_json::from_value::<MprisRequest>(body) {
                    match mpris_request {
                        MprisRequest::PlayerRequest {
                            player,
                            request_now_playing,
                            request_volume,
                            request_album_art,
                        } => {
                            debug!(
                                "mpris request received: {:?} {:?} {:?} {:?} ",
                                player, request_now_playing, request_volume, request_album_art
                            );

                            let Ok(player) = MprisPlayer::new(Some(&player)) else {
                                return;
                            };

                            let player_name = player.player.clone();
                            let art = player.album_art_url.clone();

                            if let Some(album_art_url) = request_album_art {
                                let path = album_art_url.strip_prefix("file://");

                                let Some(path) = path else {
                                    return;
                                };

                                let file = DeviceFile::open(path).await.expect("cannot open file");
                                let payload = DevicePayload::from(file);

                                let construct_packet = ProtocolPacket::new_with_payload(
                                    PacketType::Mpris,
                                    serde_json::to_value(Mpris::TransferringArt {
                                        player: player_name,
                                        album_art_url: art.unwrap_or(path.to_string()),
                                        transferring_album_art: true,
                                    })
                                    .unwrap(),
                                    payload.size,
                                    None,
                                );

                                let _ = core_tx.send(crate::event::CoreEvent::SendPaylod {
                                    device: device.device_id.clone(),
                                    packet: construct_packet,
                                    payload: Box::new(payload.buf),
                                    payload_size: payload.size,
                                });
                            } else {
                                let construct_packet = ProtocolPacket {
                                    id: None,
                                    packet_type: PacketType::Mpris,
                                    body: serde_json::to_value(Mpris::Info(player)).unwrap(),
                                    payload_size: None,
                                    payload_transfer_info: None,
                                };

                                let _ = core_tx.send(crate::event::CoreEvent::SendPacket {
                                    device: device.device_id.clone(),
                                    packet: construct_packet,
                                });
                            }
                        }
                        MprisRequest::Action { .. } | MprisRequest::List { .. } => {
                            mpris_request.received_packet(&device, core_tx).await;
                        }
                    }
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

        let free_listener = prepare_listener_for_payload()
            .await
            .expect("cannot find free port");

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
        let cert = config.key_store.get_certificateder().clone();
        let keypair = config.key_store.get_keys().clone_key();

        let verifier = Arc::new(NoCertificateVerification::new(default_provider()));

        let server_config = rustls::ServerConfig::builder()
            .with_client_cert_verifier(verifier.clone())
            .with_single_cert(vec![cert.clone()], keypair)
            .expect("creating server config");

        debug!("server config created.");

        let (incoming, peer_addr) = free_listener.accept().await.expect("accepting connection.");
        debug!("incoming connection from {}", peer_addr);

        let mut stream = tokio_rustls::TlsAcceptor::from(Arc::new(server_config))
            .accept(incoming)
            .await
            .expect("from acceptor in payload");

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
