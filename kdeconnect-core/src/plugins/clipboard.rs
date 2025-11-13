use std::{collections::HashMap, sync::Arc};

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use tokio::sync::{Mutex, mpsc};

use crate::{
    device::DeviceId,
    plugin_interface::Plugin,
    protocol::{PacketType, ProtocolPacket},
};

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Clipboard {
    pub content: String,
    pub timestamp: Option<u64>,
}

pub struct ClipboardPlugin {
    writer_map: Arc<Mutex<HashMap<DeviceId, mpsc::UnboundedSender<ProtocolPacket>>>>,
}

impl ClipboardPlugin {
    pub fn new(
        writer_map: Arc<Mutex<HashMap<DeviceId, mpsc::UnboundedSender<ProtocolPacket>>>>,
    ) -> Self {
        Self { writer_map }
    }
}

#[async_trait]
impl Plugin for ClipboardPlugin {
    fn id(&self) -> &'static str {
        "kdeconnect.clipboard"
    }
}

impl ClipboardPlugin {
    pub async fn send_clipboard(
        &self,
        device_id: &DeviceId,
        content: String,
    ) -> Result<(), String> {
        let clipboard = Clipboard {
            content,
            timestamp: None,
        };

        let packet = ProtocolPacket::new(
            PacketType::Clipboard,
            serde_json::to_value(clipboard).map_err(|e| e.to_string())?,
        );

        let writer_map = self.writer_map.lock().await;
        if let Some(sender) = writer_map.get(device_id) {
            sender
                .send(packet)
                .map_err(|e| format!("Failed to send clipboard packet: {}", e))?;
            Ok(())
        } else {
            Err(format!("No writer found for device ID: {}", device_id))
        }
    }
}
