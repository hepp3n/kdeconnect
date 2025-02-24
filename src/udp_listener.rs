use serde_json as json;
use std::{
    collections::HashMap,
    net::{Ipv4Addr, SocketAddrV4},
    path::PathBuf,
};
use tokio::{
    fs::File,
    io::{AsyncReadExt, AsyncWriteExt, BufReader},
    net::{TcpStream, UdpSocket},
    sync::mpsc,
};
use tokio_native_tls::{native_tls, TlsAcceptor};
use tracing::{error, info};

use crate::{
    device::{ConnectedDevice, Device, DeviceStream},
    packets::{Identity, IdentityPacket},
};

pub const KDECONNECT_PORT: u16 = 1716;

#[derive(Debug)]
pub struct UdpListener {
    udp_socket: UdpSocket,
    tls_acceptor: TlsAcceptor,
    stream_tx: mpsc::UnboundedSender<DeviceStream>,
    connected_devices: mpsc::UnboundedSender<ConnectedDevice>,
}

impl UdpListener {
    pub async fn new(
        stream_tx: mpsc::UnboundedSender<DeviceStream>,
        tx: mpsc::UnboundedSender<ConnectedDevice>,
        root_ca: PathBuf,
        key: PathBuf,
    ) -> anyhow::Result<Self> {
        let socket_addr = SocketAddrV4::new(Ipv4Addr::BROADCAST, KDECONNECT_PORT);
        let udp_socket = UdpSocket::bind(socket_addr).await?;
        udp_socket.set_broadcast(true)?;

        let mut cert_file = File::open(&root_ca).await?;
        let mut certs = vec![];
        cert_file.read_to_end(&mut certs).await?;
        let mut key_file = File::open(&key).await?;
        let mut key = vec![];
        key_file.read_to_end(&mut key).await?;
        let pkcs8 = native_tls::Identity::from_pkcs8(&certs, &key)?;

        let inner = native_tls::TlsAcceptor::builder(pkcs8).build()?;
        let tls_acceptor = TlsAcceptor::from(inner);

        Ok(Self {
            udp_socket,
            tls_acceptor,
            stream_tx,
            connected_devices: tx,
        })
    }

    pub async fn listen(&mut self) -> anyhow::Result<()> {
        info!("[UDP] Listening on socket");

        let mut this_identity = Identity::default();
        let mut buffer = vec![0; 8192];

        loop {
            let (len, mut addr) = self.udp_socket.recv_from(&mut buffer).await?;
            if let Ok(packet) = json::from_slice::<IdentityPacket>(&buffer[..len]) {
                let identity = packet.body;

                if identity.device_id == this_identity.device_id {
                    info!("[UDP] Dont respond to the same device");
                    continue;
                }

                info!("[UDP] New device found: {}", identity.device_name);

                if let Some(port) = identity.tcp_port {
                    addr.set_port(port);
                }

                let packet = this_identity.create_packet(None);
                let data = json::to_string(&packet).expect("Creating packet") + "\n";

                let mut stream = BufReader::new(TcpStream::connect(addr).await?);

                stream
                    .write_all(data.as_bytes())
                    .await
                    .expect("[TCP] Sending packet");

                match self.tls_acceptor.accept(stream).await {
                    Ok(stream) => {
                        info!(
                            "[via UDP] Connected with device: {} and IP: {}",
                            identity.device_name, addr
                        );

                        let device = Device::new(stream);

                        self.connected_devices.send(ConnectedDevice {
                            id: device.config.device_id.clone(),
                            name: device.config.device_name.clone(),
                        })?;

                        let _ = self
                            .stream_tx
                            .send(HashMap::from([(device.config.device_id, device.stream)]));
                    }
                    Err(e) => error!("Error while accepting stream: {}", e),
                }
            };
        }
    }
}
