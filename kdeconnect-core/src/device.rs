use ron::ser::PrettyConfig;
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    fmt::Display,
    net::{Ipv4Addr, SocketAddr, SocketAddrV4},
    sync::Arc,
};
use tokio::{
    fs,
    io::{AsyncReadExt, AsyncWriteExt},
    sync::{
        RwLock,
        mpsc::{self},
    },
};
use tracing::info;

use crate::{
    config::CONFIG_DIR,
    event::CoreEvent,
    transport::DEFAULT_LISTEN_PORT,
};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub struct DeviceId(pub String);

impl Display for DeviceId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

#[derive(Debug, Clone)]
pub enum DeviceState {
    Battery { level: u8, charging: bool },
    Connectivity((String, i32)),
}

#[derive(Debug, Default, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum PairState {
    #[default]
    NotPaired,
    Requesting,
    Requested,
    Paired,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Device {
    pub name: String,
    pub device_id: DeviceId,
    #[serde(default = "default_device_type")]
    pub device_type: String,
    #[serde(default)]
    pub incoming_capabilities: Vec<String>,
    #[serde(default)]
    pub outgoing_capabilities: Vec<String>,
    pub address: SocketAddr,
    pub pair_state: PairState,
    #[serde(default = "default_protocol_version")]
    pub protocol_version: usize,
    /// UNIX timestamp (seconds) from the most recent pairing handshake,
    /// used for clock-sync validation per the KDE Connect protocol v8+.
    #[serde(default)]
    pub pairing_timestamp: u64,
}

fn default_protocol_version() -> usize {
    0
}

fn default_device_type() -> String {
    "phone".to_string()
}

impl Default for Device {
    fn default() -> Self {
        Self {
            name: String::new(),
            device_id: DeviceId(String::new()),
            device_type: default_device_type(),
            incoming_capabilities: Vec::new(),
            outgoing_capabilities: Vec::new(),
            address: SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::LOCALHOST, DEFAULT_LISTEN_PORT)),
            pair_state: PairState::default(),
            protocol_version: default_protocol_version(),
            pairing_timestamp: 0,
        }
    }
}

impl Device {
    pub async fn new(
        id: String,
        name: String,
        device_type: String,
        incoming_capabilities: Vec<String>,
        outgoing_capabilities: Vec<String>,
        address: SocketAddr,
    ) -> anyhow::Result<Self> {
        let device_id = DeviceId(validate_device_id(&id)?);
        let name = sanitize_device_name(&name);
        let config_dir = dirs::config_dir()
            .ok_or_else(|| anyhow::anyhow!("cannot find config dir"))?
            .join(CONFIG_DIR);
        let file_path = config_dir.join(format!("{}.ron", &device_id));

        if file_path.exists() {
            let mut buffer = String::new();
            let mut file = fs::File::open(file_path).await?;
            file.read_to_string(&mut buffer).await?;

            let mut device = ron::de::from_str::<Self>(&buffer)?;
            device.name = name;
            device.address = address;
            device.device_type = device_type;
            device.incoming_capabilities = incoming_capabilities;
            device.outgoing_capabilities = outgoing_capabilities;
            return Ok(device);
        }

        Ok(Self {
            name,
            device_id,
            device_type,
            incoming_capabilities,
            outgoing_capabilities,
            address,
            pair_state: PairState::NotPaired,
            protocol_version: 0,
            pairing_timestamp: 0,
        })
    }

    pub async fn store_device_identity(&self, pair_state: PairState) -> anyhow::Result<()> {
        let mut data = self.clone();
        data.pair_state = pair_state;

        if let Ok(file_content) = ron::ser::to_string_pretty(&data, PrettyConfig::new()) {
            let config_dir = dirs::config_dir()
                .ok_or_else(|| anyhow::anyhow!("cannot find config dir"))?
                .join(CONFIG_DIR);
            let file_path = config_dir.join(format!("{}.ron", self.device_id));

            if file_path.exists() {
                fs::remove_file(&file_path).await?
            }

            let mut file = fs::File::create(file_path).await?;
            file.write_all(file_content.as_bytes()).await?;
        }
        Ok(())
    }

    pub async fn update_pair_state(&self, state: PairState) {
        let _ = self.store_device_identity(state).await;
    }
}

#[derive(Debug, Clone)]
pub struct DeviceManager {
    devices: Arc<RwLock<HashMap<DeviceId, Device>>>,
    pub(crate) event_tx: mpsc::UnboundedSender<CoreEvent>,
}

impl DeviceManager {
    pub fn new(event_tx: mpsc::UnboundedSender<CoreEvent>) -> Self {
        Self {
            devices: Default::default(),
            event_tx,
        }
    }

    pub async fn add_or_update_device(&self, device_id: DeviceId, device: Device) {
        info!("updating: {}", device_id);
        let mut guard = self.devices.write().await;
        guard.entry(device_id).insert_entry(device.clone());
    }

    pub async fn get_device(&self, id: &DeviceId) -> Option<Device> {
        let guard = self.devices.read().await;
        guard.get(id).cloned()
    }

    pub async fn get_devices(&self) -> Vec<Device> {
        let guard = self.devices.read().await;
        guard.values().cloned().collect()
    }

    pub async fn set_paired(&self, id: &DeviceId, flag: bool) {
        let mut guard = self.devices.write().await;

        if let Some(device) = guard.get_mut(id) {
            if flag {
                device.pair_state = PairState::Paired;
                let _ = self.event_tx.send(CoreEvent::DevicePaired((
                    device.device_id.clone(),
                    device.clone(),
                )));
                let _ = device.update_pair_state(PairState::Paired).await;
            } else {
                device.pair_state = PairState::NotPaired;
                let _ = self
                    .event_tx
                    .send(CoreEvent::DevicePairCancelled(device.device_id.clone()));
                let _ = device.update_pair_state(PairState::NotPaired).await;
            }
        }
    }

    pub async fn update_pair_state(&self, id: &DeviceId, state: PairState) {
        let mut guard = self.devices.write().await;

        if let Some(device) = guard.get_mut(id) {
            device.pair_state = state;
            let _ = self
                .event_tx
                .send(CoreEvent::DevicePairStateChanged((id.clone(), state)));
            let _ = device.update_pair_state(state).await;
        }
    }

    pub async fn set_protocol_version(&self, id: &DeviceId, version: usize) {
        let mut guard = self.devices.write().await;
        if let Some(device) = guard.get_mut(id) {
            device.protocol_version = version;
        }
    }

    pub async fn set_pairing_timestamp(&self, id: &DeviceId, timestamp: u64) {
        let mut guard = self.devices.write().await;
        if let Some(device) = guard.get_mut(id) {
            device.pairing_timestamp = timestamp;
        }
    }

    pub async fn get_pairing_timestamp(&self, id: &DeviceId) -> u64 {
        let guard = self.devices.read().await;
        guard
            .get(id)
            .map(|d| d.pairing_timestamp)
            .unwrap_or(0)
    }
}

/// Validate a device ID per the KDE Connect protocol.
/// Must match `^[a-zA-Z0-9_-]{32,38}$` (32-38 alphanumeric chars, hyphens, underscores).
pub fn validate_device_id(id: &str) -> anyhow::Result<String> {
    if id.len() < 32 || id.len() > 38 || !id.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-') {
        return Err(anyhow::anyhow!(
            "Invalid device ID '{}': must be 32-38 alphanumeric characters, underscores, or hyphens",
            id
        ));
    }
    Ok(id.to_string())
}

/// Sanitize a device name by removing forbidden characters.
/// Valid names contain only characters that are NOT in the set: `"',;:.!?()[]<>`
/// and must be 1-32 characters after trimming.
pub fn sanitize_device_name(name: &str) -> String {
    let sanitized: String = name
        .chars()
        .filter(|c| !matches!(c, '"' | '\'' | ',' | ';' | ':' | '.' | '!' | '?' | '(' | ')' | '[' | ']' | '<' | '>'))
        .collect::<String>()
        .trim()
        .to_string();

    if sanitized.is_empty() {
        "Unknown Device".to_string()
    } else if sanitized.len() > 32 {
        sanitized[..32].to_string()
    } else {
        sanitized
    }
}
