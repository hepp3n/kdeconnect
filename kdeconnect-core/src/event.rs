use std::collections::HashMap;
use tokio::io::AsyncRead;

use crate::{
    device::{Device, DeviceId, DeviceState, PairState},
    plugins::mpris::{Mpris, MprisAction, MprisRequest},
    plugins::sms::SmsMessages,
    protocol::ProtocolPacket,
};

pub enum CoreEvent {
    DeviceDiscovered(Device),
    DevicePaired((DeviceId, Device)),
    DevicePairCancelled(DeviceId),
    DevicePairStateChanged((DeviceId, PairState)),
    PacketReceived {
        device: DeviceId,
        packet: ProtocolPacket,
    },
    SendPacket {
        device: DeviceId,
        packet: ProtocolPacket,
    },
    SendPaylod {
        device: DeviceId,
        packet: ProtocolPacket,
        payload: Box<dyn AsyncRead + Sync + Send + Unpin>,
        payload_size: u64,
    },
    Error(String),
}

#[derive(Debug, Clone)]
pub enum AppEvent {
    Broadcasting,
    Disconnect(DeviceId),
    Pair(DeviceId),
    AcceptPairing(DeviceId),
    RejectPairing(DeviceId),
    Ping((DeviceId, String)),
    Unpair(DeviceId),
    SendFiles((DeviceId, Vec<String>)),
    MprisAction((DeviceId, String, MprisAction)),
    SendMprisRequest((DeviceId, MprisRequest)),
    SendPacket(DeviceId, ProtocolPacket),
    SetPluginEnabled {
        device_id: DeviceId,
        plugin_id: String,
        enabled: bool,
    },
}

#[derive(Debug, Clone)]
pub enum ConnectionEvent {
    ClipboardReceived(String),
    Connected((DeviceId, Device)),
    DevicePaired((DeviceId, Device)),
    Disconnected(DeviceId),
    StateUpdated(DeviceState),
    PairStateChanged((DeviceId, PairState)),
    Mpris((DeviceId, Mpris)),
    SmsMessages(SmsMessages),
    ContactsReceived(HashMap<String, String>),
    UpdateTransferProgress(u8),
    /// Phone sent pair:true and is waiting for user decision.
    /// Payload is (device_id, device_name).
    PairingRequested((DeviceId, String)),
}
