use anyhow::Result;
use serde_json as json;
use std::{
    net::{Ipv4Addr, SocketAddr, SocketAddrV4},
    sync::Arc,
    time::Duration,
};
use tokio::{
    io::{AsyncBufReadExt as _, AsyncWriteExt as _, BufReader},
    net::{TcpListener, TcpStream, UdpSocket},
    sync::{
        mpsc::{unbounded_channel, UnboundedReceiver, UnboundedSender},
        Mutex,
    },
};
use tokio_rustls::{
    rustls::{
        self,
        crypto::aws_lc_rs::default_provider,
        pki_types::{
            pem::PemObject as _, CertificateDer, IpAddr, PrivateKeyDer, PrivatePkcs8KeyDer,
            ServerName,
        },
        ClientConfig, RootCertStore,
    },
    TlsAcceptor, TlsConnector,
};
use tracing::{error, info};

use crate::{
    cert::NoCertificateVerification,
    config::KdeConnectConfig,
    device::Device,
    packets::{Identity, IdentityPacket},
    KDECONNECT_PORT,
};

pub struct Backend {
    identity: Identity,
    udp_socket: UdpSocket,
    tls_acceptor: TlsAcceptor,
    listener: TcpListener,
    connector: TlsConnector,
    new_device_tx: Arc<Mutex<UnboundedSender<Device>>>,
}

pub trait Connector {
    async fn udp(&self) -> Result<()>;
    async fn tcp(&self) -> Result<()>;
    async fn send_identity(&self, socket_addr: SocketAddr) -> Result<()>;
    async fn broadcast(&self) -> Result<()>;
    async fn listen(&self) -> Result<()>;
}

impl Backend {
    pub async fn new(config: KdeConnectConfig) -> Result<(Self, UnboundedReceiver<Device>)> {
        let identity = config.identity;

        let socket_addr = SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, KDECONNECT_PORT);
        let udp_socket = UdpSocket::bind(socket_addr).await?;
        udp_socket.set_broadcast(true)?;

        let certs = CertificateDer::from_pem_file(&config.signed_ca)?;

        let tls_acceptor = TlsAcceptor::from(Arc::new(
            rustls::ServerConfig::builder()
                .with_no_client_auth()
                .with_single_cert(
                    vec![certs.clone()],
                    PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from_pem_file(&config.priv_key)?),
                )
                .expect("Building server config"),
        ));

        let no_verifier = NoCertificateVerification::new(default_provider());

        let mut trusted = RootCertStore::empty();
        trusted.add(certs)?;
        let connector: TlsConnector = TlsConnector::from(Arc::new(
            ClientConfig::builder()
                .dangerous()
                .with_custom_certificate_verifier(Arc::new(no_verifier))
                .with_no_client_auth(),
        ));

        let socket_addr = SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, KDECONNECT_PORT);
        let listener = TcpListener::bind(&socket_addr)
            .await
            .expect("Cannot bind TCP Listener");

        let (new_device_tx, new_device_rx) = unbounded_channel();
        let new_device_tx = Arc::new(Mutex::new(new_device_tx));

        Ok((
            Self {
                identity,
                udp_socket,
                tls_acceptor,
                listener,
                connector,
                new_device_tx,
            },
            new_device_rx,
        ))
    }
}

impl Connector for Backend {
    async fn udp(&self) -> Result<()> {
        info!("[UDP] Listening on socket");

        loop {
            let mut buffer = vec![0; 8192];

            let (len, mut addr) = self.udp_socket.recv_from(&mut buffer).await?;
            if let Ok(packet) = json::from_slice::<IdentityPacket>(&buffer[..len]) {
                let identity = packet.body;

                if identity.device_id == self.identity.device_id {
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

                        let device =
                            Device::new(identity.clone(), tokio_rustls::TlsStream::Server(stream))?;

                        let _ = self.new_device_tx.lock().await.send(device);
                        let _ = self.send_identity(addr).await;
                    }
                    Err(e) => error!("Error while accepting stream: {}", e),
                }
            };
        }
    }

    async fn tcp(&self) -> Result<()> {
        info!("[TCP] Listening on socket");

        while let Ok((stream, addr)) = self.listener.accept().await {
            info!("Accepting stream");
            let mut stream_reader = BufReader::new(stream);
            let mut identity = String::new();

            stream_reader.read_line(&mut identity).await?;

            if let Ok(packet) = json::from_str::<IdentityPacket>(&identity) {
                let identity = packet.body;
                let this_device = &self.identity;

                if identity.device_id == this_device.device_id {
                    info!("[TCP] Dont respond to the same device");
                    continue;
                }

                let ipv4 = match addr.ip() {
                    std::net::IpAddr::V4(ipv4_addr) => ipv4_addr,
                    _ => Ipv4Addr::LOCALHOST,
                };

                info!("Try connecting to: {}", ipv4);

                let tls_stream = self
                    .connector
                    .connect(ServerName::IpAddress(IpAddr::from(ipv4)), stream_reader)
                    .await
                    .expect("[TCP] Connecting streams");

                info!(
                    "[via TCP] Connected with device: {} and IP: {}",
                    identity.device_name, addr
                );

                let device = Device::new(
                    identity.clone(),
                    tokio_rustls::TlsStream::Client(tls_stream),
                )?;

                let _ = self.new_device_tx.lock().await.send(device);
            };
        }

        Ok(())
    }

    async fn send_identity(&self, socket_addr: SocketAddr) -> Result<()> {
        let packet = self.identity.create_packet(Some(KDECONNECT_PORT));
        let data = json::to_string(&packet)? + "\n";

        let _ = self
            .udp_socket
            .send_to(data.as_bytes(), socket_addr)
            .await
            .expect("Sending packet over UDP");

        Ok(())
    }

    async fn broadcast(&self) -> Result<()> {
        let socket_addr = SocketAddr::new(
            IpAddr::V4(Ipv4Addr::BROADCAST.into()).into(),
            KDECONNECT_PORT,
        );

        loop {
            tokio::time::sleep(Duration::from_secs(15)).await;

            let _ = self.send_identity(socket_addr).await;

            info!("[UDP] Broadcasting....");
        }
    }

    async fn listen(&self) -> Result<()> {
        tokio::select! {
            x = self.udp() => x,
            x = self.tcp() => x,
            x = self.broadcast() => x,
        }
    }
}
