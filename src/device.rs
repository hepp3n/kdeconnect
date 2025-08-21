use notify_rust::Notification;
use serde::{Deserialize, Serialize};
use serde_json as json;
use std::{collections::HashMap, fmt::Display, sync::Arc};
use tokio::{
    io::{AsyncBufReadExt as _, AsyncWriteExt, BufReader, ReadHalf, WriteHalf},
    net::TcpStream,
    sync::{
        Mutex,
        mpsc::{self},
    },
    task::{self, JoinHandle},
};
use tokio_native_tls::TlsStream;
use tokio_stream::{StreamExt as _, wrappers::UnboundedReceiverStream};
use tracing::{debug, error, info, warn};

use crate::{
    config::CONFIG,
    make_packet, make_packet_str,
    packet::{
        Battery, Clipboard, ClipboardConnect, ConnectivityReport, Mpris, MprisPlayer, Packet,
        PacketType, Pair, Ping, RunCommandItem, RunCommandRequest, SystemVolume,
        SystemVolumeStream,
    },
    pairing_handler::{PairingHandler, PairingHandlerExt},
    plugins::PluginHandler,
};

#[derive(Default, Debug, Deserialize, Serialize, Clone, PartialEq)]
pub enum PairingState {
    Requested,
    RequestedByPeer,
    Paired,
    #[default]
    NotPaired,
}

impl Display for PairingState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PairingState::Requested => write!(f, "Requested"),
            PairingState::RequestedByPeer => write!(f, "Requested by Peer"),
            PairingState::Paired => write!(f, "Paired"),
            PairingState::NotPaired => write!(f, "Not Paired"),
        }
    }
}

#[derive(Debug, Clone)]
pub enum DeviceResponse {
    Refresh((Box<Device>, Box<DeviceState>)),
    SyncClipboard(String),
}

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
pub enum DeviceAction {
    Disconnect,
    Refresh,
    RequestedPairByPeer(Packet),
    Pair,
    UnPair,
    Ping(String),
}

impl Display for DeviceAction {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DeviceAction::Disconnect => write!(f, "Disconnect action"),
            DeviceAction::Refresh => write!(f, "Refresh action"),
            DeviceAction::RequestedPairByPeer(packet) => {
                write!(f, "Requested pair by peer: {}", packet.packet_type)
            }
            DeviceAction::Pair => write!(f, "Pair action"),
            DeviceAction::UnPair => write!(f, "UnPair action"),
            DeviceAction::Ping(msg) => write!(f, "Ping action: {}", msg),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Hash, Eq, PartialEq)]
pub struct DeviceId {
    pub id: ConnectedId,
    pub name: ConnectedDeviceName,
}

impl Default for DeviceId {
    fn default() -> Self {
        Self {
            id: "localhost".to_string(),
            name: "Unknown Device".to_string(),
        }
    }
}

impl Display for DeviceId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} ({}...)", self.name, self.id.split_at(5).0)
    }
}

#[derive(Debug, Clone)]
pub struct Device {
    pub id: DeviceId,
    pub(crate) reader: Arc<Mutex<ReadHalf<TlsStream<TcpStream>>>>,
    pub(crate) writer: Arc<Mutex<WriteHalf<TlsStream<TcpStream>>>>,
    action_tx: mpsc::UnboundedSender<DeviceAction>,
    action_rx: Arc<Mutex<UnboundedReceiverStream<DeviceAction>>>,
    state: Arc<Mutex<DeviceState>>,
    pairing_handler: PairingHandler,
}

impl Device {
    pub async fn new(
        id: DeviceId,
        reader: Arc<Mutex<ReadHalf<TlsStream<TcpStream>>>>,
        writer: Arc<Mutex<WriteHalf<TlsStream<TcpStream>>>>,
    ) -> Self {
        let (action_tx, action_rx) = mpsc::unbounded_channel::<DeviceAction>();

        let pairing_state = CONFIG
            .lock()
            .await
            .paired
            .as_ref()
            .and_then(|(device_id, state)| {
                if device_id == &id {
                    Some(state.clone())
                } else {
                    None
                }
            })
            .unwrap_or(PairingState::NotPaired);
        let device_state = DeviceState::new(id.clone(), pairing_state.clone());
        let pairing_handler = PairingHandler::new(&id, writer.clone(), pairing_state);

        let action_rx = UnboundedReceiverStream::new(action_rx);

        Self {
            id: id.clone(),
            reader,
            writer,
            action_tx,
            action_rx: Arc::new(Mutex::new(action_rx)),
            state: Arc::new(Mutex::new(device_state)),
            pairing_handler,
        }
    }
}

pub(crate) async fn create_device(
    id: DeviceId,
    reader: Arc<Mutex<ReadHalf<TlsStream<TcpStream>>>>,
    writer: Arc<Mutex<WriteHalf<TlsStream<TcpStream>>>>,
) -> Device {
    Device::new(id, reader, writer).await
}

impl PluginHandler for Device {
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
    pub(crate) async fn process_stream(
        &mut self,
        device_response: mpsc::UnboundedSender<DeviceResponse>,
    ) {
        let c_reader = Arc::clone(&self.reader);
        let id = self.id.clone();

        self.writer
            .lock()
            .await
            .flush()
            .await
            .expect("Failed to flush writer");

        let action = self.action_tx.clone();
        let responsder = device_response.clone();

        let mut device_state = Box::new(self.state.lock().await.clone());
        let task_response = device_response.clone();
        let boxed_self = Box::new(self.to_owned());
        let task_self = Box::clone(&boxed_self);
        let mut task_state = Box::clone(&device_state);

        task::spawn(async move {
            let mut reader = c_reader.lock().await;
            let mut reader = BufReader::new(Box::new(&mut *reader));

            loop {
                let mut buffer = String::new();

                match reader.read_line(&mut buffer).await {
                    Ok(0) => {
                        error!("EOF reached.");
                        action.send(DeviceAction::Disconnect).unwrap_or_else(|e| {
                            error!("Failed to send Disconnect action: {}", e);
                        });
                        break; // Exit on EOF
                    }
                    Ok(_) => {
                        if let Ok(packet) = json::from_str::<Packet>(&buffer) {
                            match packet.packet_type.as_str() {
                                Battery::TYPE => {
                                    debug!("Received Battery packet: {}", buffer);

                                    if let Ok(battery) = json::from_value::<Battery>(packet.body) {
                                        task_state.battery = Some(battery);
                                        info!("Battery state updated: {:?}", task_state.battery);
                                    } else {
                                        warn!("Failed to parse Battery data");
                                    }
                                }
                                Clipboard::TYPE => {
                                    debug!("Received Clipboard packet: {}", buffer);

                                    if let Ok(clipboard) =
                                        json::from_value::<Clipboard>(packet.body)
                                    {
                                        task_state.clipboard = Some(clipboard.content.clone());

                                        responsder
                                            .send(DeviceResponse::SyncClipboard(
                                                clipboard.content.clone(),
                                            ))
                                            .unwrap_or_else(|e| {
                                                error!(
                                                    "Failed to send SyncClipboard response: {}",
                                                    e
                                                );
                                            });

                                        info!(
                                            "Clipboard state updated: {:?}",
                                            task_state.clipboard
                                        );
                                    } else {
                                        warn!("Failed to parse Battery data");
                                    }
                                }
                                ClipboardConnect::TYPE => {
                                    debug!("Received ClipboardConnect packet: {}", buffer);
                                }
                                ConnectivityReport::TYPE => {
                                    debug!("Received ConnectivityReport packet: {}", buffer);

                                    if let Ok(connectivity) =
                                        json::from_value::<ConnectivityReport>(packet.body)
                                    {
                                        task_state.connectivity = Some(connectivity);
                                        info!(
                                            "Connectivity state updated: {:?}",
                                            task_state.connectivity
                                        );
                                    } else {
                                        warn!("Failed to parse ConnectivityReport data");
                                    }
                                }
                                Mpris::TYPE => {
                                    debug!("Received Mpris packet: {}", buffer);

                                    if let Ok(mpris) = json::from_value::<Mpris>(packet.body) {
                                        match mpris {
                                            Mpris::List {
                                                player_list,
                                                supports_album_art_payload,
                                            } => warn!("not implemented yet"),
                                            Mpris::TransferringArt {
                                                player,
                                                album_art_url,
                                                transferring_album_art,
                                            } => warn!("not implemented yet"),
                                            Mpris::Info(mpris_player) => {
                                                info!(
                                                    "Received Mpris player info: {:?}",
                                                    mpris_player
                                                );
                                            }
                                        }
                                    } else {
                                        warn!("Failed to parse Mpris data");
                                    }
                                }
                                Pair::TYPE => {
                                    debug!("Received Pair packet: {}", buffer);

                                    if let Ok(pair) = json::from_value::<Pair>(packet.body.clone())
                                    {
                                        if pair.timestamp.is_some() {
                                            action
                                                .send(DeviceAction::RequestedPairByPeer(packet))
                                                .unwrap_or_else(|e| {
                                                    error!("Failed to send Pair action: {}", e);
                                                });
                                        }

                                        if pair.pair {
                                            debug!(
                                                "Device {} sent accept for pairing we can assume success.",
                                                &task_self.id
                                            );
                                            task_state.pairing_state = PairingState::Paired;
                                        }

                                        if !pair.pair {
                                            debug!(
                                                "Device {} sent reject for pairing, setting state to NotPaired.",
                                                &task_self.id
                                            );
                                            task_state.pairing_state = PairingState::NotPaired;
                                            action.send(DeviceAction::UnPair).unwrap_or_else(|e| {
                                                error!("Failed to send Pair action: {}", e);
                                            });

                                            break; // Exit on reject
                                        }
                                    } else {
                                        warn!("Failed to parse Pair data");
                                    }
                                }
                                Ping::TYPE => {
                                    debug!("Received Ping packet: {}", buffer);

                                    Notification::new()
                                        .appname("KDE Connect")
                                        .summary("KDE Connect")
                                        .body("Ping!")
                                        .icon("display-symbolic")
                                        .show()
                                        .expect("Showing notification failed");
                                }
                                RunCommandRequest::TYPE => {
                                    debug!("Received RunCommandRequest packet: {}", buffer);
                                }
                                SystemVolume::TYPE => {
                                    debug!("Received SystemVolume packet: {}", buffer);

                                    if let Ok(system_volume) =
                                        json::from_value::<SystemVolume>(packet.body)
                                    {
                                        match system_volume {
                                            SystemVolume::List { sink_list } => {
                                                task_state.systemvolume = Some(sink_list);
                                            }
                                            SystemVolume::Update {
                                                name,
                                                enabled,
                                                muted,
                                                volume,
                                            } => todo!(),
                                        }
                                    } else {
                                        warn!("Failed to parse SystemVolume data");
                                    }
                                }
                                _ => {
                                    warn!("Received unknown packet type: {}", packet.packet_type);
                                }
                            }
                        }
                    }
                    Err(e) => {
                        error!("Failed to read from stream: {}", e);
                        break; // Exit on read error
                    }
                }
                buffer.clear(); // Clear the buffer for the next read
                task_response
                    .send(DeviceResponse::Refresh((
                        task_self.clone(),
                        task_state.clone(),
                    )))
                    .unwrap_or_else(|e| {
                        error!("Failed to send device refresh response: {}", e);
                    });
            }

            task_response
                .send(DeviceResponse::Refresh((
                    task_self.clone(),
                    task_state.clone(),
                )))
                .unwrap_or_else(|e| {
                    error!("Failed to send device refresh response: {}", e);
                });
        });

        device_response
            .send(DeviceResponse::Refresh((
                boxed_self.clone(),
                device_state.clone(),
            )))
            .unwrap_or_else(|e| {
                error!("Failed to send device refresh response: {}", e);
            });

        loop {
            match self.action_rx.lock().await.next().await {
                Some(action) => {
                    debug!("Received action: {}", &action);
                    match &action {
                        DeviceAction::Disconnect => {
                            info!("Disconnecting device: {}", self.id);

                            self.writer
                                .lock()
                                .await
                                .shutdown()
                                .await
                                .unwrap_or_else(|e| {
                                    error!("Failed to shutdown writer: {}", e);
                                });

                            drop(boxed_self); // Drop the boxed self to close the stream

                            break; // Exit the loop on disconnect
                        }
                        DeviceAction::Refresh => {
                            debug!("Refreshing device state: {}", self.id);
                            device_response
                                .send(DeviceResponse::Refresh((
                                    boxed_self.clone(),
                                    device_state.clone(),
                                )))
                                .unwrap_or_else(|e| {
                                    error!("Failed to send device refresh response: {}", e);
                                });
                        }
                        DeviceAction::RequestedPairByPeer(packet) => {
                            debug!("Received pairing request from peer: {}", packet.packet_type);

                            if let Ok(pair) = json::from_value::<Pair>(packet.body.clone()) {
                                if pair.pair
                                    && let Some(_timestamp) = pair.timestamp
                                {
                                    debug!(
                                        "Setting pairing state to RequestedByPeer for device: {}",
                                        self.id
                                    );
                                    self.pairing_handler.pairing_state =
                                        PairingState::RequestedByPeer;

                                    self.pairing_handler.request_pairing().await;
                                    device_state.pairing_state = PairingState::Paired;
                                };

                                if !pair.pair {
                                    debug!(
                                        "Pairing request not accepted, setting state to NotPaired for device: {}",
                                        self.id
                                    );
                                    self.pairing_handler.pairing_state = PairingState::NotPaired;
                                    self.pairing_handler.unpair().await;
                                };
                            } else {
                                warn!("Failed to parse Pair data");
                            }
                        }
                        DeviceAction::Pair => {
                            self.pairing_handler.pairing_state = PairingState::Requested;
                            self.pairing_handler.request_pairing().await;
                            device_state.pairing_state = PairingState::Paired;
                        }
                        DeviceAction::UnPair => {
                            self.pairing_handler.unpair().await;
                            device_state.pairing_state = PairingState::NotPaired;

                            debug!("Unpairing finished: {}", id);
                        }
                        DeviceAction::Ping(msg) => {
                            self.ping(msg.clone()).await;
                        }
                    }

                    debug!("Action processed: {}", action);
                }
                None => {
                    debug!("No more actions to process, exiting loop");
                    break; // Exit if no more actions
                }
            }

            device_response
                .send(DeviceResponse::Refresh((
                    boxed_self.clone(),
                    device_state.clone(),
                )))
                .unwrap_or_else(|e| {
                    error!("Failed to send device refresh response: {}", e);
                });
        }
    }

    pub fn send(&self, action: DeviceAction) {
        if let Err(e) = self.action_tx.send(action) {
            error!("Failed to send action: {}", e);
        } else {
            debug!("Action sent successfully");
        }
    }
}

type DeviceStatePlayers =
    HashMap<String, (MprisPlayer, Option<String>, Option<Arc<JoinHandle<()>>>)>;

#[derive(Default, Debug, Clone)]
pub struct DeviceState {
    pub device_id: DeviceId,
    pub battery: Option<Battery>,
    pub clipboard: Option<String>,
    pub connectivity: Option<ConnectivityReport>,
    pub systemvolume: Option<Vec<SystemVolumeStream>>,
    pub players: DeviceStatePlayers,
    pub commands: HashMap<String, RunCommandItem>,
    pub pairing_state: PairingState,
}

impl DeviceState {
    pub fn new(device_id: DeviceId, pairing_state: PairingState) -> Self {
        Self {
            device_id,
            battery: None,
            clipboard: None,
            connectivity: None,
            systemvolume: None,
            players: HashMap::new(),
            commands: HashMap::new(),
            pairing_state,
        }
    }
}
