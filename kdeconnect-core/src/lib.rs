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
    protocol::{DeviceFile, DevicePayload, PacketType, Pair, ProtocolPacket},
    transport::{TcpTransport, TransportEvent, UdpTransport},
};

pub mod config;
pub(crate) mod crypto;
pub mod device;
pub mod event;
pub(crate) mod pairing;
pub(crate) mod plugin_interface;
pub(crate) mod plugins;
pub(crate) mod protocol;
pub(crate) mod transport;

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
}

impl KdeConnectCore {
    pub async fn new() -> anyhow::Result<(Self, mpsc::UnboundedReceiver<ConnectionEvent>)> {
        let (out_tx, in_rx) = mpsc::unbounded_channel();
        let (conn_tx, conn_rx) = mpsc::unbounded_channel();

        let plugin_registry = Arc::new(PluginRegistry::new());

        let outgoing_capabilities = plugin_registry.list_plugins().await;
        let config = config::Config::load(outgoing_capabilities).await?;
        GLOBAL_CONFIG.set(config).unwrap();

        let (event_tx, event_rx) = mpsc::unbounded_channel();
        let (transport_tx, transport_rx) = mpsc::unbounded_channel();
        let transport = Arc::new(TcpTransport::new(&transport_tx));
        let udp_transport = Arc::new(UdpTransport::new(&transport_tx).await);
        let device_manager = DeviceManager::new(event_tx.clone());
        let pairing = Arc::new(PairingManager::new(device_manager.clone()));
        let writer_map = Arc::new(Mutex::new(HashMap::new()));

        // start tcp transport
        tokio::spawn(async move {
            let _ = transport.listen().await;
        });

        let udp = Arc::clone(&udp_transport);

        tokio::spawn(async move {
            let _ = udp.listen().await;
        });

        // register plugins
        let ping_plugin = plugins::ping::Ping::default();
        plugin_registry.register(Arc::new(ping_plugin)).await;
        let battery_plugin = plugins::battery::Battery::default();
        plugin_registry.register(Arc::new(battery_plugin)).await;
        let clipboard_plugin = plugins::clipboard::Clipboard::default();
        plugin_registry.register(Arc::new(clipboard_plugin)).await;
        let mousepad_keyboardstate = plugins::mousepad::KeyboardState::default();
        plugin_registry
            .register(Arc::new(mousepad_keyboardstate))
            .await;
        let mpris_plugin = plugins::mpris::Mpris::default();
        plugin_registry.register(Arc::new(mpris_plugin)).await;
        let notification_plugin = plugins::notification::Notification::default();
        plugin_registry
            .register(Arc::new(notification_plugin))
            .await;
        let connectivity_report_plugin =
            plugins::connectivity_report::ConnectivityReport::default();
        plugin_registry
            .register(Arc::new(connectivity_report_plugin))
            .await;
        let run_command_plugin = plugins::run_command::RunCommandRequest::default();
        plugin_registry.register(Arc::new(run_command_plugin)).await;
        let share_request_plugin = plugins::share::ShareRequest::default();
        plugin_registry
            .register(Arc::new(share_request_plugin))
            .await;

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
                info!("[core] packet received: {}", packet.packet_type);

                if let Some(device) = self.device_manager.get_device(&device).await {
                    // dispatch to plugins
                    self.plugin_registry
                        .dispatch(
                            device,
                            packet.clone(),
                            self.event_tx.clone(),
                            self.conn_tx.clone(),
                        )
                        .await;
                };
            }
            CoreEvent::DeviceDiscovered(device) => {
                info!("[core] device discovered.");
                let _ = self.conn_tx.send(ConnectionEvent::Connected((
                    device.device_id.clone(),
                    device,
                )));
            }
            CoreEvent::DevicePaired((device_id, device)) => {
                info!("[core] device paired.");

                let sender = guard.get(&device_id);

                let pair = Pair::new(true);
                let value = serde_json::to_value(pair).expect("failed serialize packet");
                let pair_packet = ProtocolPacket::new(PacketType::Pair, value);

                if let Some(sender) = sender {
                    sender.send(pair_packet).unwrap();
                }

                let _ = self
                    .conn_tx
                    .send(ConnectionEvent::DevicePaired((device_id, device)));
            }
            CoreEvent::DevicePairCancelled(device_id) => {
                info!("[core] device pair cancelled.");

                let sender = guard.get(&device_id);

                let pair = Pair::new(false);
                let value = serde_json::to_value(pair).expect("failed serialize packet");
                let pair_packet = ProtocolPacket::new(PacketType::Pair, value);

                if let Some(sender) = sender {
                    sender.send(pair_packet).unwrap();
                }

                let _ = self.conn_tx.send(ConnectionEvent::PairStateChanged((
                    device_id,
                    crate::device::PairState::NotPaired,
                )));
            }
            CoreEvent::DevicePairStateChanged((device_id, state)) => {
                info!("[core] pair state updated: {:?}", state);
                let _ = self
                    .conn_tx
                    .send(ConnectionEvent::PairStateChanged((device_id, state)));
            }
            CoreEvent::SendPacket { device, packet } => {
                info!(
                    "[core] send packet: [{}] to device: {}",
                    packet.packet_type, device
                );

                if let Some(sender) = guard.get(&device) {
                    debug!("sender available.");
                    let _ = sender.send(packet);
                    debug!("packet sent....");
                }
            }
            CoreEvent::SendPaylod {
                device: device_id,
                packet,
                payload,
                payload_size,
            } => {
                if let Some(sender) = guard.get(&device_id) {
                    debug!("sender available.");

                    if let Some(_device) = self.device_manager.get_device(&device_id).await {
                        // dispatch to plugins
                        self.plugin_registry
                            .send_payload(packet, sender, payload, payload_size)
                            .await;
                    };

                    debug!("packet sent....");
                }
            }
            CoreEvent::Error(e) => {
                tracing::error!("Core error: {}", e);
            }
        }
    }

    async fn transport_events(&self, event: TransportEvent) {
        match event {
            TransportEvent::NewConnection {
                addr,
                id,
                name,
                write_tx,
            } => {
                // store write_tx for this addr
                debug!("[core] new connection from: {}", addr);

                // create device entry
                let device = Device::new(id.0.clone(), name, addr)
                    .await
                    .expect("cannot create new device from metadata");

                tracing::info!("new connection sent to frontend");

                self.device_manager
                    .add_or_update_device(id.clone(), device.clone())
                    .await;

                let _ = self
                    .conn_tx
                    .send(ConnectionEvent::Connected((id.clone(), device.clone())));

                {
                    self.writer_map.lock().await.insert(id, write_tx.clone());
                }
            }
            TransportEvent::IncomingPacket { addr, id, raw } => {
                info!("[core] incoming packet.");
                match serde_json::from_str::<ProtocolPacket>(&raw) {
                    Ok(pkt) => {
                        // if packet is type `pair` handle it immediately
                        if let PacketType::Pair = pkt.packet_type {
                            if let Ok(_pair_body) = serde_json::from_value::<Pair>(pkt.body.clone())
                                && let Some(device) = self.device_manager.get_device(&id).await
                            {
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
                let _ = self.pairing.request_pairing(device_id).await;
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
                            // dispatch to plugins
                            self.plugin_registry
                                .send_payload(packet, sender, payload.buf, payload.size)
                                .await;
                        };
                    }

                    debug!("packet sent....");
                }
            }
            AppEvent::Unpair(device_id) => {
                info!("frontend sent pair event to device: {}", device_id);
                let _ = self.pairing.cancel_pairing(device_id).await;
            }
            AppEvent::Disconnect(device_id) => {
                info!("frontend sent disconnect event to device: {}", device_id);
                if guard.remove(&device_id).is_some() {
                    let _ = self.conn_tx.send(ConnectionEvent::Disconnected(device_id));
                    info!("Connection closed.");
                }
            }
        };
    }

    pub fn take_events(&self) -> Arc<mpsc::UnboundedSender<AppEvent>> {
        self.out_tx.clone()
    }
}
