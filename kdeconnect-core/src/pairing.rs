use std::net::SocketAddr;

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

    /// Handle an incoming pair packet from the phone.
    ///
    /// Returns `true` if the phone is requesting to pair with us (state → Requested).
    /// The caller is responsible for emitting `ConnectionEvent::PairingRequested` and
    /// routing the decision through the applet UI — we never block here waiting for
    /// a notification action since COSMIC's daemon does not support `wait_for_action`.
    pub async fn handle_pair_request(
        &self,
        id: DeviceId,
        name: String,
        addr: SocketAddr,
        packet: ProtocolPacket,
    ) -> anyhow::Result<bool> {
        info!(
            "Handling pair request from {}: {:?}",
            id, packet.packet_type
        );

        let pair_packet = match serde_json::from_value::<Pair>(packet.body) {
            Ok(p) => p,
            Err(e) => {
                debug!("Failed to parse pair packet: {}", e);
                return Ok(false);
            }
        };

        // Ensure device is known and up to date.
        let device = Device::new(id.0.clone(), name.clone(), addr).await?;
        self.device_manager
            .add_or_update_device(id.clone(), device.clone())
            .await;

        let current_state = device.pair_state;

        // pair:false is handled upstream in lib.rs before this is called.
        if !pair_packet.pair {
            return Ok(false);
        }

        // We sent a pair request and the phone accepted it.
        if current_state == PairState::Requesting {
            info!("Pairing accepted by {}", name);
            self.device_manager.set_paired(&id, true).await;
            return Ok(false);
        }

        // Already paired — nothing to do.
        if current_state == PairState::Paired {
            debug!("Already paired with {}", name);
            return Ok(false);
        }

        // Incoming pair request — mark as Requested and signal the caller to
        // surface Accept/Decline UI in the applet.
        self.device_manager
            .update_pair_state(&id, PairState::Requested)
            .await;

        info!("Pair request received from {} — awaiting user decision", name);
        Ok(true)
    }

    pub async fn cancel_pairing(&self, device_id: DeviceId) {
        self.device_manager.set_paired(&device_id, false).await
    }
}
