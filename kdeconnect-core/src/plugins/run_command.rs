use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;

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

impl RunCommandRequest {
    pub async fn received_packet(
        &self,
        _device: &crate::device::Device,
        _connection_tx: mpsc::UnboundedSender<crate::event::ConnectionEvent>,
        _core_tx: mpsc::UnboundedSender<crate::event::CoreEvent>,
    ) {
    }
}
