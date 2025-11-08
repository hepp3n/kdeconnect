use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::{collections::HashMap, sync::Arc};
use tokio::sync::{Mutex, mpsc};
use tracing::info;

use crate::{
    device::{Device, DeviceId},
    plugin_interface::Plugin,
    protocol::ProtocolPacket,
};

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Ping {
    pub message: Option<String>,
}

pub struct PingPlugin {
    writer_map: Arc<Mutex<HashMap<DeviceId, mpsc::UnboundedSender<ProtocolPacket>>>>,
}

impl PingPlugin {
    pub fn new(
        writer_map: Arc<Mutex<HashMap<DeviceId, mpsc::UnboundedSender<ProtocolPacket>>>>,
    ) -> Self {
        Self { writer_map }
    }
}

#[async_trait]
impl Plugin for PingPlugin {
    fn id(&self) -> &'static str {
        "kdeconnect.ping"
    }

    async fn handle_packet(&self, device: Device, packet: ProtocolPacket) {
        let value = serde_json::to_value(Ping {
            message: Some("Pong".to_string()),
        })
        .expect("fail serializing packet body");

        if packet.packet_type == "kdeconnect.ping" {
            let reply = ProtocolPacket {
                id: packet.id,
                packet_type: "kdeconnect.ping".to_string(),
                body: value,
                payload_size: None,
                payload_transfer_info: None,
            };

            if let Some(tx) = self.writer_map.lock().await.get(&device.device_id) {
                let _ = tx.send(reply);
            } else {
                info!("No writer found for device {}", device.device_id);
            }
        }
    }

    async fn send_packet(&self, device_id: &DeviceId, packet: ProtocolPacket) {
        if let Some(tx) = self.writer_map.lock().await.get(device_id) {
            let _ = tx.send(packet);
        } else {
            info!("No writer found for device {}", device_id);
        }
    }
}
