use std::{
    fmt::Display,
    time::{SystemTime, UNIX_EPOCH},
};

use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const PROTOCOL_VERSION: usize = 8;

pub enum Capabilities {
    Ping,
}

impl From<Capabilities> for String {
    fn from(value: Capabilities) -> Self {
        match value {
            Capabilities::Ping => "kdeconnect.ping".to_string(),
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct ProtocolPacket {
    pub id: Option<u128>,
    #[serde(rename = "type")]
    pub packet_type: String,
    pub body: Value,
    #[serde(rename = "payloadSize")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub payload_size: Option<i64>,
    #[serde(rename = "payloadTransferInfo")]
    #[serde(skip_serializing_if = "Option::is_none")]
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

impl ProtocolPacket {
    pub fn new<S: Into<String>>(t: S, body: Value) -> Self {
        Self {
            id: None,
            packet_type: t.into(),
            body,
            payload_size: None,
            payload_transfer_info: None,
        }
    }

    pub fn from_raw(raw: &[u8]) -> anyhow::Result<Self> {
        let pkt: ProtocolPacket =
            serde_json::from_slice(raw).expect("Failed to parse ProtocolPacket from raw data");
        Ok(pkt)
    }

    pub fn as_raw(&self) -> anyhow::Result<Vec<u8>> {
        let str = serde_json::to_string(self)?;
        Ok(format!("{}\n", str).as_bytes().to_vec())
    }
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

#[derive(Serialize, Deserialize, Copy, Clone, Debug)]
pub struct Pair {
    pub pair: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timestamp: Option<u64>,
}

impl Pair {
    pub fn new(response: bool) -> Self {
        if response {
            let timestamp = Some(
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs(),
            );

            return Pair {
                pair: true,
                timestamp,
            };
        }

        Pair {
            pair: response,
            timestamp: None,
        }
    }
}
