use std::net::SocketAddr;
use std::time::{SystemTime, UNIX_EPOCH};

use tracing::{debug, info};

use crate::{
    device::{Device, DeviceId, DeviceManager, PairState},
    event::CoreEvent,
    protocol::{Pair, PacketType, ProtocolPacket},
};

pub struct PairingManager {
    pub device_manager: DeviceManager,
    event_tx: tokio::sync::mpsc::UnboundedSender<CoreEvent>,
}

impl PairingManager {
    pub fn new(device_manager: DeviceManager) -> Self {
        let event_tx = device_manager.event_tx.clone();
        Self {
            device_manager,
            event_tx,
        }
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
        let existing = self.device_manager.get_device(&id).await;
        let protocol_version = existing
            .as_ref()
            .map(|d| d.protocol_version)
            .unwrap_or(0);
        let mut device = Device::new(
            id.0.clone(),
            name.clone(),
            existing
                .as_ref()
                .map(|device| device.device_type.clone())
                .unwrap_or_else(|| "phone".to_string()),
            existing
                .as_ref()
                .map(|device| device.incoming_capabilities.clone())
                .unwrap_or_default(),
            existing
                .as_ref()
                .map(|device| device.outgoing_capabilities.clone())
                .unwrap_or_default(),
            addr,
        )
        .await?;
        device.protocol_version = protocol_version;

        // Store the phone's pairing timestamp for later clock-sync validation.
        if let Some(ts) = pair_packet.timestamp {
            device.pairing_timestamp = ts;
            self.device_manager
                .set_pairing_timestamp(&id, ts)
                .await;
        }

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

        // Already paired — re-confirm by sending pair:true back.
        // This handles the case where the phone's app data was reset or
        // the user cleared app storage: the phone sends pair:true to
        // re-establish trust, and we acknowledge it.
        if current_state == PairState::Paired {
            info!("[pairing] {} is already paired — re-confirming", name);
            let now = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs();
            let confirm = Pair {
                pair: true,
                timestamp: Some(now),
            };
            let value = serde_json::to_value(confirm).expect("fail serializing pair");
            let pkt = ProtocolPacket::new(PacketType::Pair, value);

            // Queue a re-confirmation packet via CoreEvent::SendPacket.
            let _ = self.event_tx.send(CoreEvent::SendPacket {
                device: id.clone(),
                packet: pkt,
            });

            // Re-emit DevicePaired so the D-Bus service updates its state.
            let _ = self.event_tx.send(CoreEvent::DevicePaired((
                id.clone(),
                device.clone(),
            )));
            return Ok(false);
        }

        // Incoming pair request — mark as Requested and signal the caller to
        // surface Accept/Decline UI in the applet.
        self.device_manager
            .update_pair_state(&id, PairState::Requested)
            .await;

        info!(
            "Pair request received from {} — awaiting user decision",
            name
        );
        Ok(true)
    }

    pub async fn cancel_pairing(&self, device_id: DeviceId) {
        self.device_manager.set_paired(&device_id, false).await
    }
}
