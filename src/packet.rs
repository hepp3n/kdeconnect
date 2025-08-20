/// src: https://github.com/r58Playz/kdeconnect/blob/main/kdeconnect/src/packets.rs
///
///
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::Value;
use std::{collections::HashMap, fmt::Display};

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

#[derive(Serialize, Deserialize, Copy, Clone, Debug)]
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
derive_type!(Battery, "kdeconnect.battery");

#[derive(Serialize, Deserialize, Copy, Clone, Debug)]
pub struct BatteryRequest {
    pub request: bool,
}
derive_type!(BatteryRequest, "kdeconnect.battery.request");

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Clipboard {
    pub content: String,
}
derive_type!(Clipboard, "kdeconnect.clipboard");

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct ClipboardConnect {
    pub content: String,
    pub timestamp: u128,
}
derive_type!(ClipboardConnect, "kdeconnect.clipboard.connect");

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct ConnectivityReport {
    pub signal_strengths: HashMap<String, ConnectivityReportSignal>,
}
derive_type!(ConnectivityReport, "kdeconnect.connectivity_report");

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct ConnectivityReportSignal {
    pub network_type: ConnectivityReportNetworkType,
    pub signal_strength: i32,
}

#[derive(Serialize, Deserialize, Copy, Clone, Debug)]
pub enum ConnectivityReportNetworkType {
    #[serde(rename = "GSM")]
    Gsm,
    #[serde(rename = "CDMA")]
    Cdma,
    #[serde(rename = "iDEN")]
    Iden,
    #[serde(rename = "UMTS")]
    Umts,
    #[serde(rename = "CDMA2000")]
    Cdma2000,
    #[serde(rename = "EDGE")]
    Edge,
    #[serde(rename = "GPRS")]
    Gprs,
    #[serde(rename = "HSPA")]
    Hspa,
    #[serde(rename = "LTE")]
    Lte,
    #[serde(rename = "5G")]
    FiveG,
    #[serde(rename = "Unknown")]
    Unknown,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct SystemVolumeStream {
    pub name: String,
    pub description: String,
    pub enabled: Option<bool>,
    pub muted: bool,
    pub max_volume: Option<i32>,
    pub volume: i32,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct SystemVolumeRequest {
    // this may happen again
    #[serde(skip_serializing_if = "Option::is_none")]
    pub request_sinks: Option<bool>,
    pub name: Option<String>,
    pub enabled: Option<bool>,
    pub muted: Option<bool>,
    pub volume: Option<i32>,
}
derive_type!(SystemVolumeRequest, "kdeconnect.systemvolume.request");

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(untagged)]
pub enum Mpris {
    List {
        #[serde(rename = "playerList")]
        player_list: Vec<String>,
        #[serde(rename = "supportAlbumArtPayload")]
        supports_album_art_payload: bool,
    },
    TransferringArt {
        player: String,
        #[serde(rename = "albumArtUrl")]
        album_art_url: String,
        #[serde(rename = "transferringAlbumArt")]
        transferring_album_art: bool,
    },
    Info(MprisPlayer),
}
derive_type!(Mpris, "kdeconnect.mpris");

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct MprisPlayer {
    pub player: String,
    pub title: Option<String>,
    pub artist: Option<String>,
    pub album: Option<String>,
    #[serde(rename = "isPlaying")]
    pub is_playing: Option<bool>,
    #[serde(rename = "canPause")]
    pub can_pause: Option<bool>,
    #[serde(rename = "canPlay")]
    pub can_play: Option<bool>,
    #[serde(rename = "canGoNext")]
    pub can_go_next: Option<bool>,
    #[serde(rename = "canGoPrevious")]
    pub can_go_previous: Option<bool>,
    #[serde(rename = "canSeek")]
    pub can_seek: Option<bool>,
    #[serde(rename = "loopStatus")]
    pub loop_status: Option<MprisLoopStatus>,
    pub shuffle: Option<bool>,
    pub pos: Option<i32>,
    pub length: Option<i32>,
    pub volume: Option<i32>,
    #[serde(rename = "albumArtUrl")]
    pub album_art_url: Option<String>,
    // undocumented kdeconnect-kde field
    pub url: Option<String>,
}

#[derive(Serialize, Deserialize, Copy, Clone, Debug)]
pub enum MprisLoopStatus {
    None,
    Track,
    Playlist,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(untagged)]
pub enum MprisRequest {
    List {
        #[serde(rename = "requestPlayerList")]
        request_player_list: bool,
    },
    PlayerRequest {
        player: String,
        #[serde(rename = "requestNowPlaying")]
        request_now_playing: Option<bool>,
        #[serde(rename = "requestVolume")]
        request_volume: Option<bool>,
        // set to a file:// string to get kdeconnect-kde to send (local) album art
        #[serde(rename = "albumArtUrl", skip_serializing_if = "Option::is_none")]
        request_album_art: Option<String>,
    },
    Action(MprisRequestAction),
}
derive_type!(MprisRequest, "kdeconnect.mpris.request");

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct MprisRequestAction {
    pub player: String,
    // ????
    #[serde(rename = "Seek", skip_serializing_if = "Option::is_none")]
    pub seek: Option<i64>,
    #[serde(rename = "setVolume", skip_serializing_if = "Option::is_none")]
    pub set_volume: Option<i64>,
    #[serde(rename = "setLoopStatus", skip_serializing_if = "Option::is_none")]
    pub set_loop_status: Option<MprisLoopStatus>,
    // ??????
    #[serde(rename = "SetPosition", skip_serializing_if = "Option::is_none")]
    pub set_position: Option<i64>,
    #[serde(rename = "setShuffle", skip_serializing_if = "Option::is_none")]
    pub set_shuffle: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub action: Option<MprisAction>,
}

#[derive(Serialize, Deserialize, Copy, Clone, Debug)]
pub enum MprisAction {
    Play,
    Pause,
    PlayPause,
    Stop,
    Next,
    Previous,
}

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
