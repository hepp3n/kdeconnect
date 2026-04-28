use serde::{Deserialize, Serialize};
use base64::{engine::general_purpose, Engine as _};
use std::sync::{Mutex, OnceLock};
use tokio::sync::mpsc;
use tracing::info;

use crate::{
    device::Device,
    event::CoreEvent,
    plugin_interface::Plugin,
    protocol::{PacketType, ProtocolPacket},
};

// ---------------------------------------------------------------------------
// Pause-on-call state
// Names of desktop MPRIS players that were paused due to an active call.
// Persists across packet boundaries so we know what to resume on isCancel.
// ---------------------------------------------------------------------------

static PAUSED_PLAYERS: OnceLock<Mutex<Vec<String>>> = OnceLock::new();

fn paused_players_store() -> &'static Mutex<Vec<String>> {
    PAUSED_PLAYERS.get_or_init(|| Mutex::new(Vec::new()))
}

/// Pause every currently-playing desktop MPRIS player and remember their names.
fn pause_playing_players() {
    let finder = match mpris::PlayerFinder::new() {
        Ok(f) => f,
        Err(e) => {
            info!("[telephony] MPRIS PlayerFinder unavailable: {}", e);
            return;
        }
    };

    let players = match finder.find_all() {
        Ok(p) => p,
        Err(e) => {
            info!("[telephony] could not list MPRIS players: {}", e);
            return;
        }
    };

    let mut paused = paused_players_store().lock().unwrap();
    paused.clear();

    for player in players {
        if matches!(player.get_playback_status(), Ok(mpris::PlaybackStatus::Playing)) {
            let bus_name = player.bus_name().to_string();
            if player.pause().is_ok() {
                info!("[telephony] paused MPRIS player: {}", bus_name);
                paused.push(bus_name);
            }
        }
    }
}

/// Resume the players that were paused by `pause_playing_players`.
fn resume_paused_players() {
    let finder = match mpris::PlayerFinder::new() {
        Ok(f) => f,
        Err(e) => {
            info!("[telephony] MPRIS PlayerFinder unavailable: {}", e);
            return;
        }
    };

    let mut paused = paused_players_store().lock().unwrap();
    if paused.is_empty() {
        return;
    }

    let all_players = match finder.find_all() {
        Ok(p) => p,
        Err(e) => {
            info!("[telephony] could not list MPRIS players for resume: {}", e);
            return;
        }
    };

    for player in all_players {
        if paused.contains(&player.bus_name().to_string()) {
            if player.play_pause().is_ok() {
                info!("[telephony] resumed MPRIS player: {}", player.bus_name());
            }
        }
    }

    paused.clear();
}

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
            tokio::task::spawn_blocking(resume_paused_players).await.ok();
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
            tokio::task::spawn_blocking(pause_playing_players).await.ok();
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
