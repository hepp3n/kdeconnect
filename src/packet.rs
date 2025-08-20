/// src: https://github.com/r58Playz/kdeconnect/blob/main/kdeconnect/src/packets.rs
///
///
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::fmt::Display;

pub const PROTOCOL_VERSION: usize = 8;

#[allow(dead_code)]
pub const ALL_CAPABILITIES: &[&str] = &[Ping::TYPE, RunCommand::TYPE, RunCommandRequest::TYPE];

macro_rules! derive_type {
    ($struct:ty, $type:literal) => {
        impl PacketType for $struct {
            fn get_type_self(&self) -> &'static str {
                $type
            }
        }
        #[allow(dead_code)]
        impl $struct {
            pub const TYPE: &'static str = $type;
        }
    };
}

pub(crate) trait PacketType {
    fn get_type_self(&self) -> &'static str;
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Packet {
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
derive_type!(Identity, "kdeconnect.identity");

#[derive(Serialize, Deserialize, Copy, Clone, Debug)]
pub struct Pair {
    pub pair: bool,
    pub timestamp: u64,
}
derive_type!(Pair, "kdeconnect.pair");

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Ping {
    pub message: Option<String>,
}
derive_type!(Ping, "kdeconnect.ping");

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct RunCommand {
    #[serde(rename = "commandList")]
    pub command_list: String,
}
derive_type!(RunCommand, "kdeconnect.runcommand");

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct RunCommandItem {
    pub name: String,
    pub command: String,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct RunCommandRequest {
    pub key: Option<String>,
    #[serde(rename = "requestCommandList")]
    pub request_command_list: Option<bool>,
}
derive_type!(RunCommandRequest, "kdeconnect.runcommand.request");

// to_value should never fail, as Serialize will always be successful and packets should never
// contain non-string keys anyway
#[macro_export]
macro_rules! make_packet {
    ($packet:ident) => {
        Packet {
            packet_type: $packet.get_type_self().to_string(),
            body: serde_json::value::to_value($packet).expect("packet was invalid"),
            payload_size: None,
            payload_transfer_info: None,
        }
    };
}

#[macro_export]
macro_rules! make_packet_payload {
    ($packet:ident, $payload_size:expr, $payload_port:expr) => {
        Packet {
            packet_type: $packet.get_type_self().to_string(),
            body: serde_json::value::to_value($packet).expect("packet was invalid"),
            payload_size: Some($payload_size),
            payload_transfer_info: Some(PacketPayloadTransferInfo {
                port: $payload_port,
            }),
        }
    };
}

#[macro_export]
macro_rules! make_packet_str {
    ($packet:ident) => {
        serde_json::to_string(&make_packet!($packet)).map(|x| x + "\n")
    };
}

#[macro_export]
macro_rules! make_packet_str_payload {
    ($packet:ident, $payload_size:expr, $payload_port:expr) => {
        serde_json::to_string(&make_packet_payload!($packet, $payload_size, $payload_port))
            .map(|x| x + "\n")
    };
}
