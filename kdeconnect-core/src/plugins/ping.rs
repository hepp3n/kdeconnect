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

    async fn received(
        &self,
        device: &Device,
        _event: Arc<mpsc::UnboundedSender<ConnectionEvent>>,
        core_event: Arc<broadcast::Sender<CoreEvent>>,
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

    async fn send(&self, device: &Device, core_event: Arc<broadcast::Sender<CoreEvent>>) {
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
