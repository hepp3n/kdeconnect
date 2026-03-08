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

// Re-export commonly used protocol types for external crates - M4L
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
    // Devices we already sent pair:true to this session — prevent double-sending
    #[allow(dead_code)]
    pending_pair: Arc<Mutex<std::collections::HashSet<DeviceId>>>,
}

impl KdeConnectCore {
    pub async fn new() -> anyhow::Result<(Self, mpsc::UnboundedReceiver<ConnectionEvent>)> {
        let (out_tx, in_rx) = mpsc::unbounded_channel();
        let (conn_tx, conn_rx) = mpsc::unbounded_channel();
        // Create separate channel for MPRIS proxy
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

        // Suppress unused warning for mpris_conn_rx — it is consumed by the mpris proxy
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
                info!(
                    "[core] packet received from device: {}",
                    device
                );
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

                // Send contacts request on pairing
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
                eprintln!("!!! SENDING PACKET !!!");
                eprintln!("  Device: {}", device_id.0);
                eprintln!("  Packet type: {:?}", packet.packet_type);
                eprintln!(
                    "  Body: {}",
                    serde_json::to_string_pretty(&packet.body).unwrap_or_default()
                );

                if let Some(sender) = guard.get(&device_id) {
                    eprintln!("  ✓ Found sender for device");
                    let result = sender.send(packet);
                    eprintln!("  Send result: {:?}", result.is_ok());
                } else {
                    eprintln!("  ✗ NO SENDER found for device!");
                    eprintln!(
                        "  Available devices: {:?}",
                        guard.keys().collect::<Vec<_>>()
                    );
                }
            }
            AppEvent::SendFiles((device_id, files_list)) => {
                info!("frontend trying to sent files to device: {}", device_id);

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

                    debug!("packet sent....");
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

                // Load current disabled set, apply change, save, update registry
                let mut disabled =
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

    async fn transport_events(&self, event: TransportEvent) {
        match event {
            TransportEvent::IncomingPacket { addr, id, raw } => {
                match ProtocolPacket::from_raw(raw.as_bytes()) {
                    Ok(packet) => {
                        let _ = self.event_tx.send(CoreEvent::PacketReceived {
                            device: id,
                            packet,
                        });
                    }
                    Err(e) => {
                        let _ = self.event_tx.send(CoreEvent::Error(format!(
                            "Invalid packet from {}: {}",
                            addr, e
                        )));
                    }
                }
            }
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

                // Duplicate guard: keep existing connection if still live.
                {
                    let guard = self.writer_map.lock().await;
                    if guard.contains_key(&id) {
                        debug!(
                            "[core] duplicate connection for {}, keeping existing",
                            id
                        );
                        return;
                    }
                }

                self.writer_map
                    .lock()
                    .await
                    .insert(id.clone(), write_tx.clone());

                // Load and apply saved plugin config for this device
                let disabled = plugin_config::load_disabled_plugins(&id.0).await;
                self.plugin_registry
                    .set_device_disabled(&id.0, disabled)
                    .await;

                let conn_event =
                    ConnectionEvent::Connected((id.clone(), device.clone()));
                let _ = self.conn_tx.send(conn_event.clone());
                let _ = self.mpris_conn_tx.send(conn_event);

                if device.pair_state == crate::device::PairState::Paired {
                    // Check plugin gating before auto-requesting
                    let sms_enabled = self
                        .plugin_registry
                        .is_plugin_enabled(&id.0, "sms")
                        .await;
                    let contacts_enabled = self
                        .plugin_registry
                        .is_plugin_enabled(&id.0, "contacts")
                        .await;

                    let sender = self.out_tx.clone();
                    let did = id.0.clone();
                    tokio::spawn(async move {
                        tokio::time::sleep(std::time::Duration::from_millis(500)).await;

                        if sms_enabled {
                            let sms_packet = ProtocolPacket::new(
                                PacketType::SmsRequestConversations,
                                serde_json::json!({}),
                            );
                            let _ = sender.send(AppEvent::SendPacket(
                                DeviceId(did.clone()),
                                sms_packet,
                            ));
                            eprintln!("📱 Auto-requested SMS conversations on connect");
                        } else {
                            eprintln!("📱 SMS plugin disabled — skipping auto-request");
                        }

                        if contacts_enabled {
                            let contacts_packet = ProtocolPacket::new(
                                PacketType::ContactsRequestAllUidsTimestamps,
                                serde_json::json!({}),
                            );
                            let _ = sender.send(AppEvent::SendPacket(
                                DeviceId(did),
                                contacts_packet,
                            ));
                            eprintln!("📇 Auto-requested live contacts sync on connect");
                        } else {
                            eprintln!("📇 Contacts plugin disabled — skipping auto-request");
                        }
                    });
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

    pub fn take_events(&self) -> Arc<mpsc::UnboundedSender<AppEvent>> {
        self.out_tx.clone()
    }
}
