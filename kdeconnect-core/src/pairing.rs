use std::{net::SocketAddr, time::Duration};

use notify_rust::{Hint, Notification, Timeout};
use tokio::time::timeout;
use tracing::{debug, info};

use crate::{
    device::{Device, DeviceId, DeviceManager, PairState},
    protocol::{Pair, ProtocolPacket},
};

pub struct PairingManager {
    pub device_manager: DeviceManager,
}

impl PairingManager {
    pub fn new(device_manager: DeviceManager) -> Self {
        Self { device_manager }
    }

    pub async fn handle_pair_request(
        &self,
        id: DeviceId,
        name: String,
        addr: SocketAddr,
        packet: ProtocolPacket,
    ) -> anyhow::Result<()> {
        info!(
            "Handling pair request from {}: {:?}",
            id, packet.packet_type
        );

        let pair_packet = match serde_json::from_value::<Pair>(packet.body) {
            Ok(p) => p,
            Err(e) => {
                debug!("Failed to parse pair packet: {}", e);
                return Ok(());
            }
        };

        // Ensure device is known and up to date
        let device = Device::new(id.0.clone(), name.clone(), addr).await?;
        self.device_manager
            .add_or_update_device(id.clone(), device.clone())
            .await;

        let current_state = device.pair_state;

        // Handle Unpair Request
        if !pair_packet.pair {
            info!("Unpair request from {}", name);
            self.device_manager.set_paired(&id, false).await;
            return Ok(());
        }

        // Handle Pair Request (pair: true)
        // If we are already requesting, this is an acceptance
        if current_state == PairState::Requesting {
            info!("Pairing accepted by {}", name);
            self.device_manager.set_paired(&id, true).await;
            return Ok(());
        }

        // If already paired, ignore (or maybe update timestamp if we tracked it)
        if current_state == PairState::Paired {
            debug!("Already paired with {}", name);
            return Ok(());
        }

        // Incoming Pair Request - Ask User
        // Update state to Requested
        self.device_manager
            .update_pair_state(&id, PairState::Requested)
            .await;

        let label = format!(
            "Device '{}' requests pairing.\nDo you want to accept?",
            name
        );

        debug!("Showing pairing notification for {}", name);

        let pair_decision_task = tokio::task::spawn_blocking(move || {
            let mut decision = false;

            let handle = Notification::new()
                .appname("KDE Connect")
                .summary("Pairing Request")
                .body(&label)
                .action("accept", "Accept")
                .action("decline", "Decline")
                .hint(Hint::Resident(true))
                .timeout(Timeout::Milliseconds(30000))
                .show();

            match handle {
                Ok(notification) => {
                    notification.wait_for_action(|action| match action {
                        "accept" => decision = true,
                        "decline" => decision = false,
                        "__closed" => decision = false,
                        _ => decision = false, // Default to safe (decline)
                    });
                }
                Err(e) => {
                    tracing::error!("Failed to show notification: {}", e);
                    decision = false;
                }
            }

            decision
        });

        // Add an async timeout wrapper to ensure we don't wait forever even if notification hangs
        let pair_decision = match timeout(Duration::from_secs(35), pair_decision_task).await {
            Ok(result) => result.unwrap_or(false),
            Err(_) => {
                debug!("Pairing decision timed out (async limit)");
                false
            }
        };

        info!("User pair decision for {}: {}", name, pair_decision);

        self.device_manager.set_paired(&id, pair_decision).await;

        Ok(())
    }

    pub async fn request_pairing(&self, device_id: DeviceId) {
        self.device_manager.set_paired(&device_id, true).await;
    }

    pub async fn cancel_pairing(&self, device_id: DeviceId) {
        self.device_manager.set_paired(&device_id, false).await
    }
}
