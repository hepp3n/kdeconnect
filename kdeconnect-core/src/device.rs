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

use crate::{config::CONFIG_DIR, event::CoreEvent, transport::DEFAULT_LISTEN_PORT};

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
    pub address: SocketAddr,
    pub pair_state: PairState,
}

impl Default for Device {
    fn default() -> Self {
        Self {
            name: String::new(),
            device_id: DeviceId(String::new()),
            address: SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::LOCALHOST, DEFAULT_LISTEN_PORT)),
            pair_state: PairState::default(),
        }
    }
}

impl Device {
    pub async fn new(id: String, name: String, address: SocketAddr) -> anyhow::Result<Self> {
        let device_id = DeviceId(id);
        let config_dir = dirs::config_dir().unwrap().join(CONFIG_DIR);
        let file_path = config_dir.join(format!("{}.ron", &device_id));

        if file_path.exists() {
            let mut buffer = String::new();
            let mut file = fs::File::open(file_path).await?;
            file.read_to_string(&mut buffer).await?;

            return Ok(ron::de::from_str::<Self>(&buffer)?);
        }

        Ok(Self {
            name,
            device_id,
            address,
            pair_state: PairState::NotPaired,
        })
    }

    pub async fn store_device_identity(&self, pair_state: PairState) -> anyhow::Result<()> {
        let mut data = self.clone();
        data.pair_state = pair_state;

        if let Ok(file_content) = ron::ser::to_string_pretty(&data, PrettyConfig::new()) {
            let config_dir = dirs::config_dir().unwrap().join(CONFIG_DIR);
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
    event_tx: mpsc::UnboundedSender<CoreEvent>,
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
}
