pub(crate) mod backends;
pub(crate) mod config;
pub(crate) mod device;
pub(crate) mod helpers;
pub(crate) mod packet;
pub(crate) mod plugins;
pub(crate) mod ssl;

use backends::start_backends;
use std::{collections::HashMap, sync::Arc};
use tokio::{sync::mpsc, task};
use tokio_stream::{StreamExt as _, wrappers::UnboundedReceiverStream};
use tracing::debug;

use crate::device::Device;

pub struct KdeConnect {
    devices: Option<HashMap<String, Arc<Device>>>,
}

impl Default for KdeConnect {
    fn default() -> Self {
        Self::new()
    }
}

impl KdeConnect {
    pub fn new() -> Self {
        Self { devices: None }
    }
}

impl KdeConnect {
    pub async fn run_server(&mut self) {
        let (tx, rx) = mpsc::unbounded_channel();

        debug!("Starting KDE Connect");

        // This is where the server will run, handling incoming devices.
        task::spawn(async move {
            start_backends(tx).await;
        });

        debug!("Server is running, waiting for devices...");

        let mut stream = UnboundedReceiverStream::new(rx);

        while let Some(device) = stream.next().await {
            debug!("Received device");
            let dev = Arc::new(device.1);
            let cloned = Arc::clone(&dev);

            task::spawn(async move {
                cloned.process_stream().await;
            });

            self.devices
                .get_or_insert_with(HashMap::new)
                .insert(device.0, dev);
        }
    }

    pub fn send_action(&self, device_id: &str, action: device::DeviceAction) {
        self.devices
            .as_ref()
            .and_then(|devices| devices.get(device_id))
            .expect("Device not found")
            .send_action(action);
    }
}
