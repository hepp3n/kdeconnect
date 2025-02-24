use std::sync::Arc;

use anyhow::Result;
use config::KdeConnectConfig;
use device::{ConnectedDevice, Device, DeviceStream};
use tcp_listener::TcpConnection;
use tokio::{
    sync::{mpsc, Mutex},
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
    PairDevice,
    SendPing,
}

#[derive(Debug)]
pub struct KdeConnectServer {}

impl KdeConnectServer {
    pub async fn stream(
        receiver: Arc<Mutex<mpsc::UnboundedReceiver<DeviceStream>>>,
        message: Arc<Mutex<mpsc::UnboundedReceiver<KdeConnectAction>>>,
    ) {
        while let Some(stream) = receiver.lock().await.recv().await {
            for (_device_id, stream) in stream.into_iter() {
                let mut device = Device::new(stream);
                while let Some(message) = message.lock().await.recv().await {
                    device.inner_task(message).await;
                }
            }
        }
    }
}

#[derive(Debug, Clone)]
pub struct KdeConnectClient {
    pub action_tx: mpsc::UnboundedSender<KdeConnectAction>,
    pub config: KdeConnectConfig,
}

impl KdeConnectClient {
    pub fn new(tx: mpsc::UnboundedSender<ConnectedDevice>) -> Self {
        let config = KdeConnectConfig::default();

        let (action_tx, action_rx) = mpsc::unbounded_channel::<KdeConnectAction>();

        let config_inner = config.clone();

        tokio::spawn(async move {
            let (stream_tx, stream_rx) = mpsc::unbounded_channel::<DeviceStream>();

            info!("Starting listening");

            // start udp listener
            let mut udp = UdpListener::new(
                stream_tx,
                tx.clone(),
                config_inner.root_ca.clone(),
                config_inner.priv_key.clone(),
            )
            .await
            .unwrap();

            tokio::spawn(async move {
                udp.listen().await.unwrap();
            });

            // start tcp listener
            let mut tcp =
                TcpConnection::new(config_inner.root_ca.clone(), config_inner.priv_key.clone())
                    .await
                    .unwrap();

            tokio::spawn(async move {
                tcp.listen().await.unwrap();
            });

            let stream_rx = Arc::new(Mutex::new(stream_rx));
            let action_rx = Arc::new(Mutex::new(action_rx));

            tokio::spawn(async move {
                KdeConnectServer::stream(stream_rx.clone(), action_rx).await;
            });
        });

        KdeConnectClient { action_tx, config }
    }
    pub fn send(
        &self,
        message: KdeConnectAction,
    ) -> JoinHandle<Result<(), mpsc::error::SendError<KdeConnectAction>>> {
        let tx = self.action_tx.clone();
        tokio::spawn(async move { tx.send(message) })
    }
}
