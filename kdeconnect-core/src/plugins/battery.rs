use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::path::PathBuf;
use tokio::{fs, sync::mpsc};
use tracing::debug;

use crate::{
    device::{DeviceId, DeviceState},
    event::{ConnectionEvent, CoreEvent},
    plugin_interface::Plugin,
    protocol::{PacketType, ProtocolPacket},
};

fn serialize_threshold<S>(x: &bool, s: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    s.serialize_i32(if *x { 1 } else { 0 })
}

fn deserialize_threshold<'de, D>(deserializer: D) -> Result<bool, D::Error>
where
    D: Deserializer<'de>,
{
    let buf = i32::deserialize(deserializer)?;

    match buf {
        0 => Ok(false),
        1 => Ok(true),
        _ => Err(serde::de::Error::invalid_value(
            serde::de::Unexpected::Signed(buf.into()),
            &"0 or 1",
        )),
    }
}

#[derive(Serialize, Deserialize, Copy, Clone, Debug, Default, PartialEq, Eq)]
pub struct Battery {
    #[serde(rename = "currentCharge")]
    pub charge: i32,
    #[serde(rename = "isCharging")]
    pub is_charging: bool,
    #[serde(
        rename = "thresholdEvent",
        serialize_with = "serialize_threshold",
        deserialize_with = "deserialize_threshold"
    )]
    pub under_threshold: bool,
}

impl Plugin for Battery {
    fn id(&self) -> &'static str {
        "kdeconnect.battery"
    }
}

impl Battery {
    pub async fn received_packet(
        &self,
        device_id: DeviceId,
        event: mpsc::UnboundedSender<ConnectionEvent>,
    ) {
        let _ = event.send(ConnectionEvent::StateUpdated((
            device_id,
            DeviceState::Battery {
                level: self.charge,
                charging: self.is_charging,
            },
        )));
    }
}

pub async fn send_local_state(device: DeviceId, event: mpsc::UnboundedSender<CoreEvent>) {
    let battery = read_local_battery().await.unwrap_or(Battery {
        charge: 100,
        is_charging: false,
        under_threshold: false,
    });

    let packet = ProtocolPacket::new(
        PacketType::Battery,
        serde_json::to_value(battery).unwrap_or_default(),
    );
    let _ = event.send(CoreEvent::SendPacket { device, packet });
}

async fn read_local_battery() -> Option<Battery> {
    let mut entries = fs::read_dir("/sys/class/power_supply").await.ok()?;
    while let Ok(Some(entry)) = entries.next_entry().await {
        let path = entry.path();
        let kind = read_trimmed(path.join("type")).await.unwrap_or_default();
        if kind != "Battery" {
            continue;
        }

        let charge = read_trimmed(path.join("capacity"))
            .await
            .and_then(|value| value.parse::<i32>().ok())
            .unwrap_or(100);
        let status = read_trimmed(path.join("status")).await.unwrap_or_default();
        let is_charging = matches!(status.as_str(), "Charging" | "Full");

        return Some(Battery {
            charge,
            is_charging,
            under_threshold: charge <= 15,
        });
    }

    debug!("[battery] no local battery found; reporting desktop default");
    None
}

async fn read_trimmed(path: PathBuf) -> Option<String> {
    fs::read_to_string(path)
        .await
        .ok()
        .map(|value| value.trim().to_string())
}

#[cfg(test)]
mod tests {
    use super::Battery;

    #[test]
    fn accepts_protocol_no_battery_sentinel() {
        let battery: Battery = serde_json::from_value(serde_json::json!({
            "currentCharge": -1,
            "isCharging": false,
            "thresholdEvent": 0
        }))
        .unwrap();

        assert_eq!(battery.charge, -1);
        assert!(!battery.is_charging);
    }
}
