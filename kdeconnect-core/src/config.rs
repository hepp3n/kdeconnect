use std::{net::SocketAddr, time::Duration};

use tokio::{
    fs,
    io::{AsyncReadExt, AsyncWriteExt},
};
use tracing::debug;
use x509_parser::prelude::*;

use crate::{
    crypto::KeyStore,
    protocol::{DeviceType, Identity, PROTOCOL_VERSION},
    transport::{DEFAULT_DISCOVERY_INTERVAL, DEFAULT_LISTEN_ADDR, DEFAULT_LISTEN_PORT},
};

pub const CONFIG_DIR: &str = "kdeconnect";
pub const DEVICE_ID_STORE: &str = "device_id";

/// Packet types we can RECEIVE from the phone (phone's outgoingCapabilities must overlap these).
/// The phone checks this list before sending unsolicited data (battery, SMS messages, etc.).
const INCOMING_CAPABILITIES: &[&str] = &[
    "kdeconnect.battery",
    "kdeconnect.clipboard",
    "kdeconnect.clipboard.connect",
    "kdeconnect.connectivity_report",
    "kdeconnect.contacts.response_uids_timestamps",
    "kdeconnect.contacts.response_vcards",
    "kdeconnect.findmyphone.request",
    "kdeconnect.mousepad.echo",
    "kdeconnect.mousepad.keyboardstate",
    "kdeconnect.mousepad.request",
    "kdeconnect.mpris",
    "kdeconnect.mpris.request",
    "kdeconnect.notification",
    "kdeconnect.ping",
    "kdeconnect.presenter",
    "kdeconnect.runcommand.request",
    "kdeconnect.share.request",
    "kdeconnect.sftp",
    "kdeconnect.sms.messages",
    "kdeconnect.sms.attachment_file",
    "kdeconnect.systemvolume.request",
    "kdeconnect.telephony",
];

const OUTGOING_CAPABILITIES: &[&str] = &[
    "kdeconnect.battery.request",
    "kdeconnect.clipboard",
    "kdeconnect.contacts.request_all_uids_timestamps",
    "kdeconnect.contacts.request_vcards_by_uid",
    "kdeconnect.connectivity_report.request",
    "kdeconnect.findmyphone.request",
    "kdeconnect.mousepad.echo",
    "kdeconnect.mousepad.keyboardstate",
    "kdeconnect.mousepad.request",
    "kdeconnect.mpris",
    "kdeconnect.mpris.request",
    "kdeconnect.notification.request",
    "kdeconnect.notification",
    "kdeconnect.notification.action",
    "kdeconnect.notification.reply",
    "kdeconnect.ping",
    "kdeconnect.runcommand",
    "kdeconnect.share.request",
    "kdeconnect.share.request.update",
    "kdeconnect.sftp.request",
    "kdeconnect.sms.request",
    "kdeconnect.sms.request_conversations",
    "kdeconnect.sms.request_conversation",
    "kdeconnect.sms.request_attachment",
    "kdeconnect.systemvolume",
];

#[derive(Debug)]
pub struct Config {
    pub device_name: String,
    pub listen_addr: SocketAddr,
    pub discovery_interval: Duration,
    pub key_store: KeyStore,
    pub identity: Identity,
}

impl Config {
    pub async fn load(_out_caps: Vec<String>) -> anyhow::Result<Self> {
        let config_dir = dirs::config_dir()
            .expect("cannot find config dir")
            .join(CONFIG_DIR);

        if !config_dir.exists() {
            fs::create_dir_all(&config_dir)
                .await
                .expect("cannot create config dir");
        }

        let id_file = config_dir.join(DEVICE_ID_STORE);
        let stored_device_id = if id_file.exists() {
            let mut buffer = String::new();
            let mut file = fs::File::open(&id_file).await?;
            file.read_to_string(&mut buffer).await?;
            Some(buffer.trim().to_string())
        } else {
            None
        };

        let cert_device_id = certificate_common_name(&config_dir.join("certificate.pem")).await;
        let device_id = cert_device_id
            .or(stored_device_id)
            .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());

        if fs::read_to_string(&id_file).await.ok().as_deref().map(str::trim) != Some(device_id.as_str()) {
            let mut file = fs::File::create(&id_file).await?;
            file.write_all(device_id.as_bytes()).await?;
        }

        let identity = make_identity(device_id, Some(DEFAULT_LISTEN_PORT)).await;

        debug!("CONFIG initialized.");

        Ok(Self {
            device_name: hostname::get()
                .unwrap_or_else(|_| "localhost".into())
                .to_string_lossy()
                .to_string(),
            listen_addr: DEFAULT_LISTEN_ADDR,
            discovery_interval: DEFAULT_DISCOVERY_INTERVAL,
            key_store: KeyStore::load(&identity.device_id)
                .await
                .expect("failed to create keystore"),
            identity,
        })
    }
}

async fn certificate_common_name(path: &std::path::Path) -> Option<String> {
    let bytes = fs::read(path).await.ok()?;
    let (_, pem) = parse_x509_pem(&bytes).ok()?;
    let cert = pem.parse_x509().ok()?;
    cert.subject()
        .iter_common_name()
        .next()
        .and_then(|cn| cn.as_str().ok())
        .map(str::to_string)
        .filter(|id| !id.is_empty())
}

async fn make_identity(device_id: String, tcp_port: Option<u16>) -> Identity {
    Identity {
        device_id,
        device_name: hostname::get()
            .unwrap_or_else(|_| "localhost".into())
            .to_string_lossy()
            .to_string(),
        device_type: identify_device_type().await,
        incoming_capabilities: INCOMING_CAPABILITIES
            .iter()
            .map(|s| s.to_string())
            .collect(),
        outgoing_capabilities: OUTGOING_CAPABILITIES
            .iter()
            .map(|s| s.to_string())
            .collect(),
        protocol_version: PROTOCOL_VERSION,
        tcp_port,
    }
}

async fn identify_device_type() -> DeviceType {
    if cfg!(target_os = "linux") {
        if let Ok(mut file) = fs::File::open("/sys/class/dmi/id/chassis_type").await {
            let mut contents = String::new();
            if file.read_to_string(&mut contents).await.is_ok() {
                let chassis_type: u8 = contents.trim().parse().unwrap_or(0);
                match chassis_type {
                    8 | 9 | 10 | 14 => DeviceType::Laptop,
                    _ => DeviceType::Desktop,
                }
            } else {
                DeviceType::Desktop
            }
        } else {
            DeviceType::Desktop
        }
    } else {
        DeviceType::Desktop
    }
}
