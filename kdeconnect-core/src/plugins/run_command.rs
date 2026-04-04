use std::collections::HashMap;
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;
use tracing::{info, warn};

use crate::event::RemoteCommand;
use crate::plugin_interface::Plugin;

#[derive(Serialize, Deserialize, Clone, Debug)]
#[allow(dead_code)]
pub struct RunCommand {
    #[serde(rename = "commandList")]
    pub command_list: String,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[allow(dead_code)]
pub struct RunCommandItem {
    pub name: String,
    pub command: String,
}

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

impl RunCommand {
    pub async fn received_packet(
        &self,
        device: &crate::device::Device,
        connection_tx: mpsc::UnboundedSender<crate::event::ConnectionEvent>,
        _core_tx: mpsc::UnboundedSender<crate::event::CoreEvent>,
    ) {
        // commandList is a JSON-encoded string: {"key": {"name": "...", "command": "..."}}
        let parsed: HashMap<String, RunCommandItem> =
            match serde_json::from_str(&self.command_list) {
                Ok(m) => m,
                Err(e) => {
                    warn!("[runcommand] failed to parse commandList: {}", e);
                    return;
                }
            };

        let commands: Vec<RemoteCommand> = parsed
            .into_iter()
            .map(|(key, item)| RemoteCommand {
                key,
                name: item.name,
                command: item.command,
            })
            .collect();

        info!(
            "[runcommand] received {} commands from {}",
            commands.len(),
            device.device_id
        );

        let _ = connection_tx.send(
            crate::event::ConnectionEvent::RunCommandListReceived((
                device.device_id.clone(),
                commands,
            )),
        );
    }
}

impl RunCommandRequest {
    pub async fn received_packet(
        &self,
        _device: &crate::device::Device,
        _connection_tx: mpsc::UnboundedSender<crate::event::ConnectionEvent>,
        _core_tx: mpsc::UnboundedSender<crate::event::CoreEvent>,
    ) {
        // Phone requesting our local command list or asking us to run a local
        // command. Local command execution is not yet implemented.
    }
}
