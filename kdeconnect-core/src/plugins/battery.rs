use async_trait::async_trait;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use tokio::sync::mpsc;

use crate::{device::Device, event::ConnectionEvent, plugin_interface::Plugin};

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
    pub charge: u8,
    #[serde(rename = "isCharging")]
    pub is_charging: bool,
    #[serde(
        rename = "thresholdEvent",
        serialize_with = "serialize_threshold",
        deserialize_with = "deserialize_threshold"
    )]
    pub under_threshold: bool,
}

#[async_trait]
impl Plugin for Battery {
    fn id(&self) -> &'static str {
        "kdeconnect.battery"
    }
    async fn received(
        &self,
        _device: &Device,
        event: mpsc::UnboundedSender<ConnectionEvent>,
        _core_event: mpsc::UnboundedSender<crate::event::CoreEvent>,
    ) {
        event
            .send(ConnectionEvent::StateUpdated(
                crate::event::DeviceState::Battery {
                    level: self.charge,
                    charging: self.is_charging,
                },
            ))
            .unwrap();
    }

    async fn send(
        &self,
        _device: &Device,
        _core_event: mpsc::UnboundedSender<crate::event::CoreEvent>,
    ) {
        // Battery plugin does not send events on its own
    }
}
