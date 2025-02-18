use serde::{Deserialize, Deserializer, Serialize};
use serde_json::Value;
use std::{fmt::Display, str::FromStr, time};

pub const PROTOCOL_VERSION: usize = 7;

struct DeserializeIDVisitor;

impl serde::de::Visitor<'_> for DeserializeIDVisitor {
    type Value = u128;

    fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
        formatter.write_str("an u128 or a string")
    }

    fn visit_u64<E>(self, v: u64) -> Result<Self::Value, E>
    where
        E: serde::de::Error,
    {
        Ok(v as u128)
    }

    fn visit_u128<E>(self, v: u128) -> Result<Self::Value, E>
    where
        E: serde::de::Error,
    {
        Ok(v)
    }

    fn visit_str<E>(self, v: &str) -> Result<Self::Value, E>
    where
        E: serde::de::Error,
    {
        FromStr::from_str(v).map_err(serde::de::Error::custom)
    }
}

fn deserialize_id<'de, D>(deserializer: D) -> Result<u128, D::Error>
where
    D: Deserializer<'de>,
{
    deserializer.deserialize_any(DeserializeIDVisitor)
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Packet {
    // kdeconnect-kde set this to a string but it's supposed to be an int... :(
    // kdeconnect-android follows the protocol!! so we crash!!
    // so we coerce to a u128
    #[serde(deserialize_with = "deserialize_id")]
    pub id: u128,
    #[serde(rename = "type")]
    pub packet_type: String,
    pub body: Value,
    #[serde(rename = "payloadSize")]
    pub payload_size: Option<i64>,
    #[serde(rename = "payloadTransferInfo")]
    pub payload_transfer_info: Option<PacketPayloadTransferInfo>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct PacketPayloadTransferInfo {
    pub port: u16,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug)]
#[serde(rename_all = "lowercase")]
pub enum DeviceType {
    Desktop,
    Laptop,
    Phone,
    Tablet,
    Tv,
}

impl Display for DeviceType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DeviceType::Desktop => write!(f, "desktop"),
            DeviceType::Laptop => write!(f, "laptop"),
            DeviceType::Phone => write!(f, "phone"),
            DeviceType::Tablet => write!(f, "tablet"),
            DeviceType::Tv => write!(f, "tv"),
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct IdentityPacket {
    #[serde(deserialize_with = "deserialize_id")]
    pub id: u128,
    #[serde(rename = "type")]
    pub packet_type: String,
    pub body: Identity,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Identity {
    pub device_id: String,
    pub device_name: String,
    pub device_type: DeviceType,
    pub incoming_capabilities: Vec<String>,
    pub outgoing_capabilities: Vec<String>,
    pub protocol_version: usize,
    pub tcp_port: Option<u16>,
}

impl Identity {
    pub fn new(device_id: String, device_name: String, tcp_port: Option<u16>) -> Self {
        Self {
            device_id,
            device_name,
            device_type: DeviceType::Desktop,
            incoming_capabilities: vec!["kdeconnect.ping".into()],
            outgoing_capabilities: vec!["kdeconnect.ping".into()],
            protocol_version: PROTOCOL_VERSION,
            tcp_port,
        }
    }

    pub fn create_packet(&mut self, port: Option<u16>) -> IdentityPacket {
        if let Some(port) = port {
            self.tcp_port = Some(port);
        }

        let id = time::SystemTime::now()
            .duration_since(time::SystemTime::UNIX_EPOCH)
            .expect("time went backwards")
            .as_millis();

        IdentityPacket {
            id,
            packet_type: "kdeconnect.identity".into(),
            body: self.to_owned(),
        }
    }
}
