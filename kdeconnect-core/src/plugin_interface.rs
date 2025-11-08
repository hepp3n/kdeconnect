use std::sync::Arc;

use async_trait::async_trait;
use tokio::sync::RwLock;
use tracing::info;

use crate::{
    device::{Device, DeviceId},
    protocol::ProtocolPacket,
};

/// All plugins must implement this trait.
/// Plugins are loaded into the core and can react to incoming packets.
#[async_trait]
pub trait Plugin: Send + Sync {
    /// Unique identifier of the plugin.
    fn id(&self) -> &'static str;

    /// Called by the core when a packet arrives for a device.
    async fn handle_packet(&self, device: Device, packet: ProtocolPacket);
    async fn send_packet(&self, device_id: &DeviceId, packet: ProtocolPacket);
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

    /// Dispatches a packet to all registered plugins concurrently.
    pub async fn dispatch(&self, device: Device, packet: ProtocolPacket) {
        let plugins = self.plugins.read().await;

        for plugin in plugins.iter() {
            let plugin = plugin.clone();
            let device = device.clone();
            let packet = packet.clone();

            tokio::spawn(async move {
                plugin.handle_packet(device, packet).await;
            });
        }
    }

    /// Use for handling frontend packet to send to device.
    pub async fn send_back(&self, device: Device, packet: ProtocolPacket) {
        let plugins = self.plugins.read().await;

        for plugin in plugins.iter() {
            let plugin = plugin.clone();
            let device = device.clone();
            let packet = packet.clone();

            tokio::spawn(async move {
                plugin.send_packet(&device.device_id, packet).await;
            });
        }
    }

    /// Returns a list of registered plugin IDs.
    pub async fn list_plugins(&self) -> Vec<String> {
        let plugins = self.plugins.read().await;
        plugins.iter().map(|p| p.id().to_string()).collect()
    }
}
