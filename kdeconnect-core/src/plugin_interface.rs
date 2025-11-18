use std::sync::Arc;

use async_trait::async_trait;
use tokio::sync::{RwLock, broadcast, mpsc};
use tracing::{info, warn};

use crate::{
    device::Device,
    event::{ConnectionEvent, CoreEvent},
    plugins::{
        self,
        battery::Battery,
        clipboard::Clipboard,
        mpris::{Mpris, MprisLoopStatus, MprisRequest},
    },
    protocol::{PacketType, ProtocolPacket},
};

/// All plugins must implement this trait.
/// Plugins are loaded into the core and can react to incoming packets.
#[async_trait]
pub trait Plugin: Send + Sync {
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
        core_tx: Arc<broadcast::Sender<CoreEvent>>,
        tx: Arc<mpsc::UnboundedSender<ConnectionEvent>>,
    ) {
        let plugins = self.plugins.read().await;

        for plugin in plugins.iter() {
            let body = packet.body.clone();

            match packet.packet_type {
                // if it's indentity we can skip
                PacketType::Identity => continue,
                // if it's pair we can also skip because we handle it elsewhere
                PacketType::Pair => continue,

                // handle other plugins
                PacketType::Battery => {
                    // handle battery packet
                    if let Ok(bat_body) = serde_json::from_value::<Battery>(body) {
                        let _ = tx.send(ConnectionEvent::StateUpdated(
                            crate::event::DeviceState::Battery {
                                level: bat_body.charge as u8,
                                charging: bat_body.is_charging,
                            },
                        ));
                    }
                }
                PacketType::BatteryRequest => todo!(),
                PacketType::Clipboard => {
                    // Handle clipboard packet
                    if let Ok(clipboard) = serde_json::from_value::<Clipboard>(body) {
                        let _ = tx.send(ConnectionEvent::ClipboardReceived(clipboard.content));
                    }
                }
                PacketType::ClipboardConnect => {
                    if let Ok(clipboard) = serde_json::from_value::<Clipboard>(body)
                        && let Some(timestamp) = clipboard.timestamp
                    {
                        if timestamp > 0 {
                            info!("Clipboard sync requested with timestamp: {}", timestamp);
                            let _ = tx.send(ConnectionEvent::ClipboardReceived(clipboard.content));
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
                            MprisRequest::List {
                                request_player_list,
                            } => {
                                if request_player_list {
                                    info!("MPRIS Player list requested");
                                    let players = plugins::mpris::get_mpris_players();

                                    if let Ok(player) = players {
                                        let packet = ProtocolPacket {
                                            id: None,
                                            packet_type: PacketType::Mpris,
                                            body: serde_json::to_value(Mpris::List {
                                                player_list: vec![player.identity().into()],
                                                supports_album_art_payload: false,
                                            })
                                            .unwrap(),
                                            payload_size: None,
                                            payload_transfer_info: None,
                                        };

                                        let _ = core_tx.send(CoreEvent::SendPacket {
                                            device: device.device_id.clone(),
                                            packet,
                                        });
                                    }
                                }
                            }
                            MprisRequest::PlayerRequest { player, .. } => {
                                info!("MPRIS Player request received for player: {}", player);
                                let Some(metadata) = plugins::mpris::get_mpris_metadata() else {
                                    continue;
                                };
                                let artist = metadata
                                    .artists()
                                    .map_or("Unknown Artist".to_string(), |a| a.join(", "));
                                let title = metadata
                                    .title()
                                    .map_or("Unknown Title".to_string(), |t| t.to_string());
                                let album = metadata
                                    .album_name()
                                    .map_or("Unknown Album".to_string(), |al| al.to_string());

                                let album_art_url = metadata
                                    .art_url()
                                    .map_or("".to_string(), |url| url.to_string());

                                let length = metadata.length().map_or(0, |l| l.as_millis() as i32);

                                let now_playing_string =
                                    format!("{} - {}, ({})", &artist, &title, &album,);

                                let Ok(player) = plugins::mpris::get_mpris_players() else {
                                    continue;
                                };

                                let volume = (player.get_volume().unwrap_or(1.0) * 100.0) as i32;
                                let can_pause = player.can_pause().unwrap_or(false);
                                let can_play = player.can_play().unwrap_or(false);
                                let can_go_next = player.can_go_next().unwrap_or(false);
                                let can_go_previous = player.can_go_previous().unwrap_or(false);
                                let can_seek = player.can_seek().unwrap_or(false);
                                let is_playing = player.is_running();
                                let loop_status = MprisLoopStatus::from(
                                    player.get_loop_status().unwrap_or(mpris::LoopStatus::None),
                                );
                                let shuffle = player.get_shuffle().unwrap_or(false);
                                let pos = player.get_position().map_or(0, |p| p.as_millis() as i32);

                                let construct_packet = ProtocolPacket {
                                    id: None,
                                    packet_type: PacketType::Mpris,
                                    body: serde_json::to_value(Mpris::Info(
                                        plugins::mpris::MprisPlayer {
                                            player: player.identity().into(),
                                            title: Some(title),
                                            artist: Some(artist),
                                            album: Some(album),
                                            is_playing: Some(is_playing),
                                            can_pause: Some(can_pause),
                                            can_play: Some(can_play),
                                            can_go_next: Some(can_go_next),
                                            can_go_previous: Some(can_go_previous),
                                            can_seek: Some(can_seek),
                                            loop_status: Some(loop_status),
                                            shuffle: Some(shuffle),
                                            pos: Some(pos),
                                            length: Some(length),
                                            volume: Some(volume),
                                            album_art_url: Some(album_art_url),
                                            url: None,
                                        },
                                    ))
                                    .unwrap(),
                                    payload_size: None,
                                    payload_transfer_info: None,
                                };

                                let _ = core_tx.send(CoreEvent::SendPacket {
                                    device: device.device_id.clone(),
                                    packet: construct_packet,
                                });
                            }
                            MprisRequest::Action { .. } => {
                                info!("MPRIS Action request received");
                            }
                        }
                    }
                }
                PacketType::Ping => {}
                _ => {
                    warn!(
                        "No plugin found to handle packet type: {:?}",
                        packet.packet_type
                    );
                }
            }
        }
    }

    /// Use for handling frontend packet to send to device.
    pub async fn send_plugin(&self, device: Device, packet: ProtocolPacket) {
        let plugins = self.plugins.read().await;
    }

    /// Returns a list of registered plugin IDs.
    pub async fn list_plugins(&self) -> Vec<String> {
        let plugins = self.plugins.read().await;
        plugins.iter().map(|p| p.id().to_string()).collect()
    }
}
