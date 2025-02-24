use anyhow::Result;
use serde_json as json;
use std::{
    collections::HashMap,
    net::{Ipv4Addr, SocketAddrV4},
    path::PathBuf,
};
use tokio::{
    fs::File,
    io::{AsyncBufReadExt, AsyncReadExt, BufReader},
    net::TcpListener,
    sync::mpsc,
};
use tokio_native_tls::{
    native_tls::{self},
    TlsConnector,
};
use tracing::info;

use crate::{
    device::DeviceStream,
    packets::{self, IdentityPacket},
};

pub const KDECONNECT_PORT: u16 = 1716;

#[derive(Debug)]
pub struct TcpConnection {
    tcp_listener: TcpListener,
    tls_connector: TlsConnector,
    stream_tx: mpsc::UnboundedSender<DeviceStream>,
    stream_rx: mpsc::UnboundedReceiver<DeviceStream>,
}

impl TcpConnection {
    pub async fn new(root_ca: PathBuf, key: PathBuf) -> Result<Self> {
        let mut connector_builder = native_tls::TlsConnector::builder();

        let mut cert_file = File::open(&root_ca).await?;
        let mut certs = vec![];
        cert_file.read_to_end(&mut certs).await?;
        let mut key_file = File::open(&key).await?;
        let mut key = vec![];
        key_file.read_to_end(&mut key).await?;
        let pkcs8 = native_tls::Identity::from_pkcs8(&certs, &key)?;

        let tls_connector = connector_builder.identity(pkcs8).build()?;
        let tls_connector = TlsConnector::from(tls_connector);

        let socket_addr = SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, KDECONNECT_PORT);
        let tcp_listener = TcpListener::bind(socket_addr)
            .await
            .expect("Cannot bind TCP Listener");

        let (stream_tx, stream_rx) = mpsc::unbounded_channel();

        Ok(Self {
            stream_tx,
            stream_rx,
            tcp_listener,
            tls_connector,
        })
    }

    pub async fn listen(&mut self) -> Result<()> {
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

                let _ = self
                    .stream_tx
                    .send(HashMap::from([(identity.device_id, tls_stream)]));

                while let Some(stream) = self.stream_rx.recv().await {
                    for (_device_id, mut stream) in stream {
                        let mut buffer = String::new();

                        stream.read_to_string(&mut buffer).await?;

                        println!("{}", buffer);
                    }
                }
            }
        }
        Ok(())
    }
}
