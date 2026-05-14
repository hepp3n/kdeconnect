use std::{
    collections::HashMap,
    sync::{Arc, OnceLock},
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tokio::{
    select,
    sync::{Mutex, mpsc},
    time::MissedTickBehavior,
};
use tracing::{debug, error, info, warn};

use crate::{
    device::{Device, DeviceId, DeviceManager, PairState},
    event::{AppEvent, ConnectionEvent, CoreEvent},
    filetransfer::TransferAdapter,
    pairing::PairingManager,
    plugin_interface::PluginRegistry,
    plugins::{ping::Ping, share::ShareRequest},
    protocol::{DeviceFile, DevicePayload, Pair},
    transport::{TcpTransport, TransportEvent, UdpTransport},
};

/// Maximum allowed difference between device clocks for pairing (30 minutes in seconds).
const ALLOWED_TIMESTAMP_DIFF_SECS: u64 = 1800;

/// Auto-reject incoming/outgoing pairing requests after this duration (30 seconds).
const PAIRING_TIMEOUT_SECS: u64 = 30;

pub mod config;
pub(crate) mod crypto;
pub mod device;
pub mod event;
pub mod filetransfer;
pub(crate) mod pairing;
pub mod plugin_config;
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
    /// Tracks the conn_id of the most recently accepted connection per device.
    /// A `Disconnected` event whose conn_id doesn't match is from a superseded
    /// connection and is discarded so it cannot wipe a live writer_map entry.
    conn_id_map: Arc<Mutex<HashMap<DeviceId, u64>>>,
    event_tx: mpsc::UnboundedSender<CoreEvent>,
    event_rx: mpsc::UnboundedReceiver<CoreEvent>,
    udp_transport: Arc<UdpTransport>,
    out_tx: Arc<mpsc::UnboundedSender<AppEvent>>,
    in_rx: mpsc::UnboundedReceiver<AppEvent>,
    conn_tx: mpsc::UnboundedSender<ConnectionEvent>,
    mpris_conn_tx: mpsc::UnboundedSender<ConnectionEvent>,
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
        let conn_id_map = Arc::new(Mutex::new(HashMap::new()));

        let device_manager = DeviceManager::new(event_tx.clone());
        let pairing = Arc::new(PairingManager::new(device_manager.clone()));

        let tcp_transport = TcpTransport::new(&transport_tx);
        let udp_transport = Arc::new(UdpTransport::new(&transport_tx).await);

        tokio::spawn(async move {
            if let Err(e) = tcp_transport.listen().await {
                tracing::error!("TCP listener failed: {}", e);
            }
        });

        let udp = Arc::clone(&udp_transport);
        tokio::spawn(async move {
            if let Err(e) = udp.listen().await {
                error!("UDP listener failed: {}", e);
            }
        });

        // Load previously paired devices from disk so list_devices() returns
        // them immediately and we can actively try to reconnect them.
        if let Some(config_dir) = dirs::config_dir() {
            let kc_dir = config_dir.join(config::CONFIG_DIR);
            if let Ok(mut entries) = tokio::fs::read_dir(&kc_dir).await {
                while let Ok(Some(entry)) = entries.next_entry().await {
                    let path = entry.path();
                    if path.extension().and_then(|e| e.to_str()) == Some("ron") {
                        match tokio::fs::read_to_string(&path).await {
                            Ok(raw) => {
                                if let Ok(dev) = ron::de::from_str::<Device>(&raw)
                                    && dev.pair_state == PairState::Paired
                                {
                                    info!(
                                        "Restored paired device: {} ({})",
                                        dev.name, dev.device_id
                                    );
                                    device_manager
                                        .add_or_update_device(dev.device_id.clone(), dev.clone())
                                        .await;
                                }
                            }
                            Err(e) => {
                                warn!("Failed to read device file {}: {}", path.display(), e);
                            }
                        }
                    }
                }
            }
        }

        // Start broadcasting immediately so phones can discover us after restart.
        let udp_bc = Arc::clone(&udp_transport);
        tokio::spawn(async move {
            if let Err(e) = udp_bc.send_identity().await {
                error!("Initial UDP broadcast failed: {}", e);
            }
        });

        // Periodic reconnect: broadcast identity every 60s so disconnected
        // paired devices will see us and reconnect via UDP → TCP discovery.
        let reconnect_udp = Arc::clone(&udp_transport);
        let reconnect_dm = device_manager.clone();
        let reconnect_wm = Arc::clone(&writer_map);
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(60));
            interval.set_missed_tick_behavior(MissedTickBehavior::Skip);
            // Skip first tick — broadcast above already covers t=0.
            interval.tick().await;
            loop {
                interval.tick().await;
                let has_disconnected_paired = {
                    let devices = reconnect_dm.get_devices().await;
                    let writers = reconnect_wm.lock().await;
                    devices.iter().any(|d| {
                        d.pair_state == PairState::Paired && !writers.contains_key(&d.device_id)
                    })
                };
                if has_disconnected_paired {
                    debug!(
                        "Reconnect timer: broadcasting identity for disconnected paired devices"
                    );
                    if let Err(e) = reconnect_udp.send_identity().await {
                        error!("Reconnect broadcast failed: {}", e);
                    }
                }
            }
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
            device_id: None,
        };
        plugin_registry.register(Arc::new(sms_plugin)).await;

        plugins::mpris::init_telephony_signal();
        plugins::mpris::expose_phone_mpris(mpris_conn_rx, event_tx.clone());

        Ok((
            Self {
                device_manager: Arc::new(device_manager),
                pairing,
                plugin_registry,
                transport_rx,
                writer_map,
                conn_id_map,
                event_tx,
                event_rx,
                udp_transport,
                out_tx: Arc::new(out_tx),
                in_rx,
                conn_tx,
                mpris_conn_tx,
            },
            conn_rx,
        ))
    }

    pub async fn run_event_loop(&mut self) {
        info!("Starting KdeConnect event loop");

        plugins::mpris::monitor_mpris((*self.device_manager).clone(), self.event_tx.clone());

        loop {
            select! {
                maybe = self.event_rx.recv() => {
                    match maybe {
                        Some(event) => self.core_events(event).await,
                        None => {
                            error!("CoreEvent channel closed — aborting event loop");
                            break;
                        }
                    }
                }
                maybe_event = self.transport_rx.recv() => {
                    match maybe_event {
                        Some(event) => self.transport_events(event).await,
                        None => {
                            error!("Transport channel closed — aborting event loop");
                            break;
                        }
                    }
                }
                maybe_kde = self.in_rx.recv() => {
                    match maybe_kde {
                        Some(event) => self.kde_events(event).await,
                        None => {
                            error!("KdeEvent channel closed — aborting event loop");
                            break;
                        }
                    }
                }
            }
        }
    }

    async fn core_events(&self, event: CoreEvent) {
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
                    let contacts_pkt = ProtocolPacket::new(
                        PacketType::ContactsRequestAllUidsTimestamps,
                        serde_json::json!({}),
                    );
                    let _ = self.queue_packet(&device_id, contacts_pkt).await;
                }

                if self
                    .plugin_registry
                    .is_plugin_enabled(&device_id.0, "sms")
                    .await
                {
                    let sms_pkt = ProtocolPacket::new(
                        PacketType::SmsRequestConversations,
                        serde_json::json!({}),
                    );
                    let _ = self.queue_packet(&device_id, sms_pkt).await;
                }

                // Bootstrap MPRIS: request the phone's player list so
                // expose_phone_mpris can register D-Bus proxies for them.
                if self
                    .plugin_registry
                    .is_plugin_enabled(&device_id.0, "mpris")
                    .await
                {
                    let mpris_pkt = ProtocolPacket::new(
                        PacketType::MprisRequest,
                        serde_json::to_value(crate::plugins::mpris::MprisRequest {
                            request_player_list: Some(true),
                            ..Default::default()
                        })
                        .unwrap(),
                    );
                    let _ = self.queue_packet(&device_id, mpris_pkt).await;
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
                let _ = self.queue_packet(&device, packet).await;
            }
            CoreEvent::SendPaylod {
                device,
                packet,
                payload,
                payload_size,
            } => {
                info!("[core] sending packet w/ payload");

                let sender = {
                    let guard = self.writer_map.lock().await;
                    guard.get(&device).cloned()
                };

                if let Some(sender) = sender {
                    let transfer_adapter =
                        TransferAdapter::new(payload, payload_size, self.conn_tx.clone());
                    self.plugin_registry
                        .send_payload(packet, &sender, transfer_adapter, payload_size)
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
                device_type,
                incoming_capabilities,
                outgoing_capabilities,
                protocol_version,
                pairing_timestamp,
                peer_certificate,
                write_tx,
                conn_id,
            } => {
                debug!("[core] new connection from: {}", addr);

                let mut device = match Device::new(
                    id.0.clone(),
                    name,
                    device_type,
                    incoming_capabilities,
                    outgoing_capabilities,
                    addr,
                )
                .await
                {
                    Ok(d) => d,
                    Err(e) => {
                        tracing::error!("Failed to create device from metadata: {}", e);
                        return;
                    }
                };

                let stored_remote_certificate = device.remote_certificate.clone();
                if device.pair_state == crate::device::PairState::Paired
                    && !stored_remote_certificate.is_empty()
                    && stored_remote_certificate != peer_certificate
                {
                    warn!(
                        "[core] TLS certificate changed for paired device {}; unpairing",
                        id
                    );
                    device.remote_certificate.clear();
                    self.device_manager
                        .add_or_update_device(id.clone(), device.clone())
                        .await;
                    self.device_manager
                        .update_pair_state(&id, crate::device::PairState::NotPaired)
                        .await;
                    cleanup_device_data(&id.0).await;
                    let conn_event = ConnectionEvent::PairStateChanged((
                        id,
                        crate::device::PairState::NotPaired,
                    ));
                    let _ = self.conn_tx.send(conn_event.clone());
                    let _ = self.mpris_conn_tx.send(conn_event);
                    return;
                }

                let should_backfill_certificate = device.pair_state
                    == crate::device::PairState::Paired
                    && stored_remote_certificate.is_empty()
                    && !peer_certificate.is_empty();

                if !peer_certificate.is_empty() {
                    device.remote_certificate = peer_certificate;
                }

                if should_backfill_certificate {
                    let _ = device
                        .update_pair_state(crate::device::PairState::Paired)
                        .await;
                }

                self.device_manager
                    .add_or_update_device(id.clone(), device.clone())
                    .await;

                // Store the peer's protocol version and pairing timestamp.
                self.device_manager
                    .set_protocol_version(&id, protocol_version)
                    .await;
                if pairing_timestamp > 0 {
                    self.device_manager
                        .set_pairing_timestamp(&id, pairing_timestamp)
                        .await;
                }

                // Record this connection's ID before inserting the writer so that
                // any Disconnected event from a previous connection that arrives
                // concurrently will see a mismatched conn_id and be dropped.
                self.conn_id_map.lock().await.insert(id.clone(), conn_id);

                // Always replace — the phone reconnecting means the old connection
                // is intentionally superseded. Dropping the old write_tx here causes
                // the old writer task's recv() to return None, ending that task cleanly.
                self.writer_map.lock().await.insert(id.clone(), write_tx);

                if device.pair_state == crate::device::PairState::Paired {
                    if self
                        .plugin_registry
                        .is_plugin_enabled(&id.0, "contacts")
                        .await
                    {
                        let contacts_pkt = ProtocolPacket::new(
                            PacketType::ContactsRequestAllUidsTimestamps,
                            serde_json::json!({}),
                        );
                        let _ = self.queue_packet(&id, contacts_pkt).await;
                    }

                    if self
                        .plugin_registry
                        .is_plugin_enabled(&id.0, "notification")
                        .await
                    {
                        let notification_pkt = ProtocolPacket::new(
                            PacketType::NotificationRequest,
                            serde_json::json!({ "request": true }),
                        );
                        let _ = self.queue_packet(&id, notification_pkt).await;
                    }

                    if self.plugin_registry.is_plugin_enabled(&id.0, "sms").await {
                        let sms_pkt = ProtocolPacket::new(
                            PacketType::SmsRequestConversations,
                            serde_json::json!({}),
                        );
                        let _ = self.queue_packet(&id, sms_pkt).await;
                    }

                    if self
                        .plugin_registry
                        .is_plugin_enabled(&id.0, "battery")
                        .await
                    {
                        let battery_pkt = ProtocolPacket::new(
                            PacketType::BatteryRequest,
                            serde_json::json!({ "request": true }),
                        );
                        let _ = self.queue_packet(&id, battery_pkt).await;
                        plugins::battery::send_local_state(id.clone(), self.event_tx.clone()).await;
                    }

                    if self
                        .plugin_registry
                        .is_plugin_enabled(&id.0, "connectivity_report")
                        .await
                    {
                        let connectivity_pkt = ProtocolPacket::new(
                            PacketType::ConnectivityReportRequest,
                            serde_json::json!({}),
                        );
                        let _ = self.queue_packet(&id, connectivity_pkt).await;
                    }

                    if self
                        .plugin_registry
                        .is_plugin_enabled(&id.0, "mousepad")
                        .await
                    {
                        let keyboard_state_pkt = ProtocolPacket::new(
                            PacketType::MousePadKeyboardState,
                            serde_json::json!({ "state": true }),
                        );
                        let _ = self.queue_packet(&id, keyboard_state_pkt).await;
                    }

                    // Send our local command list so the Android app shows
                    // the Run Command option (requires canAddCommand: true).
                    if self
                        .plugin_registry
                        .is_plugin_enabled(&id.0, "runcommand")
                        .await
                    {
                        plugins::run_command::send_command_list(&id, self.event_tx.clone()).await;
                    }

                    if self
                        .plugin_registry
                        .is_plugin_enabled(&id.0, "systemvolume")
                        .await
                    {
                        plugins::systemvolume::on_device_connect(id.clone(), self.event_tx.clone());
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
                                    // Phone sent pair:false — either it's unpairing from us, or
                                    // it's a fresh device announcing it doesn't know us yet.
                                    match device.pair_state {
                                        crate::device::PairState::Paired => {
                                            info!(
                                                "[core] Phone unpairing from us — cleaning up {}",
                                                id
                                            );
                                            self.device_manager
                                                .update_pair_state(
                                                    &id,
                                                    crate::device::PairState::NotPaired,
                                                )
                                                .await;
                                            cleanup_device_data(&id.0).await;
                                            self.drop_connection(&id).await;
                                            let conn_event = ConnectionEvent::PairStateChanged((
                                                id.clone(),
                                                crate::device::PairState::NotPaired,
                                            ));
                                            let _ = self.conn_tx.send(conn_event.clone());
                                            let _ = self.mpris_conn_tx.send(conn_event);
                                        }
                                        crate::device::PairState::Requesting
                                        | crate::device::PairState::Requested => {
                                            info!(
                                                "[core] pairing with {} was rejected/cancelled by peer",
                                                id
                                            );
                                            self.device_manager
                                                .update_pair_state(
                                                    &id,
                                                    crate::device::PairState::NotPaired,
                                                )
                                                .await;
                                            let ev = ConnectionEvent::PairingTimedOut(id.clone());
                                            let _ = self.conn_tx.send(ev.clone());
                                            let _ = self.mpris_conn_tx.send(ev);
                                        }
                                        crate::device::PairState::NotPaired => {
                                            info!(
                                                "[core] pair:false from {} — device not paired, ignoring",
                                                id
                                            );
                                        }
                                    }
                                } else {
                                    let device_name = device.name.clone();
                                    let device_id_clone = id.clone();
                                    let is_new_request = self
                                        .pairing
                                        .handle_pair_request(
                                            device.device_id,
                                            device.name,
                                            device.address,
                                            pkt,
                                        )
                                        .await
                                        .unwrap_or(false);
                                    if is_new_request {
                                        let ev = ConnectionEvent::PairingRequested((
                                            device_id_clone,
                                            device_name,
                                        ));
                                        let _ = self.conn_tx.send(ev.clone());
                                        let _ = self.mpris_conn_tx.send(ev);

                                        // 30-second auto-reject: if the user doesn't respond,
                                        // reject the pairing so neither side waits indefinitely.
                                        let dm = self.device_manager.clone();
                                        let event_tx = self.event_tx.clone();
                                        let conn_tx = self.conn_tx.clone();
                                        let mpris_tx = self.mpris_conn_tx.clone();
                                        let did = id.clone();
                                        tokio::spawn(async move {
                                            tokio::time::sleep(Duration::from_secs(
                                                PAIRING_TIMEOUT_SECS,
                                            ))
                                            .await;
                                            if let Some(dev) = dm.get_device(&did).await
                                                && dev.pair_state
                                                    == crate::device::PairState::Requested
                                            {
                                                info!(
                                                    "[core] incoming pair request from {} timed out after {}s",
                                                    did, PAIRING_TIMEOUT_SECS
                                                );
                                                let pair = Pair::reject();
                                                let value = serde_json::to_value(pair)
                                                    .expect("fail serializing pair");
                                                let pkt =
                                                    ProtocolPacket::new(PacketType::Pair, value);
                                                let _ = event_tx.send(CoreEvent::SendPacket {
                                                    device: did.clone(),
                                                    packet: pkt,
                                                });
                                                dm.update_pair_state(
                                                    &did,
                                                    crate::device::PairState::NotPaired,
                                                )
                                                .await;
                                                let ev = ConnectionEvent::PairingTimedOut(did);
                                                let _ = conn_tx.send(ev.clone());
                                                let _ = mpris_tx.send(ev);
                                            }
                                        });
                                    }
                                }
                            }
                        } else if matches!(pkt.packet_type, PacketType::Identity) {
                            debug!("[core] identity packet from {}; ignoring in event loop", id);
                        } else if self
                            .device_manager
                            .get_device(&id)
                            .await
                            .map(|device| device.pair_state == crate::device::PairState::Paired)
                            .unwrap_or(false)
                        {
                            let _ = self.event_tx.send(CoreEvent::PacketReceived {
                                device: id.clone(),
                                packet: pkt.clone(),
                            });
                        } else {
                            warn!(
                                "[core] ignoring {:?} from unpaired device {}",
                                pkt.packet_type, id
                            );
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
            TransportEvent::Disconnected { id, conn_id } => {
                // Check whether this disconnect belongs to the connection that is
                // currently live for this device. If conn_id_map holds a *different*
                // (higher) ID, a newer connection has already taken over and this
                // event is stale — dropping it avoids wiping the live writer entry.
                let is_current = {
                    let guard = self.conn_id_map.lock().await;
                    guard
                        .get(&id)
                        .map(|&current| current == conn_id)
                        .unwrap_or(false)
                };

                if !is_current {
                    info!(
                        "[core] stale Disconnected for {} (conn_id {} != current) — ignoring",
                        id, conn_id
                    );
                    return;
                }

                self.writer_map.lock().await.remove(&id);
                self.conn_id_map.lock().await.remove(&id);
                info!("[core] removed dead connection for {}", id);
                let conn_event = ConnectionEvent::Disconnected(id);
                let _ = self.conn_tx.send(conn_event.clone());
                let _ = self.mpris_conn_tx.send(conn_event);
            }
            TransportEvent::PairTrustFailed { id } => {
                warn!(
                    "[core] certificate trust failed for paired device {}; unpairing",
                    id
                );
                if let Some(mut device) = self.device_manager.get_device(&id).await {
                    device.remote_certificate.clear();
                    self.device_manager
                        .add_or_update_device(id.clone(), device)
                        .await;
                }
                self.device_manager
                    .update_pair_state(&id, crate::device::PairState::NotPaired)
                    .await;
                cleanup_device_data(&id.0).await;
                let conn_event =
                    ConnectionEvent::PairStateChanged((id, crate::device::PairState::NotPaired));
                let _ = self.conn_tx.send(conn_event.clone());
                let _ = self.mpris_conn_tx.send(conn_event);
            }
        }
    }

    async fn kde_events(&self, event: AppEvent) {
        match event {
            AppEvent::Broadcasting => {
                let _ = self.udp_transport.send_identity().await;
            }
            AppEvent::Pair(device_id) => {
                info!("frontend sent pair event to device: {}", device_id);
                let pair = Pair::request();
                let value = serde_json::to_value(pair).expect("fail serializing pair");
                let pkt = ProtocolPacket::new(PacketType::Pair, value);
                if self.queue_packet(&device_id, pkt).await {
                    info!("Sent pair request packet to device: {}", device_id);
                    self.device_manager
                        .update_pair_state(&device_id, crate::device::PairState::Requesting)
                        .await;

                    // 30-second auto-cancel: if the phone doesn't respond in time,
                    // revert to NotPaired so the user isn't stuck in the pairing state.
                    let dm = self.device_manager.clone();
                    let event_tx = self.event_tx.clone();
                    let conn_tx = self.conn_tx.clone();
                    let mpris_tx = self.mpris_conn_tx.clone();
                    let did = device_id.clone();
                    tokio::spawn(async move {
                        tokio::time::sleep(Duration::from_secs(PAIRING_TIMEOUT_SECS)).await;
                        if let Some(dev) = dm.get_device(&did).await
                            && dev.pair_state == crate::device::PairState::Requesting
                        {
                            info!(
                                "[core] outgoing pair request to {} timed out after {}s",
                                did, PAIRING_TIMEOUT_SECS
                            );
                            let pair = Pair::reject();
                            let value = serde_json::to_value(pair).expect("fail serializing pair");
                            let pkt = ProtocolPacket::new(PacketType::Pair, value);
                            let _ = event_tx.send(CoreEvent::SendPacket {
                                device: did.clone(),
                                packet: pkt,
                            });
                            dm.update_pair_state(&did, crate::device::PairState::NotPaired)
                                .await;
                            let ev = ConnectionEvent::PairingTimedOut(did);
                            let _ = conn_tx.send(ev.clone());
                            let _ = mpris_tx.send(ev);
                        }
                    });
                } else {
                    warn!(
                        "[core] failed to send pair request to {} (no connection)",
                        device_id
                    );
                    let ev = ConnectionEvent::PairingTimedOut(device_id);
                    let _ = self.conn_tx.send(ev.clone());
                    let _ = self.mpris_conn_tx.send(ev);
                }
            }
            AppEvent::Ping((device_id, msg)) => {
                info!("frontend sent ping event to device: {}", device_id);
                let value = serde_json::to_value(Ping {
                    message: Some(msg),
                    ..Default::default()
                })
                .expect("fail serializing packet body");
                let pkt = ProtocolPacket::new(PacketType::Ping, value);
                let _ = self.queue_packet(&device_id, pkt).await;
            }
            AppEvent::SendPacket(device_id, packet) => {
                info!("Sending packet to device: {}", device_id);
                let _ = self.queue_packet(&device_id, packet).await;
            }
            AppEvent::SendFiles((device_id, files_list)) => {
                info!("frontend trying to sent files to device: {}", device_id);

                let sender = {
                    let guard = self.writer_map.lock().await;
                    guard.get(&device_id).cloned()
                };

                if let Some(sender) = sender {
                    debug!("sender available.");
                    let pkts = match ShareRequest::share_files(files_list).await {
                        Ok(pkts) => pkts,
                        Err(e) => {
                            tracing::warn!("[share] failed to prepare share request: {}", e);
                            return;
                        }
                    };
                    for (pkt_body, path) in pkts {
                        let packet = ProtocolPacket::new(
                            PacketType::ShareRequest,
                            serde_json::to_value(pkt_body).expect("serializing packet body"),
                        );
                        let file = match DeviceFile::open(&path).await {
                            Ok(file) => file,
                            Err(e) => {
                                tracing::warn!("[share] failed to open '{}': {}", path, e);
                                continue;
                            }
                        };
                        let payload = DevicePayload::from(file);

                        let transfer_adapter =
                            TransferAdapter::new(payload.buf, payload.size, self.conn_tx.clone());

                        self.plugin_registry
                            .send_payload(packet, &sender, transfer_adapter, payload.size)
                            .await;
                    }

                    debug!("file transfer tasks spawned.");
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

                let pair = Pair::reject();
                let value = serde_json::to_value(pair).expect("fail serializing pair");
                let pkt = ProtocolPacket::new(PacketType::Pair, value);
                if self.queue_packet(&device_id, pkt).await {
                    info!("[core] sent pair:false to {} on unpair", device_id);
                }

                self.device_manager
                    .update_pair_state(&device_id, crate::device::PairState::NotPaired)
                    .await;
                cleanup_device_data(&device_id.0).await;
                self.drop_connection(&device_id).await;

                let conn_event = ConnectionEvent::PairStateChanged((
                    device_id,
                    crate::device::PairState::NotPaired,
                ));
                let _ = self.conn_tx.send(conn_event.clone());
                let _ = self.mpris_conn_tx.send(conn_event);
            }
            AppEvent::AcceptPairing(device_id) => {
                info!("User accepted pairing from {}", device_id);

                // Clock-sync validation per KDE Connect protocol v8+:
                // if the phone's pairing timestamp and our current time differ
                // by more than 30 minutes, reject the pairing to prevent
                // security issues from clock skew.
                if let Some(dev) = self.device_manager.get_device(&device_id).await
                    && dev.protocol_version >= 8
                {
                    let phone_ts = dev.pairing_timestamp;
                    if phone_ts > 0 {
                        let now = SystemTime::now()
                            .duration_since(UNIX_EPOCH)
                            .unwrap_or_default()
                            .as_secs();
                        let diff = phone_ts.abs_diff(now);
                        if diff > ALLOWED_TIMESTAMP_DIFF_SECS {
                            warn!(
                                "[core] pairing rejected for {}: clocks out of sync (phone_ts={}, local_ts={}, diff={}s)",
                                device_id, phone_ts, now, diff
                            );
                            // Reject by sending pair:false and un-setting paired state.
                            let pair = Pair::reject();
                            let value = serde_json::to_value(pair).expect("fail serializing pair");
                            let pkt = ProtocolPacket::new(PacketType::Pair, value);
                            if self.queue_packet(&device_id, pkt).await {
                                info!(
                                    "[core] sent pair:false to {} due to clock mismatch",
                                    device_id
                                );
                            }
                            self.device_manager
                                .update_pair_state(&device_id, crate::device::PairState::NotPaired)
                                .await;
                            // Emit a timed-out event so the UI can show a message.
                            let ev = ConnectionEvent::PairingTimedOut(device_id);
                            let _ = self.conn_tx.send(ev.clone());
                            let _ = self.mpris_conn_tx.send(ev);
                            return;
                        }
                    }
                }

                let pair = Pair::accept();
                let value = serde_json::to_value(pair).expect("fail serializing pair");
                let pkt = ProtocolPacket::new(PacketType::Pair, value);
                if self.queue_packet(&device_id, pkt).await {
                    info!("[core] sent pair:true to {} on accept", device_id);
                    self.device_manager.set_paired(&device_id, true).await;
                } else {
                    warn!(
                        "[core] failed to send pair acceptance to {} (no connection)",
                        device_id
                    );
                    self.device_manager
                        .update_pair_state(&device_id, crate::device::PairState::NotPaired)
                        .await;
                    let ev = ConnectionEvent::PairingTimedOut(device_id);
                    let _ = self.conn_tx.send(ev.clone());
                    let _ = self.mpris_conn_tx.send(ev);
                }
            }
            AppEvent::RejectPairing(device_id) => {
                info!("User rejected pairing from {}", device_id);
                let pair = Pair::reject();
                let value = serde_json::to_value(pair).expect("fail serializing pair");
                let pkt = ProtocolPacket::new(PacketType::Pair, value);
                if self.queue_packet(&device_id, pkt).await {
                    info!("[core] sent pair:false to {} on reject", device_id);
                }
                self.device_manager
                    .update_pair_state(&device_id, crate::device::PairState::NotPaired)
                    .await;
            }
            AppEvent::Disconnect(device_id) => {
                info!("frontend sent disconnect event to device: {}", device_id);
                self.conn_id_map.lock().await.remove(&device_id);
                let mut guard = self.writer_map.lock().await;
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
                    disabled.insert(plugin_id.clone());
                }
                plugin_config::save_disabled_plugins(&device_id.0, &disabled).await;
                self.plugin_registry
                    .set_device_disabled(&device_id.0, disabled.clone())
                    .await;

                {
                    self.conn_id_map.lock().await.remove(&device_id);
                    let mut guard = self.writer_map.lock().await;
                    if guard.remove(&device_id).is_some() {
                        info!(
                            "[plugin] dropped connection to {} — phone will reconnect with updated capabilities",
                            device_id
                        );
                    }
                }
            }
        };
    }

    async fn queue_packet(&self, device_id: &DeviceId, packet: ProtocolPacket) -> bool {
        let sender = {
            let guard = self.writer_map.lock().await;
            guard.get(device_id).cloned()
        };

        let Some(sender) = sender else {
            debug!(
                "No sender for device {} — available: {:?}",
                device_id,
                self.writer_map.lock().await.keys().collect::<Vec<_>>()
            );
            return false;
        };

        if let Err(e) = sender.send(packet) {
            tracing::warn!(
                "[core] failed to queue packet for {}: {}; removing stale writer",
                device_id,
                e
            );
            self.writer_map.lock().await.remove(device_id);
            self.conn_id_map.lock().await.remove(device_id);
            let conn_event = ConnectionEvent::Disconnected(device_id.clone());
            let _ = self.conn_tx.send(conn_event.clone());
            let _ = self.mpris_conn_tx.send(conn_event);
            return false;
        }

        true
    }

    async fn drop_connection(&self, device_id: &DeviceId) -> bool {
        self.conn_id_map.lock().await.remove(device_id);
        let removed = self.writer_map.lock().await.remove(device_id).is_some();
        if removed {
            info!("[core] dropped connection for {}", device_id);
        }
        removed
    }

    pub fn take_events(&self) -> Arc<mpsc::UnboundedSender<AppEvent>> {
        self.out_tx.clone()
    }
}

/// Remove all persisted data for a device on unpair.
/// Cleans plugin config (~/.config/kdeconnect/) and cache (~/.local/share/kdeconnect/).
pub async fn cleanup_device_data(device_id: &str) {
    // Plugin enabled/disabled config
    if let Some(config_dir) = dirs::config_dir() {
        let plugin_file = config_dir
            .join("kdeconnect")
            .join(format!("{}_plugins.json", device_id));
        if let Err(e) = tokio::fs::remove_file(&plugin_file).await
            && e.kind() != std::io::ErrorKind::NotFound
        {
            tracing::warn!(
                "[cleanup] failed to remove plugin config for {}: {}",
                device_id,
                e
            );
        }

        let device_file = config_dir
            .join("kdeconnect")
            .join(format!("{}.ron", device_id));
        if let Err(e) = tokio::fs::remove_file(&device_file).await
            && e.kind() != std::io::ErrorKind::NotFound
        {
            tracing::warn!(
                "[cleanup] failed to remove device identity for {}: {}",
                device_id,
                e
            );
        }
    }

    // SMS and contacts cache directory
    if let Some(data_dir) = dirs::data_local_dir() {
        let cache_dir = data_dir.join("kdeconnect").join(device_id);
        if let Err(e) = tokio::fs::remove_dir_all(&cache_dir).await
            && e.kind() != std::io::ErrorKind::NotFound
        {
            tracing::warn!(
                "[cleanup] failed to remove cache dir for {}: {}",
                device_id,
                e
            );
        }
    }

    tracing::info!("[cleanup] removed persisted data for device {}", device_id);
}
