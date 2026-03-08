//! Persists per-device plugin enabled/disabled state.
//!
//! Stored as a JSON array of disabled plugin IDs at:
//!   ~/.config/kdeconnect/{device_id}_plugins.json

use std::collections::HashSet;
use tokio::fs;

use crate::config::CONFIG_DIR;

fn config_path(device_id: &str) -> std::path::PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("/tmp"))
        .join(CONFIG_DIR)
        .join(format!("{}_plugins.json", device_id))
}

/// Load the set of disabled plugin IDs for a device.
/// Returns an empty set (all enabled) if no config exists yet.
pub async fn load_disabled_plugins(device_id: &str) -> HashSet<String> {
    let path = config_path(device_id);
    match fs::read_to_string(&path).await {
        Ok(json) => serde_json::from_str::<HashSet<String>>(&json).unwrap_or_default(),
        Err(_) => HashSet::new(),
    }
}

/// Persist the set of disabled plugin IDs for a device.
pub async fn save_disabled_plugins(device_id: &str, disabled: &HashSet<String>) {
    let path = config_path(device_id);
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent).await;
    }
    match serde_json::to_string(disabled) {
        Ok(json) => {
            if let Err(e) = fs::write(&path, json).await {
                tracing::warn!("[plugin_config] failed to save for {}: {}", device_id, e);
            }
        }
        Err(e) => tracing::warn!("[plugin_config] serialize failed: {}", e),
    }
}
