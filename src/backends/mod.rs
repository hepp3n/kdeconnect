use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4};
use tokio::sync::mpsc;

use crate::device;

pub(crate) mod lan;

pub(crate) const DEFAULT_PORT: u16 = 1716;
pub(crate) const BROADCAST_ADDR: SocketAddr =
    SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::BROADCAST, DEFAULT_PORT));
pub(crate) const UNSPECIFIED_ADDR: SocketAddr =
    SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, DEFAULT_PORT));
pub(crate) const LOCALHOST: SocketAddr =
    SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::LOCALHOST, DEFAULT_PORT));

pub(crate) async fn start_backends(device_tx: mpsc::UnboundedSender<(String, device::Device)>) {
    let mut lan_link_provider = lan::LanLinkProvider::new();

    if !lan_link_provider.is_disabled() {
        lan_link_provider.on_start(device_tx.clone()).await;
    };
}
