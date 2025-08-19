use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::fmt::Display;

use crate::helpers::packet_id;

pub const PROTOCOL_VERSION: usize = 8;

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Packet {
    pub id: u128,
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

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct IdentityPacket {
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
    pub fn new(device_id: String, device_name: String) -> Self {
        Self {
            device_id,
            device_name,
            device_type: DeviceType::Desktop,
            incoming_capabilities: vec!["kdeconnect.ping".into()],
            outgoing_capabilities: vec!["kdeconnect.ping".into()],
            protocol_version: PROTOCOL_VERSION,
            tcp_port: None,
        }
    }

    pub fn create_packet(&self, port: Option<u16>) -> IdentityPacket {
        let mut body = self.clone();

        if let Some(port) = port {
            body.tcp_port = Some(port);
        }

        IdentityPacket {
            id: packet_id(),
            packet_type: "kdeconnect.identity".into(),
            body,
        }
    }

    pub(crate) fn to_string(&self, port: Option<u16>) -> String {
        serde_json::to_string(&self.create_packet(port)).expect("serializing packet")
    }
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct PairPacket {
    pub id: u128,
    #[serde(rename = "type")]
    pub packet_type: String,
    pub body: Pair,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Pair {
    pair: bool,
}

impl Pair {
    pub fn new(pair: bool) -> Self {
        Self { pair }
    }

    pub fn create_packet(&self) -> PairPacket {
        PairPacket {
            id: packet_id(),
            packet_type: "kdeconnect.pair".into(),
            body: self.clone(),
        }
    }

    pub fn to_string(&self) -> String {
        serde_json::to_string(&self.create_packet()).expect("serializing packet")
    }
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct PingPacket {
    pub id: u128,
    #[serde(rename = "type")]
    pub packet_type: String,
    pub body: Ping,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Ping {
    message: String,
}

impl Ping {
    pub fn create_packet(message: String) -> PingPacket {
        let ping = Ping { message };

        PingPacket {
            id: packet_id(),
            packet_type: "kdeconnect.ping".into(),
            body: ping,
        }
    }
}
