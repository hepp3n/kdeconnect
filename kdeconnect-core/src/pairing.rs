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
        let pair_state = existing
            .as_ref()
            .map(|d| d.pair_state)
            .unwrap_or(PairState::NotPaired);
        let remote_certificate = existing
            .as_ref()
            .map(|d| d.remote_certificate.clone())
            .unwrap_or_default();

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
        device.pair_state = pair_state;
        device.remote_certificate = remote_certificate;

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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::PacketType;
    use serde_json::json;
    use std::net::{Ipv4Addr, SocketAddrV4};
    use tokio::sync::mpsc;

    fn setup_test_env() -> tempfile::TempDir {
        let temp_dir = tempfile::tempdir().unwrap();
        unsafe {
            std::env::set_var("HOME", temp_dir.path());
        }
        // Ensure the config directory exists for Device::new
        let kc_dir = temp_dir.path().join(".config").join("kdeconnect");
        std::fs::create_dir_all(kc_dir).unwrap();
        temp_dir
    }

    #[tokio::test]
    async fn handles_incoming_pair_request_transitions_to_requested() {
        let _td = setup_test_env();
        let (tx, _rx) = mpsc::unbounded_channel();
        let dm = DeviceManager::new(tx);
        let pm = PairingManager::new(dm.clone());

        let id = DeviceId("test-device-id-000000000000000000".to_string());
        let addr = SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 1716));
        let packet = ProtocolPacket::new(
            PacketType::Pair,
            json!({
                "pair": true,
                "timestamp": SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs()
            }),
        );

        let result = pm
            .handle_pair_request(id.clone(), "Test Phone".into(), addr, packet)
            .await
            .unwrap();

        assert!(result);
        let dev = dm.get_device(&id).await.unwrap();
        assert_eq!(dev.pair_state, PairState::Requested);
    }

    #[tokio::test]
    async fn accepts_pairing_when_in_requesting_state() {
        let _td = setup_test_env();
        let (tx, _rx) = mpsc::unbounded_channel();
        let dm = DeviceManager::new(tx);
        let pm = PairingManager::new(dm.clone());

        let id = DeviceId("test-device-id-000000000000000000".to_string());
        let addr = SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 1716));

        // Manually set state to Requesting (as if we initiated pairing)
        let mut device = Device::default();
        device.device_id = id.clone();
        device.pair_state = PairState::Requesting;
        dm.add_or_update_device(id.clone(), device).await;

        let packet = ProtocolPacket::new(PacketType::Pair, json!({ "pair": true }));

        let result = pm
            .handle_pair_request(id.clone(), "Test Phone".into(), addr, packet)
            .await
            .unwrap();

        assert!(!result); // Returns false because it's already auto-accepted
        let dev = dm.get_device(&id).await.unwrap();
        assert_eq!(dev.pair_state, PairState::Paired);
    }

    #[tokio::test]
    async fn pair_false_on_not_paired_returns_false() {
        let _td = setup_test_env();
        let (tx, _rx) = mpsc::unbounded_channel();
        let dm = DeviceManager::new(tx);
        let pm = PairingManager::new(dm.clone());

        let id = DeviceId("test-device-false-000000000000000".to_string());
        let addr = SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 1716));
        let packet = ProtocolPacket::new(PacketType::Pair, json!({ "pair": false }));

        let result = pm
            .handle_pair_request(id.clone(), "Test".into(), addr, packet)
            .await
            .unwrap();

        assert!(!result);
    }

    #[tokio::test]
    async fn already_paired_device_gets_fresh_request_treatment() {
        let _td = setup_test_env();
        let (tx, _rx) = mpsc::unbounded_channel();
        let dm = DeviceManager::new(tx);
        let pm = PairingManager::new(dm.clone());

        let id = DeviceId("test-device-repaired-00000000000".to_string());
        let addr = SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 1716));

        let mut device = Device::default();
        device.device_id = id.clone();
        device.pair_state = PairState::Paired;
        dm.add_or_update_device(id.clone(), device).await;

        let packet = ProtocolPacket::new(
            PacketType::Pair,
            json!({
                "pair": true,
                "timestamp": SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs()
            }),
        );

        let result = pm
            .handle_pair_request(id.clone(), "Test Phone".into(), addr, packet)
            .await
            .unwrap();

        assert!(result);
        let dev = dm.get_device(&id).await.unwrap();
        assert_eq!(dev.pair_state, PairState::Requested);
    }

    #[tokio::test]
    async fn rejects_pair_request_without_timestamp_for_v8() {
        let _td = setup_test_env();
        let (tx, _rx) = mpsc::unbounded_channel();
        let dm = DeviceManager::new(tx);
        let pm = PairingManager::new(dm.clone());

        let id = DeviceId("test-device-no-ts-00000000000000".to_string());
        let addr = SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 1716));

        let mut device = Device::default();
        device.device_id = id.clone();
        device.protocol_version = 8;
        dm.add_or_update_device(id.clone(), device).await;

        let packet = ProtocolPacket::new(PacketType::Pair, json!({ "pair": true }));

        let result = pm
            .handle_pair_request(id.clone(), "Test".into(), addr, packet)
            .await
            .unwrap();

        assert!(!result);
        let dev = dm.get_device(&id).await.unwrap();
        assert_eq!(dev.pair_state, PairState::NotPaired);
    }

    #[tokio::test]
    async fn rejects_pair_request_with_large_clock_skew() {
        let _td = setup_test_env();
        let (tx, _rx) = mpsc::unbounded_channel();
        let dm = DeviceManager::new(tx);
        let pm = PairingManager::new(dm.clone());

        let id = DeviceId("test-device-id-000000000000000000".to_string());
        let addr = SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 1716));

        // Set protocol version to 8 to enable clock check
        let mut device = Device::default();
        device.device_id = id.clone();
        device.protocol_version = 8;
        dm.add_or_update_device(id.clone(), device).await;

        // Timestamp 1 hour in the past
        let skewed_ts = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs()
            - 3601;

        let packet = ProtocolPacket::new(
            PacketType::Pair,
            json!({
                "pair": true,
                "timestamp": skewed_ts
            }),
        );

        let result = pm
            .handle_pair_request(id.clone(), "Test Phone".into(), addr, packet)
            .await
            .unwrap();

        assert!(!result);
        let dev = dm.get_device(&id).await.unwrap();
        assert_eq!(dev.pair_state, PairState::NotPaired);
    }
}
