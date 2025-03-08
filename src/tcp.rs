use anyhow::Result;
use serde_json as json;
use std::{
    net::{Ipv4Addr, SocketAddrV4},
    sync::Arc,
};
use tokio::{
    io::{AsyncBufReadExt as _, BufReader},
    net::TcpListener,
    sync::mpsc::UnboundedSender,
};
use tokio_rustls::{
    rustls::{
        crypto::aws_lc_rs::default_provider,
        pki_types::{pem::PemObject, CertificateDer, IpAddr, ServerName},
        ClientConfig, RootCertStore,
    },
    TlsConnector,
};
use tracing::info;

use crate::{
    cert::NoCertificateVerification,
    config::KdeConnectConfig,
    device::{Device, DeviceStream},
    packets::IdentityPacket,
    KDECONNECT_PORT,
};

pub struct Tcp {
    pub config: KdeConnectConfig,
    pub listener: TcpListener,
    pub connector: TlsConnector,
    new_device_tx: UnboundedSender<DeviceStream>,
}

impl Tcp {
    pub async fn new(
        config: KdeConnectConfig,
        new_device_tx: UnboundedSender<DeviceStream>,
    ) -> Result<Tcp> {
        let certs = CertificateDer::from_pem_file(&config.signed_ca)?;

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

        Ok(Tcp {
            config,
            listener,
            connector,
            new_device_tx,
        })
    }
    pub async fn start(&self) -> Result<()> {
        info!("[TCP] Listening on socket");

        while let Ok((stream, addr)) = self.listener.accept().await {
            info!("Accepting stream");
            let mut stream_reader = BufReader::new(stream);
            let mut identity = String::new();

            stream_reader.read_line(&mut identity).await?;

            if let Ok(packet) = json::from_str::<IdentityPacket>(&identity) {
                let identity = packet.body;
                let this_device = &self.config.identity;

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

                let _ = self
                    .new_device_tx
                    .send(DeviceStream::from([(identity.device_id, device)]));
            };
        }

        Ok(())
    }
}
