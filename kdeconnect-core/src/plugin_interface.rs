use std::sync::Arc;

use async_trait::async_trait;
use tokio::sync::{RwLock, mpsc};
use tracing::{info, warn};

use crate::{
    device::Device,
    event::ConnectionEvent,
    plugins::{battery::Battery, clipboard::Clipboard},
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
                PacketType::Ping => todo!(),
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
