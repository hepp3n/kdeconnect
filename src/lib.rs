pub(crate) mod backends;
pub(crate) mod config;
pub mod device;
pub(crate) mod helpers;
pub(crate) mod packet;
pub(crate) mod plugins;
pub(crate) mod ssl;

use backends::start_backends;
use std::{collections::HashMap, sync::Arc};
use tokio::{
    sync::{Mutex, mpsc},
    task,
};
use tokio_stream::{StreamExt as _, wrappers::UnboundedReceiverStream};
use tracing::debug;

use crate::device::Device;

pub struct KdeConnect {
    devices: Arc<Mutex<HashMap<String, Device>>>,
}

impl Default for KdeConnect {
    fn default() -> Self {
        Self::new()
    }
}

impl KdeConnect {
    pub fn new() -> Self {
        Self {
            devices: Arc::new(Mutex::new(HashMap::new())),
        }
    }
}

impl KdeConnect {
    pub async fn run_server(&self) {
        let (tx, rx) = mpsc::unbounded_channel();

        debug!("Starting KDE Connect");

        // This is where the server will run, handling incoming devices.
        task::spawn(async move {
            start_backends(tx).await;
        });

        debug!("Server is running, waiting for devices...");

        let mut stream = UnboundedReceiverStream::new(rx);

        while let Some(data) = stream.next().await {
            debug!("Received device");
            self.devices.lock().await.insert(data.0, data.1.clone());

            task::spawn(async move {
                data.1.process_stream().await;
            });
        }
    }

    pub fn send_action(&self, device_id: String, action: device::DeviceAction) {
        let devices = Arc::clone(&self.devices);

        tokio::spawn(async move {
            let guard = devices.lock().await;

            let Some(device) = guard.get(&device_id) else {
                tracing::warn!("Device with ID {} not found", device_id);
                return;
            };

            device.send_action(action);
        });
    }
}
