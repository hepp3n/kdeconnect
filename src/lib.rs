use anyhow::Result;
use config::KdeConnectConfig;
use device::{ConnectedDevice, DeviceStream};
use packets::{Pair, Ping};
use serde_json as json;
use std::sync::Arc;
use tcp_listener::TcpConnection;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    sync::{mpsc, oneshot},
    task::JoinHandle,
};
use tracing::info;
use udp_listener::UdpListener;

mod cert;
mod config;
pub mod device;
mod packets;
mod tcp_listener;
mod udp_listener;
mod utils;

#[derive(Debug)]
pub enum KdeConnectAction {
    Stop {
        tx: oneshot::Sender<()>,
    },

    StartListener {
        config: Arc<KdeConnectConfig>,
        tx: mpsc::UnboundedSender<ConnectedDevice>,
    },

    PairDevice {
        id: String,
    },

    SendPing {
        id: String,
    },
}

#[derive(Debug)]
pub struct KdeConnectServer {
    action_rx: mpsc::UnboundedReceiver<KdeConnectAction>,
    stream_tx: Arc<mpsc::UnboundedSender<DeviceStream>>,
    stream_rx: mpsc::UnboundedReceiver<DeviceStream>,
}

impl KdeConnectServer {
    async fn start(&mut self) {
        while let Some(message) = self.action_rx.recv().await {
            self.update(message).await;
        }
    }

    async fn update(&mut self, message: KdeConnectAction) {
        match message {
            KdeConnectAction::Stop { tx } => {
                let _ = tx.send(());
                self.action_rx.close();
            }
            KdeConnectAction::StartListener { config, tx } => {
                info!("Starting listening");

                let arc_tx = Arc::new(tx);
                let stream_tx = Arc::clone(&self.stream_tx);
                let tx = Arc::clone(&arc_tx);
                let arc_config = Arc::clone(&config);

                tokio::spawn(async move {
                    let mut udp = UdpListener::new(
                        stream_tx,
                        tx,
                        arc_config.root_ca.clone(),
                        arc_config.priv_key.clone(),
                    )
                    .await
                    .unwrap();

                    udp.listen(arc_config.device_id.clone(), arc_config.device_name.clone())
                        .await
                        .unwrap();
                });

                let stream_tx = Arc::clone(&self.stream_tx);
                let arc_config = Arc::clone(&config);

                tokio::spawn(async move {
                    let tcp = TcpConnection::new(
                        stream_tx,
                        arc_config.root_ca.clone(),
                        arc_config.priv_key.clone(),
                    )
                    .await
                    .unwrap();

                    tcp.listen(arc_config.device_id.clone()).await.unwrap();
                });
            }
            KdeConnectAction::PairDevice { id } => {
                let pair_packet = Pair::create_packet(true);
                let data = json::to_string(&pair_packet).expect("Creating packet") + "\n";

                while let Some(rx) = self.stream_rx.recv().await {
                    info!("Trying to pair new device");

                    if let Some(stream) = rx.get(&id) {
                        stream
                            .lock()
                            .await
                            .write_all(data.as_bytes())
                            .await
                            .expect("Writing to device");

                        break;
                    }
                }
            }
            KdeConnectAction::SendPing { id } => {
                let ping_packet = Ping::create_packet("Hello COSMIC!".into());

                let data = json::to_string(&ping_packet).expect("Creating packet") + "\n";

                while let Some(rx) = self.stream_rx.recv().await {
                    info!("Trying to send ping");

                    if let Some(stream) = rx.get(&id) {
                        stream
                            .lock()
                            .await
                            .write_all(data.as_bytes())
                            .await
                            .expect("Writing to device");

                        break;
                    }
                }
            }
        };
    }
}

#[derive(Debug, Clone)]
pub struct KdeConnectClient {
    pub tx: mpsc::UnboundedSender<KdeConnectAction>,
    pub config: KdeConnectConfig,
}

impl KdeConnectClient {
    pub fn new(tx: mpsc::UnboundedSender<KdeConnectAction>) -> Self {
        let config = KdeConnectConfig::default();

        KdeConnectClient { tx, config }
    }

    pub fn send(
        &self,
        message: KdeConnectAction,
    ) -> JoinHandle<Result<(), mpsc::error::SendError<KdeConnectAction>>> {
        let tx = self.tx.clone();
        tokio::spawn(async move { tx.send(message) })
    }
}

pub async fn run_server(rx: mpsc::UnboundedReceiver<KdeConnectAction>) {
    info!("test from server");

    let (stream_tx, stream_rx) = mpsc::unbounded_channel();
    let stream_tx = Arc::new(stream_tx);

    let mut server = KdeConnectServer {
        action_rx: rx,
        stream_tx,
        stream_rx,
    };

    server.start().await;
}
