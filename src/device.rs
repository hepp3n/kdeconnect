use std::{collections::HashMap, sync::Arc};

use tokio::{
    io::AsyncWriteExt as _,
    net,
    sync::{Mutex, mpsc},
};
use tokio_native_tls::TlsStream;
use tracing::info;

use crate::{
    packet::{Pair, Ping},
    plugins::PluginHandler,
};

pub type NewDevice = HashMap<String, Device>;
pub type ConnectedId = String;

pub enum DeviceAction {
    Pair(bool),
    Ping(String),
}

#[derive(Clone)]
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

impl PluginHandler for Device {
    async fn ping(&self, message: String) {
        if let Some(stream) = &self.stream {
            let mut stream = stream.lock().await;

            let ping_packet = Ping::new(message).to_string() + "\n";

            if let Err(e) = stream.write_all(ping_packet.as_bytes()).await {
                tracing::error!("Failed to send Pair packet: {}", e);
            } else {
                tracing::info!("Pair packet sent successfully");
                info!("{ping_packet}");
            }
        } else {
            tracing::warn!("No stream available for pairing");
        }
    }
}

impl Device {
    pub(crate) async fn process_stream(&self) {
        while let Some(action) = self.action_rx.lock().await.recv().await {
            match action {
                DeviceAction::Pair(flag) => self.pair_handler(flag).await,
                DeviceAction::Ping(msg) => self.ping(msg).await,
            }
        }
    }

    async fn pair_handler(&self, pair: bool) {
        if let Some(stream) = &self.stream {
            let mut stream = stream.lock().await;
            let pair_packet = Pair::new(pair).to_string() + "\n";

            if let Err(e) = stream.write_all(pair_packet.as_bytes()).await {
                tracing::error!("Failed to send Pair packet: {}", e);
            } else {
                tracing::info!("Pair packet sent successfully");
                info!("{pair_packet}");
            }
        } else {
            tracing::warn!("No stream available for pairing");
        }
    }

    pub(crate) fn send_action(&self, action: DeviceAction) {
        match action {
            DeviceAction::Pair(flag) => {
                self.action_tx
                    .send(DeviceAction::Pair(flag))
                    .expect("Failed to send Pair action");
            }
            DeviceAction::Ping(msg) => {
                self.action_tx
                    .send(DeviceAction::Ping(msg))
                    .expect("Failed to send Ping action");
            }
        }
    }
}
