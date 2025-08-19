use std::sync::Arc;

use tokio::{
    io::AsyncWriteExt as _,
    net,
    sync::{Mutex, mpsc},
    task,
};
use tokio_native_tls::TlsStream;
use tracing::error;

use crate::packet::Pair;

pub enum DeviceAction {
    Pair(bool),
}

pub struct Device {
    stream: Option<Arc<Mutex<TlsStream<net::TcpStream>>>>,
    action_tx: mpsc::UnboundedSender<DeviceAction>,
    action_rx: Arc<Mutex<mpsc::UnboundedReceiver<DeviceAction>>>,
}

impl Device {
    pub fn new(stream: Option<Arc<Mutex<TlsStream<net::TcpStream>>>>) -> Self {
        let (action_tx, action_rx) = mpsc::unbounded_channel();

        Self {
            stream,
            action_tx,
            action_rx: Arc::new(Mutex::new(action_rx)),
        }
    }
}

pub(crate) fn create_device(stream: Option<Arc<Mutex<TlsStream<net::TcpStream>>>>) -> Device {
    Device::new(stream)
}

impl Device {
    pub(crate) async fn process_stream(&self) {
        if let Some(stream) = self.stream.clone() {
            let stream = Arc::clone(&stream);
            let rx = Arc::clone(&self.action_rx);

            task::spawn(async move {
                while let Some(action) = rx.lock().await.recv().await {
                    match action {
                        DeviceAction::Pair(flag) => {
                            let data = Pair::new(flag).to_string() + "\n";
                            stream
                                .lock()
                                .await
                                .write_all(data.as_bytes())
                                .await
                                .expect("Failed to send Pair action");
                        }
                    }
                }
            });
        } else {
            error!("No stream available to process.");
        }
    }

    pub(crate) fn send_action(&self, action: DeviceAction) {
        match action {
            DeviceAction::Pair(flag) => {
                // Handle pairing logic here
                // For example, read from the stream, send a response, etc.
                // This is just a placeholder for actual implementation.
                self.action_tx
                    .send(DeviceAction::Pair(flag))
                    .expect("Failed to send Pair action");
            }
        }
    }
}
