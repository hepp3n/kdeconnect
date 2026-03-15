//! Backend interface using D-Bus client to communicate with kdeconnect-service

use anyhow::Result;
use cosmic::iced::Subscription;
use futures::StreamExt;
use kdeconnect_dbus_client::{KdeConnectClient, ServiceEvent};
use std::sync::Arc;
use std::{any::TypeId, collections::HashMap};
use tokio::sync::Mutex;
use tracing::{debug, error, info, warn};

use crate::models::Device;


lazy_static::lazy_static! {
    static ref CLIENT: Arc<Mutex<Option<Arc<KdeConnectClient>>>> = Arc::new(Mutex::new(None));
    static ref DEVICE_CACHE: Arc<Mutex<HashMap<String, Device>>> = Arc::new(Mutex::new(HashMap::new()));
}

/// Initialize the D-Bus client connection
pub async fn initialize() -> Result<()> {
    info!("Initializing D-Bus client");

    let client = KdeConnectClient::new().await?;

    let mut client_guard = CLIENT.lock().await;
    *client_guard = Some(Arc::new(client));

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
    let mut cache = DEVICE_CACHE.lock().await;
    cache.insert(device_id, device);
}

/// Remove device from cache
#[allow(dead_code)]
pub async fn remove_device(device_id: &str) {
    let mut cache = DEVICE_CACHE.lock().await;
    cache.remove(device_id);
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

/// Accept a pairing request
pub async fn accept_pairing(device_id: String) -> Result<()> {
    pair_device(device_id).await
}

/// Reject a pairing request
pub async fn reject_pairing(device_id: String) -> Result<()> {
    unpair_device(device_id).await
}

/// Ring a device (findmyphone)
pub async fn ring_device(device_id: String) -> Result<()> {
    let client_guard = CLIENT.lock().await;
    let Some(client) = client_guard.as_ref() else {
        return Err(anyhow::anyhow!("D-Bus client not initialized"));
    };
    client.ring_device(&device_id).await
}

/// Enable or disable a plugin for a device.
/// The change is forwarded to kdeconnect-service, which persists it and
/// gates all subsequent incoming packets for that plugin.
#[allow(dead_code)]
pub async fn set_plugin_enabled(
    device_id: String,
    plugin_id: String,
    enabled: bool,
) -> Result<()> {
    let client_guard = CLIENT.lock().await;
    let Some(client) = client_guard.as_ref() else {
        return Err(anyhow::anyhow!("D-Bus client not initialized"));
    };
    client
        .set_plugin_enabled(&device_id, &plugin_id, enabled)
        .await
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

/// Create a stream of service events
#[allow(dead_code)]
pub async fn event_stream() -> futures::stream::BoxStream<'static, ServiceEvent> {
    use tokio::sync::mpsc;
    use tokio::time::{Duration, sleep};

    let (tx, rx) = mpsc::channel::<ServiceEvent>(100);

    tokio::spawn(async move {
        let mut attempts = 0;
        let client = loop {
            let client_guard = CLIENT.lock().await;
            if let Some(client) = client_guard.clone() {
                drop(client_guard);
                break client;
            }
            drop(client_guard);

            attempts += 1;
            if attempts > 20 {
                warn!("Timeout waiting for D-Bus client initialization");
                return;
            }

            debug!(
                "Waiting for D-Bus client initialization (attempt {})",
                attempts
            );
            sleep(Duration::from_millis(100)).await;
        };

        info!("Event stream: D-Bus client ready");

        let mut stream = client.listen_for_events().await;

        while let Some(event) = stream.next().await {
            if tx.send(event).await.is_err() {
                warn!("Event receiver dropped, stopping event listener");
                break;
            }
        }

        debug!("Event stream ended");
    });

    futures::stream::unfold(rx, |mut rx| async move {
        match rx.recv().await {
            Some(event) => Some((event, rx)),
            None => loop {
                tokio::time::sleep(tokio::time::Duration::from_secs(3600)).await;
            },
        }
    })
    .boxed()
}

/// Create a file transfer subscription for updating progress state
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
            };


            futures::pending!()
        }
    })
}
