use std::{collections::HashMap, sync::Arc};
use tokio::{
    select,
    sync::{Mutex, broadcast, mpsc, oneshot},
};
use tracing::{debug, error, info};

use crate::{
    device::{Device, DeviceId, DeviceManager},
    event::{AppEvent, ConnectionEvent, CoreEvent},
    pairing::PairingManager,
    plugin_interface::PluginRegistry,
    plugins::ping::Ping,
    protocol::{PacketType, Pair, ProtocolPacket},
    transport::{TcpTransport, Transport, TransportEvent, UdpTransport},
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

pub struct KdeConnectCore {
    pub config: config::Config,
    device_manager: Arc<DeviceManager>,
    pairing: Arc<PairingManager>,
    plugin_registry: Arc<PluginRegistry>,
    transport_rx: mpsc::UnboundedReceiver<TransportEvent>,
    writer_map: Arc<Mutex<HashMap<DeviceId, mpsc::UnboundedSender<ProtocolPacket>>>>,
    event_tx: broadcast::Sender<CoreEvent>,
    shutdown_tx: Arc<Mutex<Option<oneshot::Sender<()>>>>,
    transport: Arc<Mutex<Option<Box<dyn Transport + Send + Sync>>>>,
    udp_transport: Arc<Mutex<Option<Box<dyn Transport + Send + Sync>>>>,
    // channel for 3rd party communication
    out_tx: Arc<mpsc::UnboundedSender<AppEvent>>,
    in_rx: mpsc::UnboundedReceiver<AppEvent>,
    conn_tx: Arc<mpsc::UnboundedSender<ConnectionEvent>>,
}

impl KdeConnectCore {
    pub async fn new() -> anyhow::Result<(Self, mpsc::UnboundedReceiver<ConnectionEvent>)> {
        let (out_tx, in_rx) = mpsc::unbounded_channel();
        let (conn_tx, conn_rx) = mpsc::unbounded_channel();

        let plugin_registry = Arc::new(PluginRegistry::new());

        let outgoing_capabilities = plugin_registry.list_plugins().await;
        let config = config::Config::new(outgoing_capabilities);

        let (event_tx, _event_rx) = broadcast::channel(128);
        let (transport_tx, transport_rx) = mpsc::unbounded_channel();
        let transport = TcpTransport::new(&config, &transport_tx);
        let udp_transport = UdpTransport::new(&config, &transport_tx);
        let device_manager = DeviceManager::new(event_tx.clone());
        let pairing = Arc::new(PairingManager::new(device_manager.clone()));
        let writer_map = Arc::new(Mutex::new(HashMap::new()));

        // register plugins
        let ping_plugin = plugins::ping::PingPlugin::new(writer_map.clone());
        plugin_registry.register(Arc::new(ping_plugin)).await;
        let battery_plugin = plugins::battery::BatteryPlugin::new();
        plugin_registry.register(Arc::new(battery_plugin)).await;
        let clipboard_plugin = plugins::clipboard::ClipboardPlugin::new(writer_map.clone());
        plugin_registry.register(Arc::new(clipboard_plugin)).await;

        Ok((
            Self {
                config,
                device_manager: Arc::new(device_manager),
                pairing,
                plugin_registry,
                transport_rx,
                writer_map,
                event_tx,
                shutdown_tx: Arc::new(Mutex::new(None)),
                transport: Arc::new(Mutex::new(Some(Box::new(transport)))),
                udp_transport: Arc::new(Mutex::new(Some(Box::new(udp_transport)))),
                out_tx: Arc::new(out_tx),
                in_rx,
                conn_tx: Arc::new(conn_tx),
            },
            conn_rx,
        ))
    }

    pub async fn stop(&self) -> anyhow::Result<()> {
        // stop transport
        let mut guard = self.transport.lock().await;
        if let Some(transport) = guard.as_mut() {
            transport.stop().await?;
        }
        *guard = None;

        let mut sguard = self.shutdown_tx.lock().await;
        if let Some(shutdown_tx) = sguard.take() {
            let _ = shutdown_tx.send(());
        }
        info!("KdeConnect stopped");
        Ok(())
    }

    pub async fn run_event_loop(&mut self) {
        // start tcp transport
        let tcp = Arc::clone(&self.transport);

        tokio::spawn(async move {
            if let Some(transport) = tcp.lock().await.as_mut() {
                let _ = transport.listen().await;
            }
        });

        let udp = Arc::clone(&self.udp_transport);

        tokio::spawn(async move {
            if let Some(transport) = udp.lock().await.as_mut() {
                let _ = transport.listen().await;
            }
        });

        let mut rx = self.event_tx.subscribe();

        let (sutdown_tx, mut shutdown_rx) = oneshot::channel();

        {
            let mut sguard = self.shutdown_tx.lock().await;
            *sguard = Some(sutdown_tx);
        }

        info!("Starting KdeConnect event loop");

        loop {
            select! {
                maybe = rx.recv() => {
                    match maybe {
                        Ok(event) => self.core_events(event).await,
                        Err(broadcast::error::RecvError::Lagged(n)) => {
                            error!("Event loop lagged by {} messages", n);
                        }
                        Err(broadcast::error::RecvError::Closed) => {
                            info!("Core channel closed");
                            break;
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
                _ = &mut shutdown_rx => {
                    info!("Event loop received shutdown");
                    break;
                }
            }
        }
    }

    async fn core_events(&self, event: CoreEvent) {
        let guard = self.writer_map.lock().await;

        match event {
            CoreEvent::PacketReceived { device, packet } => {
                info!("[core] packet received: {}", packet.packet_type);

                if let Some(dev) = self.device_manager.get_device(&device).await {
                    // dispatch to plugins
                    self.plugin_registry
                        .dispatch(dev, packet.clone(), self.conn_tx.clone())
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

                let _ = self.conn_tx.send(ConnectionEvent::Disconnected(device_id));
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
                            if let Ok(pair_body) = serde_json::from_value::<Pair>(pkt.body.clone())
                                && pair_body.timestamp.is_some()
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
        match event {
            AppEvent::Pair(device_id) => {
                info!("frontend sent pair event to device: {}", device_id);
                let _ = self.pairing.request_pairing(device_id).await;
            }
            AppEvent::Ping((device_id, msg)) => {
                info!("frontend sent ping event to device: {}", device_id);

                let value = serde_json::to_value(Ping { message: Some(msg) })
                    .expect("fail serializing packet body");
                let pkt = ProtocolPacket::new(PacketType::Ping, value);

                if let Some(dev) = self.device_manager.get_device(&device_id).await {
                    self.plugin_registry.send_plugin(dev.clone(), pkt).await;
                };
            }
            AppEvent::Unpair(device_id) => {
                info!("frontend sent pair event to device: {}", device_id);
                let _ = self.pairing.cancel_pairing(device_id).await;
            }
        };
    }

    pub fn take_events(&self) -> Arc<mpsc::UnboundedSender<AppEvent>> {
        self.out_tx.clone()
    }
}
