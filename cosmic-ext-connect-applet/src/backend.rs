//! Backend interface using D-Bus client to communicate with kdeconnect-service

use anyhow::Result;
use cosmic::iced::Subscription;
use futures::StreamExt;
use kdeconnect_dbus_client::{KdeConnectClient, ServiceEvent};
use std::sync::Arc;
use std::{any::TypeId, collections::HashMap};
use tokio::sync::Mutex;
use tracing::{error, info, warn};

use crate::models::Device;

lazy_static::lazy_static! {
    static ref CLIENT: Arc<Mutex<Option<Arc<KdeConnectClient>>>> = Arc::new(Mutex::new(None));
    static ref DEVICE_CACHE: Arc<Mutex<HashMap<String, Device>>> = Arc::new(Mutex::new(HashMap::new()));
}

/// Initialize the D-Bus client connection
pub async fn initialize() -> Result<()> {
    info!("Initializing D-Bus client");
    let client = KdeConnectClient::new().await?;
    *CLIENT.lock().await = Some(Arc::new(client));
    info!("D-Bus client connected to kdeconnect-service");
    Ok(())
}

/// Fetch all devices from the service
pub async fn fetch_devices() -> Vec<Device> {
    let client_guard = CLIENT.lock().await;
    let Some(client) = client_guard.as_ref() else {
        warn!("D-Bus client not initialized");
        return vec![];
    };

    match client.list_devices().await {
        Ok(dbus_devices) => {
            let mut cache = DEVICE_CACHE.lock().await;
            let devices: Vec<Device> = dbus_devices
                .into_iter()
                .map(|d| {
                    let device = Device {
                        id: d.id.clone(),
                        name: d.name.clone(),
                        device_type: "phone".to_string(),
                        is_paired: d.is_paired,
                        is_reachable: d.is_reachable,
                        battery_level: None,
                        is_charging: None,
                        network_type: None,
                        signal_strength: None,
                        pairing_requests: 0,
                        has_battery: false,
                        has_ping: true,
                        has_sms: true,
                        has_contacts: false,
                        has_clipboard: true,
                        has_findmyphone: true,
                        has_share: true,
                        share_progress: None,
                        has_sftp: false,
                        has_mpris: false,
                        has_remote_keyboard: false,
                        has_presenter: false,
                        has_lockdevice: false,
                        has_virtualmonitor: false,
                    };
                    cache.insert(d.id.clone(), device.clone());
                    device
                })
                .collect();
            devices
        }
        Err(e) => {
            error!("Failed to fetch devices: {:?}", e);
            vec![]
        }
    }
}

/// Update device in cache
#[allow(dead_code)]
pub async fn update_device(device_id: String, device: Device) {
    DEVICE_CACHE.lock().await.insert(device_id, device);
}

/// Remove device from cache
#[allow(dead_code)]
pub async fn remove_device(device_id: &str) {
    DEVICE_CACHE.lock().await.remove(device_id);
}

/// Pair with a device
pub async fn pair_device(device_id: String) -> Result<()> {
    let client_guard = CLIENT.lock().await;
    let Some(client) = client_guard.as_ref() else {
        return Err(anyhow::anyhow!("D-Bus client not initialized"));
    };
    client.pair_device(&device_id).await
}

/// Unpair from a device
pub async fn unpair_device(device_id: String) -> Result<()> {
    let client_guard = CLIENT.lock().await;
    let Some(client) = client_guard.as_ref() else {
        return Err(anyhow::anyhow!("D-Bus client not initialized"));
    };
    client.unpair_device(&device_id).await
}

/// Send a ping to a device
pub async fn ping_device(device_id: String) -> Result<()> {
    let client_guard = CLIENT.lock().await;
    let Some(client) = client_guard.as_ref() else {
        return Err(anyhow::anyhow!("D-Bus client not initialized"));
    };
    client.send_ping(&device_id, "Ping from COSMIC!").await
}

/// Send files to a device
pub async fn send_files(device_id: String, files: Vec<String>) -> Result<()> {
    let client_guard = CLIENT.lock().await;
    let Some(client) = client_guard.as_ref() else {
        return Err(anyhow::anyhow!("D-Bus client not initialized"));
    };
    client.send_files(&device_id, files).await
}

/// Send clipboard content to a device
pub async fn send_clipboard(device_id: String, content: String) -> Result<()> {
    let client_guard = CLIENT.lock().await;
    let Some(client) = client_guard.as_ref() else {
        return Err(anyhow::anyhow!("D-Bus client not initialized"));
    };
    client.send_clipboard(&device_id, &content).await
}

/// Browse device filesystem (via SFTP)
pub async fn browse_device_filesystem(_device_id: String) -> Result<()> {
    warn!("Browse filesystem not yet implemented via D-Bus");
    Ok(())
}

/// Accept an incoming pairing request from a device
pub async fn accept_pairing(device_id: String) -> Result<()> {
    let client_guard = CLIENT.lock().await;
    let Some(client) = client_guard.as_ref() else {
        return Err(anyhow::anyhow!("D-Bus client not initialized"));
    };
    client.accept_pairing(&device_id).await
}

/// Reject an incoming pairing request from a device
pub async fn reject_pairing(device_id: String) -> Result<()> {
    let client_guard = CLIENT.lock().await;
    let Some(client) = client_guard.as_ref() else {
        return Err(anyhow::anyhow!("D-Bus client not initialized"));
    };
    client.reject_pairing(&device_id).await
}

/// Ring a device (findmyphone)
pub async fn ring_device(device_id: String) -> Result<()> {
    let client_guard = CLIENT.lock().await;
    let Some(client) = client_guard.as_ref() else {
        return Err(anyhow::anyhow!("D-Bus client not initialized"));
    };
    client.ring_device(&device_id).await
}

/// Enable or disable a plugin for a device
#[allow(dead_code)]
pub async fn set_plugin_enabled(device_id: String, plugin_id: String, enabled: bool) -> Result<()> {
    let client_guard = CLIENT.lock().await;
    let Some(client) = client_guard.as_ref() else {
        return Err(anyhow::anyhow!("D-Bus client not initialized"));
    };
    client.set_plugin_enabled(&device_id, &plugin_id, enabled).await
}

/// Return the list of disabled plugin IDs for a device
#[allow(dead_code)]
pub async fn get_disabled_plugins(device_id: String) -> Vec<String> {
    let client_guard = CLIENT.lock().await;
    let Some(client) = client_guard.as_ref() else {
        warn!("D-Bus client not initialized");
        return vec![];
    };
    match client.get_disabled_plugins(&device_id).await {
        Ok(disabled) => disabled,
        Err(e) => {
            warn!("Failed to get disabled plugins for {}: {:?}", device_id, e);
            vec![]
        }
    }
}

/// Broadcast our identity packet over UDP to trigger device discovery
#[allow(dead_code)]
pub async fn broadcast_identity() -> Result<()> {
    let client_guard = CLIENT.lock().await;
    let Some(client) = client_guard.as_ref() else {
        return Err(anyhow::anyhow!("D-Bus client not initialized"));
    };
    client.broadcast_identity().await
}

/// Request SMS conversations from a device
#[allow(dead_code)]
pub async fn request_conversations(device_id: String) -> Result<()> {
    let client_guard = CLIENT.lock().await;
    let Some(client) = client_guard.as_ref() else {
        return Err(anyhow::anyhow!("D-Bus client not initialized"));
    };
    client.request_conversations(&device_id).await
}

/// Request a specific SMS conversation thread
#[allow(dead_code)]
pub async fn request_conversation(device_id: String, thread_id: i64) -> Result<()> {
    let client_guard = CLIENT.lock().await;
    let Some(client) = client_guard.as_ref() else {
        return Err(anyhow::anyhow!("D-Bus client not initialized"));
    };
    client.request_conversation(&device_id, thread_id).await
}

/// Send an SMS message
#[allow(dead_code)]
pub async fn send_sms(device_id: String, phone_number: String, message: String) -> Result<()> {
    let client_guard = CLIENT.lock().await;
    let Some(client) = client_guard.as_ref() else {
        return Err(anyhow::anyhow!("D-Bus client not initialized"));
    };
    client.send_sms(&device_id, &phone_number, &message).await
}

/// Stream of service events. Reconnects automatically when the client is
/// replaced (e.g. after session logout/login) or the stream ends.
#[allow(dead_code)]
pub async fn event_stream() -> futures::stream::BoxStream<'static, ServiceEvent> {
    use tokio::sync::mpsc;
    use tokio::time::{Duration, sleep};

    let (tx, rx) = mpsc::channel::<ServiceEvent>(100);

    tokio::spawn(async move {
        'reconnect: loop {
            // Wait for the D-Bus client to be ready.
            let client = 'wait: loop {
                if let Some(client) = CLIENT.lock().await.clone() {
                    break 'wait client;
                }
                sleep(Duration::from_millis(100)).await;
            };

            info!("Event stream: D-Bus client ready, subscribing");

            let mut stream = client.listen_for_events().await;

            loop {
                tokio::select! {
                    event = stream.next() => {
                        match event {
                            Some(e) => {
                                if tx.send(e).await.is_err() {
                                    return; // applet exiting
                                }
                            }
                            None => {
                                warn!("Event stream ended, reconnecting in 1s");
                                sleep(Duration::from_secs(1)).await;
                                continue 'reconnect;
                            }
                        }
                    }
                    // Detect if CLIENT was replaced by a new initialize() call
                    // (happens after session logout/login while applet stays running).
                    _ = async {
                        loop {
                            sleep(Duration::from_millis(500)).await;
                            if let Some(current) = CLIENT.lock().await.clone() {
                                if !Arc::ptr_eq(&current, &client) {
                                    return;
                                }
                            }
                        }
                    } => {
                        info!("D-Bus client replaced, reconnecting event stream");
                        continue 'reconnect;
                    }
                }
            }
        }
    });

    Box::pin(tokio_stream::wrappers::ReceiverStream::new(rx))
}

/// Subscription that watches for the kdeconnect service reappearing on the bus
/// after a session logout/login. Reinitializes the D-Bus client and yields
/// a refresh so the applet picks up devices from the new service instance.
pub fn service_watcher_subscription() -> Subscription<crate::messages::Message> {
    struct ServiceWatcher;

    Subscription::run_with(TypeId::of::<ServiceWatcher>(), |_| {
        async_stream::stream! {
            use zbus::{MatchRule, MessageStream};
            use futures::StreamExt;

            let Ok(connection) = zbus::Connection::session().await else {
                return;
            };

            let rule = MatchRule::builder()
                .msg_type(zbus::message::Type::Signal)
                .interface("org.freedesktop.DBus").unwrap()
                .member("NameOwnerChanged").unwrap()
                .arg(0, "io.github.hepp3n.kdeconnect").unwrap()
                .build();

            let Ok(mut stream): Result<zbus::MessageStream, _> =
                MessageStream::for_match_rule(rule, &connection, None).await else {
                return;
            };

            while let Some(Ok(msg)) = stream.next().await {
                let msg: zbus::Message = msg;
                info!("NameOwnerChanged received");
                if let Ok((_name, _old, new_owner)) = msg.body().deserialize::<(String, String, String)>() {
                    info!("NameOwnerChanged: name={} old={} new={}", _name, _old, new_owner);
                    if !new_owner.is_empty() {
                        // Service has a new owner — reinitialize the client.
                        info!("kdeconnect service reappeared on bus — reinitializing client");
                        tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;
                        if let Err(e) = initialize().await {
                            error!("Failed to reinitialize D-Bus client: {:?}", e);
                            continue;
                        }
                        // Give the service time to settle then fetch devices.
                        tokio::time::sleep(tokio::time::Duration::from_secs(3)).await;
                        yield crate::messages::Message::RefreshDevices;
                    }
                }
            }
        }
    })
}

/// Subscription for file transfer progress updates
#[allow(dead_code)]
pub fn filetransfer_subscription() -> Subscription<crate::messages::Message> {
    struct Worker;

    Subscription::run_with(TypeId::of::<Worker>(), |_| {
        async_stream::stream! {
            let Ok(client) = KdeConnectClient::new().await else {
                return;
            };

            let mut progress_stream = client.transfer_progress_stream().await;
            while let Some(progress) = progress_stream.next().await {
                yield crate::messages::Message::UpdateTransferProgress(progress);
            }
        }
    })
}
