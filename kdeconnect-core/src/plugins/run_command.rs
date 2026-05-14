use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;
use tracing::{info, warn};

use crate::device::DeviceId;
use crate::event::{ConnectionEvent, RemoteCommand};
use crate::plugin_interface::Plugin;
use crate::protocol::{PacketType, ProtocolPacket};

// ---------------------------------------------------------------------------
// Local command store
// ---------------------------------------------------------------------------

/// A command defined on this desktop that the phone can trigger.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct LocalCommand {
    /// UUID used as the map key in the KDE Connect packet.
    pub id: String,
    pub name: String,
    pub command: String,
}

/// Path to the global run-command config file.
/// Stored as a JSON array of LocalCommand so the settings UI can read/write it.
fn commands_config_path() -> std::path::PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("/tmp"))
        .join("kdeconnect")
        .join("runcommand.json")
}

async fn load_local_commands() -> Vec<LocalCommand> {
    match tokio::fs::read_to_string(commands_config_path()).await {
        Ok(json) => serde_json::from_str(&json).unwrap_or_default(),
        Err(_) => vec![],
    }
}

// ---------------------------------------------------------------------------
// Protocol structs
// ---------------------------------------------------------------------------

/// Incoming `kdeconnect.runcommand` from the phone (rare — phone also has
/// its own commands, but the primary direction is desktop -> phone).
#[derive(Serialize, Deserialize, Clone, Debug)]
#[allow(dead_code)]
pub struct RunCommand {
    #[serde(rename = "commandList")]
    pub command_list: String,
    #[serde(rename = "canAddCommand", default)]
    pub can_add_command: bool,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[allow(dead_code)]
pub struct RunCommandItem {
    pub name: String,
    pub command: String,
}

/// Incoming `kdeconnect.runcommand.request` from the phone:
///   - `key` -> execute the named command on this desktop
///   - `requestCommandList` -> send our command list back
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct RunCommandRequest {
    pub key: Option<String>,
    #[serde(rename = "requestCommandList")]
    pub request_command_list: Option<bool>,
}

impl Plugin for RunCommandRequest {
    fn id(&self) -> &'static str {
        "kdeconnect.runcommand.request"
    }
}

// ---------------------------------------------------------------------------
// Packet handlers
// ---------------------------------------------------------------------------

impl RunCommand {
    /// Desktop received a command list from the phone (bidirectional support).
    pub async fn received_packet(
        &self,
        device: &crate::device::Device,
        connection_tx: mpsc::UnboundedSender<crate::event::ConnectionEvent>,
        _core_tx: mpsc::UnboundedSender<crate::event::CoreEvent>,
    ) {
        let commands = parse_remote_commands(&self.command_list);
        info!(
            "[runcommand] received {} remote command(s) from {} (canAddCommand={})",
            commands.len(),
            device.device_id,
            self.can_add_command
        );
        let _ = connection_tx.send(ConnectionEvent::RunCommandListReceived((
            device.device_id.clone(),
            commands,
        )));
    }
}

impl RunCommandRequest {
    pub async fn received_packet(
        &self,
        device: &crate::device::Device,
        _connection_tx: mpsc::UnboundedSender<crate::event::ConnectionEvent>,
        core_tx: mpsc::UnboundedSender<crate::event::CoreEvent>,
    ) {
        if let Some(key) = &self.key {
            // Phone is asking us to execute a local command by its UUID key.
            let commands = load_local_commands().await;
            if let Some(cmd) = commands.iter().find(|c| c.id == *key) {
                info!("[runcommand] executing '{}': {}", cmd.name, cmd.command);
                if let Err(e) = std::process::Command::new("sh")
                    .arg("-c")
                    .arg(&cmd.command)
                    .spawn()
                {
                    warn!("[runcommand] failed to spawn '{}': {}", cmd.name, e);
                }
            } else {
                warn!(
                    "[runcommand] unknown key '{}' from {}",
                    key, device.device_id
                );
            }
        } else if self.request_command_list == Some(true) {
            // Phone is asking for our current command list.
            send_command_list(&device.device_id, core_tx).await;
        }
    }
}

// ---------------------------------------------------------------------------
// Outgoing — send our command list to the phone
// ---------------------------------------------------------------------------

/// Build and send a `kdeconnect.runcommand` packet to the phone containing
/// all locally defined commands and `canAddCommand: true`.
///
/// Called on device connect (kdeconnect-core/src/lib.rs) and when the phone
/// requests the list via `requestCommandList: true`.
pub async fn send_command_list(
    device_id: &DeviceId,
    core_tx: mpsc::UnboundedSender<crate::event::CoreEvent>,
) {
    let commands = load_local_commands().await;

    // commandList must be a *JSON-encoded string*, not a nested object.
    // Format: "{\"<uuid>\": {\"name\": \"...\", \"command\": \"...\"}, ...}"
    let mut map = serde_json::Map::new();
    for cmd in &commands {
        map.insert(
            cmd.id.clone(),
            serde_json::json!({ "name": cmd.name, "command": cmd.command }),
        );
    }
    let command_list_str =
        serde_json::to_string(&serde_json::Value::Object(map)).unwrap_or_else(|_| "{}".to_string());

    info!(
        "[runcommand] sending {} command(s) to {}",
        commands.len(),
        device_id
    );

    let packet = ProtocolPacket::new(
        PacketType::RunCommand,
        serde_json::json!({
            "commandList": command_list_str,
            "canAddCommand": true,
        }),
    );

    let _ = core_tx.send(crate::event::CoreEvent::SendPacket {
        device: device_id.clone(),
        packet,
    });
}

fn parse_remote_commands(command_list: &str) -> Vec<RemoteCommand> {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(command_list) else {
        warn!("[runcommand] received invalid commandList JSON");
        return Vec::new();
    };

    let Some(map) = value.as_object() else {
        warn!("[runcommand] commandList is not a JSON object");
        return Vec::new();
    };

    map.iter()
        .filter_map(|(key, value)| {
            let name = value.get("name")?.as_str()?.to_string();
            let command = value
                .get("command")
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string();
            Some(RemoteCommand {
                key: key.clone(),
                name,
                command,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::parse_remote_commands;

    #[test]
    fn parses_remote_command_list_json_string() {
        let commands = parse_remote_commands(
            r#"{"abc":{"name":"Lock screen","command":"loginctl lock-session"}}"#,
        );

        assert_eq!(commands.len(), 1);
        assert_eq!(commands[0].key, "abc");
        assert_eq!(commands[0].name, "Lock screen");
        assert_eq!(commands[0].command, "loginctl lock-session");
    }

    #[test]
    fn invalid_remote_command_list_is_empty() {
        assert!(parse_remote_commands("not json").is_empty());
        assert!(parse_remote_commands("[]").is_empty());
    }
}
