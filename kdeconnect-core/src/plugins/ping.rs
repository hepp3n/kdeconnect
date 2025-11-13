use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::{collections::HashMap, sync::Arc};
use tokio::sync::{Mutex, mpsc};

use crate::{device::DeviceId, plugin_interface::Plugin, protocol::ProtocolPacket};

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Ping {
    pub message: Option<String>,
}

pub struct PingPlugin {
    writer_map: Arc<Mutex<HashMap<DeviceId, mpsc::UnboundedSender<ProtocolPacket>>>>,
}

impl PingPlugin {
    pub fn new(
        writer_map: Arc<Mutex<HashMap<DeviceId, mpsc::UnboundedSender<ProtocolPacket>>>>,
    ) -> Self {
        Self { writer_map }
    }
}

#[async_trait]
impl Plugin for PingPlugin {
    fn id(&self) -> &'static str {
        "kdeconnect.ping"
    }
}
