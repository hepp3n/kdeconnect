use crate::{
    device::Device,
    event::{ConnectionEvent, CoreEvent},
    plugin_interface::Plugin,
    protocol::ProtocolPacket,
};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio::sync::{broadcast, mpsc};

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct Ping {
    pub message: Option<String>,
}

#[async_trait]
impl Plugin for Ping {
    fn id(&self) -> &'static str {
        "kdeconnect.ping"
    }

    fn received(
        &self,
        _device: &Device,
        _event: Arc<mpsc::UnboundedSender<ConnectionEvent>>,
        _core_event: Arc<broadcast::Sender<CoreEvent>>,
    ) {
        // Ping plugin does not send events on its own
    }

    fn send(&self, device: &Device, core_event: Arc<broadcast::Sender<CoreEvent>>) {
        let packet = ProtocolPacket::new(
            crate::protocol::PacketType::Ping,
            serde_json::to_value(self).unwrap_or_default(),
        );

        let _ = core_event.send(CoreEvent::SendPacket {
            device: device.device_id.clone(),
            packet,
        });
    }
}
