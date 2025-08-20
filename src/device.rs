use serde::{Deserialize, Serialize};
use serde_json as json;
use std::{fmt::Display, sync::Arc};
use tokio::{
    io::{AsyncBufReadExt as _, AsyncWriteExt, BufReader, ReadHalf, WriteHalf},
    net::TcpStream,
    sync::{Mutex, mpsc},
    task,
};
use tokio_native_tls::TlsStream;
use tracing::{debug, error, info};

use crate::{
    helpers::pair_timestamp,
    make_packet, make_packet_str,
    packet::{Packet, PacketType, Pair, Ping},
    plugins::PluginHandler,
};

pub type Linked = (ConnectedId, ConnectedDeviceName, ConnectionType);

pub type ConnectedId = String;
pub type ConnectedDeviceName = String;

#[derive(Debug, Clone, Serialize, Deserialize, Hash, Eq, PartialEq)]
pub enum ConnectionType {
    Client,
    Server,
}

impl Display for ConnectionType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ConnectionType::Client => write!(f, "Client"),
            ConnectionType::Server => write!(f, "Server"),
        }
    }
}

#[derive(Debug, Clone)]
pub struct NewClient(pub Linked, pub Device);

#[derive(Debug, Clone)]
pub enum DeviceAction {
    Disconnect,
    Pair(bool),
    Ping(String),
}

impl Display for DeviceAction {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DeviceAction::Disconnect => write!(f, "Disconnect action"),
            DeviceAction::Pair(flag) => write!(f, "Pair action: {}", flag),
            DeviceAction::Ping(msg) => write!(f, "Ping action: {}", msg),
        }
    }
}

#[derive(Debug, Clone)]
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
    async fn pair(&self, flag: bool) {
        let pair = Pair {
            pair: flag,
            timestamp: pair_timestamp(),
        };
        let pair_packet = make_packet_str!(pair).expect("Failed to create Pair packet");

        let mut writer = self.writer.lock().await;

        if let Err(e) = writer.write_all(pair_packet.as_bytes()).await {
            error!("Failed to send Pair packet: {}", e);
        } else {
            info!("Pair packet sent successfully");
            info!("{}", pair_packet);
        }
    }

    async fn ping(&self, message: String) {
        let ping = Ping {
            message: Some(message),
        };

        let ping_packet = make_packet_str!(ping).expect("Failed to create Ping packet");

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

        self.writer
            .lock()
            .await
            .flush()
            .await
            .expect("Failed to flush writer");

        task::spawn(async move {
            let mut reader = c_reader.lock().await;

            let mut reader = BufReader::new(Box::new(&mut *reader));

            loop {
                let mut buffer = String::new();

                match reader.read_line(&mut buffer).await {
                    Ok(0) => {
                        info!("EOF reached.");
                        break; // EOF reached
                    }
                    Ok(_) => {
                        if let Ok(packet) = json::from_str::<Packet>(&buffer) {
                            debug!("Received NetworkPacket {}", packet.body);
                        }
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
                DeviceAction::Disconnect => {
                    info!("Disconnect action received. Closing connection.");
                    let mut writer = self.writer.lock().await;
                    if let Err(e) = writer.shutdown().await {
                        error!("Failed to shutdown writer: {}", e);
                    }
                    break;
                }
                DeviceAction::Pair(flag) => self.pair(flag).await,
                DeviceAction::Ping(msg) => self.ping(msg).await,
            }
        }
    }

    pub fn send(&self, action: DeviceAction) {
        self.action_tx
            .send(action)
            .unwrap_or_else(|e| error!("Failed to send action: {}", e));
    }
}
