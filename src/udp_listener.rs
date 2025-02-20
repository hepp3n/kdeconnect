use serde_json as json;
use std::{
    net::{Ipv4Addr, SocketAddrV4},
    path::PathBuf,
    sync::Arc,
};
use tokio::{
    fs::File,
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpStream, UdpSocket},
    sync::{mpsc, Mutex},
};
use tokio_native_tls::{native_tls, TlsAcceptor};

use crate::{
    device::Device,
    packets::{Identity, IdentityPacket},
};

pub const KDECONNECT_PORT: u16 = 1716;

#[derive(Debug)]
pub struct UdpListener {
    udp_socket: UdpSocket,
    tls_acceptor: TlsAcceptor,
    identity: Identity,
    device_tx: mpsc::Sender<Device>,
}

impl UdpListener {
    pub async fn new(
        device_id: String,
        device_name: String,
        root_ca: PathBuf,
        key: PathBuf,
    ) -> anyhow::Result<(Arc<Mutex<Self>>, Arc<Mutex<mpsc::Receiver<Device>>>)> {
        let socket_addr = SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, KDECONNECT_PORT);
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

        let (device_tx, device_rx) = mpsc::channel(4);

        let identity = Identity::new(device_id, device_name, None);

        let device_rx = Arc::new(Mutex::new(device_rx));

        Ok((
            Arc::new(Mutex::new(Self {
                udp_socket,
                tls_acceptor,
                identity,
                device_tx,
            })),
            device_rx,
        ))
    }

    pub async fn listen(&mut self) -> anyhow::Result<()> {
        println!("[UDP] Listening on socket");

        let mut buffer = vec![0; 8192];

        while let Ok((len, mut addr)) = self.udp_socket.recv_from(&mut buffer).await {
            if let Ok(packet) = json::from_slice::<IdentityPacket>(&buffer[..len]) {
                let identity = packet.body;

                if identity.device_id == self.identity.device_id {
                    println!("[UDP] Dont respond to the same device");
                    continue;
                }

                println!("[UDP] New device found: {}", identity.device_name);

                if let Some(port) = identity.tcp_port {
                    addr.set_port(port);
                }

                let packet = self.identity.create_packet(None);
                let data = json::to_string(&packet).expect("Creating packet") + "\n";

                let mut stream = TcpStream::connect(addr).await?;

                stream
                    .write_all(data.as_bytes())
                    .await
                    .expect("[TCP] Sending packet");

                match self.tls_acceptor.accept(stream).await {
                    Ok(stream) => {
                        println!(
                            "[TCP] Connected with device: {} and IP: {}",
                            identity.device_name, addr
                        );

                        let device = Device::new(identity.clone(), stream).await?;

                        self.device_tx
                            .send(device)
                            .await
                            .expect("[Device] Adding new device");
                    }
                    Err(e) => eprintln!("Error while accepting stream: {}", e),
                }
            };
        }

        Ok(())
    }
}
