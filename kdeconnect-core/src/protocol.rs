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

fn serialize_packet_type<S>(pt: &PacketType, serializer: S) -> Result<S::Ok, S::Error>
where
    S: serde::Serializer,
{
    let s: String = pt.to_string();
    serializer.serialize_str(&s)
}

fn deserialize_packet_type<'de, D>(deserializer: D) -> Result<PacketType, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let s: String = String::deserialize(deserializer)?;
    Ok(PacketType::from(s))
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub enum PacketType {
    Battery,
    BatteryRequest,
    Clipboard,
    ClipboardConnect,
    ConnectivityReport,
    ConnectivityReportRequest,
    ContactsRequestAllUidsTimestamps,
    ContactsRequestVcardsByUid,
    ContactsResponseUidsTimestamps,
    ContactsResponseVcards,
    FindMyPhoneRequest,
    Lock,
    LockRequest,
    MousePadEcho,
    MousePadKeyboardState,
    MousePadRequest,
    Mpris,
    MprisRequest,
    Notification,
    NotificationAction,
    NotificationReply,
    NotificationRequest,
    Identity,
    Pair,
    Ping,
    Presenter,
    RunCommand,
    RunCommandRequest,
    Sftp,
    SftpRequest,
    ShareRequest,
    ShareRequestUpdate,
    SmsAttachmentFile,
    SmsMessages,
    SmsRequest,
    SmsRequestAttachment,
    SmsRequestConversation,
    SmsRequestConversations,
    SystemVolume,
    SystemVolumeRequest,
    Telephony,
    TelephonyRequestMute,
}

impl From<String> for PacketType {
    fn from(value: String) -> Self {
        match value.as_str() {
            "kdeconnect.battery" => PacketType::Battery,
            "kdeconnect.battery.request" => PacketType::BatteryRequest,
            "kdeconnect.clipboard" => PacketType::Clipboard,
            "kdeconnect.clipboard.connect" => PacketType::ClipboardConnect,
            "kdeconnect.connectivity_report" => PacketType::ConnectivityReport,
            "kdeconnect.connectivity_report.request" => PacketType::ConnectivityReportRequest,
            "kdeconnect.contacts.request_all_uids_timestamps" => {
                PacketType::ContactsRequestAllUidsTimestamps
            }
            "kdeconnect.contacts.request_vcards_by_uid" => PacketType::ContactsRequestVcardsByUid,
            "kdeconnect.contacts.response_uids_timestamps" => {
                PacketType::ContactsResponseUidsTimestamps
            }
            "kdeconnect.contacts.response_vcards" => PacketType::ContactsResponseVcards,
            "kdeconnect.findmyphone.request" => PacketType::FindMyPhoneRequest,
            "kdeconnect.lock" => PacketType::Lock,
            "kdeconnect.lock.request" => PacketType::LockRequest,
            "kdeconnect.mousepad.echo" => PacketType::MousePadEcho,
            "kdeconnect.mousepad.keyboard_state" => PacketType::MousePadKeyboardState,
            "kdeconnect.mousepad.request" => PacketType::MousePadRequest,
            "kdeconnect.mpris" => PacketType::Mpris,
            "kdeconnect.mpris.request" => PacketType::MprisRequest,
            "kdeconnect.notification" => PacketType::Notification,
            "kdeconnect.notification.action" => PacketType::NotificationAction,
            "kdeconnect.notification.reply" => PacketType::NotificationReply,
            "kdeconnect.notification.request" => PacketType::NotificationRequest,
            "kdeconnect.identity" => PacketType::Identity,
            "kdeconnect.pair" => PacketType::Pair,
            "kdeconnect.ping" => PacketType::Ping,
            "kdeconnect.presenter" => PacketType::Presenter,
            "kdeconnect.runcommand" => PacketType::RunCommand,
            "kdeconnect.runcommand.request" => PacketType::RunCommandRequest,
            "kdeconnect.sftp" => PacketType::Sftp,
            "kdeconnect.sftp.request" => PacketType::SftpRequest,
            "kdeconnect.share.request" => PacketType::ShareRequest,
            "kdeconnect.share.request.update" => PacketType::ShareRequestUpdate,
            "kdeconnect.sms.attachment_file" => PacketType::SmsAttachmentFile,
            "kdeconnect.sms.messages" => PacketType::SmsMessages,
            "kdeconnect.sms.request" => PacketType::SmsRequest,
            "kdeconnect.sms.request_attachment" => PacketType::SmsRequestAttachment,
            "kdeconnect.sms.request_conversation" => PacketType::SmsRequestConversation,
            "kdeconnect.sms.request_conversations" => PacketType::SmsRequestConversations,
            "kdeconnect.systemvolume" => PacketType::SystemVolume,
            "kdeconnect.systemvolume.request" => PacketType::SystemVolumeRequest,
            "kdeconnect.telephony" => PacketType::Telephony,
            "kdeconnect.telephony.request_mute" => PacketType::TelephonyRequestMute,
            _ => panic!("Unknown packet type received: {}", value),
        }
    }
}

impl Display for PacketType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PacketType::Battery => write!(f, "kdeconnect.battery"),
            PacketType::BatteryRequest => write!(f, "kdeconnect.battery.request"),
            PacketType::Clipboard => write!(f, "kdeconnect.clipboard"),
            PacketType::ClipboardConnect => write!(f, "kdeconnect.clipboard.connect"),
            PacketType::ConnectivityReport => write!(f, "kdeconnect.connectivity_report"),
            PacketType::ConnectivityReportRequest => {
                write!(f, "kdeconnect.connectivity_report.request")
            }
            PacketType::ContactsRequestAllUidsTimestamps => {
                write!(f, "kdeconnect.contacts.request_all_uids_timestamps")
            }
            PacketType::ContactsRequestVcardsByUid => {
                write!(f, "kdeconnect.contacts.request_vcards_by_uid")
            }
            PacketType::ContactsResponseUidsTimestamps => {
                write!(f, "kdeconnect.contacts.response_uids_timestamps")
            }
            PacketType::ContactsResponseVcards => {
                write!(f, "kdeconnect.contacts.response_vcards")
            }
            PacketType::FindMyPhoneRequest => write!(f, "kdeconnect.findmyphone.request"),
            PacketType::Lock => write!(f, "kdeconnect.lock"),
            PacketType::LockRequest => write!(f, "kdeconnect.lock.request"),
            PacketType::MousePadEcho => write!(f, "kdeconnect.mousepad.echo"),
            PacketType::MousePadKeyboardState => write!(f, "kdeconnect.mousepad.keyboard_state"),
            PacketType::MousePadRequest => write!(f, "kdeconnect.mousepad.request"),
            PacketType::Mpris => write!(f, "kdeconnect.mpris"),
            PacketType::MprisRequest => write!(f, "kdeconnect.mpris.request"),
            PacketType::Notification => write!(f, "kdeconnect.notification"),
            PacketType::NotificationAction => write!(f, "kdeconnect.notification.action"),
            PacketType::NotificationReply => write!(f, "kdeconnect.notification.reply"),
            PacketType::NotificationRequest => write!(f, "kdeconnect.notification.request"),
            PacketType::Identity => write!(f, "kdeconnect.identity"),
            PacketType::Pair => write!(f, "kdeconnect.pair"),
            PacketType::Ping => write!(f, "kdeconnect.ping"),
            PacketType::Presenter => write!(f, "kdeconnect.presenter"),
            PacketType::RunCommand => write!(f, "kdeconnect.runcommand"),
            PacketType::RunCommandRequest => write!(f, "kdeconnect.runcommand.request"),
            PacketType::Sftp => write!(f, "kdeconnect.sftp"),
            PacketType::SftpRequest => write!(f, "kdeconnect.sftp.request"),
            PacketType::ShareRequest => write!(f, "kdeconnect.share.request"),
            PacketType::ShareRequestUpdate => write!(f, "kdeconnect.share.request.update"),
            PacketType::SmsAttachmentFile => write!(f, "kdeconnect.sms.attachment_file"),
            PacketType::SmsMessages => write!(f, "kdeconnect.sms.messages"),
            PacketType::SmsRequest => write!(f, "kdeconnect.sms.request"),
            PacketType::SmsRequestAttachment => write!(f, "kdeconnect.sms.request_attachment"),
            PacketType::SmsRequestConversation => {
                write!(f, "kdeconnect.sms.request_conversation")
            }
            PacketType::SmsRequestConversations => {
                write!(f, "kdeconnect.sms.request_conversations")
            }
            PacketType::SystemVolume => write!(f, "kdeconnect.systemvolume"),
            PacketType::SystemVolumeRequest => write!(f, "kdeconnect.systemvolume.request"),
            PacketType::Telephony => write!(f, "kdeconnect.telephony"),
            PacketType::TelephonyRequestMute => write!(f, "kdeconnect.telephony.request_mute"),
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct ProtocolPacket {
    pub id: Option<u128>,
    #[serde(rename = "type")]
    #[serde(
        serialize_with = "serialize_packet_type",
        deserialize_with = "deserialize_packet_type"
    )]
    pub packet_type: PacketType,
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
    pub fn new(t: PacketType, body: Value) -> Self {
        Self {
            id: None,
            packet_type: t,
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
