use crate::plugin_interface::Plugin;

use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct Clipboard {
    pub content: String,
    pub timestamp: Option<u64>,
}

impl Plugin for Clipboard {
    fn id(&self) -> &'static str {
        "kdeconnect.clipboard"
    }
}
impl Clipboard {
    pub async fn received_packet(
        &self,
        event: mpsc::UnboundedSender<crate::event::ConnectionEvent>,
    ) {
        let _ = event.send(crate::event::ConnectionEvent::ClipboardReceived(
            self.content.clone(),
        ));
    }
}

// impl Clipboard {
//     pub async fn send_clipboard(
//         &self,
//         device_id: &DeviceId,
//         content: String,
//     ) -> Result<(), String> {
//         let clipboard = Clipboard {
//             content,
//             timestamp: None,
//         };
//
//         let packet = ProtocolPacket::new(
//             PacketType::Clipboard,
//             serde_json::to_value(clipboard).map_err(|e| e.to_string())?,
//         );
//
//         let writer_map = self.writer_map.lock().await;
//         if let Some(sender) = writer_map.get(device_id) {
//             sender
//                 .send(packet)
//                 .map_err(|e| format!("Failed to send clipboard packet: {}", e))?;
//             Ok(())
//         } else {
//             Err(format!("No writer found for device ID: {}", device_id))
//         }
//     }
// }
