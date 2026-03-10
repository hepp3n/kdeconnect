use futures::StreamExt;
use kdeconnect_dbus_client::{KdeConnectClient, ServiceEvent};
use tokio::sync::mpsc;
use tracing::{error, info, warn};

#[allow(dead_code)]
#[derive(Debug, Clone)]
pub struct PairingNotification {
    pub device_id: String,
    pub device_name: String,
    pub device_type: String,
}

/// Start listening for pairing notifications via D-Bus
#[allow(dead_code)]
pub fn start_notification_listener(tx: mpsc::Sender<PairingNotification>, _daemon_mode: bool) {
    tokio::spawn(async move {
        info!("Starting pairing notification listener");

        if let Err(e) = listen_for_pairing_signals(tx).await {
            error!("Pairing notification listener failed: {:?}", e);
        }
    });
}

#[allow(dead_code)]
async fn listen_for_pairing_signals(tx: mpsc::Sender<PairingNotification>) -> anyhow::Result<()> {
    info!("Connecting to KDE Connect D-Bus service");

    let client = KdeConnectClient::new().await?;
    let mut event_stream = client.listen_for_events().await;

    info!("Listening for pairing signals on D-Bus");

    while let Some(event) = event_stream.next().await {
        match event {
            ServiceEvent::DevicePaired(device_id, device) => {
                info!("Pairing notification: {} ({})", device.name, device_id);

                let notification = PairingNotification {
                    device_id,
                    device_name: device.name,
                    device_type: device.device_type,
                };

                if tx.send(notification).await.is_err() {
                    warn!("Failed to send pairing notification - receiver dropped");
                    break;
                }
            }
            _ => {}
        }
    }

    info!("Pairing signal listener ended");
    Ok(())
}
