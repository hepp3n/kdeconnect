use anyhow::Result;
use serde_json as json;
use std::{
    net::{IpAddr, Ipv4Addr, SocketAddr, SocketAddrV4},
    sync::Arc,
    time::Duration,
};
use tokio::{
    io::{AsyncWriteExt as _, BufReader},
    net::{TcpStream, UdpSocket},
    sync::mpsc::UnboundedSender,
};
use tokio_rustls::{
    rustls::{
        self,
        pki_types::{pem::PemObject, CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer},
    },
    TlsAcceptor,
};
use tracing::{error, info};

use crate::{config::KdeConnectConfig, device::Device, packets::IdentityPacket, KDECONNECT_PORT};

pub struct UdpListener {
    config: KdeConnectConfig,
    udp_socket: UdpSocket,
    tls_acceptor: TlsAcceptor,
    new_device_tx: UnboundedSender<Device>,
}

impl UdpListener {
    pub async fn new(
        config: KdeConnectConfig,
        new_device_tx: UnboundedSender<Device>,
    ) -> Result<UdpListener> {
        let socket_addr = SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, KDECONNECT_PORT);
        let udp_socket = UdpSocket::bind(socket_addr).await?;
        udp_socket.set_broadcast(true)?;

        let certs = CertificateDer::from_pem_file(&config.signed_ca)?;

        let tls_acceptor = TlsAcceptor::from(Arc::new(
            rustls::ServerConfig::builder()
                .with_no_client_auth()
                .with_single_cert(
                    vec![certs],
                    PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from_pem_file(&config.priv_key)?),
                )
                .expect("Building server config"),
        ));

        Ok(UdpListener {
            config,
            udp_socket,
            tls_acceptor,
            new_device_tx,
        })
    }

    pub async fn start(&self) -> Result<()> {
        info!("[UDP] Listening on socket");

        loop {
            let mut buffer = vec![0; 8192];

            let (len, mut addr) = self.udp_socket.recv_from(&mut buffer).await?;
            if let Ok(packet) = json::from_slice::<IdentityPacket>(&buffer[..len]) {
                let identity = packet.body;

                if identity.device_id == self.config.identity.device_id {
                    info!("[UDP] Dont respond to the same device");
                    continue;
                }

                info!("[UDP] New device found: {}", identity.device_name);

                if let Some(port) = identity.tcp_port {
                    addr.set_port(port);
                }

                let packet = self.config.identity.create_packet(None);
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

                        let _ = self.new_device_tx.send(device);
                        let _ = self.send_identity(addr).await;
                    }
                    Err(e) => error!("Error while accepting stream: {}", e),
                }
            };
        }
    }

    async fn send_identity(&self, socket_addr: SocketAddr) -> Result<()> {
        let packet = self.config.identity.create_packet(Some(KDECONNECT_PORT));
        let data = json::to_string(&packet)? + "\n";

        let _ = self
            .udp_socket
            .send_to(data.as_bytes(), socket_addr)
            .await
            .expect("Sending packet over UDP");

        Ok(())
    }

    pub async fn broadcast_identity(&self) -> Result<()> {
        let socket_addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::BROADCAST), KDECONNECT_PORT);

        loop {
            tokio::time::sleep(Duration::from_secs(15)).await;

            let _ = self.send_identity(socket_addr).await;

            info!("[UDP] Broadcasting....");
        }
    }
}
