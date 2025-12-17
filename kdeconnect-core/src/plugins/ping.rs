use crate::{device::Device, event::CoreEvent, plugin_interface::Plugin, protocol::ProtocolPacket};
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct Ping {
    pub message: Option<String>,
}

impl Plugin for Ping {
    fn id(&self) -> &'static str {
        "kdeconnect.ping"
    }
}
impl Ping {
    pub async fn received_packet(
        &self,
        device: &Device,
        core_event: mpsc::UnboundedSender<CoreEvent>,
    ) {
        let device_id = device.device_id.clone();
        let event = core_event.clone();

        let packet = ProtocolPacket::new(
            crate::protocol::PacketType::Ping,
            serde_json::to_value(Self {
                message: Some("Pong!".into()),
            })
            .unwrap_or_default(),
        );

        let summary = self.message.clone().unwrap_or_else(|| "Ping!".into());

        let _ = tokio::task::spawn_blocking(move || {
            let mut reply = false;

            notify_rust::Notification::new()
                .summary("KDE Connect")
                .body(&summary)
                .action("clicked", "Click to reply")
                .hint(notify_rust::Hint::Resident(true))
                .show()
                .unwrap()
                .wait_for_action(|action| match action {
                    "clicked" => {
                        reply = true;
                    }
                    // here "__closed" is a hard coded keyword
                    "__closed" => println!("the notification was closed"),
                    _ => (),
                });

            if reply {
                let _ = event.send(CoreEvent::SendPacket {
                    device: device_id,
                    packet,
                });
            }
        })
        .await;
    }

    pub async fn send_packet(&self, device: &Device, core_event: mpsc::UnboundedSender<CoreEvent>) {
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
