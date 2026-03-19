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
use tracing::{debug, error, info};
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

    /// Send files to a device
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

    /// Return the list of disabled plugin IDs for a device.
    async fn get_disabled_plugins(&self, device_id: String) -> Vec<String> {
        let path = dirs::config_dir()
            .unwrap_or_else(|| std::path::PathBuf::from("~/.config"))
            .join("kdeconnect")
            .join(format!("{}_plugins.json", device_id));
        match tokio::fs::read_to_string(&path).await {
            Ok(json) => serde_json::from_str::<Vec<String>>(&json).unwrap_or_default(),
            Err(_) => vec![],
        }
    }

    /// Trigger a UDP identity broadcast to discover nearby devices.
    async fn broadcast_identity(&self) -> zbus::fdo::Result<()> {
        info!("D-Bus: BroadcastIdentity called");
        self.event_sender
            .send(AppEvent::Broadcasting)
            .map_err(|e| zbus::fdo::Error::Failed(e.to_string()))?;
        Ok(())
    }

    /// Accept an incoming pairing request from a device.
    async fn accept_pairing(&self, device_id: String) -> zbus::fdo::Result<()> {
        info!("D-Bus: AcceptPairing called for {}", device_id);
        self.event_sender
            .send(AppEvent::AcceptPairing(kdeconnect_core::device::DeviceId(device_id)))
            .map_err(|e| zbus::fdo::Error::Failed(e.to_string()))?;
        Ok(())
    }

    /// Reject an incoming pairing request from a device.
    async fn reject_pairing(&self, device_id: String) -> zbus::fdo::Result<()> {
        info!("D-Bus: RejectPairing called for {}", device_id);
        self.event_sender
            .send(AppEvent::RejectPairing(kdeconnect_core::device::DeviceId(device_id)))
            .map_err(|e| zbus::fdo::Error::Failed(e.to_string()))?;
        Ok(())
    }

    /// Signal: A device is requesting to pair. Applet shows Accept/Decline UI.
    #[zbus(signal)]
    async fn pairing_requested(
        signal_emitter: &SignalEmitter<'_>,
        device_id: String,
        device_name: String,
    ) -> zbus::Result<()>;

    /// Signal: Device connected
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
            debug!("Returning in-memory SMS cache ({} bytes)", json.len());
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
            .map_err(|e| {
                error!("Failed to send packet: {}", e);
                zbus::fdo::Error::Failed(e.to_string())
            })?;
        debug!("RequestConversations sent to core");
        Ok(())
    }

    /// Request messages from a specific conversation
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

    /// Send an SMS message
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
    /// Run until session logout, system shutdown, SIGTERM, or SIGINT.
    ///
    /// Watches logind SessionRemoved (logout) and PrepareForShutdown (poweroff/reboot)
    /// signals on the system bus. XDG_SESSION_ID is inherited from the autostart
    /// environment and used to filter SessionRemoved to the current session only.
    pub async fn run(&self) -> Result<()> {
        use futures_util::StreamExt;
        use tokio::signal::unix::{SignalKind, signal};
        use zbus::{MatchRule, MessageStream};

        let mut sigterm = signal(SignalKind::terminate())?;
        let mut sigint  = signal(SignalKind::interrupt())?;

        let session_id = std::env::var("XDG_SESSION_ID").unwrap_or_default();
        info!("Watching for logout of session id={}", session_id);

        let logout_fut = async move {
            let Ok(system_bus) = zbus::Connection::system().await else {
                info!("Could not connect to system bus — falling back to pending");
                std::future::pending::<()>().await;
                return;
            };

            let removed_rule = MatchRule::builder()
                .msg_type(zbus::message::Type::Signal)
                .interface("org.freedesktop.login1.Manager").unwrap()
                .member("SessionRemoved").unwrap()
                .build();

            let shutdown_rule = MatchRule::builder()
                .msg_type(zbus::message::Type::Signal)
                .interface("org.freedesktop.login1.Manager").unwrap()
                .member("PrepareForShutdown").unwrap()
                .build();

            let removed_stream = MessageStream::for_match_rule(removed_rule, &system_bus, None).await;
            let shutdown_stream = MessageStream::for_match_rule(shutdown_rule, &system_bus, None).await;

            let (Ok(mut removed), Ok(mut shutdown)) = (removed_stream, shutdown_stream) else {
                info!("Could not subscribe to logind signals — falling back to pending");
                std::future::pending::<()>().await;
                return;
            };

            loop {
                tokio::select! {
                    Some(Ok(msg)) = removed.next() => {
                        // SessionRemoved body: (session_id: String, object_path: OwnedObjectPath)
                        if let Ok((id, _path)) = msg.body().deserialize::<(String, zbus::zvariant::OwnedObjectPath)>() {
                            if id == session_id {
                                info!("SessionRemoved for session {} — shutting down", id);
                                return;
                            }
                        }
                    }
                    Some(Ok(msg)) = shutdown.next() => {
                        // PrepareForShutdown body: (active: bool)
                        if let Ok((true,)) = msg.body().deserialize::<(bool,)>() {
                            info!("PrepareForShutdown received — shutting down");
                            return;
                        }
                    }
                    else => break,
                }
            }
        };

        tokio::select! {
            _ = sigterm.recv()  => info!("SIGTERM received — shutting down"),
            _ = sigint.recv()   => info!("SIGINT received — shutting down"),
            _ = logout_fut      => {},
        }

        Ok(())
    }
}

/// Tracks devices that have already received an initial SMS sync this session.
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

        let conn_clone = connection.clone();
        let devices_clone = devices.clone();
        let event_sender_clone = event_sender.clone();
        let sms_synced: SmsSyncedSet = Arc::new(Mutex::new(std::collections::HashSet::new()));

        tokio::spawn(async move {
            debug!("Event handler started");
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
                    error!("Event handler error: {:?}", e);
                }
            }
        });

        let core_handle = tokio::spawn(async move {
            core.run_event_loop().await;
        });

        tokio::spawn(async move {
            match core_handle.await {
                Ok(_) => error!("Core event loop exited unexpectedly - connections will fail"),
                Err(e) if e.is_panic() => error!("Core event loop PANICKED - connections will fail: {:?}", e),
                Err(e) => error!("Core event loop cancelled: {:?}", e),
            }
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

                    // Note: auto-requests for SMS/contacts are now handled in
                    // kdeconnect-core with plugin-enabled gating.
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

                // Emit device_paired for state change notification, then
                // device_connected as a second delivery path so the applet
                // updates immediately even if it missed device_paired.
                DaemonInterface::device_paired(
                    iface_ref.signal_emitter(),
                    device_id.0.clone(),
                    dbus_device.clone(),
                )
                .await?;
                DaemonInterface::device_connected(
                    iface_ref.signal_emitter(),
                    device_id.0.clone(),
                    dbus_device,
                )
                .await?;
                debug!("Device paired + connected signals emitted");

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

                // Mark unreachable but keep in map so UI can still show it
                // and allow pairing attempts after reconnect.
                {
                    let mut map = devices.lock().await;
                    if let Some(dev) = map.get_mut(&device_id.0) {
                        dev.is_reachable = false;
                    }
                }

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
            ConnectionEvent::PairStateChanged((device_id, pair_state)) => {
                info!("Event: PairStateChanged - {} → {:?}", device_id.0, pair_state);
                let is_paired = matches!(pair_state, PairState::Paired);

                {
                    let mut map = devices.lock().await;
                    if let Some(dev) = map.get_mut(&device_id.0) {
                        dev.is_paired = is_paired;
                    }
                }

                // Push updated device info immediately via device_connected signal
                // so the applet UI reflects the new pair state without waiting for poll.
                let iface_ref = connection
                    .object_server()
                    .interface::<_, DaemonInterface>(DAEMON_PATH)
                    .await?;
                if let Some(dev) = devices.lock().await.get(&device_id.0).cloned() {
                    DaemonInterface::device_connected(
                        iface_ref.signal_emitter(),
                        device_id.0.clone(),
                        dev,
                    )
                    .await?;
                }
                debug!("PairStateChanged signal emitted for {}", device_id.0);
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
                info!("Current transfer progress: {}%", progress);

                let iface_ref = connection
                    .object_server()
                    .interface::<_, DaemonInterface>(DAEMON_PATH)
                    .await?;

                DaemonInterface::update_transfer_progress(iface_ref.signal_emitter(), progress)
                    .await?;

                debug!("UpdateTransferProgress D-Bus signal emitted");
            }
            ConnectionEvent::PairingRequested((device_id, device_name)) => {
                info!("Pairing requested by {} ({})", device_name, device_id.0);

                // Emit D-Bus signal so the applet can show Accept/Decline UI.
                let iface_ref = connection
                    .object_server()
                    .interface::<_, DaemonInterface>(DAEMON_PATH)
                    .await?;
                DaemonInterface::pairing_requested(
                    iface_ref.signal_emitter(),
                    device_id.0.clone(),
                    device_name.clone(),
                )
                .await?;

                // The D-Bus signal is the primary mechanism — the applet
                // subscription delivers it immediately and opens the popup.
            }
            _ => {
                debug!("Unhandled event type received");
            }
        }

        Ok(())
    }
}
