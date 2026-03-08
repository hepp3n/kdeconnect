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
use tracing::info;
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
                eprintln!("📇 Failed to save contacts cache: {}", e);
            } else {
                eprintln!("📇 Contacts cache saved ({} entries) for {}", contacts.len(), device_id);
            }
        }
        Err(e) => eprintln!("📇 Failed to serialize contacts for cache: {}", e),
    }
}

async fn load_contacts_cache(device_id: &str) -> Option<HashMap<String, String>> {
    let path = device_cache_dir(device_id).join("contacts_cache.json");
    match tokio::fs::read_to_string(&path).await {
        Ok(json) => match serde_json::from_str(&json) {
            Ok(map) => {
                let map: HashMap<String, String> = map;
                eprintln!("📇 Loaded contacts cache ({} entries) for {}", map.len(), device_id);
                Some(map)
            }
            Err(e) => {
                eprintln!("📇 Failed to parse contacts cache: {}", e);
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
        eprintln!("📱 Failed to save SMS cache: {}", e);
    } else {
        eprintln!("📱 SMS cache saved ({} bytes) for {}", messages_json.len(), device_id);
    }
}

async fn load_sms_cache(device_id: &str) -> Option<String> {
    let path = device_cache_dir(device_id).join("sms_cache.json");
    match tokio::fs::read_to_string(&path).await {
        Ok(json) if !json.is_empty() => {
            eprintln!("📱 Loaded SMS cache ({} bytes) for {}", json.len(), device_id);
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
    /// List all known devices
    async fn list_devices(&self) -> Vec<DbusDevice> {
        info!("D-Bus: ListDevices called");
        let devices = self.devices.lock().await;
        let device_list: Vec<DbusDevice> = devices.values().cloned().collect();
        info!("D-Bus: Returning {} devices", device_list.len());
        device_list
    }

    /// Pair with a device
    async fn pair_device(&self, device_id: String) -> zbus::fdo::Result<()> {
        info!("D-Bus: PairDevice called for {}", device_id);
        self.event_sender
            .send(AppEvent::Pair(DeviceId(device_id)))
            .map_err(|e| zbus::fdo::Error::Failed(e.to_string()))?;
        Ok(())
    }

    /// Unpair from a device
    async fn unpair_device(&self, device_id: String) -> zbus::fdo::Result<()> {
        info!("D-Bus: UnpairDevice called for {}", device_id);
        self.event_sender
            .send(AppEvent::Unpair(DeviceId(device_id)))
            .map_err(|e| zbus::fdo::Error::Failed(e.to_string()))?;
        Ok(())
    }

    /// Send a ping to a device
    async fn send_ping(&self, device_id: String, message: String) -> zbus::fdo::Result<()> {
        info!("D-Bus: SendPing called for {} with message: {}", device_id, message);
        let packet = ProtocolPacket::new(PacketType::Ping, json!({ "message": message }));
        self.event_sender
            .send(AppEvent::SendPacket(DeviceId(device_id), packet))
            .map_err(|e| zbus::fdo::Error::Failed(e.to_string()))?;
        Ok(())
    }

    /// Send files to a device
    async fn send_files(&self, device_id: String, files: Vec<String>) -> zbus::fdo::Result<()> {
        info!("D-Bus: SendFiles called for {} ({} files)", device_id, files.len());
        self.event_sender
            .send(AppEvent::SendFiles((DeviceId(device_id), files)))
            .map_err(|e| zbus::fdo::Error::Failed(e.to_string()))?;
        Ok(())
    }

    /// Send clipboard content
    async fn send_clipboard(&self, device_id: String, content: String) -> zbus::fdo::Result<()> {
        info!("D-Bus: SendClipboard called for {}", device_id);
        let packet = ProtocolPacket::new(PacketType::Clipboard, json!({ "content": content }));
        self.event_sender
            .send(AppEvent::SendPacket(DeviceId(device_id), packet))
            .map_err(|e| zbus::fdo::Error::Failed(e.to_string()))?;
        Ok(())
    }

    /// Ring a device (findmyphone)
    async fn ring_device(&self, device_id: String) -> zbus::fdo::Result<()> {
        info!("D-Bus: RingDevice called for {}", device_id);
        let packet = ProtocolPacket::new(PacketType::FindMyPhoneRequest, json!({}));
        self.event_sender
            .send(AppEvent::SendPacket(DeviceId(device_id), packet))
            .map_err(|e| zbus::fdo::Error::Failed(e.to_string()))?;
        Ok(())
    }

    /// Enable or disable a plugin for a device.
    /// Changes take effect immediately and are persisted across restarts.
    async fn set_plugin_enabled(
        &self,
        device_id: String,
        plugin_id: String,
        enabled: bool,
    ) -> zbus::fdo::Result<()> {
        info!(
            "D-Bus: SetPluginEnabled device={} plugin={} enabled={}",
            device_id, plugin_id, enabled
        );
        self.event_sender
            .send(AppEvent::SetPluginEnabled {
                device_id: DeviceId(device_id),
                plugin_id,
                enabled,
            })
            .map_err(|e| zbus::fdo::Error::Failed(e.to_string()))?;
        Ok(())
    }

    /// Signal: Device connected
    #[zbus(signal)]
    async fn device_connected(
        signal_emitter: &SignalEmitter<'_>,
        device_id: String,
        device: DbusDevice,
    ) -> zbus::Result<()>;

    /// Signal: Device paired
    #[zbus(signal)]
    async fn device_paired(
        signal_emitter: &SignalEmitter<'_>,
        device_id: String,
        device: DbusDevice,
    ) -> zbus::Result<()>;

    /// Signal: Device disconnected
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
    /// Return cached SMS JSON — in-memory first, disk fallback, empty if neither
    async fn get_cached_sms(&self, device_id: String) -> String {
        if let Some(json) = self.sms_cache.lock().await.as_ref() {
            eprintln!("📱 Returning in-memory SMS cache ({} bytes)", json.len());
            return json.clone();
        }
        match load_sms_cache(&device_id).await {
            Some(json) => json,
            None => String::new(),
        }
    }

    /// Request all conversations from device
    async fn request_conversations(&self, device_id: String) -> zbus::fdo::Result<()> {
        info!("D-Bus: RequestConversations called for {}", device_id);
        let packet = ProtocolPacket::new(PacketType::SmsRequestConversations, json!({}));
        self.event_sender
            .send(AppEvent::SendPacket(DeviceId(device_id), packet))
            .map_err(|e| zbus::fdo::Error::Failed(e.to_string()))?;
        Ok(())
    }

    /// Request messages from a specific conversation
    async fn request_conversation(
        &self,
        device_id: String,
        thread_id: i64,
    ) -> zbus::fdo::Result<()> {
        info!("D-Bus: RequestConversation called for {} thread {}", device_id, thread_id);
        let packet = ProtocolPacket::new(
            PacketType::SmsRequestConversation,
            json!({ "threadID": thread_id }),
        );
        self.event_sender
            .send(AppEvent::SendPacket(DeviceId(device_id), packet))
            .map_err(|e| zbus::fdo::Error::Failed(e.to_string()))?;
        Ok(())
    }

    /// Send an SMS message
    async fn send_sms(
        &self,
        device_id: String,
        phone_number: String,
        message: String,
    ) -> zbus::fdo::Result<()> {
        info!("D-Bus: SendSms called for {}", device_id);
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
            .map_err(|e| zbus::fdo::Error::Failed(e.to_string()))?;
        Ok(())
    }

    /// Signal: SMS messages received
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
    /// Manually trigger a contacts sync from a device
    async fn request_contacts(&self, device_id: String) -> zbus::fdo::Result<()> {
        info!("D-Bus: RequestContacts called for {}", device_id);
        let packet = ProtocolPacket::new(PacketType::ContactsRequestAllUidsTimestamps, json!({}));
        self.event_sender
            .send(AppEvent::SendPacket(DeviceId(device_id), packet))
            .map_err(|e| zbus::fdo::Error::Failed(e.to_string()))?;
        Ok(())
    }

    /// Return cached contacts from disk — no phone required
    async fn get_cached_contacts(&self, device_id: String) -> String {
        match load_contacts_cache(&device_id).await {
            Some(contacts) => serde_json::to_string(&contacts).unwrap_or_else(|_| "{}".to_string()),
            None => "{}".to_string(),
        }
    }

    /// Signal: contacts received — JSON object mapping phone → name
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

impl KdeConnectService {
    /// Block until the process is killed. All work runs in spawned tasks started
    /// by `new()`; this just keeps the service process alive.
    pub async fn run(&self) -> Result<()> {
        std::future::pending::<()>().await;
        Ok(())
    }
}
/// Tracks devices that have already received an initial SMS sync this session.
type SmsSyncedSet = Arc<Mutex<std::collections::HashSet<String>>>;

impl KdeConnectService {
    pub async fn new() -> Result<Self> {
        eprintln!("=== Initializing KDE Connect D-Bus Service ===");

        let connection = Connection::session().await?;
        eprintln!("✓ D-Bus session connection established");

        connection.request_name(SERVICE_NAME).await?;
        eprintln!("✓ D-Bus service name '{}' registered", SERVICE_NAME);

        eprintln!("Initializing kdeconnect-core...");
        let (mut core, mut event_receiver) = KdeConnectCore::new().await?;
        let event_sender = core.take_events();
        eprintln!("✓ kdeconnect-core initialized");

        let devices = Arc::new(Mutex::new(HashMap::new()));

        let daemon_interface = DaemonInterface {
            event_sender: event_sender.clone(),
            devices: devices.clone(),
        };
        connection
            .object_server()
            .at(DAEMON_PATH, daemon_interface)
            .await?;
        eprintln!("✓ Daemon interface registered at {}", DAEMON_PATH);

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
        eprintln!("✓ SMS interface registered at {}", SMS_PATH);

        let contacts_interface = ContactsInterface {
            event_sender: event_sender.clone(),
        };
        connection
            .object_server()
            .at(CONTACTS_PATH, contacts_interface)
            .await?;
        eprintln!("✓ Contacts interface registered at {}", CONTACTS_PATH);

        let conn_clone = connection.clone();
        let devices_clone = devices.clone();
        let event_sender_clone = event_sender.clone();
        let sms_synced: SmsSyncedSet = Arc::new(Mutex::new(std::collections::HashSet::new()));

        tokio::spawn(async move {
            eprintln!("✓ Event handler started");
            while let Some(event) = event_receiver.recv().await {
                if let Err(e) = Self::handle_event(
                    &conn_clone,
                    event,
                    &devices_clone,
                    &event_sender_clone,
                    &sms_cache,
                    &current_device_id,
                    &sms_synced,
                )
                .await
                {
                    eprintln!("✗ Event handler error: {:?}", e);
                }
            }
        });

        tokio::spawn(async move {
            core.run_event_loop().await;
        });

        Ok(Self {
            connection,
            event_sender,
            devices,
        })
    }

    async fn handle_event(
        connection: &Connection,
        event: ConnectionEvent,
        devices: &Arc<Mutex<HashMap<String, DbusDevice>>>,
        event_sender: &Arc<mpsc::UnboundedSender<AppEvent>>,
        sms_cache: &Arc<Mutex<Option<String>>>,
        current_device_id: &Arc<Mutex<Option<String>>>,
        sms_synced: &SmsSyncedSet,
    ) -> Result<()> {
        match event {
            ConnectionEvent::Connected((device_id, device)) => {
                info!("Event: Device connected - {}", device.name);
                eprintln!("🔌 Device connected: {} ({})", device.name, device_id.0);

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
                eprintln!("✓ Device connected signal emitted");

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
                            eprintln!(
                                "📇 Emitted cached contacts on connect ({} entries)",
                                cached.len()
                            );
                        }
                    }

                    if sms_cache.lock().await.is_none() {
                        if let Some(cached_sms) = load_sms_cache(&did).await {
                            *sms_cache.lock().await = Some(cached_sms);
                            eprintln!("📱 Seeded in-memory SMS cache from disk on connect");
                        }
                    }

                    let already_synced = sms_synced.lock().await.contains(&device_id.0);
                    if !already_synced {
                        sms_synced.lock().await.insert(device_id.0.clone());
                    }

                    // Note: auto-requests for SMS/contacts are now handled in
                    // kdeconnect-core with plugin-enabled gating.
                }
            }
            ConnectionEvent::DevicePaired((device_id, device)) => {
                info!("Event: Device paired - {}", device.name);
                eprintln!("🔐 Device paired: {} ({})", device.name, device_id.0);

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
                eprintln!("✓ Device paired signal emitted");

                let sender = event_sender.clone();
                let did = device_id.0.clone();
                tokio::spawn(async move {
                    tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                    let sms_packet =
                        ProtocolPacket::new(PacketType::SmsRequestConversations, json!({}));
                    let _ = sender.send(AppEvent::SendPacket(DeviceId(did.clone()), sms_packet));
                    eprintln!("📱 Auto-requested SMS conversations after pairing (delayed)");

                    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
                    let contacts_packet = ProtocolPacket::new(
                        PacketType::ContactsRequestAllUidsTimestamps,
                        json!({}),
                    );
                    let _ = sender.send(AppEvent::SendPacket(DeviceId(did), contacts_packet));
                    eprintln!("📇 Auto-requested contacts after pairing");
                });
            }
            ConnectionEvent::Disconnected(device_id) => {
                info!("Event: Device disconnected - {}", device_id.0);
                eprintln!("🔌 Device disconnected: {}", device_id.0);

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
                eprintln!("✓ Device disconnected signal emitted");
            }
            ConnectionEvent::SmsMessages(sms_data) => {
                eprintln!("📱 !!! SMS MESSAGES EVENT RECEIVED !!!");
                eprintln!("    Number of messages: {}", sms_data.messages.len());
                info!("Event: SMS messages received - {} messages", sms_data.messages.len());

                let messages_json = serde_json::to_string(&sms_data)?;
                eprintln!("    JSON size: {} bytes", messages_json.len());

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
                eprintln!("    ✓ SMS D-Bus signal emitted successfully!");
            }
            ConnectionEvent::ContactsReceived(contacts) => {
                info!("Event: Contacts received - {} contacts", contacts.len());
                eprintln!("📇 Contacts received: {} entries", contacts.len());

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
                eprintln!("✓ Contacts D-Bus signal emitted");
            }
            _ => {
                eprintln!("📬 Other event received");
            }
        }

        Ok(())
    }
}
