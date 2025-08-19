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

use crate::device::{ConnectedId, Device, NewDevice};

pub struct KdeConnect {
    devices: Arc<Mutex<NewDevice>>,
    device_tx: mpsc::UnboundedSender<ConnectedId>,
}

impl KdeConnect {
    pub fn new(device_tx: mpsc::UnboundedSender<ConnectedId>) -> Self {
        Self {
            devices: Arc::new(Mutex::new(HashMap::new())),
            device_tx,
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
            self.devices
                .lock()
                .await
                .insert(data.0.clone(), data.1.clone());

            self.device_tx.send(data.0.clone()).unwrap_or_else(|e| {
                tracing::error!("Failed to send device ID: {}", e);
            });

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
