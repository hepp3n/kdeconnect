//! D-Bus interface implementation for KDE Connect service

use anyhow::Result;
use kdeconnect_core::{
    KdeConnectCore, PacketType, ProtocolPacket,
    device::{DeviceId, PairState},
    event::{AppEvent, ConnectionEvent},
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{Mutex, mpsc};
use tracing::{debug, error, info, warn};
use zbus::object_server::SignalEmitter;
use zbus::{Connection, interface};

const SERVICE_NAME: &str = "io.github.hepp3n.kdeconnect";
const DAEMON_PATH: &str = "/io/github/hepp3n/kdeconnect/Daemon";
const SMS_PATH: &str = "/io/github/hepp3n/kdeconnect/Sms";
const CONTACTS_PATH: &str = "/io/github/hepp3n/kdeconnect/Contacts";

/// Simplified device info for D-Bus
#[derive(
    Debug,
    Clone,
    Serialize,
    Deserialize,
    zbus::zvariant::Type,
    zbus::zvariant::Value,
    zbus::zvariant::OwnedValue,
)]
pub struct DbusDevice {
    pub id: String,
    pub name: String,
    pub device_type: String,
    pub is_paired: bool,
    pub is_reachable: bool,
}

// --- Per-device cache helpers ------------------------------------------------

fn device_cache_dir(device_id: &str) -> std::path::PathBuf {
    let base = dirs::data_local_dir().unwrap_or_else(|| std::path::PathBuf::from("~/.local/share"));
    base.join("kdeconnect").join(device_id)
}

async fn save_contacts_cache(device_id: &str, contacts: &HashMap<String, String>) {
    let path = device_cache_dir(device_id).join("contacts_cache.json");
    if let Some(parent) = path.parent() {
        let _ = tokio::fs::create_dir_all(parent).await;
    }
    match serde_json::to_string(contacts) {
        Ok(json) => {
            if let Err(e) = tokio::fs::write(&path, json).await {
                error!("Failed to save contacts cache: {}", e);
            } else {
                debug!(
                    "Contacts cache saved ({} entries) for {}",
                    contacts.len(),
                    device_id
                );
            }
        }
        Err(e) => error!("Failed to serialize contacts for cache: {}", e),
    }
}

async fn load_contacts_cache(device_id: &str) -> Option<HashMap<String, String>> {
    let path = device_cache_dir(device_id).join("contacts_cache.json");
    match tokio::fs::read_to_string(&path).await {
        Ok(json) => match serde_json::from_str(&json) {
            Ok(map) => {
                let map: HashMap<String, String> = map;
                debug!(
                    "Loaded contacts cache ({} entries) for {}",
                    map.len(),
                    device_id
                );
                Some(map)
            }
            Err(e) => {
                error!("Failed to parse contacts cache: {}", e);
                None
            }
        },
        Err(_) => None,
    }
}

async fn save_sms_cache(device_id: &str, messages_json: &str) {
    let path = device_cache_dir(device_id).join("sms_cache.json");
    if let Some(parent) = path.parent() {
        let _ = tokio::fs::create_dir_all(parent).await;
    }
    if let Err(e) = tokio::fs::write(&path, messages_json).await {
        error!("Failed to save SMS cache: {}", e);
    } else {
        debug!(
            "SMS cache saved ({} bytes) for {}",
            messages_json.len(),
            device_id
        );
    }
}

async fn load_sms_cache(device_id: &str) -> Option<String> {
    let path = device_cache_dir(device_id).join("sms_cache.json");
    match tokio::fs::read_to_string(&path).await {
        Ok(json) if !json.is_empty() => {
            debug!("Loaded SMS cache ({} bytes) for {}", json.len(), device_id);
            Some(json)
        }
        _ => None,
    }
}

// ----------------------------------------------------------------------------

/// Main daemon D-Bus interface
pub struct DaemonInterface {
    event_sender: Arc<mpsc::UnboundedSender<AppEvent>>,
    devices: Arc<Mutex<HashMap<String, DbusDevice>>>,
}

#[interface(name = "io.github.hepp3n.kdeconnect.Daemon")]
impl DaemonInterface {
    async fn list_devices(&self) -> Vec<DbusDevice> {
        info!("D-Bus: ListDevices called");
        let devices = self.devices.lock().await;
        let device_list: Vec<DbusDevice> = devices.values().cloned().collect();
        info!("D-Bus: Returning {} devices", device_list.len());
        device_list
    }

    async fn pair_device(&self, device_id: String) -> zbus::fdo::Result<()> {
        info!("D-Bus: PairDevice called for {}", device_id);
        self.event_sender
            .send(AppEvent::Pair(DeviceId(device_id)))
            .map_err(|e| zbus::fdo::Error::Failed(e.to_string()))?;
        Ok(())
    }

    async fn unpair_device(&self, device_id: String) -> zbus::fdo::Result<()> {
        info!("D-Bus: UnpairDevice called for {}", device_id);
        self.event_sender
            .send(AppEvent::Unpair(DeviceId(device_id)))
            .map_err(|e| zbus::fdo::Error::Failed(e.to_string()))?;
        Ok(())
    }

    async fn send_ping(&self, device_id: String, message: String) -> zbus::fdo::Result<()> {
        info!(
            "D-Bus: SendPing called for {} with message: {}",
            device_id, message
        );
        let packet = ProtocolPacket::new(PacketType::Ping, json!({ "message": message }));
        self.event_sender
            .send(AppEvent::SendPacket(DeviceId(device_id), packet))
            .map_err(|e| zbus::fdo::Error::Failed(e.to_string()))?;
        Ok(())
    }

    async fn send_files(&self, device_id: String, files: Vec<String>) -> zbus::fdo::Result<()> {
        info!(
            "D-Bus: SendFiles called for {} ({} files)",
            device_id,
            files.len()
        );
        self.event_sender
            .send(AppEvent::SendFiles((DeviceId(device_id), files)))
            .map_err(|e| zbus::fdo::Error::Failed(e.to_string()))?;
        Ok(())
    }

    async fn send_clipboard(&self, device_id: String, content: String) -> zbus::fdo::Result<()> {
        info!("D-Bus: SendClipboard called for {}", device_id);
        let packet = ProtocolPacket::new(PacketType::Clipboard, json!({ "content": content }));
        self.event_sender
            .send(AppEvent::SendPacket(DeviceId(device_id), packet))
            .map_err(|e| zbus::fdo::Error::Failed(e.to_string()))?;
        Ok(())
    }

    async fn ring_device(&self, device_id: String) -> zbus::fdo::Result<()> {
        info!("D-Bus: RingDevice called for {}", device_id);
        let packet = ProtocolPacket::new(PacketType::FindMyPhoneRequest, json!({}));
        self.event_sender
            .send(AppEvent::SendPacket(DeviceId(device_id), packet))
            .map_err(|e| zbus::fdo::Error::Failed(e.to_string()))?;
        Ok(())
    }

    #[zbus(signal)]
    async fn update_transfer_progress(
        signal_emitter: &SignalEmitter<'_>,
        progress: u8,
    ) -> zbus::Result<()>;

    #[zbus(signal)]
    async fn device_connected(
        signal_emitter: &SignalEmitter<'_>,
        device_id: String,
        device: DbusDevice,
    ) -> zbus::Result<()>;

    #[zbus(signal)]
    async fn device_paired(
        signal_emitter: &SignalEmitter<'_>,
        device_id: String,
        device: DbusDevice,
    ) -> zbus::Result<()>;

    #[zbus(signal)]
    async fn device_disconnected(
        signal_emitter: &SignalEmitter<'_>,
        device_id: String,
    ) -> zbus::Result<()>;
}

/// SMS-specific D-Bus interface
pub struct SmsInterface {
    event_sender: Arc<mpsc::UnboundedSender<AppEvent>>,
    sms_cache: Arc<Mutex<Option<String>>>,
    #[allow(dead_code)]
    current_device_id: Arc<Mutex<Option<String>>>,
}

#[interface(name = "io.github.hepp3n.kdeconnect.Sms")]
impl SmsInterface {
    async fn get_cached_sms(&self, device_id: String) -> String {
        if let Some(json) = self.sms_cache.lock().await.as_ref() {
            debug!("Returning in-memory SMS cache ({} bytes)", json.len());
            return json.clone();
        }
        match load_sms_cache(&device_id).await {
            Some(json) => json,
            None => String::new(),
        }
    }

    async fn request_conversations(&self, device_id: String) -> zbus::fdo::Result<()> {
        info!("D-Bus: RequestConversations called for {}", device_id);
        let packet = ProtocolPacket::new(PacketType::SmsRequestConversations, json!({}));
        debug!("Sending packet type: {:?}", packet.packet_type);
        self.event_sender
            .send(AppEvent::SendPacket(DeviceId(device_id), packet))
            .map_err(|e| {
                error!("Failed to send packet: {}", e);
                zbus::fdo::Error::Failed(e.to_string())
            })?;
        debug!("RequestConversations sent to core");
        Ok(())
    }

    async fn request_conversation(
        &self,
        device_id: String,
        thread_id: i64,
    ) -> zbus::fdo::Result<()> {
        info!(
            "D-Bus: RequestConversation called for {} thread {}",
            device_id, thread_id
        );
        let packet = ProtocolPacket::new(
            PacketType::SmsRequestConversation,
            json!({ "threadID": thread_id }),
        );
        self.event_sender
            .send(AppEvent::SendPacket(DeviceId(device_id), packet))
            .map_err(|e| {
                error!("Failed to send packet: {}", e);
                zbus::fdo::Error::Failed(e.to_string())
            })?;
        debug!("RequestConversation sent to core");
        Ok(())
    }

    async fn send_sms(
        &self,
        device_id: String,
        phone_number: String,
        message: String,
    ) -> zbus::fdo::Result<()> {
        info!(
            "D-Bus: SendSms called for {} to {}",
            device_id, phone_number
        );
        let packet = ProtocolPacket::new(
            PacketType::SmsRequest,
            json!({
                "sendSms": true,
                "addresses": [{ "address": phone_number }],
                "messageBody": message,
                "version": 2
            }),
        );
        self.event_sender
            .send(AppEvent::SendPacket(DeviceId(device_id), packet))
            .map_err(|e| {
                error!("Failed to send SMS: {}", e);
                zbus::fdo::Error::Failed(e.to_string())
            })?;
        debug!("SMS send request sent to core");
        Ok(())
    }

    #[zbus(signal)]
    async fn sms_messages_received(
        signal_emitter: &SignalEmitter<'_>,
        messages_json: String,
    ) -> zbus::Result<()>;
}

/// Contacts D-Bus interface
pub struct ContactsInterface {
    event_sender: Arc<mpsc::UnboundedSender<AppEvent>>,
}

#[interface(name = "io.github.hepp3n.kdeconnect.Contacts")]
impl ContactsInterface {
    async fn request_contacts(&self, device_id: String) -> zbus::fdo::Result<()> {
        info!("D-Bus: RequestContacts called for {}", device_id);
        let packet = ProtocolPacket::new(PacketType::ContactsRequestAllUidsTimestamps, json!({}));
        self.event_sender
            .send(AppEvent::SendPacket(DeviceId(device_id), packet))
            .map_err(|e| zbus::fdo::Error::Failed(e.to_string()))?;
        Ok(())
    }

    async fn get_cached_contacts(&self, device_id: String) -> String {
        match load_contacts_cache(&device_id).await {
            Some(contacts) => serde_json::to_string(&contacts).unwrap_or_else(|_| "{}".to_string()),
            None => "{}".to_string(),
        }
    }

    #[zbus(signal)]
    async fn contacts_received(
        signal_emitter: &SignalEmitter<'_>,
        contacts_json: String,
    ) -> zbus::Result<()>;
}

/// Main service coordinator
pub struct KdeConnectService {
    #[allow(dead_code)]
    connection: Connection,
    #[allow(dead_code)]
    event_sender: Arc<mpsc::UnboundedSender<AppEvent>>,
    #[allow(dead_code)]
    devices: Arc<Mutex<HashMap<String, DbusDevice>>>,
}

type SmsSyncedSet = Arc<Mutex<std::collections::HashSet<String>>>;

impl KdeConnectService {
    pub async fn new() -> Result<Self> {
        info!("Initializing KDE Connect D-Bus service");

        let connection = Connection::session().await?;
        info!("D-Bus session connection established");

        connection.request_name(SERVICE_NAME).await?;
        info!("D-Bus service name '{}' registered", SERVICE_NAME);

        info!("Initializing kdeconnect-core");
        let (mut core, mut event_receiver) = KdeConnectCore::new().await?;
        let event_sender = core.take_events();
        info!("kdeconnect-core initialized");

        let devices = Arc::new(Mutex::new(HashMap::new()));

        let daemon_interface = DaemonInterface {
            event_sender: event_sender.clone(),
            devices: devices.clone(),
        };
        connection
            .object_server()
            .at(DAEMON_PATH, daemon_interface)
            .await?;
        info!("Daemon interface registered at {}", DAEMON_PATH);

        let sms_cache: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(None));
        let current_device_id: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(None));

        let sms_interface = SmsInterface {
            event_sender: event_sender.clone(),
            sms_cache: sms_cache.clone(),
            current_device_id: current_device_id.clone(),
        };
        connection
            .object_server()
            .at(SMS_PATH, sms_interface)
            .await?;
        info!("SMS interface registered at {}", SMS_PATH);

        let contacts_interface = ContactsInterface {
            event_sender: event_sender.clone(),
        };
        connection
            .object_server()
            .at(CONTACTS_PATH, contacts_interface)
            .await?;
        info!("Contacts interface registered at {}", CONTACTS_PATH);

        info!("Starting core event loop");
        tokio::spawn(async move {
            core.run_event_loop().await;
        });

        info!("Starting event processor");
        let connection_clone = connection.clone();
        let devices_clone = devices.clone();
        let event_sender_clone = event_sender.clone();
        let sms_synced: SmsSyncedSet = Arc::new(Mutex::new(std::collections::HashSet::new()));
        let sms_cache_clone = sms_cache.clone();
        let current_device_id_clone = current_device_id.clone();
        tokio::spawn(async move {
            debug!("Event processor task running");
            loop {
                if let Some(event) = event_receiver.recv().await {
                    if let Err(e) = Self::handle_event(
                        event,
                        &connection_clone,
                        &devices_clone,
                        &event_sender_clone,
                        &sms_synced,
                        &sms_cache_clone,
                        &current_device_id_clone,
                    )
                    .await
                    {
                        error!("Error handling event: {:?}", e);
                    }
                } else {
                    warn!("Event receiver channel closed");
                    break;
                }
            }
        });
        info!("KDE Connect D-Bus service ready");

        Ok(Self {
            connection,
            event_sender,
            devices,
        })
    }

    pub async fn run(self) -> Result<()> {
        info!("Service running, waiting for events");
        std::future::pending::<()>().await;
        Ok(())
    }

    async fn handle_event(
        event: ConnectionEvent,
        connection: &Connection,
        devices: &Arc<Mutex<HashMap<String, DbusDevice>>>,
        event_sender: &Arc<mpsc::UnboundedSender<AppEvent>>,
        sms_synced: &SmsSyncedSet,
        sms_cache: &Arc<Mutex<Option<String>>>,
        current_device_id: &Arc<Mutex<Option<String>>>,
    ) -> Result<()> {
        match event {
            ConnectionEvent::Connected((device_id, device)) => {
                info!("Device connected: {} ({})", device.name, device_id.0);

                *current_device_id.lock().await = Some(device_id.0.clone());

                let is_paired = matches!(device.pair_state, PairState::Paired);
                let dbus_device = DbusDevice {
                    id: device_id.0.clone(),
                    name: device.name.clone(),
                    device_type: "phone".to_string(),
                    is_paired,
                    is_reachable: true,
                };

                devices
                    .lock()
                    .await
                    .insert(device_id.0.clone(), dbus_device.clone());

                let iface_ref = connection
                    .object_server()
                    .interface::<_, DaemonInterface>(DAEMON_PATH)
                    .await?;

                DaemonInterface::device_connected(
                    iface_ref.signal_emitter(),
                    device_id.0.clone(),
                    dbus_device,
                )
                .await?;
                debug!("Device connected signal emitted");

                if is_paired {
                    let did = device_id.0.clone();

                    if let Some(cached) = load_contacts_cache(&did).await {
                        if let Ok(contacts_json) = serde_json::to_string(&cached) {
                            let iface_ref = connection
                                .object_server()
                                .interface::<_, ContactsInterface>(CONTACTS_PATH)
                                .await?;
                            ContactsInterface::contacts_received(
                                iface_ref.signal_emitter(),
                                contacts_json,
                            )
                            .await?;
                            debug!(
                                "Emitted cached contacts on connect ({} entries)",
                                cached.len()
                            );
                        }
                    }

                    if sms_cache.lock().await.is_none() {
                        if let Some(cached_sms) = load_sms_cache(&did).await {
                            *sms_cache.lock().await = Some(cached_sms);
                            debug!("Seeded in-memory SMS cache from disk on connect");
                        }
                    }

                    let already_synced = sms_synced.lock().await.contains(&device_id.0);
                    if !already_synced {
                        sms_synced.lock().await.insert(device_id.0.clone());
                    }

                    let sender = event_sender.clone();
                    tokio::spawn(async move {
                        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
                        let sms_packet =
                            ProtocolPacket::new(PacketType::SmsRequestConversations, json!({}));
                        let _ =
                            sender.send(AppEvent::SendPacket(DeviceId(did.clone()), sms_packet));
                        debug!("Auto-requested SMS conversations on connect");
                        let contacts_packet = ProtocolPacket::new(
                            PacketType::ContactsRequestAllUidsTimestamps,
                            json!({}),
                        );
                        let _ = sender.send(AppEvent::SendPacket(DeviceId(did), contacts_packet));
                        debug!("Auto-requested live contacts sync on connect");
                    });
                }
            }
            ConnectionEvent::DevicePaired((device_id, device)) => {
                info!("Device paired: {} ({})", device.name, device_id.0);

                *current_device_id.lock().await = Some(device_id.0.clone());

                let dbus_device = DbusDevice {
                    id: device_id.0.clone(),
                    name: device.name.clone(),
                    device_type: "phone".to_string(),
                    is_paired: true,
                    is_reachable: true,
                };

                devices
                    .lock()
                    .await
                    .insert(device_id.0.clone(), dbus_device.clone());

                let iface_ref = connection
                    .object_server()
                    .interface::<_, DaemonInterface>(DAEMON_PATH)
                    .await?;

                DaemonInterface::device_paired(
                    iface_ref.signal_emitter(),
                    device_id.0.clone(),
                    dbus_device,
                )
                .await?;
                debug!("Device paired signal emitted");

                let sender = event_sender.clone();
                let did = device_id.0.clone();
                tokio::spawn(async move {
                    tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                    let sms_packet =
                        ProtocolPacket::new(PacketType::SmsRequestConversations, json!({}));
                    let _ = sender.send(AppEvent::SendPacket(DeviceId(did.clone()), sms_packet));
                    debug!("Auto-requested SMS conversations after pairing");

                    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
                    let contacts_packet = ProtocolPacket::new(
                        PacketType::ContactsRequestAllUidsTimestamps,
                        json!({}),
                    );
                    let _ = sender.send(AppEvent::SendPacket(DeviceId(did), contacts_packet));
                    debug!("Auto-requested contacts after pairing");
                });
            }
            ConnectionEvent::Disconnected(device_id) => {
                info!("Device disconnected: {}", device_id.0);

                sms_synced.lock().await.remove(&device_id.0);
                devices.lock().await.remove(&device_id.0);

                let mut cid = current_device_id.lock().await;
                if cid.as_deref() == Some(&device_id.0) {
                    *cid = None;
                }

                let iface_ref = connection
                    .object_server()
                    .interface::<_, DaemonInterface>(DAEMON_PATH)
                    .await?;

                DaemonInterface::device_disconnected(iface_ref.signal_emitter(), device_id.0)
                    .await?;
                debug!("Device disconnected signal emitted");
            }
            ConnectionEvent::SmsMessages(sms_data) => {
                info!(
                    "SMS messages received: {} messages",
                    sms_data.messages.len()
                );

                let messages_json = serde_json::to_string(&sms_data)?;
                debug!("SMS JSON size: {} bytes", messages_json.len());

                *sms_cache.lock().await = Some(messages_json.clone());

                if let Some(did) = current_device_id.lock().await.as_deref() {
                    save_sms_cache(did, &messages_json).await;
                }

                let iface_ref = connection
                    .object_server()
                    .interface::<_, SmsInterface>(SMS_PATH)
                    .await?;

                SmsInterface::sms_messages_received(iface_ref.signal_emitter(), messages_json)
                    .await?;
                debug!("SMS D-Bus signal emitted");
            }
            ConnectionEvent::ContactsReceived(contacts) => {
                info!("Contacts received: {} entries", contacts.len());

                if let Some(did) = current_device_id.lock().await.as_deref() {
                    save_contacts_cache(did, &contacts).await;
                }

                let contacts_json = serde_json::to_string(&contacts)?;

                let iface_ref = connection
                    .object_server()
                    .interface::<_, ContactsInterface>(CONTACTS_PATH)
                    .await?;

                ContactsInterface::contacts_received(iface_ref.signal_emitter(), contacts_json)
                    .await?;
                debug!("Contacts D-Bus signal emitted");
            }
            ConnectionEvent::UpdateTransferProgress(progress) => {
                info!("Current transfer progress: {}&", progress);

                let iface_ref = connection
                    .object_server()
                    .interface::<_, DaemonInterface>(DAEMON_PATH)
                    .await?;

                DaemonInterface::update_transfer_progress(iface_ref.signal_emitter(), progress)
                    .await?;

                debug!("UpdateTransferProgress D-Bus signal emitted");
            }
            _ => {
                debug!("Other event received");
            }
        }

        Ok(())
    }
}
