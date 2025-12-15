use tokio::io::AsyncRead;

use crate::{
    device::{Device, DeviceId},
    protocol::ProtocolPacket,
};

pub enum CoreEvent {
    DeviceDiscovered(Device),
    DevicePaired((DeviceId, Device)),
    DevicePairCancelled(DeviceId),
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
        payload_size: i64,
    },
    Error(String),
}

#[derive(Debug, Clone)]
pub enum AppEvent {
    Pair(DeviceId),
    Ping((DeviceId, String)),
    Unpair(DeviceId),
}

#[derive(Debug, Clone)]
pub enum ConnectionEvent {
    ClipboardReceived(String),
    Connected((DeviceId, Device)),
    DevicePaired((DeviceId, Device)),
    Disconnected(DeviceId),
    StateUpdated(DeviceState),
}

#[derive(Debug, Clone)]
pub enum DeviceState {
    Battery { level: u8, charging: bool },
}
