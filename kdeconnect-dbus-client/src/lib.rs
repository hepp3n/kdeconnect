// kdeconnect-dbus-client/src/lib.rs
//! D-Bus client library for KDE Connect service

use anyhow::Result;
use serde::{Deserialize, Serialize};
use zbus::{Connection, proxy};
use futures::StreamExt;

/// Device information
#[derive(Debug, Clone, Serialize, Deserialize, zbus::zvariant::Type, zbus::zvariant::Value, zbus::zvariant::OwnedValue)]
pub struct Device {
    pub id: String,
    pub name: String,
    pub device_type: String,
    pub is_paired: bool,
    pub is_reachable: bool,
}

/// Events from the D-Bus service
#[derive(Debug, Clone)]
pub enum ServiceEvent {
    DeviceConnected(String, Device),
    DevicePaired(String, Device),
    DeviceDisconnected(String),
    SmsMessagesReceived(String), // JSON string
}

/// D-Bus proxy for daemon interface
#[proxy(
    interface = "org.cosmic.KdeConnect.Daemon",
    default_service = "org.cosmic.KdeConnect",
    default_path = "/org/cosmic/KdeConnect/Daemon"
)]
trait Daemon {
    async fn list_devices(&self) -> zbus::Result<Vec<Device>>;
    async fn pair_device(&self, device_id: &str) -> zbus::Result<()>;
    async fn unpair_device(&self, device_id: &str) -> zbus::Result<()>;
    async fn send_ping(&self, device_id: &str, message: &str) -> zbus::Result<()>;
    async fn send_files(&self, device_id: &str, files: Vec<String>) -> zbus::Result<()>;
    async fn send_clipboard(&self, device_id: &str, content: &str) -> zbus::Result<()>;
    async fn ring_device(&self, device_id: &str) -> zbus::Result<()>;

    #[zbus(signal)]
    async fn device_connected(&self, device_id: String, device: Device) -> zbus::Result<()>;
    
    #[zbus(signal)]
    async fn device_paired(&self, device_id: String, device: Device) -> zbus::Result<()>;
    
    #[zbus(signal)]
    async fn device_disconnected(&self, device_id: String) -> zbus::Result<()>;
}

/// D-Bus proxy for SMS interface
#[proxy(
    interface = "org.cosmic.KdeConnect.Sms",
    default_service = "org.cosmic.KdeConnect",
    default_path = "/org/cosmic/KdeConnect/Sms"
)]
trait Sms {
    async fn request_conversations(&self, device_id: &str) -> zbus::Result<()>;
    async fn request_conversation(&self, device_id: &str, thread_id: i64) -> zbus::Result<()>;
    async fn send_sms(&self, device_id: &str, phone_number: &str, message: &str) -> zbus::Result<()>;

    #[zbus(signal)]
    async fn sms_messages_received(&self, messages_json: String) -> zbus::Result<()>;
}

/// Main client for KDE Connect service
pub struct KdeConnectClient {
    daemon_proxy: DaemonProxy<'static>,
    sms_proxy: SmsProxy<'static>,
}

impl KdeConnectClient {
    /// Connect to the KDE Connect service
    pub async fn new() -> Result<Self> {
        let connection = Connection::session().await?;
        
        let daemon_proxy = DaemonProxy::new(&connection).await?;
        let sms_proxy = SmsProxy::new(&connection).await?;

        Ok(Self {
            daemon_proxy,
            sms_proxy,
        })
    }

    /// List all devices
    pub async fn list_devices(&self) -> Result<Vec<Device>> {
        Ok(self.daemon_proxy.list_devices().await?)
    }

    /// Pair with a device
    pub async fn pair_device(&self, device_id: &str) -> Result<()> {
        Ok(self.daemon_proxy.pair_device(device_id).await?)
    }

    /// Unpair from a device
    pub async fn unpair_device(&self, device_id: &str) -> Result<()> {
        Ok(self.daemon_proxy.unpair_device(device_id).await?)
    }

    /// Send a ping
    pub async fn send_ping(&self, device_id: &str, message: &str) -> Result<()> {
        Ok(self.daemon_proxy.send_ping(device_id, message).await?)
    }

    /// Send files
    pub async fn send_files(&self, device_id: &str, files: Vec<String>) -> Result<()> {
        Ok(self.daemon_proxy.send_files(device_id, files).await?)
    }

    /// Send clipboard content
    pub async fn send_clipboard(&self, device_id: &str, content: &str) -> Result<()> {
        Ok(self.daemon_proxy.send_clipboard(device_id, content).await?)
    }

    /// Ring a device (findmyphone)
    pub async fn ring_device(&self, device_id: &str) -> Result<()> {
        Ok(self.daemon_proxy.ring_device(device_id).await?)
    }

    /// Request SMS conversations
    pub async fn request_conversations(&self, device_id: &str) -> Result<()> {
        Ok(self.sms_proxy.request_conversations(device_id).await?)
    }

    /// Request specific conversation
    pub async fn request_conversation(&self, device_id: &str, thread_id: i64) -> Result<()> {
        Ok(self.sms_proxy.request_conversation(device_id, thread_id).await?)
    }

    /// Send SMS
    pub async fn send_sms(&self, device_id: &str, phone_number: &str, message: &str) -> Result<()> {
        Ok(self.sms_proxy.send_sms(device_id, phone_number, message).await?)
    }

    /// Listen for service events (signals)
    pub async fn listen_for_events(&self) -> impl futures::Stream<Item = ServiceEvent> + '_ {
        let daemon_connected = self.daemon_proxy.receive_device_connected().await.unwrap();
        let daemon_paired = self.daemon_proxy.receive_device_paired().await.unwrap();
        let daemon_disconnected = self.daemon_proxy.receive_device_disconnected().await.unwrap();
        let sms_messages = self.sms_proxy.receive_sms_messages_received().await.unwrap();

        // Map each stream to ServiceEvent - args() returns specific Args structs
        let connected_stream = daemon_connected.filter_map(|signal| async move {
            match signal.args() {
                Ok(args) => Some(ServiceEvent::DeviceConnected(args.device_id, args.device)),
                Err(e) => {
                    eprintln!("Failed to parse DeviceConnected signal: {:?}", e);
                    None
                }
            }
        });

        let paired_stream = daemon_paired.filter_map(|signal| async move {
            match signal.args() {
                Ok(args) => Some(ServiceEvent::DevicePaired(args.device_id, args.device)),
                Err(e) => {
                    eprintln!("Failed to parse DevicePaired signal: {:?}", e);
                    None
                }
            }
        });

        let disconnected_stream = daemon_disconnected.filter_map(|signal| async move {
            match signal.args() {
                Ok(args) => Some(ServiceEvent::DeviceDisconnected(args.device_id)),
                Err(e) => {
                    eprintln!("Failed to parse DeviceDisconnected signal: {:?}", e);
                    None
                }
            }
        });

        let sms_stream = sms_messages.filter_map(|signal| async move {
            match signal.args() {
                Ok(args) => Some(ServiceEvent::SmsMessagesReceived(args.messages_json)),
                Err(e) => {
                    eprintln!("Failed to parse SmsMessagesReceived signal: {:?}", e);
                    None
                }
            }
        });

        // Merge all streams
        use futures::stream::select_all;
        select_all(vec![
            Box::pin(connected_stream) as std::pin::Pin<Box<dyn futures::Stream<Item = ServiceEvent> + Send + '_>>,
            Box::pin(paired_stream),
            Box::pin(disconnected_stream),
            Box::pin(sms_stream),
        ])
    }
}
