use serde::{Deserialize, Serialize};

use crate::plugin_interface::Plugin;

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct RunCommand {
    #[serde(rename = "commandList")]
    pub command_list: String,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
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

#[async_trait::async_trait]
impl Plugin for RunCommandRequest {
    fn id(&self) -> &'static str {
        "kdeconnect.runcommand.request"
    }

    async fn received(
        &self,
        _device: &crate::device::Device,
        _connection_tx: std::sync::Arc<
            tokio::sync::mpsc::UnboundedSender<crate::event::ConnectionEvent>,
        >,
        _core_tx: std::sync::Arc<tokio::sync::broadcast::Sender<crate::event::CoreEvent>>,
    ) {
    }

    async fn send(
        &self,
        _device: &crate::device::Device,
        _core_tx: std::sync::Arc<tokio::sync::broadcast::Sender<crate::event::CoreEvent>>,
    ) {
    }
}
