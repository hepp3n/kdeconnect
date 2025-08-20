use std::{
    net::{Ipv4Addr, SocketAddr, SocketAddrV4},
    sync::Arc,
};
use tokio::sync::{Mutex, mpsc};

use crate::{ClientAction, device::NewClient, packet::Packet};

pub(crate) mod lan;

pub(crate) const DEFAULT_PORT: u16 = 1716;
pub(crate) const BROADCAST_ADDR: SocketAddr =
    SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::BROADCAST, DEFAULT_PORT));
pub(crate) const UNSPECIFIED_ADDR: SocketAddr =
    SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, DEFAULT_PORT));
pub(crate) const LOCALHOST: SocketAddr =
    SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::LOCALHOST, DEFAULT_PORT));

pub(crate) async fn start_backends(
    client_tx: Arc<Mutex<mpsc::UnboundedReceiver<ClientAction>>>,
    device_tx: mpsc::UnboundedSender<NewClient>,
    identity_packet: Packet,
) {
    let mut lan_link_provider = lan::LanLinkProvider::new(identity_packet);

    if !lan_link_provider.is_disabled() {
        lan_link_provider.on_start(client_tx, device_tx).await;
    };
}
