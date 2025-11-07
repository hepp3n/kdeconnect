use cosmic::cosmic_config::{self, CosmicConfigEntry, cosmic_config_derive::CosmicConfigEntry};
use kdeconnect_core::device::DeviceId;

use crate::{APP_ID, CONFIG_VERSION};

#[derive(Debug, Default, Clone, CosmicConfigEntry, Eq, PartialEq)]
#[version = 1]
pub struct ConnectConfig {
    pub paired: Option<DeviceId>,
}

impl ConnectConfig {
    pub fn config_handler() -> Option<cosmic_config::Config> {
        cosmic_config::Config::new(APP_ID, CONFIG_VERSION).ok()
    }
    pub fn config() -> ConnectConfig {
        match Self::config_handler() {
            Some(config_handler) => {
                ConnectConfig::get_entry(&config_handler).unwrap_or_else(|(errs, config)| {
                    tracing::info!("errors loading config: {:?}", errs);
                    config
                })
            }
            None => ConnectConfig::default(),
        }
    }
}
