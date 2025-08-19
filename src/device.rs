use std::{collections::HashMap, fmt::Display, sync::Arc};

use tokio::{
    io::{AsyncBufReadExt as _, AsyncWriteExt, BufReader, ReadHalf, WriteHalf},
    net::TcpStream,
    sync::{Mutex, mpsc},
    task,
};
use tokio_native_tls::TlsStream;
use tracing::{debug, error, info};

use crate::{
    packet::{Pair, Ping},
    plugins::PluginHandler,
};

pub type ConnectedId = String;
pub type NewDevice = HashMap<ConnectedId, Device>;

#[derive(Debug, Clone)]
pub enum DeviceAction {
    Pair(bool),
    Ping(String),
}

impl Display for DeviceAction {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DeviceAction::Pair(flag) => write!(f, "Pair action: {}", flag),
            DeviceAction::Ping(msg) => write!(f, "Ping action: {}", msg),
        }
    }
}

#[derive(Clone)]
pub struct Device {
    reader: Arc<Mutex<ReadHalf<TlsStream<TcpStream>>>>,
    writer: Arc<Mutex<WriteHalf<TlsStream<TcpStream>>>>,
    pub action_tx: mpsc::UnboundedSender<DeviceAction>,
    action_rx: Arc<Mutex<mpsc::UnboundedReceiver<DeviceAction>>>,
}

impl Device {
    pub fn new(
        reader: Arc<Mutex<ReadHalf<TlsStream<TcpStream>>>>,
        writer: Arc<Mutex<WriteHalf<TlsStream<TcpStream>>>>,
    ) -> Self {
        let (action_tx, action_rx) = mpsc::unbounded_channel();

        Self {
            reader,
            writer,
            action_tx,
            action_rx: Arc::new(Mutex::new(action_rx)),
        }
    }
}

pub(crate) fn create_device(
    reader: Arc<Mutex<ReadHalf<TlsStream<TcpStream>>>>,
    writer: Arc<Mutex<WriteHalf<TlsStream<TcpStream>>>>,
) -> Device {
    Device::new(reader, writer)
}

impl PluginHandler for Device {
    async fn ping(&self, message: String) {
        let ping_packet = Ping::new(message).to_string() + "\n";

        let mut writer = self.writer.lock().await;

        if let Err(e) = writer.write_all(ping_packet.as_bytes()).await {
            error!("Failed to send Ping packet: {}", e);
        } else {
            info!("Ping packet sent successfully");
            info!("{}", ping_packet);
        }
    }
}

impl Device {
    pub(crate) async fn process_stream(&mut self) {
        let c_reader = Arc::clone(&self.reader);

        task::spawn(async move {
            let mut reader = c_reader.lock().await;

            let mut reader = BufReader::new(Box::new(&mut *reader));

            loop {
                let mut buffer = String::new();

                match reader.read_line(&mut buffer).await {
                    Ok(0) => {
                        info!("Connection closed by peer");
                        break; // EOF reached
                    }
                    Ok(_) => {
                        info!("{}", buffer);
                    }
                    Err(e) => {
                        error!("Failed to read from stream: {}", e);
                        break; // Exit on read error
                    }
                }
                buffer.clear(); // Clear the buffer for the next read
            }
        });

        while let Some(action) = self.action_rx.lock().await.recv().await {
            debug!("Received action: {}", action);

            match action {
                DeviceAction::Pair(flag) => self.pair_handler(flag).await,
                DeviceAction::Ping(msg) => self.ping(msg).await,
            }
        }
    }

    async fn pair_handler(&self, pair: bool) {
        let pair_packet = Pair::new(pair).to_string() + "\n";

        let mut writer = self.writer.lock().await;

        if let Err(e) = writer.write_all(pair_packet.as_bytes()).await {
            error!("Failed to send Pair packet: {}", e);
        } else {
            info!("Pair packet sent successfully");
            info!("{}", pair_packet);
        }
    }
}
