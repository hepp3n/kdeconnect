// kdeconnect-service/src/dbus_interface.rs
//! D-Bus interface implementation for KDE Connect service

use anyhow::Result;
use kdeconnect_core::{
    KdeConnectCore,
    event::{AppEvent, ConnectionEvent},
    device::{DeviceId, PairState},
    ProtocolPacket,
    PacketType,
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{mpsc, Mutex};
use tracing::info;
use zbus::{Connection, interface};
use zbus::object_server::SignalEmitter;

const SERVICE_NAME: &str = "org.cosmic.KdeConnect";
const DAEMON_PATH: &str = "/org/cosmic/KdeConnect/Daemon";
const SMS_PATH: &str = "/org/cosmic/KdeConnect/Sms";

/// Simplified device info for D-Bus
#[derive(Debug, Clone, Serialize, Deserialize, zbus::zvariant::Type, zbus::zvariant::Value, zbus::zvariant::OwnedValue)]
pub struct DbusDevice {
    pub id: String,
    pub name: String,
    pub device_type: String,
    pub is_paired: bool,
    pub is_reachable: bool,
}

/// Main daemon D-Bus interface
pub struct DaemonInterface {
    event_sender: Arc<mpsc::UnboundedSender<AppEvent>>,
    devices: Arc<Mutex<HashMap<String, DbusDevice>>>,
}

#[interface(name = "org.cosmic.KdeConnect.Daemon")]
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
        self.event_sender.send(AppEvent::Pair(DeviceId(device_id)))
            .map_err(|e| zbus::fdo::Error::Failed(e.to_string()))?;
        Ok(())
    }

    /// Unpair from a device
    async fn unpair_device(&self, device_id: String) -> zbus::fdo::Result<()> {
        info!("D-Bus: UnpairDevice called for {}", device_id);
        self.event_sender.send(AppEvent::Unpair(DeviceId(device_id)))
            .map_err(|e| zbus::fdo::Error::Failed(e.to_string()))?;
        Ok(())
    }

    /// Send a ping to a device
    async fn send_ping(&self, device_id: String, message: String) -> zbus::fdo::Result<()> {
        info!("D-Bus: SendPing called for {} with message: {}", device_id, message);
        
        // Use SendPacket directly for instant response
        let packet = ProtocolPacket::new(
            PacketType::Ping,
            json!({ "message": message })
        );
        
        self.event_sender.send(AppEvent::SendPacket(DeviceId(device_id), packet))
            .map_err(|e| zbus::fdo::Error::Failed(e.to_string()))?;
        
        Ok(())
    }

    /// Send files to a device
    async fn send_files(&self, device_id: String, files: Vec<String>) -> zbus::fdo::Result<()> {
        info!("D-Bus: SendFiles called for {} ({} files)", device_id, files.len());
        self.event_sender.send(AppEvent::SendFiles((DeviceId(device_id), files)))
            .map_err(|e| zbus::fdo::Error::Failed(e.to_string()))?;
        Ok(())
    }

    /// Send clipboard content
    async fn send_clipboard(&self, device_id: String, content: String) -> zbus::fdo::Result<()> {
        info!("D-Bus: SendClipboard called for {}", device_id);
        let packet = ProtocolPacket::new(
            PacketType::Clipboard,
            json!({ "content": content })
        );
        self.event_sender.send(AppEvent::SendPacket(DeviceId(device_id), packet))
            .map_err(|e| zbus::fdo::Error::Failed(e.to_string()))?;
        Ok(())
    }

    /// Ring a device (findmyphone)
    async fn ring_device(&self, device_id: String) -> zbus::fdo::Result<()> {
        info!("D-Bus: RingDevice called for {}", device_id);
        
        let packet = ProtocolPacket::new(
            PacketType::FindMyPhoneRequest,
            json!({})
        );
        
        self.event_sender.send(AppEvent::SendPacket(DeviceId(device_id), packet))
            .map_err(|e| zbus::fdo::Error::Failed(e.to_string()))?;
        
        Ok(())
    }

    /// Signal: Device connected
    #[zbus(signal)]
    async fn device_connected(signal_emitter: &SignalEmitter<'_>, device_id: String, device: DbusDevice) -> zbus::Result<()>;

    /// Signal: Device paired
    #[zbus(signal)]
    async fn device_paired(signal_emitter: &SignalEmitter<'_>, device_id: String, device: DbusDevice) -> zbus::Result<()>;

    /// Signal: Device disconnected
    #[zbus(signal)]
    async fn device_disconnected(signal_emitter: &SignalEmitter<'_>, device_id: String) -> zbus::Result<()>;
}

/// SMS-specific D-Bus interface
pub struct SmsInterface {
    event_sender: Arc<mpsc::UnboundedSender<AppEvent>>,
}

#[interface(name = "org.cosmic.KdeConnect.Sms")]
impl SmsInterface {
    /// Request all conversations from device
    async fn request_conversations(&self, device_id: String) -> zbus::fdo::Result<()> {
        info!("D-Bus: RequestConversations called for {}", device_id);
        eprintln!("=== SMS D-Bus Request ===");
        eprintln!("Device: {}", device_id);
    
        let packet = ProtocolPacket::new(
            PacketType::SmsRequestConversations,
            json!({})
        );
    
        eprintln!("Packet type: {:?}", packet.packet_type);
    
        self.event_sender.send(AppEvent::SendPacket(DeviceId(device_id.clone()), packet))
            .map_err(|e| {
                eprintln!("✗ Failed to send packet: {}", e);
                zbus::fdo::Error::Failed(e.to_string())
            })?;
    
        eprintln!("✓ Request sent to core");
        Ok(())
    }

    /// Request messages from a specific conversation
    async fn request_conversation(&self, device_id: String, thread_id: i64) -> zbus::fdo::Result<()> {
        info!("D-Bus: RequestConversation called for {} thread {}", device_id, thread_id);
        eprintln!("=== SMS Conversation Request ===");
        eprintln!("Device: {}, Thread: {}", device_id, thread_id);
        
        let packet = ProtocolPacket::new(
            PacketType::SmsRequestConversation,
            json!({
                "threadID": thread_id
            })
        );
        
        eprintln!("Packet: {:?}", packet.packet_type);
        
        self.event_sender.send(AppEvent::SendPacket(DeviceId(device_id), packet))
            .map_err(|e| {
                eprintln!("✗ Failed to send packet: {}", e);
                zbus::fdo::Error::Failed(e.to_string())
            })?;
        
        eprintln!("✓ Conversation request sent");
        Ok(())
    }

    /// Send an SMS message
    async fn send_sms(&self, device_id: String, phone_number: String, message: String) -> zbus::fdo::Result<()> {
        info!("D-Bus: SendSms called for {}", device_id);
        eprintln!("=== SMS Send Request ===");
        eprintln!("Device: {}", device_id);
        eprintln!("To: {}", phone_number);
        eprintln!("Message: {}", message);
        
        let packet = ProtocolPacket::new(
            PacketType::SmsRequest,
            json!({
                "sendSms": true,
                "phoneNumber": phone_number,
                "messageBody": message
            })
        );
        
        self.event_sender.send(AppEvent::SendPacket(DeviceId(device_id), packet))
            .map_err(|e| {
                eprintln!("✗ Failed to send SMS: {}", e);
                zbus::fdo::Error::Failed(e.to_string())
            })?;
        
        eprintln!("✓ SMS send request sent");
        Ok(())
    }

    /// Signal: SMS messages received
    #[zbus(signal)]
    async fn sms_messages_received(signal_emitter: &SignalEmitter<'_>, messages_json: String) -> zbus::Result<()>;
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

/// Tracks devices that have already received an initial SMS sync this session.
/// Prevents re-flooding the phone with SmsRequestConversations on every ~90s keepalive.
type SmsSyncedSet = Arc<Mutex<std::collections::HashSet<String>>>;

impl KdeConnectService {
    pub async fn new() -> Result<Self> {
        eprintln!("=== Initializing KDE Connect D-Bus Service ===");
        
        let connection = Connection::session().await?;
        eprintln!("✓ D-Bus session connection established");

        // Request service name
        connection.request_name(SERVICE_NAME).await?;
        eprintln!("✓ D-Bus service name '{}' registered", SERVICE_NAME);

        // Initialize kdeconnect-core
        eprintln!("Initializing kdeconnect-core...");
        let (mut core, mut event_receiver) = KdeConnectCore::new().await?;
        let event_sender = core.take_events();
        eprintln!("✓ kdeconnect-core initialized");

        let devices = Arc::new(Mutex::new(HashMap::new()));

        // Register daemon interface
        let daemon_interface = DaemonInterface {
            event_sender: event_sender.clone(),
            devices: devices.clone(),
        };
        connection.object_server().at(DAEMON_PATH, daemon_interface).await?;
        eprintln!("✓ Daemon interface registered at {}", DAEMON_PATH);

        // Register SMS interface
        let sms_interface = SmsInterface {
            event_sender: event_sender.clone(),
        };
        connection.object_server().at(SMS_PATH, sms_interface).await?;
        eprintln!("✓ SMS interface registered at {}", SMS_PATH);

        // Spawn core event loop
        eprintln!("Starting core event loop...");
        tokio::spawn(async move {
            core.run_event_loop().await;
        });
        eprintln!("✓ Core event loop started");

        // Spawn event processor
        eprintln!("Starting event processor...");
        let connection_clone = connection.clone();
        let devices_clone = devices.clone();
        let event_sender_clone = event_sender.clone();
        let sms_synced: SmsSyncedSet = Arc::new(Mutex::new(std::collections::HashSet::new()));
        tokio::spawn(async move {
            eprintln!("Event processor task running");
            loop {
                if let Some(event) = event_receiver.recv().await {
                    eprintln!("📨 Received event from core");
                    if let Err(e) = Self::handle_event(event, &connection_clone, &devices_clone, &event_sender_clone, &sms_synced).await {
                        eprintln!("❌ Error handling event: {:?}", e);
                    }
                } else {
                    eprintln!("⚠️  Event receiver channel closed");
                    break;
                }
            }
        });
        eprintln!("✓ Event processor started");
        
        eprintln!("=== KDE Connect D-Bus Service Ready ===");

        Ok(Self {
            connection,
            event_sender,
            devices,
        })
    }

    pub async fn run(self) -> Result<()> {
        eprintln!("Service running, waiting for events...");
        // Keep service alive
        std::future::pending::<()>().await;
        Ok(())
    }

    async fn handle_event(
        event: ConnectionEvent,
        connection: &Connection,
        devices: &Arc<Mutex<HashMap<String, DbusDevice>>>,
        event_sender: &Arc<mpsc::UnboundedSender<AppEvent>>,
        sms_synced: &SmsSyncedSet,
    ) -> Result<()> {
        match event {
            ConnectionEvent::Connected((device_id, device)) => {
                info!("Event: Device connected - {}", device.name);
                eprintln!("🔌 Device connected: {} ({})", device.name, device_id.0);
                
                let is_paired = matches!(device.pair_state, PairState::Paired);
                let dbus_device = DbusDevice {
                    id: device_id.0.clone(),
                    name: device.name.clone(),
                    device_type: "phone".to_string(),
                    is_paired,
                    is_reachable: true,
                };
                
                devices.lock().await.insert(device_id.0.clone(), dbus_device.clone());
                
                let iface_ref = connection.object_server()
                    .interface::<_, DaemonInterface>(DAEMON_PATH).await?;
                
                DaemonInterface::device_connected(iface_ref.signal_emitter(), device_id.0.clone(), dbus_device).await?;
                eprintln!("✓ Device connected signal emitted");

                // Only request SMS once per device per session.
                // The phone re-broadcasts its identity every ~90s; without this guard
                // every keepalive would flood new SmsRequestConversations → duplicates.
                if is_paired {
                    let already_synced = sms_synced.lock().await.contains(&device_id.0);
                    if !already_synced {
                        sms_synced.lock().await.insert(device_id.0.clone());
                        let sender = event_sender.clone();
                        let did = device_id.0.clone();
                        tokio::spawn(async move {
                            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
                            let sms_packet = ProtocolPacket::new(PacketType::SmsRequestConversations, json!({}));
                            let _ = sender.send(AppEvent::SendPacket(DeviceId(did), sms_packet));
                            eprintln!("📱 Auto-requested SMS conversations (first connect this session)");
                        });
                    } else {
                        eprintln!("📱 Skipping SMS re-sync — already synced this session");
                    }
                }
            }
            ConnectionEvent::DevicePaired((device_id, device)) => {
                info!("Event: Device paired - {}", device.name);
                eprintln!("🔐 Device paired: {} ({})", device.name, device_id.0);
                
                let dbus_device = DbusDevice {
                    id: device_id.0.clone(),
                    name: device.name.clone(),
                    device_type: "phone".to_string(),
                    is_paired: true,
                    is_reachable: true,
                };
                
                devices.lock().await.insert(device_id.0.clone(), dbus_device.clone());
                
                let iface_ref = connection.object_server()
                    .interface::<_, DaemonInterface>(DAEMON_PATH).await?;
                
                DaemonInterface::device_paired(iface_ref.signal_emitter(), device_id.0.clone(), dbus_device).await?;
                eprintln!("✓ Device paired signal emitted");

                // Wait 2s for the phone to settle after pairing, then request SMS.
                // The phone may open a fresh connection after accepting — we need to
                // ensure that connection is established before sending plugin requests.
                let sender = event_sender.clone();
                let did = device_id.0.clone();
                tokio::spawn(async move {
                    tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                    let sms_packet = ProtocolPacket::new(PacketType::SmsRequestConversations, json!({}));
                    let _ = sender.send(AppEvent::SendPacket(DeviceId(did), sms_packet));
                    eprintln!("📱 Auto-requested SMS conversations after pairing (delayed)");
                });
            }
            ConnectionEvent::Disconnected(device_id) => {
                info!("Event: Device disconnected - {}", device_id.0);
                eprintln!("🔌 Device disconnected: {}", device_id.0);
                
                // Clear sms_synced so the next genuine reconnect gets a fresh sync.
                sms_synced.lock().await.remove(&device_id.0);
                devices.lock().await.remove(&device_id.0);
                
                let iface_ref = connection.object_server()
                    .interface::<_, DaemonInterface>(DAEMON_PATH).await?;
                
                DaemonInterface::device_disconnected(iface_ref.signal_emitter(), device_id.0).await?;
                eprintln!("✓ Device disconnected signal emitted");
            }
            ConnectionEvent::SmsMessages(sms_data) => {
                eprintln!("📱 !!! SMS MESSAGES EVENT RECEIVED !!!");
                eprintln!("    Number of messages: {}", sms_data.messages.len());
                info!("Event: SMS messages received - {} messages", sms_data.messages.len());
                
                // Show first few messages for debugging
                for (i, msg) in sms_data.messages.iter().take(3).enumerate() {
                    let preview = if msg.body.len() > 50 {
                        format!("{}...", &msg.body[..50])
                    } else {
                        msg.body.clone()
                    };
                    eprintln!("    Message {}: thread={}, body={}", i + 1, msg.thread_id, preview);
                }
                
                // Serialize to JSON for D-Bus signal
                let messages_json = serde_json::to_string(&sms_data)?;
                eprintln!("    JSON size: {} bytes", messages_json.len());
                
                let iface_ref = connection.object_server()
                    .interface::<_, SmsInterface>(SMS_PATH).await?;
                
                eprintln!("    Emitting D-Bus signal...");
                SmsInterface::sms_messages_received(iface_ref.signal_emitter(), messages_json).await?;
                eprintln!("    ✓ SMS D-Bus signal emitted successfully!");
            }
            _ => {
                eprintln!("📬 Other event received");
            }
        }
        
        Ok(())
    }
}
