use std::{
    fs,
    io::{Read as _, Write},
    net::SocketAddr,
    time::Duration,
};

use tracing::debug;

use crate::{
    crypto::KeyStore,
    protocol::{Capabilities, DeviceType, Identity, PROTOCOL_VERSION},
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
    pub fn new(out_caps: Vec<String>) -> Self {
        let config_dir = dirs::config_dir()
            .expect("cannot find config dir")
            .join(CONFIG_DIR);

        if !config_dir.exists() {
            fs::create_dir_all(&config_dir).expect("cannot create config dir");
        }

        let id_file = config_dir.join(DEVICE_ID_STORE);

        let identity = match id_file.exists() {
            true => {
                let mut buffer = String::new();
                let mut file = fs::File::open(&id_file).expect("cannot open file");

                file.read_to_string(&mut buffer)
                    .expect("fail reading file content");

                let device_id = buffer.trim().to_string();
                make_identity(device_id, out_caps, Some(DEFAULT_LISTEN_PORT))
            }
            false => {
                let device_id = uuid::Uuid::new_v4().to_string();

                let mut file = fs::File::create(&id_file).expect("cannot create file");
                file.write_all(device_id.as_bytes())
                    .expect("fail writing device id to file");

                make_identity(device_id, out_caps, Some(DEFAULT_LISTEN_PORT))
            }
        };

        debug!("CONFIG initialized.");

        Self {
            device_name: hostname::get()
                .unwrap_or_else(|_| "localhost".into())
                .to_string_lossy()
                .to_string(),
            listen_addr: DEFAULT_LISTEN_ADDR,
            discovery_interval: DEFAULT_DISCOVERY_INTERVAL,
            key_store: KeyStore::new(&identity.device_id).expect("failed to create keystore"),
            identity,
        }
    }
}

fn make_identity(
    device_id: String,
    outgoing_capabilities: Vec<String>,
    tcp_port: Option<u16>,
) -> Identity {
    Identity {
        device_id,
        device_name: hostname::get()
            .unwrap_or_else(|_| "localhost".into())
            .to_string_lossy()
            .to_string(),
        device_type: identify_device_type(),
        incoming_capabilities: vec![Capabilities::Ping.into()],
        outgoing_capabilities,
        protocol_version: PROTOCOL_VERSION,
        tcp_port,
    }
}

fn identify_device_type() -> DeviceType {
    if cfg!(target_os = "linux") {
        // check if /sys/class/dmi/id/chassis_type exists
        if let Ok(mut file) = fs::File::open("/sys/class/dmi/id/chassis_type") {
            let mut contents = String::new();
            if file.read_to_string(&mut contents).is_ok() {
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
