use serde::{Deserialize, Serialize};
use std::{
    fs,
    io::{Read, Write},
    path::PathBuf,
};

use crate::{
    cert::{generate_cert_and_keypair, CERTIFICATE, PRIVATE_KEY},
    utils::{generate_device_id, get_default_devicename},
};

const CONFIG_FILE: &str = "config.ron";

#[derive(Debug, Deserialize, Serialize)]
pub struct KdeConnectConfig {
    pub device_id: String,
    pub device_name: String,
    pub root_ca: PathBuf,
    pub priv_key: PathBuf,
}

impl Default for KdeConnectConfig {
    fn default() -> Self {
        let path = dirs::config_dir()
            .expect("cant find config directory")
            .join("kdeconnect");

        let _ = fs::create_dir_all(&path);

        let device_id = generate_device_id();
        let device_name = get_default_devicename();

        let root_ca = path.join(CERTIFICATE);
        let priv_key = path.join(PRIVATE_KEY);

        if !root_ca.exists() && !priv_key.exists() {
            let _ = generate_cert_and_keypair(&device_name, &device_id, &path);
        }

        if path.join(CONFIG_FILE).exists() {
            let mut buffer = String::new();
            let mut file = fs::File::open(path.join(CONFIG_FILE)).unwrap();
            let _ = file.read_to_string(&mut buffer);

            let x: KdeConnectConfig = ron::from_str(&buffer).unwrap();

            return x;
        }

        let kdeconnect_config = KdeConnectConfig {
            device_id,
            device_name,
            root_ca,
            priv_key,
        };

        if !path.join(CONFIG_FILE).exists() {
            kdeconnect_config.store_config().unwrap();
        }

        kdeconnect_config
    }
}

impl KdeConnectConfig {
    pub fn store_config(&self) -> anyhow::Result<()> {
        let path = dirs::config_dir()
            .expect("cant find config directory")
            .join("kdeconnect");

        let _ = fs::create_dir_all(&path);

        let cfg_to_str = ron::ser::to_string_pretty(&self, ron::ser::PrettyConfig::default())?;

        let mut config_file = fs::File::create(path.join(CONFIG_FILE))?;
        config_file.write_all(cfg_to_str.as_bytes())?;

        Ok(())
    }
}
