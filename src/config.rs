use std::{
    fs::{self, File},
    io::{self, Read, Write as _},
    path::PathBuf,
};

use rcgen::KeyPair;
use serde::{Deserialize, Serialize};
use tracing::error;

use crate::{
    helpers::{default_hostname, generate_device_uuid},
    ssl::{certificate_generator, store_certificate_files},
};

lazy_static::lazy_static! {
    pub(crate) static ref CONFIG: KdeConnectConfig = KdeConnectConfig::default();
}

pub(crate) const CONFIG_DIR: &str = "kdeconnect";
const CONFIG_FILE: &str = "config.ron";

pub(crate) const CERTIFICATE: &str = "self_signed.pem";
pub(crate) const KEY_PAIR: &str = "key_pair.pem";

#[derive(Deserialize, Serialize)]
pub(crate) struct KdeConnectConfig {
    pub(crate) device_name: String,
    pub(crate) device_uuid: String,
    signed_ca: PathBuf,
    priv_key: PathBuf,
}

impl Default for KdeConnectConfig {
    fn default() -> Self {
        let path = dirs::config_dir()
            .expect("cant find config directory")
            .join(CONFIG_DIR);

        fs::create_dir_all(&path).unwrap_or_else(|_| {
            error!("Failed to create config directory at {:?}", path);
        });

        let self_signed_path = path.join(CERTIFICATE);
        let keypair_path = path.join(KEY_PAIR);

        let device_uuid = generate_device_uuid();
        let device_name = default_hostname();

        if let Ok(keypair) = KeyPair::generate()
            && !self_signed_path.exists()
            && !keypair_path.exists()
        {
            match certificate_generator(&keypair, &device_uuid) {
                Ok(cert) => {
                    let _ =
                        store_certificate_files(&cert, &self_signed_path, &keypair, &keypair_path)
                            .is_ok();
                }
                Err(e) => error!("{e:?}"),
            }
        };

        if path.join(CONFIG_FILE).exists() {
            let mut buffer = String::new();
            let mut file = File::open(path.join(CONFIG_FILE)).unwrap();
            let _ = file.read_to_string(&mut buffer);

            let x: KdeConnectConfig = ron::from_str(&buffer).unwrap();

            return x;
        }

        let kdeconnect_config = KdeConnectConfig {
            signed_ca: self_signed_path,
            priv_key: keypair_path,
            device_name,
            device_uuid,
        };

        if !path.join(CONFIG_FILE).exists() {
            kdeconnect_config.store_config().unwrap();
        }

        kdeconnect_config
    }
}

impl KdeConnectConfig {
    pub fn store_config(&self) -> io::Result<()> {
        let path = dirs::config_dir()
            .expect("cant find config directory")
            .join("kdeconnect");

        let _ = fs::create_dir_all(&path);

        if let Ok(cfg_to_str) = ron::ser::to_string_pretty(&self, ron::ser::PrettyConfig::default())
        {
            let mut config_file = fs::File::create(path.join(CONFIG_FILE))?;
            config_file.write_all(cfg_to_str.as_bytes())?;
        };

        Ok(())
    }
}
