use crate::{
    device::{Device, DeviceId},
    protocol::ProtocolPacket,
};

#[derive(Debug, Clone)]
pub enum CoreEvent {
    DeviceDiscovered(Device),
    DevicePaired(DeviceId),
    DevicePairCancelled(DeviceId),
    PacketReceived {
        device: DeviceId,
        packet: ProtocolPacket,
    },
    Error(String),
}

#[derive(Debug, Clone)]
pub enum KdeEvent {
    Pair(DeviceId),
    Ping((DeviceId, String)),
    Unpair(DeviceId),
}

#[derive(Debug, Clone)]
pub enum ConnectionEvent {
    Connected((DeviceId, Device)),
    Disconnected(DeviceId),
}
