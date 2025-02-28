use anyhow::Result;
use config::KdeConnectConfig;
use device::{ConnectedDevice, Device, DeviceStream};
use packets::{Identity, IdentityPacket};
use serde_json as json;
use std::{
    collections::HashMap,
    net::{Ipv4Addr, SocketAddrV4},
    sync::Arc,
};
use tokio::{
    fs::File,
    io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader},
    net::{TcpListener, TcpStream, UdpSocket},
    select,
    sync::{
        mpsc::{channel, Receiver, Sender},
        Mutex,
    },
};
use tokio_native_tls::{native_tls, TlsAcceptor, TlsConnector};
use tracing::{error, info};

mod cert;
mod config;
pub mod device;
mod packets;
mod utils;

pub const KDECONNECT_PORT: u16 = 1716;

#[derive(Debug, Clone)]
pub enum KdeConnectAction {
    PairDevice,
    SendPing,
}

#[derive(Debug)]
pub struct KdeConnectServer {
    config: KdeConnectConfig,
    identity: Identity,
    udp_socket: UdpSocket,
    tcp_listener: TcpListener,
    tls_connector: TlsConnector,
    tls_acceptor: TlsAcceptor,
    streams: Arc<Mutex<Vec<DeviceStream>>>,
    connected_dev_tx: Sender<ConnectedDevice>,
    action_rx: Arc<Mutex<Receiver<KdeConnectAction>>>,
}

impl KdeConnectServer {
    pub async fn new(
        connected_dev_tx: Sender<ConnectedDevice>,
        action_rx: Arc<Mutex<Receiver<KdeConnectAction>>>,
    ) -> Result<KdeConnectServer> {
        let config = KdeConnectConfig::default();
        let identity = Identity::default();

        let mut connector_builder = native_tls::TlsConnector::builder();

        let mut cert_file = File::open(&config.root_ca).await?;
        let mut certs = vec![];
        cert_file.read_to_end(&mut certs).await?;
        let mut key_file = File::open(&config.priv_key).await?;
        let mut key = vec![];
        key_file.read_to_end(&mut key).await?;
        let pkcs8 = native_tls::Identity::from_pkcs8(&certs, &key)?;

        let tls_connector = connector_builder.identity(pkcs8.clone()).build()?;
        let tls_connector = TlsConnector::from(tls_connector);

        let socket_addr = SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, KDECONNECT_PORT);
        let tcp_listener = TcpListener::bind(&socket_addr)
            .await
            .expect("Cannot bind TCP Listener");

        let udp_socket = UdpSocket::bind(socket_addr).await?;
        udp_socket.set_broadcast(true)?;

        let inner = native_tls::TlsAcceptor::builder(pkcs8).build()?;
        let tls_acceptor = TlsAcceptor::from(inner);

        let streams = Arc::new(Mutex::new(Vec::new()));

        Ok(KdeConnectServer {
            config,
            identity,
            udp_socket,
            tcp_listener,
            tls_connector,
            tls_acceptor,
            streams,
            connected_dev_tx,
            action_rx,
        })
    }

    pub async fn tcp_listener(&self) -> Result<()> {
        info!("[TCP] Listening on socket");

        while let Ok((stream, addr)) = self.tcp_listener.accept().await {
            info!("{:?}", stream);
            let mut stream_reader = BufReader::new(stream);
            let mut identity = String::new();

            stream_reader.read_line(&mut identity).await?;

            info!("[TCP] {:#?}", identity);
            if let Ok(packet) = json::from_str::<IdentityPacket>(&identity) {
                let identity = packet.body;
                let this_device = packets::Identity::default();

                if identity.device_id == this_device.device_id {
                    info!("[TCP] Dont respond to the same device");
                    continue;
                }

                let tls_stream = self
                    .tls_connector
                    .connect(&identity.device_id, stream_reader)
                    .await?;

                info!(
                    "[via TCP] Connected with device: {} and IP: {}",
                    identity.device_name, addr
                );

                let device = Device::new(identity, tls_stream);

                self.streams
                    .lock()
                    .await
                    .push(HashMap::from([(device.config.device_id.clone(), device)]));
            }
        }
        Ok(())
    }

    pub async fn udp_listener(&self) -> Result<()> {
        info!("[UDP] Listening on socket");

        let mut buffer = vec![0; 8192];

        loop {
            let (len, mut addr) = self.udp_socket.recv_from(&mut buffer).await?;
            if let Ok(packet) = json::from_slice::<IdentityPacket>(&buffer[..len]) {
                let identity = packet.body;

                println!(
                    "found: {}, this: {}",
                    identity.device_id, self.config.device_id
                );

                if identity.device_id == self.config.device_id {
                    info!("[UDP] Dont respond to the same device");
                    continue;
                }

                info!("[UDP] New device found: {}", identity.device_name);

                if let Some(port) = identity.tcp_port {
                    addr.set_port(port);
                }

                let packet = self.identity.create_packet(None);
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

                        let device = Device::new(identity, stream);

                        self.connected_dev_tx
                            .send(ConnectedDevice {
                                id: device.config.device_id.clone(),
                                name: device.config.device_name.clone(),
                            })
                            .await?;

                        self.streams
                            .lock()
                            .await
                            .push(HashMap::from([(device.config.device_id.clone(), device)]));
                    }
                    Err(e) => error!("Error while accepting stream: {}", e),
                }
            };
        }
    }

    pub async fn start_server(&mut self) -> Result<()> {
        select! {
            _x = self.udp_listener() => (),
            _x = self.tcp_listener() => (),
            _x = self.messenger() => (),
        };

        Ok(())
    }

    async fn messenger(&self) -> Result<()> {
        while let Some(action) = self.action_rx.lock().await.recv().await {
            let _ = self.update_message(action).await;
        }

        Ok(())
    }

    pub async fn update_message(&self, message: KdeConnectAction) -> Result<()> {
        for stream in self.streams.lock().await.iter_mut() {
            for device in stream.values_mut() {
                let _ = device.inner_task(&message).await;
            }
        }

        Ok(())
    }
}

#[derive(Debug, Clone)]
pub struct KdeConnectClient {
    pub config: KdeConnectConfig,
    action_tx: Sender<KdeConnectAction>,
}

impl KdeConnectClient {
    pub async fn new(tx: Sender<ConnectedDevice>) -> Result<Self> {
        let (action_tx, action_rx) = channel(1);
        let action_rx = Arc::new(Mutex::new(action_rx));
        let mut server = KdeConnectServer::new(tx, action_rx).await?;

        tokio::spawn(async move {
            let _ = server.start_server().await;
        });

        let config = KdeConnectConfig::default();

        Ok(Self { config, action_tx })
    }

    pub fn send_action(&self, action: KdeConnectAction) -> Result<()> {
        let tx = self.action_tx.clone();

        tokio::spawn(async move {
            tx.send(action).await.unwrap();
        });

        Ok(())
    }
}
