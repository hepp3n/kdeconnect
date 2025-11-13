use std::sync::Arc;

use async_trait::async_trait;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use tokio::sync::mpsc;

use crate::{
    device::{Device, DeviceId},
    event::ConnectionEvent,
    plugin_interface::Plugin,
    protocol::ProtocolPacket,
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

#[derive(Serialize, Deserialize, Copy, Clone, Debug, PartialEq, Eq)]
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

pub struct BatteryPlugin {}

impl BatteryPlugin {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl Plugin for BatteryPlugin {
    fn id(&self) -> &'static str {
        "kdeconnect.ping"
    }

    async fn handle_packet(
        &self,
        _device: Device,
        packet: ProtocolPacket,
        tx: Arc<mpsc::UnboundedSender<ConnectionEvent>>,
    ) {
        if packet.packet_type == "kdeconnect.battery"
            && let Ok(bat_body) = serde_json::from_value::<Battery>(packet.body)
        {
            let _ = tx.send(ConnectionEvent::StateUpdated(
                crate::event::DeviceState::Battery {
                    level: bat_body.charge as u8,
                    charging: bat_body.is_charging,
                },
            ));
        }
    }

    async fn send_packet(&self, _device: &DeviceId, _protocol_packet: ProtocolPacket) {}
}
