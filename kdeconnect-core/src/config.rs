use std::{net::SocketAddr, time::Duration};

use tokio::{
    fs,
    io::{AsyncReadExt, AsyncWriteExt},
};
use tracing::debug;

use crate::{
    crypto::KeyStore,
    protocol::{DeviceType, Identity, PROTOCOL_VERSION},
    transport::{DEFAULT_DISCOVERY_INTERVAL, DEFAULT_LISTEN_ADDR, DEFAULT_LISTEN_PORT},
};
pub const CONFIG_DIR: &str = "kdeconnect";
pub const DEVICE_ID_STORE: &str = "device_id";

#[derive(Debug)]
pub struct Config {
    pub device_name: String,
    pub listen_addr: SocketAddr,
    pub discovery_interval: Duration,
    pub key_store: KeyStore,
    pub identity: Identity,
}

impl Config {
    pub async fn load(out_caps: Vec<String>) -> anyhow::Result<Self> {
        let config_dir = dirs::config_dir()
            .expect("cannot find config dir")
            .join(CONFIG_DIR);

        if !config_dir.exists() {
            fs::create_dir_all(&config_dir)
                .await
                .expect("cannot create config dir");
        }

        let id_file = config_dir.join(DEVICE_ID_STORE);

        let identity = match id_file.exists() {
            true => {
                let mut buffer = String::new();
                let mut file = fs::File::open(&id_file).await.expect("cannot open file");

                file.read_to_string(&mut buffer)
                    .await
                    .expect("fail reading file content");

                let device_id = buffer.trim().to_string();
                make_identity(device_id, out_caps, Some(DEFAULT_LISTEN_PORT)).await
            }
            false => {
                let device_id = uuid::Uuid::new_v4().to_string();

                let mut file = fs::File::create(&id_file)
                    .await
                    .expect("cannot create file");
                file.write_all(device_id.as_bytes())
                    .await
                    .expect("fail writing device id to file");

                make_identity(device_id, out_caps, Some(DEFAULT_LISTEN_PORT)).await
            }
        };

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

async fn make_identity(
    device_id: String,
    capabilities: Vec<String>,
    tcp_port: Option<u16>,
) -> Identity {
    Identity {
        device_id,
        device_name: hostname::get()
            .unwrap_or_else(|_| "localhost".into())
            .to_string_lossy()
            .to_string(),
        device_type: identify_device_type().await,
        incoming_capabilities: capabilities.clone(),
        outgoing_capabilities: capabilities,
        protocol_version: PROTOCOL_VERSION,
        tcp_port,
    }
}

async fn identify_device_type() -> DeviceType {
    if cfg!(target_os = "linux") {
        // check if /sys/class/dmi/id/chassis_type exists
        if let Ok(mut file) = fs::File::open("/sys/class/dmi/id/chassis_type").await {
            let mut contents = String::new();
            if file.read_to_string(&mut contents).await.is_ok() {
                let chassis_type: u8 = contents.trim().parse().unwrap_or(0);
                match chassis_type {
                    8 | 9 | 10 | 14 => DeviceType::Laptop, // Portable, Laptop, Notebook, SubNotebook
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
