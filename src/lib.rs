pub(crate) mod backends;
pub(crate) mod config;
pub mod device;
pub(crate) mod helpers;
pub(crate) mod packet;
pub(crate) mod plugins;
pub(crate) mod ssl;

use backends::start_backends;
use std::sync::Arc;
use tokio::{
    sync::{
        Mutex,
        mpsc::{self},
    },
    task,
};
use tokio_stream::{StreamExt as _, wrappers::UnboundedReceiverStream};
use tracing::{debug, error};

use crate::device::{Linked, NewClient};

#[derive(Debug, Clone)]
pub struct KdeConnect {
    devices: Arc<Mutex<Vec<NewClient>>>,
    device_tx: mpsc::UnboundedSender<Linked>,
}

impl KdeConnect {
    pub fn new() -> (Self, UnboundedReceiverStream<Linked>) {
        let (conn_tx, conn_rx) = mpsc::unbounded_channel::<Linked>();

        (
            Self {
                devices: Arc::new(Mutex::new(Vec::new())),
                device_tx: conn_tx,
            },
            conn_rx.into(),
        )
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
            self.device_tx
                .send((data.0.clone(), data.1.clone()))
                .unwrap_or_else(|e| {
                    error!("Failed to send device ID: {}", e);
                });

            self.devices.lock().await.push(data.clone());

            task::spawn(async move {
                let mut device = data.2;
                device.process_stream().await;
            });
        }
    }

    pub fn send_action(&self, device_id: String, action: device::DeviceAction) {
        let devices = Arc::clone(&self.devices);

        tokio::spawn(async move {
            let guard = devices.lock().await;

            for device in guard.iter() {
                if device.0 == device_id {
                    device.2.send(action.clone())
                } else {
                    error!("Device with ID {} not found", device_id);
                }
            }
        });
    }
}
