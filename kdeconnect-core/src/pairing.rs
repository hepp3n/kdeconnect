use std::net::SocketAddr;

use notify_rust::{Hint, Notification};
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

        if let Ok(pair_packet) = serde_json::from_value::<Pair>(packet.body) {
            if pair_packet.timestamp.is_none() && pair_packet.pair {
                self.device_manager
                    .update_pair_state(&id, PairState::Paired)
                    .await;
            };
            if pair_packet.timestamp.is_none() && !pair_packet.pair {
                self.device_manager
                    .update_pair_state(&id, PairState::NotPaired)
                    .await;
            }
        }

        let device = Device::new(id.0.clone(), name, addr).await?;

        let label = format!(
            "The {} wants to Pair this device. Click this notification to accept and allow pairing.",
            device.name
        );

        debug!(label);

        let pair_state = tokio::task::spawn_blocking(move || {
            let mut pair_state = false;

            Notification::new()
                .appname("KDE Connect")
                .summary("KDE Connect")
                .body(&label)
                .action("clicked", "clicked") // IDENTIFIER, LABEL
                .action("default", "default")
                .hint(Hint::Resident(true))
                .show()
                .unwrap()
                .wait_for_action(|action| match action {
                    "clicked" => pair_state = true,
                    "__closed" => pair_state = false,
                    _ => pair_state = true,
                });

            pair_state
        })
        .await
        .unwrap();

        info!("User choice: {}", pair_state);

        self.device_manager.set_paired(&id, pair_state).await;

        self.device_manager
            .add_or_update_device(id.clone(), device)
            .await;

        Ok(())
    }

    pub async fn request_pairing(&self, device_id: DeviceId) {
        self.device_manager.set_paired(&device_id, true).await;
    }

    pub async fn cancel_pairing(&self, device_id: DeviceId) {
        self.device_manager.set_paired(&device_id, false).await
    }
}
