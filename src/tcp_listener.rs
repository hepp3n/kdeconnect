use anyhow::Result;
use serde_json as json;
use std::{
    collections::HashMap,
    net::{Ipv4Addr, SocketAddrV4},
    path::PathBuf,
    sync::Arc,
};
use tokio::{
    fs::File,
    io::{AsyncBufReadExt, AsyncReadExt, BufReader},
    net::TcpListener,
    sync::{mpsc, Mutex},
};
use tokio_native_tls::{
    native_tls::{self},
    TlsConnector,
};
use tracing::info;

use crate::{device::DeviceStream, packets::IdentityPacket};

pub const KDECONNECT_PORT: u16 = 1716;

#[derive(Debug)]
pub struct TcpConnection {
    tcp_listener: TcpListener,
    tls_connector: TlsConnector,
    stream_tx: Arc<mpsc::UnboundedSender<DeviceStream>>,
}

impl TcpConnection {
    pub async fn new(
        stream_tx: Arc<mpsc::UnboundedSender<DeviceStream>>,
        root_ca: PathBuf,
        key: PathBuf,
    ) -> Result<Self> {
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
        let tcp_listener = TcpListener::bind(socket_addr).await?;

        Ok(Self {
            stream_tx,
            tcp_listener,
            tls_connector,
        })
    }

    pub async fn listen(&self, device_id: String) -> Result<()> {
        info!("[TCP] Listening on socket");

        while let Ok((stream, _)) = self.tcp_listener.accept().await {
            let mut stream_reader = BufReader::new(stream);
            let mut identity = String::new();

            stream_reader.read_line(&mut identity).await?;

            if let Ok(packet) = json::from_str::<IdentityPacket>(&identity) {
                let identity = packet.body;

                if identity.device_id == device_id {
                    info!("[UDP] Dont respond to the same device");
                    continue;
                }

                let tls_stream = self
                    .tls_connector
                    .connect(&identity.device_id, stream_reader)
                    .await?;

                let _ = self.stream_tx.send(HashMap::from([(
                    identity.device_id,
                    Arc::new(Mutex::new(tls_stream)),
                )]));
            }
        }
        Ok(())
    }
}
