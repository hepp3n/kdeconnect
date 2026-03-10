use std::{
    collections::HashMap,
    sync::{Arc, OnceLock},
};
use tokio::{
    select,
    sync::{Mutex, mpsc},
};
use tracing::{debug, info};

use crate::{
    device::{Device, DeviceId, DeviceManager},
    event::{AppEvent, ConnectionEvent, CoreEvent},
    pairing::PairingManager,
    plugin_interface::PluginRegistry,
    plugins::{ping::Ping, share::ShareRequest},
    protocol::{DeviceFile, DevicePayload, Pair},
    transport::{TcpTransport, TransportEvent, UdpTransport},
};

pub mod config;
pub mod plugin_config;
pub(crate) mod crypto;
pub mod device;
pub mod event;
pub(crate) mod pairing;
pub(crate) mod plugin_interface;
pub mod plugins;
pub(crate) mod protocol;
pub(crate) mod transport;

// Re-export commonly used protocol types for external crates
pub use protocol::{PacketType, ProtocolPacket};

pub static GLOBAL_CONFIG: OnceLock<config::Config> = OnceLock::new();

pub struct KdeConnectCore {
    device_manager: Arc<DeviceManager>,
    pairing: Arc<PairingManager>,
    plugin_registry: Arc<PluginRegistry>,
    transport_rx: mpsc::UnboundedReceiver<TransportEvent>,
    writer_map: Arc<Mutex<HashMap<DeviceId, mpsc::UnboundedSender<ProtocolPacket>>>>,
    event_tx: mpsc::UnboundedSender<CoreEvent>,
    event_rx: mpsc::UnboundedReceiver<CoreEvent>,
    udp_transport: Arc<UdpTransport>,
    out_tx: Arc<mpsc::UnboundedSender<AppEvent>>,
    in_rx: mpsc::UnboundedReceiver<AppEvent>,
    conn_tx: mpsc::UnboundedSender<ConnectionEvent>,
    mpris_conn_tx: mpsc::UnboundedSender<ConnectionEvent>,
    pending_pair: Arc<Mutex<std::collections::HashSet<DeviceId>>>,
}

impl KdeConnectCore {
    pub async fn new() -> anyhow::Result<(Self, mpsc::UnboundedReceiver<ConnectionEvent>)> {
        let (out_tx, in_rx) = mpsc::unbounded_channel();
        let (conn_tx, conn_rx) = mpsc::unbounded_channel();
        let (mpris_conn_tx, mpris_conn_rx) = mpsc::unbounded_channel();

        let plugin_registry = Arc::new(PluginRegistry::new());

        let outgoing_capabilities = plugin_registry.list_plugins().await;
        let config = config::Config::load(outgoing_capabilities).await?;

        GLOBAL_CONFIG
            .set(config)
            .expect("Config already initialized");

        let (transport_tx, transport_rx) = mpsc::unbounded_channel();
        let (event_tx, event_rx) = mpsc::unbounded_channel();
        let writer_map = Arc::new(Mutex::new(HashMap::new()));

        let device_manager = DeviceManager::new(event_tx.clone());
        let pairing = Arc::new(PairingManager::new(device_manager.clone()));

        let tcp_transport = TcpTransport::new(&transport_tx);
        let udp_transport = Arc::new(UdpTransport::new(&transport_tx).await);

        tokio::spawn(async move {
            let _ = tcp_transport.listen().await;
        });

        let run_command_plugin = plugins::run_command::RunCommandRequest::default();
        plugin_registry.register(Arc::new(run_command_plugin)).await;
        let share_request_plugin = plugins::share::ShareRequest::default();
        plugin_registry
            .register(Arc::new(share_request_plugin))
            .await;
        let sms_plugin = plugins::sms::SmsMessages {
            messages: Vec::new(),
            version: None,
        };
        plugin_registry.register(Arc::new(sms_plugin)).await;

        let _ = mpris_conn_rx;

        Ok((
            Self {
                device_manager: Arc::new(device_manager),
                pairing,
                plugin_registry,
                transport_rx,
                writer_map,
                event_tx,
                event_rx,
                udp_transport,
                out_tx: Arc::new(out_tx),
                in_rx,
                conn_tx,
                mpris_conn_tx,
                pending_pair: Arc::new(Mutex::new(std::collections::HashSet::new())),
            },
            conn_rx,
        ))
    }

    pub async fn run_event_loop(&mut self) {
        info!("Starting KdeConnect event loop");

        loop {
            select! {
                maybe = self.event_rx.recv() => {
                    match maybe {
                        Some(event) => self.core_events(event).await,
                        None => {
                            let _ = self.event_tx.send(CoreEvent::Error("CoreEvent channel closed".to_string()));
                        }
                    }
                }
                maybe_event = self.transport_rx.recv() => {
                    match maybe_event {
                        Some(event) => self.transport_events(event).await,
                        None => {
                            let _ = self.event_tx.send(CoreEvent::Error("Transport channel closed".to_string()));
                        }
                    }
                }
                maybe_kde = self.in_rx.recv() => {
                    match maybe_kde {
                        Some(event) => self.kde_events(event).await,
                        None => {
                            let _ = self.event_tx.send(CoreEvent::Error("KdeEvent channel closed".to_string()));
                        }
                    }
                }
            }
        }
    }

    async fn core_events(&self, event: CoreEvent) {
        let guard = self.writer_map.lock().await;

        match event {
            CoreEvent::PacketReceived { device, packet } => {
                info!("[core] packet received from device: {}", device);
                if let Some(device_obj) = self.device_manager.get_device(&device).await {
                    self.plugin_registry
                        .dispatch(
                            device_obj,
                            packet,
                            self.event_tx.clone(),
                            self.conn_tx.clone(),
                            self.mpris_conn_tx.clone(),
                        )
                        .await;
                }
            }
            CoreEvent::DeviceDiscovered(_device) => {
                debug!("[core] device discovered.");
            }
            CoreEvent::DevicePaired((device_id, device)) => {
                info!("[core] device paired: {}", device_id);

                if self
                    .plugin_registry
                    .is_plugin_enabled(&device_id.0, "contacts")
                    .await
                {
                    if let Some(sender) = guard.get(&device_id) {
                        let contacts_pkt = ProtocolPacket::new(
                            PacketType::ContactsRequestAllUidsTimestamps,
                            serde_json::json!({}),
                        );
                        let _ = sender.send(contacts_pkt);
                    }
                }

                let conn_event = ConnectionEvent::DevicePaired((device_id, device));
                let _ = self.conn_tx.send(conn_event.clone());
                let _ = self.mpris_conn_tx.send(conn_event);
            }
            CoreEvent::DevicePairCancelled(device_id) => {
                info!("[core] device pair cancelled.");
                let conn_event = ConnectionEvent::PairStateChanged((
                    device_id,
                    crate::device::PairState::NotPaired,
                ));
                let _ = self.conn_tx.send(conn_event.clone());
                let _ = self.mpris_conn_tx.send(conn_event);
            }
            CoreEvent::DevicePairStateChanged((device_id, pair_state)) => {
                let conn_event = ConnectionEvent::PairStateChanged((device_id, pair_state));
                let _ = self.conn_tx.send(conn_event.clone());
                let _ = self.mpris_conn_tx.send(conn_event);
            }
            CoreEvent::SendPacket { device, packet } => {
                info!("[core] sending packet");
                if let Some(sender) = guard.get(&device) {
                    sender.send(packet).unwrap();
                }
            }
            CoreEvent::SendPaylod {
                device,
                packet,
                payload,
                payload_size,
            } => {
                info!("[core] sending packet w/ payload");
                if let Some(sender) = guard.get(&device) {
                    self.plugin_registry
                        .send_payload(packet, sender, payload, payload_size)
                        .await;
                }
            }
            CoreEvent::Error(msg) => {
                tracing::error!("{}", msg);
            }
        };
    }

    async fn transport_events(&self, event: TransportEvent) {
        match event {
            TransportEvent::NewConnection {
                addr,
                id,
                name,
                write_tx,
            } => {
                debug!("[core] new connection from: {}", addr);

                let device = Device::new(id.0.clone(), name, addr)
                    .await
                    .expect("cannot create new device from metadata");

                self.device_manager
                    .add_or_update_device(id.clone(), device.clone())
                    .await;

                let accepted = {
                    let mut guard = self.writer_map.lock().await;
                    match guard.get(&id) {
                        Some(existing) if !existing.is_closed() => {
                            info!(
                                "[core] Duplicate connection from {} — keeping existing live connection",
                                id
                            );
                            false
                        }
                        _ => {
                            guard.insert(id.clone(), write_tx);
                            true
                        }
                    }
                };

                if !accepted {
                    return;
                }

                info!("new connection sent to frontend");

                self.pending_pair.lock().await.remove(&id);

                if device.pair_state == crate::device::PairState::Paired {
                    let guard = self.writer_map.lock().await;
                    if let Some(sender) = guard.get(&id) {
                        let contacts_pkt = ProtocolPacket::new(
                            PacketType::ContactsRequestAllUidsTimestamps,
                            serde_json::json!({}),
                        );
                        let _ = sender.send(contacts_pkt);
                    }
                }

                let conn_event = ConnectionEvent::Connected((id.clone(), device.clone()));
                let _ = self.conn_tx.send(conn_event.clone());
                let _ = self.mpris_conn_tx.send(conn_event);
            }
            TransportEvent::IncomingPacket { addr, id, raw } => {
                info!("[core] incoming packet.");
                match serde_json::from_str::<ProtocolPacket>(&raw) {
                    Ok(pkt) => {
                        if let PacketType::Pair = pkt.packet_type {
                            if let Ok(pair_body) = serde_json::from_value::<Pair>(pkt.body.clone())
                                && let Some(device) = self.device_manager.get_device(&id).await
                            {
                                if !pair_body.pair {
                                    if self.pending_pair.lock().await.contains(&id) {
                                        info!(
                                            "[core] pair:false from {} — already sent pair:true, ignoring duplicate",
                                            id
                                        );
                                    } else {
                                        info!(
                                            "[core] Phone sent pair:false — auto-initiating pair request to {}",
                                            id
                                        );
                                        let guard = self.writer_map.lock().await;
                                        if let Some(sender) = guard.get(&id) {
                                            let pair = Pair::new(true);
                                            let value = serde_json::to_value(pair)
                                                .expect("fail serializing pair");
                                            let pair_pkt =
                                                ProtocolPacket::new(PacketType::Pair, value);
                                            let _ = sender.send(pair_pkt);
                                            self.pending_pair.lock().await.insert(id.clone());
                                        }
                                        drop(guard);
                                        self.device_manager
                                            .update_pair_state(
                                                &id,
                                                crate::device::PairState::Requesting,
                                            )
                                            .await;
                                    }
                                } else {
                                    let _ = self
                                        .pairing
                                        .handle_pair_request(
                                            device.device_id,
                                            device.name,
                                            device.address,
                                            pkt,
                                        )
                                        .await;
                                }
                            }
                        } else {
                            let _ = self.event_tx.send(CoreEvent::PacketReceived {
                                device: id.clone(),
                                packet: pkt.clone(),
                            });
                        }
                    }
                    Err(e) => {
                        let _ = self.event_tx.send(CoreEvent::Error(format!(
                            "Invalid packet from {}: {}",
                            addr, e
                        )));
                    }
                }
            }
            TransportEvent::Disconnected { id } => {
                self.writer_map.lock().await.remove(&id);
                info!("[core] removed dead connection for {}", id);
                let conn_event = ConnectionEvent::Disconnected(id);
                let _ = self.conn_tx.send(conn_event.clone());
                let _ = self.mpris_conn_tx.send(conn_event);
            }
        }
    }

    async fn kde_events(&self, event: AppEvent) {
        let mut guard = self.writer_map.lock().await;

        match event {
            AppEvent::Broadcasting => {
                let _ = self.udp_transport.send_identity().await;
            }
            AppEvent::Pair(device_id) => {
                info!("frontend sent pair event to device: {}", device_id);
                if let Some(sender) = guard.get(&device_id) {
                    let pair = Pair::new(true);
                    let value = serde_json::to_value(pair).expect("fail serializing pair");
                    let pkt = ProtocolPacket::new(PacketType::Pair, value);
                    let _ = sender.send(pkt);
                    info!("Sent pair request packet to device: {}", device_id);
                }
                self.device_manager
                    .update_pair_state(&device_id, crate::device::PairState::Requesting)
                    .await;
            }
            AppEvent::Ping((device_id, msg)) => {
                info!("frontend sent ping event to device: {}", device_id);
                let value = serde_json::to_value(Ping { message: Some(msg) })
                    .expect("fail serializing packet body");
                let pkt = ProtocolPacket::new(PacketType::Ping, value);
                if let Some(device) = self.device_manager.get_device(&device_id).await {
                    self.plugin_registry
                        .send(device.clone(), pkt, self.event_tx.clone())
                        .await;
                };
            }
            AppEvent::SendPacket(device_id, packet) => {
                info!("Sending packet to device: {}", device_id);
                if let Some(sender) = guard.get(&device_id) {
                    let _ = sender.send(packet);
                } else {
                    debug!(
                        "No sender for device {} — available: {:?}",
                        device_id,
                        guard.keys().collect::<Vec<_>>()
                    );
                }
            }
            AppEvent::SendFiles((device_id, files_list)) => {
                info!("frontend trying to send files to device: {}", device_id);
                if let Some(sender) = guard.get(&device_id) {
                    debug!("sender available.");
                    let pkts = ShareRequest::share_files(files_list)
                        .await
                        .expect("creating share request");
                    for (pkt_body, path) in pkts {
                        let packet = ProtocolPacket::new(
                            PacketType::ShareRequest,
                            serde_json::to_value(pkt_body).expect("serializing packet body"),
                        );
                        let file = DeviceFile::open(path).await.expect("opening file");
                        let payload = DevicePayload::from(file);
                        if let Some(_device) = self.device_manager.get_device(&device_id).await {
                            self.plugin_registry
                                .send_payload(packet, sender, payload.buf, payload.size)
                                .await;
                        };
                    }
                    debug!("packet sent.");
                }
            }
            AppEvent::MprisAction((device_id, player_name, action)) => {
                info!(
                    "frontend sent mpris action to device: {} player: {}",
                    device_id, player_name
                );
                let request = crate::plugins::mpris::MprisRequest {
                    player: Some(player_name),
                    request_now_playing: None,
                    request_player_list: None,
                    request_volume: None,
                    seek: None,
                    set_loop_status: None,
                    set_position: None,
                    set_shuffle: None,
                    set_volume: None,
                    action: Some(action),
                    album_art_url: None,
                };
                let value = serde_json::to_value(request).expect("fail serializing packet body");
                let pkt = ProtocolPacket::new(PacketType::MprisRequest, value);
                if let Some(device) = self.device_manager.get_device(&device_id).await {
                    self.plugin_registry
                        .send(device.clone(), pkt, self.event_tx.clone())
                        .await;
                };
            }
            AppEvent::SendMprisRequest((device_id, request)) => {
                info!("frontend sent mpris request to device: {}", device_id);
                let value = serde_json::to_value(request).expect("fail serializing packet body");
                let pkt = ProtocolPacket::new(PacketType::MprisRequest, value);
                if let Some(device) = self.device_manager.get_device(&device_id).await {
                    self.plugin_registry
                        .send(device.clone(), pkt, self.event_tx.clone())
                        .await;
                };
            }
            AppEvent::Unpair(device_id) => {
                info!("frontend sent unpair event to device: {}", device_id);
                let _ = self.pairing.cancel_pairing(device_id).await;
            }
            AppEvent::Disconnect(device_id) => {
                info!("frontend sent disconnect event to device: {}", device_id);
                if guard.remove(&device_id).is_some() {
                    let conn_event = ConnectionEvent::Disconnected(device_id);
                    let _ = self.conn_tx.send(conn_event.clone());
                    let _ = self.mpris_conn_tx.send(conn_event);
                    info!("Connection closed.");
                }
            }
            AppEvent::SetPluginEnabled {
                device_id,
                plugin_id,
                enabled,
            } => {
                info!(
                    "[plugin] {} plugin '{}' for device {}",
                    if enabled { "enabling" } else { "disabling" },
                    plugin_id,
                    device_id
                );
                let mut disabled: std::collections::HashSet<String> =
                    plugin_config::load_disabled_plugins(&device_id.0).await;
                if enabled {
                    disabled.remove(&plugin_id);
                } else {
                    disabled.insert(plugin_id);
                }
                plugin_config::save_disabled_plugins(&device_id.0, &disabled).await;
                self.plugin_registry
                    .set_device_disabled(&device_id.0, disabled)
                    .await;
            }
        };
    }

    pub fn take_events(&self) -> Arc<mpsc::UnboundedSender<AppEvent>> {
        self.out_tx.clone()
    }
}
