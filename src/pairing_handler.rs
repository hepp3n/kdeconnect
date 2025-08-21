use notify_rust::{Hint, Notification};
use std::sync::Arc;
use tokio::{
    io::{AsyncWriteExt, WriteHalf},
    net::TcpStream,
    sync::Mutex,
};
use tokio_native_tls::TlsStream;
use tracing::{debug, info, warn};

use crate::{
    config::CONFIG,
    device::{DeviceId, PairingState},
    helpers::pair_timestamp,
    make_packet, make_packet_str,
    packet::{Packet, PacketType, Pair},
};

pub(crate) trait PairingHandlerExt {
    async fn request_pairing(&mut self) -> bool;
    async fn accept_pairing(&mut self) -> bool;
    async fn cancel_pairing(&mut self) -> bool;
    async fn unpair(&mut self) -> bool;
    async fn pairing_done(&mut self, is_paired: bool) -> bool;
}

#[derive(Debug, Clone)]
pub(crate) struct PairingHandler {
    device: DeviceId,
    writer: Arc<Mutex<WriteHalf<TlsStream<TcpStream>>>>,
    pub(crate) pairing_state: PairingState,
}

impl PairingHandler {
    pub(crate) fn new(
        device_id: &DeviceId,
        writer: Arc<Mutex<WriteHalf<TlsStream<TcpStream>>>>,
        pairing_state: PairingState,
    ) -> Self {
        Self {
            device: device_id.clone(),
            writer,
            pairing_state,
        }
    }
}

impl PairingHandlerExt for PairingHandler {
    async fn request_pairing(&mut self) -> bool {
        info!("PairingState: {}", self.pairing_state);

        if PairingState::Paired == self.pairing_state {
            warn!(
                "Already paired, cannot request pairing again from device: {}",
                self.device.id
            );
        }

        if PairingState::RequestedByPeer == self.pairing_state {
            let mut pair_status = false;

            let label = format!(
                "The {} wants to Pair this device. Click this notification to accept and allow pairing.",
                self.device
            );

            debug!(label);

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
                    "clicked" => pair_status = true,
                    "__closed" => pair_status = false,
                    _ => pair_status = true,
                });

            info!("User choice: {}", pair_status);

            if pair_status {
                self.accept_pairing().await;
            } else {
                self.cancel_pairing().await;
            }
        }

        if PairingState::Requested == self.pairing_state {
            let pair = Pair {
                pair: true,
                timestamp: Some(pair_timestamp()),
            };

            let packet = make_packet_str!(pair).expect("Failed to create Pair packet");

            self.writer
                .lock()
                .await
                .write_all(packet.as_bytes())
                .await
                .expect("Failed to send Pair packet");

            return self.pairing_done(true).await;
        }

        return self.pairing_done(false).await;
    }

    async fn accept_pairing(&mut self) -> bool {
        let pair = Pair {
            pair: true,
            timestamp: None,
        };

        let packet = make_packet_str!(pair).expect("Failed to create Pair packet");

        self.writer
            .lock()
            .await
            .write_all(packet.as_bytes())
            .await
            .expect("Failed to send Pair packet");

        CONFIG
            .lock()
            .await
            .update_pair(Some((self.device.clone(), self.pairing_state.clone())))
            .expect("Failed to update paired device in config");

        return self.pairing_done(true).await;
    }

    async fn cancel_pairing(&mut self) -> bool {
        let pair = Pair {
            pair: false,
            timestamp: None,
        };

        let packet = make_packet_str!(pair).expect("Failed to create Pair packet");

        self.writer
            .lock()
            .await
            .write_all(packet.as_bytes())
            .await
            .expect("Failed to send Pair packet");

        CONFIG
            .lock()
            .await
            .update_pair(None)
            .expect("Failed to update paired device in config");

        warn!("Pairing cancelled by user");

        return self.pairing_done(false).await;
    }

    async fn unpair(&mut self) -> bool {
        self.pairing_state = PairingState::NotPaired;

        let pair = Pair {
            pair: false,
            timestamp: None,
        };

        let packet = make_packet_str!(pair).expect("Failed to create Pair packet");

        self.writer
            .lock()
            .await
            .write_all(packet.as_bytes())
            .await
            .expect("Failed to send Pair packet");

        CONFIG
            .lock()
            .await
            .update_pair(None)
            .expect("Failed to update paired device in config");

        warn!("Unpairing device: {}", self.device.id);

        true
    }

    async fn pairing_done(&mut self, is_paired: bool) -> bool {
        info!(
            "Pairing completed for device: {} with: {}",
            self.device.id, is_paired
        );

        if is_paired {
            self.pairing_state = PairingState::Paired;
            info!("Device {} is now paired", self.device.id);
        } else {
            self.pairing_state = PairingState::NotPaired;
            warn!("Device {} is now unpaired", self.device.id);
        }

        is_paired
    }
}
