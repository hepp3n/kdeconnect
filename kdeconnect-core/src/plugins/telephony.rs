use serde::{Deserialize, Serialize};
use base64::{engine::general_purpose, Engine as _};
use crate::plugins::mpris;
use tokio::sync::mpsc;
use tracing::info;

use crate::{
    device::Device,
    event::CoreEvent,
    plugin_interface::Plugin,
    protocol::{PacketType, ProtocolPacket},
};

// Pause/resume is handled by mpris::monitor_mpris via telephony_call_signal()
// which reuses the persistent PlayerFinder held in that thread.

// ---------------------------------------------------------------------------
// Packet types
// ---------------------------------------------------------------------------

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct TelephonyPacket {
    pub event: Option<String>,
    #[serde(rename = "contactName")]
    pub contact_name: Option<String>,
    #[serde(rename = "phoneNumber")]
    pub phone_number: Option<String>,
    #[serde(rename = "phoneThumbnail")]
    pub phone_thumbnail: Option<String>,
    /// Deprecated — present in old packets only; use the SMS plugin for messages.
    #[serde(rename = "messageBody")]
    pub message_body: Option<String>,
    #[serde(rename = "isCancel")]
    pub is_cancel: Option<bool>,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct TelephonyRequestMute {}

impl Plugin for TelephonyPacket {
    fn id(&self) -> &'static str {
        "telephony"
    }
}

impl TelephonyPacket {
    /// Handle an incoming kdeconnect.telephony packet.
    pub async fn received_packet(
        &self,
        device: &Device,
        _core_event: mpsc::UnboundedSender<CoreEvent>,
    ) {
        // isCancel=true signals the event is over — resume any paused players.
        if self.is_cancel.unwrap_or(false) {
            info!("[telephony] isCancel received — call ended, resuming players");
            if let Some(tx) = mpris::telephony_call_signal() {
                let _ = tx.send(false);
            }
            return;
        }

        let event = match self.event.as_deref() {
            Some(e) => e,
            None => {
                info!("[telephony] packet missing event field, ignoring");
                return;
            }
        };

        // "sms" is deprecated in favour of the SMS plugin.
        if event == "sms" {
            info!("[telephony] sms event (deprecated), ignoring");
            return;
        }

        let caller = self
            .contact_name
            .clone()
            .or_else(|| self.phone_number.clone())
            .unwrap_or_else(|| device.name.clone());

        let (summary, body) = match event {
            "ringing" => (
                format!("📞 Incoming call from {}", caller),
                format!("Device: {}", device.name),
            ),
            "talking" => (
                format!("📞 Call in progress with {}", caller),
                format!("Device: {}", device.name),
            ),
            "missedCall" => (
                format!("📵 Missed call from {}", caller),
                format!("Device: {}", device.name),
            ),
            other => {
                info!("[telephony] unknown event type '{}', ignoring", other);
                return;
            }
        };

        // Pause desktop players before showing the notification.
        // missedCall arrives after the fact so we skip pausing for that.
        if event != "missedCall" {
            if let Some(tx) = mpris::telephony_call_signal() {
                let _ = tx.send(true);
            }
        }

        let thumbnail_path = self.phone_thumbnail.as_deref().and_then(|b64| {
            use std::io::Write;
            let bytes = general_purpose::STANDARD.decode(b64).ok()?;
            let mut tmp = tempfile::Builder::new()
                .prefix("kdeconnect-thumb-")
                .suffix(".jpg")
                .tempfile()
                .ok()?;
            tmp.write_all(&bytes).ok()?;
            // Keep the file alive by converting to a path-only handle
            let (_, path) = tmp.keep().ok()?;
            Some(path)
        });

        let _ = tokio::task::spawn_blocking(move || {
            let mut notif = notify_rust::Notification::new();
            notif
                .appname("KDE Connect")
                .summary(&summary)
                .body(&body)
                .timeout(notify_rust::Timeout::Milliseconds(10_000));
            if let Some(path) = thumbnail_path {
                notif.icon(&path.to_string_lossy().into_owned());
            } else {
                notif.icon("call-start-symbolic");
            }
            notif.show().ok();
        })
        .await;
    }

    /// Send a kdeconnect.telephony.request_mute to silence the phone's ringer.
    pub fn send_mute_request(device: &Device, core_event: &mpsc::UnboundedSender<CoreEvent>) {
        let packet = ProtocolPacket::new(PacketType::TelephonyRequestMute, serde_json::json!({}));
        let _ = core_event.send(CoreEvent::SendPacket {
            device: device.device_id.clone(),
            packet,
        });
    }
}
