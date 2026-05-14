use std::net::SocketAddr;
use std::time::{SystemTime, UNIX_EPOCH};

use tracing::{debug, info, warn};

use crate::{
    device::{Device, DeviceId, DeviceManager, PairState},
    event::CoreEvent,
    protocol::{PacketType, Pair, ProtocolPacket},
};

const ALLOWED_TIMESTAMP_DIFF_SECS: u64 = 1800;

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
        let protocol_version = existing.as_ref().map(|d| d.protocol_version).unwrap_or(0);
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
            self.device_manager.set_pairing_timestamp(&id, ts).await;
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

        // Already paired. Upstream KDE intentionally does not auto-accept here:
        // a timestamped pair:true is a fresh pair request and must go through
        // the normal user-confirmed flow.
        if current_state == PairState::Paired {
            warn!(
                "[pairing] received fresh pair request from already paired device {}; treating as a new request",
                name
            );
            self.device_manager
                .update_pair_state(&id, PairState::NotPaired)
                .await;
        }

        // Protocol v8 requires a timestamp on new pairing requests. The
        // timestamp is checked before surfacing the request so the UI does not
        // offer an action that KDE Connect peers will reject anyway.
        if device.protocol_version >= 8 {
            let Some(ts) = pair_packet.timestamp else {
                warn!(
                    "[pairing] rejecting pair request from {}: missing protocol v8 timestamp",
                    name
                );
                let value = serde_json::to_value(Pair::reject()).expect("fail serializing pair");
                let pkt = ProtocolPacket::new(PacketType::Pair, value);
                let _ = self.event_tx.send(CoreEvent::SendPacket {
                    device: id.clone(),
                    packet: pkt,
                });
                self.device_manager
                    .update_pair_state(&id, PairState::NotPaired)
                    .await;
                return Ok(false);
            };

            let now = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs();
            let diff = ts.abs_diff(now);
            if diff > ALLOWED_TIMESTAMP_DIFF_SECS {
                warn!(
                    "[pairing] rejecting pair request from {}: clocks out of sync (peer_ts={}, local_ts={}, diff={}s)",
                    name, ts, now, diff
                );
                let value = serde_json::to_value(Pair::reject()).expect("fail serializing pair");
                let pkt = ProtocolPacket::new(PacketType::Pair, value);
                let _ = self.event_tx.send(CoreEvent::SendPacket {
                    device: id.clone(),
                    packet: pkt,
                });
                self.device_manager
                    .update_pair_state(&id, PairState::NotPaired)
                    .await;
                return Ok(false);
            }
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
}
