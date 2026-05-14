//! Persists per-device plugin enabled/disabled state.
//!
//! Stored as a JSON array of disabled plugin IDs at:
//!   ~/.config/kdeconnect/{device_id}_plugins.json

use std::collections::HashSet;
use tokio::fs;

use crate::config::CONFIG_DIR;

fn config_path(device_id: &str) -> std::path::PathBuf {
    let safe_id: String = device_id
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == '-')
        .collect();
    dirs::config_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("/tmp"))
        .join(CONFIG_DIR)
        .join(format!("{}_plugins.json", safe_id))
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_path_sanitizes_traversal_attempts() {
        let path = config_path("../../../etc/passwd");
        let filename = path.file_name().unwrap().to_str().unwrap();
        assert_eq!(filename, "etcpasswd_plugins.json");
        assert!(!path.to_str().unwrap().contains(".."));
    }

    #[test]
    fn config_path_preserves_valid_device_ids() {
        let path = config_path("abcdef1234567890abcdef1234567890");
        let filename = path.file_name().unwrap().to_str().unwrap();
        assert_eq!(filename, "abcdef1234567890abcdef1234567890_plugins.json");
    }

    #[test]
    fn config_path_handles_hyphens_and_underscores() {
        let path = config_path("abc-def_123-456-789012345678901234");
        let filename = path.file_name().unwrap().to_str().unwrap();
        assert_eq!(filename, "abc-def_123-456-789012345678901234_plugins.json");
    }

    #[tokio::test]
    async fn load_returns_empty_set_for_missing_file() {
        let _td = tempfile::tempdir().unwrap();
        unsafe {
            std::env::set_var("HOME", _td.path());
        }
        let result = load_disabled_plugins("nonexistent_device_id_000000000000").await;
        assert!(result.is_empty());
    }

    #[tokio::test]
    async fn save_and_load_round_trips() {
        let td = tempfile::tempdir().unwrap();
        unsafe {
            std::env::set_var("HOME", td.path());
        }
        let config_dir = td.path().join(".config").join("kdeconnect");
        tokio::fs::create_dir_all(&config_dir).await.unwrap();

        let device_id = "test_device_roundtrip_000000000000";
        let mut disabled = HashSet::new();
        disabled.insert("battery".to_string());
        disabled.insert("clipboard".to_string());

        save_disabled_plugins(device_id, &disabled).await;
        let loaded = load_disabled_plugins(device_id).await;
        assert_eq!(loaded, disabled);
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
