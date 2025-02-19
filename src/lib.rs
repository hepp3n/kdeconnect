use std::{
    net::{Ipv4Addr, SocketAddrV4},
    sync::Arc,
};

use device::Device;
use packets::{Identity, IdentityPacket};
use serde_json as json;
use tokio::{
    fs::File,
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpStream, UdpSocket},
    sync::{
        mpsc::{self, Receiver, Sender},
        Mutex,
    },
};
use tokio_native_tls::{
    native_tls::{self},
    TlsAcceptor,
};

use config::KdeConnectConfig;

mod cert;
mod config;
mod device;
mod packets;
mod utils;

pub const KDECONNECT_PORT: u16 = 1716;

pub enum KdeAction {
    Idle,
    Pair,
}

pub struct KdeConnect {
    pub config: KdeConnectConfig,
    identity: Arc<Mutex<Identity>>,
    udp_socket: Arc<UdpSocket>,
    tls_acceptor: Arc<TlsAcceptor>,
    device_tx: Arc<Sender<Device>>,
    device_rx: Arc<Mutex<Receiver<Device>>>,
}

impl KdeConnect {
    pub async fn new() -> anyhow::Result<KdeConnect> {
        let config = KdeConnectConfig::default();

        let socket_addr = SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, KDECONNECT_PORT);
        let udp_socket = UdpSocket::bind(socket_addr).await?;
        udp_socket.set_broadcast(true)?;

        let identity = Identity::new(config.device_id.clone(), config.device_name.clone(), None);

        let mut cert_file = File::open(&config.root_ca).await?;
        let mut certs = vec![];
        cert_file.read_to_end(&mut certs).await?;
        let mut key_file = File::open(&config.priv_key).await?;
        let mut key = vec![];
        key_file.read_to_end(&mut key).await?;
        let pkcs8 = native_tls::Identity::from_pkcs8(&certs, &key)?;

        let inner = native_tls::TlsAcceptor::builder(pkcs8).build()?;
        let acceptor = TlsAcceptor::from(inner);

        let (device_tx, device_rx) = mpsc::channel(4);

        Ok(KdeConnect {
            config,
            identity: Arc::new(Mutex::new(identity)),
            udp_socket: Arc::new(udp_socket),
            tls_acceptor: Arc::new(acceptor),
            device_tx: Arc::new(device_tx),
            device_rx: Arc::new(Mutex::new(device_rx)),
        })
    }

    async fn handle_clients(
        device_rx: Arc<Mutex<Receiver<Device>>>,
        action_rx: Arc<Mutex<Receiver<KdeAction>>>,
    ) -> anyhow::Result<()> {
        while let Some(mut device) = device_rx.lock().await.recv().await {
            device.inner_task(action_rx.clone()).await?;
        }

        Ok(())
    }

    async fn udp_listener(
        device_tx: Arc<Sender<Device>>,
        udp_socket: Arc<UdpSocket>,
        this_identity: Arc<Mutex<Identity>>,
        acceptor: Arc<TlsAcceptor>,
    ) -> anyhow::Result<()> {
        println!("[UDP] Listening on socket");

        let mut buffer = vec![0; 8192];

        while let Ok((len, mut addr)) = udp_socket.recv_from(&mut buffer).await {
            if let Ok(packet) = json::from_slice::<IdentityPacket>(&buffer[..len]) {
                let identity = packet.body;

                if identity.device_id == this_identity.lock().await.device_id {
                    println!("[UDP] Dont respond to the same device");
                    continue;
                }

                println!("[UDP] New device found: {}", identity.device_name);

                if let Some(port) = identity.tcp_port {
                    addr.set_port(port);
                }

                let packet = this_identity.lock().await.create_packet(None);
                let data = json::to_string(&packet).expect("Creating packet") + "\n";

                let mut stream = TcpStream::connect(addr).await?;

                stream
                    .write_all(data.as_bytes())
                    .await
                    .expect("[TCP] Sending packet");

                match acceptor.accept(stream).await {
                    Ok(stream) => {
                        println!(
                            "[TCP] Connected with device: {} and IP: {}",
                            identity.device_name, addr
                        );

                        let device = Device::new(identity.clone(), stream).await?;

                        device_tx
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

    // async fn tcp_listener(
    //     listener: Arc<TcpListener>,
    //     _this_identity: Arc<Mutex<Identity>>,
    // ) -> anyhow::Result<()> {
    //     println!("[TCP] Listening...");

    //     while let Ok((mut stream, addr)) = listener.accept().await {
    //         let mut identity = String::new();
    //         stream.read_to_string(&mut identity).await?;

    //         if let Ok(packet) = json::from_str::<IdentityPacket>(&identity) {
    //             let identity = packet.body;

    //             println!("[TCP] Found {}", identity.device_name);

    //             let connector = TlsConnector::new()?;
    //             let stream = TcpStream::connect(addr)
    //                 .await
    //                 .expect("[TCP] Connecting to device");

    //             let stream = connector.connect(&addr.ip().to_string(), stream);

    //             println!("{:#?}", stream);
    //         }
    //     }

    //     Ok(())
    // }

    pub async fn start(&self, action_rx: Arc<Mutex<Receiver<KdeAction>>>) {
        let udp_socket = Arc::clone(&self.udp_socket);
        let udp_identity = Arc::clone(&self.identity);
        let acceptor = Arc::clone(&self.tls_acceptor);
        let device_tx = Arc::clone(&self.device_tx);
        let device_rx = Arc::clone(&self.device_rx);

        tokio::select! {
            _handler = Self::handle_clients(device_rx, action_rx) => {
                println!("handler")
            }
            _udp = Self::udp_listener(device_tx, udp_socket, udp_identity, acceptor) => {
                println!("udp listener")
            }
        };
    }
}
