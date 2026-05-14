use std::{
    fmt::Display,
    os::unix::fs::MetadataExt as _,
    path::Path,
    time::{SystemTime, UNIX_EPOCH},
};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::{fs::File, io::AsyncRead};

pub const PROTOCOL_VERSION: usize = 8;

fn deserialize_packet_id<'de, D>(deserializer: D) -> Result<Option<u128>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    match Option::<Value>::deserialize(deserializer)? {
        Some(Value::Number(id)) => id
            .as_u64()
            .map(|id| Some(id as u128))
            .ok_or_else(|| serde::de::Error::custom("packet id must be an unsigned integer")),
        Some(Value::String(id)) => id
            .parse::<u128>()
            .map(Some)
            .map_err(serde::de::Error::custom),
        None => Ok(None),
        Some(_) => Err(serde::de::Error::custom(
            "packet id must be a number or numeric string",
        )),
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
    /// Any packet type not recognized by this implementation.
    /// Stored so it can be logged and gracefully ignored rather than panicking.
    Unknown(String),
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
            "kdeconnect.contacts.response_uids_timestamps"
            | "kdeconnect.contacts.response_all_uids_timestamps" => {
                PacketType::ContactsResponseUidsTimestamps
            }
            "kdeconnect.contacts.request_vcards_by_uid" => PacketType::ContactsRequestVcardsByUid,
            "kdeconnect.contacts.response_vcards" => PacketType::ContactsResponseVcards,
            "kdeconnect.findmyphone.request" => PacketType::FindMyPhoneRequest,
            "kdeconnect.lock" => PacketType::Lock,
            "kdeconnect.lock.request" => PacketType::LockRequest,
            "kdeconnect.mousepad.echo" => PacketType::MousePadEcho,
            "kdeconnect.mousepad.keyboardstate" => PacketType::MousePadKeyboardState,
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
            other => {
                tracing::debug!("Unknown packet type received: {}", other);
                PacketType::Unknown(other.to_string())
            }
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
            PacketType::MousePadKeyboardState => write!(f, "kdeconnect.mousepad.keyboardstate"),
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
            PacketType::Unknown(s) => write!(f, "{}", s),
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct ProtocolPacket {
    #[serde(default, deserialize_with = "deserialize_packet_id")]
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
    pub payload_size: Option<u64>,
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
            id: Some(
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_millis(),
            ),
            packet_type: t,
            body,
            payload_size: None,
            payload_transfer_info: None,
        }
    }

    pub fn new_with_payload(
        t: PacketType,
        body: Value,
        payload_size: u64,
        payload_transfer_info: Option<PacketPayloadTransferInfo>,
    ) -> Self {
        Self {
            id: Some(
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_millis(),
            ),
            packet_type: t,
            body,
            payload_size: Some(payload_size),
            payload_transfer_info,
        }
    }

    pub fn from_raw(raw: &[u8]) -> anyhow::Result<Self> {
        Ok(serde_json::from_slice(raw)?)
    }

    pub fn as_raw(&self) -> anyhow::Result<Vec<u8>> {
        let mut buf = serde_json::to_vec(self)?;
        buf.push(b'\n');
        Ok(buf)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn packet_type_round_trips_known_protocol_names() {
        let cases = [
            (
                "kdeconnect.contacts.request_vcards_by_uid",
                PacketType::ContactsRequestVcardsByUid,
            ),
            (
                "kdeconnect.mousepad.keyboardstate",
                PacketType::MousePadKeyboardState,
            ),
        ];

        for (wire_name, packet_type) in cases {
            assert!(matches!(
                PacketType::from(wire_name.to_string()),
                pt if std::mem::discriminant(&pt) == std::mem::discriminant(&packet_type)
            ));
            assert_eq!(packet_type.to_string(), wire_name);
        }
    }

    #[test]
    fn from_raw_returns_error_for_invalid_json() {
        assert!(ProtocolPacket::from_raw(b"not json\n").is_err());
    }

    #[test]
    fn packet_id_accepts_kdeconnect_string_timestamp() {
        let decoded = ProtocolPacket::from_raw(
            br#"{"id":"1712345678901","type":"kdeconnect.ping","body":{}}"#,
        )
        .unwrap();

        assert_eq!(decoded.id, Some(1_712_345_678_901));
        assert!(matches!(decoded.packet_type, PacketType::Ping));
    }

    #[test]
    fn packet_serialization_uses_newline_delimited_json() {
        let packet = ProtocolPacket::new(PacketType::Ping, json!({"message": "hello"}));
        let raw = packet.as_raw().unwrap();
        assert_eq!(raw.last(), Some(&b'\n'));
        let decoded = ProtocolPacket::from_raw(&raw).unwrap();
        assert!(matches!(decoded.packet_type, PacketType::Ping));
        assert_eq!(decoded.body["message"], "hello");
    }

    #[test]
    fn pair_packets_only_timestamp_new_requests() {
        let request = Pair::request();
        assert!(request.pair);
        assert!(request.timestamp.is_some());

        let accept = Pair::accept();
        assert!(accept.pair);
        assert!(accept.timestamp.is_none());

        let reject = Pair::reject();
        assert!(!reject.pair);
        assert!(reject.timestamp.is_none());
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
    pub fn request() -> Self {
        let timestamp = Some(
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs(),
        );
        Pair {
            pair: true,
            timestamp,
        }
    }

    pub fn accept() -> Self {
        Pair {
            pair: true,
            timestamp: None,
        }
    }

    pub fn reject() -> Self {
        Pair {
            pair: false,
            timestamp: None,
        }
    }
}

pub struct DeviceFile<S: AsyncRead + Sync + Send + Unpin> {
    pub buf: S,
    pub size: u64,
}

pub struct DevicePayload<S: AsyncRead + Sync + Send + Unpin> {
    pub buf: S,
    pub size: u64,
}

impl<S: AsyncRead + Sync + Send + Unpin> From<DeviceFile<S>> for DevicePayload<S> {
    fn from(file: DeviceFile<S>) -> Self {
        Self {
            buf: file.buf,
            size: file.size,
        }
    }
}

impl DeviceFile<File> {
    pub async fn try_from_tokio(file: File) -> anyhow::Result<Self> {
        file.sync_all().await?;
        let metadata = file.metadata().await?;
        Ok(DeviceFile {
            buf: file,
            size: metadata.size(),
        })
    }

    pub async fn open(path: impl AsRef<Path>) -> anyhow::Result<Self> {
        let path: &Path = path.as_ref();
        Self::try_from_tokio(File::open(path).await?).await
    }
}
