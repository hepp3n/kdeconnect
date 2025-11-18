use async_trait::async_trait;
use std::sync::Arc;
use tokio::sync::{RwLock, broadcast, mpsc};
use tracing::{info, warn};

use crate::{
    device::Device,
    event::{ConnectionEvent, CoreEvent},
    plugins::{
        self,
        battery::Battery,
        clipboard::Clipboard,
        mpris::{Mpris, MprisRequest},
    },
    protocol::{PacketType, ProtocolPacket},
};

/// All plugins must implement this trait.
/// Plugins are loaded into the core and can react to incoming packets.
#[async_trait]
pub trait Plugin: Send + Sync {
    /// Unique identifier of the plugin.
    fn id(&self) -> &'static str;
    /// Called when a packet is received that this plugin can handle.
    fn received(
        &self,
        device: &Device,
        connection_event: Arc<mpsc::UnboundedSender<ConnectionEvent>>,
        core_event: Arc<broadcast::Sender<CoreEvent>>,
    ) -> ();
    /// Called to send a packet using this plugin.
    fn send(&self, device: &Device, core_event: Arc<broadcast::Sender<CoreEvent>>) -> ();
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

        for _plugin in plugins.iter() {
            let body = packet.body.clone();
            let core_tx = core_tx.clone();
            let connection_tx = tx.clone();

            match packet.packet_type {
                // if it's indentity we can skip
                PacketType::Identity => continue,
                // if it's pair we can also skip because we handle it elsewhere
                PacketType::Pair => continue,

                // handle other plugins
                PacketType::Battery => {
                    if let Ok(battery) = serde_json::from_value::<Battery>(body) {
                        battery.received(&device, connection_tx, core_tx);
                    }
                }
                PacketType::BatteryRequest => todo!(),
                PacketType::Clipboard => {
                    if let Ok(clipboard) = serde_json::from_value::<Clipboard>(body) {
                        clipboard.received(&device, connection_tx, core_tx);
                    }
                }
                PacketType::ClipboardConnect => {
                    if let Ok(clipboard) = serde_json::from_value::<Clipboard>(body)
                        && let Some(timestamp) = clipboard.timestamp
                    {
                        if timestamp > 0 {
                            info!("Clipboard sync requested with timestamp: {}", timestamp);
                            clipboard.received(&device, connection_tx, core_tx);
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
                        mpris_request.received(&device, connection_tx, core_tx);
                    }
                }
                PacketType::Ping => {
                    if let Ok(ping) = serde_json::from_value::<plugins::ping::Ping>(body) {
                        ping.received(&device, connection_tx, core_tx);
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
    }

    /// Sends a packet using the appropriate plugin.
    pub async fn send(
        &self,
        device: Device,
        packet: ProtocolPacket,
        core_tx: Arc<broadcast::Sender<CoreEvent>>,
    ) {
        let plugins = self.plugins.read().await;

        for _plugin in plugins.iter() {
            let body = packet.body.clone();
            let core_event = core_tx.clone();

            match packet.packet_type {
                PacketType::Ping => {
                    if let Ok(ping) = serde_json::from_value::<plugins::ping::Ping>(body) {
                        ping.send(&device, core_event);
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
    }

    /// Returns a list of registered plugin IDs.
    pub async fn list_plugins(&self) -> Vec<String> {
        let plugins = self.plugins.read().await;
        plugins.iter().map(|p| p.id().to_string()).collect()
    }
}
