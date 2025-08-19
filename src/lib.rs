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
    sync::{
        Mutex,
        mpsc::{self},
    },
    task,
};
use tokio_stream::{StreamExt as _, wrappers::UnboundedReceiverStream};
use tracing::{debug, error};

use crate::device::{ConnectedId, Linked, NewClient};

#[derive(Debug, Clone)]
pub struct KdeConnect {
    devices: Arc<Mutex<HashMap<ConnectedId, NewClient>>>,
    device_tx: mpsc::UnboundedSender<Linked>,
}

impl KdeConnect {
    pub fn new() -> (Self, UnboundedReceiverStream<Linked>) {
        let (conn_tx, conn_rx) = mpsc::unbounded_channel::<Linked>();

        (
            Self {
                devices: Arc::new(Mutex::new(HashMap::new())),
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

        while let Some(mut data) = stream.next().await {
            self.device_tx.send(data.0.clone()).unwrap_or_else(|e| {
                error!("Failed to send device ID: {}", e);
            });

            self.devices
                .lock()
                .await
                .insert(data.0.0.clone(), data.clone());

            task::spawn(async move {
                data.1.process_stream().await;
            });
        }
    }

    pub fn send_action(&self, device_id: String, action: device::DeviceAction) {
        let devices = Arc::clone(&self.devices);

        tokio::spawn(async move {
            let guard = devices.lock().await;

            for (id, client) in guard.iter() {
                if id == &device_id {
                    client.1.send(action.clone())
                } else {
                    error!("Device with ID {} not found", device_id);
                }
            }
        });
    }
}
