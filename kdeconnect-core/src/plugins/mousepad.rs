use serde::{Deserialize, Serialize};
use std::{
    process::Command,
    sync::atomic::{AtomicBool, Ordering},
};
use tokio::sync::mpsc;
use tracing::{debug, warn};

use crate::{
    device::Device,
    event::CoreEvent,
    plugin_interface::Plugin,
    protocol::{PacketType, ProtocolPacket},
};

#[derive(Debug, Default, Deserialize, Serialize)]
pub struct KeyboardState {
    pub state: Option<bool>,
}

impl Plugin for KeyboardState {
    fn id(&self) -> &'static str {
        "kdeconnect.mousepad.keyboardstate"
    }
}

static YDOTOOL_MISSING_WARNED: AtomicBool = AtomicBool::new(false);

#[derive(Debug, Default, Clone, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct MousepadRequest {
    pub dx: Option<f64>,
    pub dy: Option<f64>,
    pub scroll: Option<bool>,
    pub key: Option<String>,
    pub special_key: Option<u8>,
    pub alt: Option<bool>,
    pub ctrl: Option<bool>,
    pub shift: Option<bool>,
    #[serde(rename = "super")]
    pub meta: Option<bool>,
    pub singleclick: Option<bool>,
    pub doubleclick: Option<bool>,
    pub middleclick: Option<bool>,
    pub rightclick: Option<bool>,
    pub singlehold: Option<bool>,
    pub singlerelease: Option<bool>,
    #[serde(rename = "sendAck")]
    pub send_ack: Option<bool>,
    #[serde(rename = "isAck")]
    pub is_ack: Option<bool>,
}

#[derive(Debug, Default, Clone, Deserialize, Serialize, PartialEq)]
pub struct PresenterRequest {
    pub dx: Option<f64>,
    pub dy: Option<f64>,
    pub stop: Option<bool>,
}

impl MousepadRequest {
    pub async fn received_packet(
        &self,
        device: &Device,
        core_tx: mpsc::UnboundedSender<CoreEvent>,
    ) {
        let request = self.clone();
        let result = tokio::task::spawn_blocking(move || run_mousepad_request(&request)).await;

        if matches!(result, Ok(Ok(()))) && self.send_ack == Some(true) {
            let mut ack = self.clone();
            ack.send_ack = None;
            ack.is_ack = Some(true);
            let packet = ProtocolPacket::new(
                PacketType::MousePadEcho,
                serde_json::to_value(ack).unwrap_or_default(),
            );
            let _ = core_tx.send(CoreEvent::SendPacket {
                device: device.device_id.clone(),
                packet,
            });
        }
    }
}

impl PresenterRequest {
    pub async fn received_packet(&self) {
        let request = self.clone();
        let _ = tokio::task::spawn_blocking(move || run_presenter_request(&request)).await;
    }
}

fn run_presenter_request(request: &PresenterRequest) -> anyhow::Result<()> {
    if request.stop.unwrap_or(false) {
        return Ok(());
    }

    if let (Some(dx), Some(dy)) = (request.dx, request.dy) {
        return ydotool(["mousemove", "--", &(dx * 1000.0).round().to_string(), &(dy * 1000.0).round().to_string()]);
    }

    Ok(())
}

fn run_mousepad_request(request: &MousepadRequest) -> anyhow::Result<()> {
    if let Some(scroll) = request.scroll {
        let dy = request.dy.unwrap_or_default();
        if scroll && dy != 0.0 {
            let button = if dy > 0.0 { "0xC5" } else { "0xC4" };
            return ydotool(["click", button]);
        }
    }

    if let (Some(dx), Some(dy)) = (request.dx, request.dy) {
        return ydotool(["mousemove", "--", &dx.round().to_string(), &dy.round().to_string()]);
    }

    if request.singleclick.unwrap_or(false) {
        return ydotool(["click", "0xC0"]);
    }
    if request.doubleclick.unwrap_or(false) {
        ydotool(["click", "0xC0"])?;
        return ydotool(["click", "0xC0"]);
    }
    if request.middleclick.unwrap_or(false) {
        return ydotool(["click", "0xC2"]);
    }
    if request.rightclick.unwrap_or(false) {
        return ydotool(["click", "0xC1"]);
    }
    if request.singlehold.unwrap_or(false) {
        return ydotool(["click", "0x40"]);
    }
    if request.singlerelease.unwrap_or(false) {
        return ydotool(["click", "0x80"]);
    }

    if let Some(text) = request.key.as_deref() {
        if text == "\0" {
            return Ok(());
        }
        if !text.is_empty() {
            return ydotool(["type", text]);
        }
    }

    if let Some(special_key) = request.special_key {
        if let Some(code) = special_key_code(special_key) {
            return ydotool_key_with_modifiers(code, request);
        }
    }

    debug!("[mousepad] ignored unsupported request: {:?}", request);
    Ok(())
}

fn ydotool_key_with_modifiers(code: u16, request: &MousepadRequest) -> anyhow::Result<()> {
    let mut keys = Vec::new();
    if request.ctrl.unwrap_or(false) {
        keys.push(29);
    }
    if request.alt.unwrap_or(false) {
        keys.push(56);
    }
    if request.shift.unwrap_or(false) {
        keys.push(42);
    }
    if request.meta.unwrap_or(false) {
        keys.push(125);
    }

    for modifier in &keys {
        ydotool(["key", &format!("{modifier}:1")])?;
    }
    ydotool(["key", &format!("{code}:1"), &format!("{code}:0")])?;
    for modifier in keys.iter().rev() {
        ydotool(["key", &format!("{modifier}:0")])?;
    }

    Ok(())
}

fn ydotool<const N: usize>(args: [&str; N]) -> anyhow::Result<()> {
    let status = Command::new("ydotool").args(args).status();

    match status {
        Ok(status) if status.success() => Ok(()),
        Ok(status) => {
            warn!("[mousepad] ydotool exited with status {}", status);
            Ok(())
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            if !YDOTOOL_MISSING_WARNED.swap(true, Ordering::Relaxed) {
                warn!("[mousepad] ydotool is not installed; remote input is unavailable");
            }
            Ok(())
        }
        Err(e) => Err(e.into()),
    }
}

fn special_key_code(key: u8) -> Option<u16> {
    Some(match key {
        1 => 14,
        2 => 15,
        3 => 101,
        4 => 105,
        5 => 103,
        6 => 106,
        7 => 108,
        8 => 104,
        9 => 109,
        10 => 102,
        11 => 107,
        12 => 28,
        13 => 111,
        14 => 1,
        15 => 99,
        16 => 70,
        21 => 59,
        22 => 60,
        23 => 61,
        24 => 62,
        25 => 63,
        26 => 64,
        27 => 65,
        28 => 66,
        29 => 67,
        30 => 68,
        31 => 87,
        32 => 88,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_camel_case_mousepad_fields() {
        let request: MousepadRequest = serde_json::from_value(serde_json::json!({
            "specialKey": 12,
            "sendAck": true,
            "super": true
        }))
        .unwrap();

        assert_eq!(request.special_key, Some(12));
        assert_eq!(request.send_ack, Some(true));
        assert_eq!(request.meta, Some(true));
    }

    #[test]
    fn maps_kdeconnect_special_keys_to_linux_input_codes() {
        assert_eq!(special_key_code(1), Some(14));
        assert_eq!(special_key_code(12), Some(28));
        assert_eq!(special_key_code(32), Some(88));
        assert_eq!(special_key_code(99), None);
    }
}
