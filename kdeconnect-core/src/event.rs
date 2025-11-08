use crate::{
    device::{Device, DeviceId, DeviceState},
    protocol::ProtocolPacket,
};

#[derive(Debug, Clone)]
pub enum CoreEvent {
    DeviceDiscovered(Device),
    DevicePaired((DeviceId, Device)),
    DevicePairCancelled(DeviceId),
    PacketReceived {
        device: DeviceId,
        packet: ProtocolPacket,
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
    Connected((DeviceId, Device)),
    DevicePaired((DeviceId, Device)),
    Disconnected(DeviceId),
    StateUpdated(DeviceState),
}
