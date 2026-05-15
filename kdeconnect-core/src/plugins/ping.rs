use crate::{device::Device, event::CoreEvent, plugin_interface::Plugin, protocol::ProtocolPacket};
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;
use tracing::warn;

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct Ping {
    pub message: Option<String>,
    #[serde(default)]
    pub heartbeat: Option<bool>,
}

impl Plugin for Ping {
    fn id(&self) -> &'static str {
        "kdeconnect.ping"
    }
}
impl Ping {
    pub async fn received_packet(
        &self,
        _device: &Device,
        _core_event: mpsc::UnboundedSender<CoreEvent>,
    ) {
        // Heartbeat pings should not trigger desktop notifications.
        if self.heartbeat.unwrap_or(false) {
            return;
        }

        let summary = self.message.clone().unwrap_or_else(|| "Ping!".into());

        tokio::task::spawn_blocking(move || {
            match notify_rust::Notification::new()
                .summary("KDE Connect")
                .body(&summary)
                .hint(notify_rust::Hint::Resident(true))
                .show()
            {
                Ok(_) => {}
                Err(e) => {
                    warn!("[ping] failed to show notification: {}", e);
                }
            }
        });
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ping_serialization_round_trips() {
        let ping = Ping {
            message: Some("Hello".into()),
            heartbeat: None,
        };
        let json = serde_json::to_value(&ping).unwrap();
        let parsed: Ping = serde_json::from_value(json).unwrap();
        assert_eq!(parsed.message, Some("Hello".into()));
        assert_eq!(parsed.heartbeat, None);
    }

    #[test]
    fn heartbeat_ping_has_characteristic_shape() {
        let ping = Ping {
            message: None,
            heartbeat: Some(true),
        };
        let json = serde_json::to_value(&ping).unwrap();
        let parsed: Ping = serde_json::from_value(json).unwrap();
        assert_eq!(parsed.heartbeat, Some(true));
        assert_eq!(parsed.message, None);
    }

    #[test]
    fn empty_object_defaults_to_no_heartbeat() {
        let parsed: Ping = serde_json::from_str("{}").unwrap();
        assert!(parsed.message.is_none());
        assert!(parsed.heartbeat.is_none());
    }

    #[test]
    fn ping_plugin_has_correct_id() {
        assert_eq!(Ping::default().id(), "kdeconnect.ping");
    }

    #[test]
    fn received_packet_returns_immediately_without_blocking() {
        use crate::device::{Device, DeviceId};
        use std::time::Duration;

        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();

        let passed = rt.block_on(async {
            let ping = Ping {
                message: Some("test".into()),
                heartbeat: None,
            };
            let device = Device {
                device_id: DeviceId("test-ping-nonblock".into()),
                ..Default::default()
            };
            let (core_tx, _core_rx) = tokio::sync::mpsc::unbounded_channel();

            tokio::time::timeout(
                Duration::from_millis(500),
                ping.received_packet(&device, core_tx),
            )
            .await
            .is_ok()
        });

        rt.shutdown_background();

        assert!(
            passed,
            "ping received_packet must return immediately without blocking on notification"
        );
    }
}
