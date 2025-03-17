use anyhow::Result;
use backend::{Backend, Connector};
use config::KdeConnectConfig;
use device::{ConnectedDevice, Device};
use tokio::sync::mpsc::{channel, Receiver, Sender, UnboundedReceiver};
use tracing::info;

mod backend;
mod cert;
mod config;
pub mod device;
mod packets;
mod utils;

pub const KDECONNECT_PORT: u16 = 1716;

#[derive(Debug, Clone)]
pub enum KdeConnectAction {
    Disconnect,
    PairDevice,
    SendPing,
}

#[derive(Debug)]
pub struct KdeConnectServer {
    connected_devices: Sender<ConnectedDevice>,
    new_device_rx: UnboundedReceiver<Device>,
    action_rx: Receiver<KdeConnectAction>,
}

impl KdeConnectServer {
    pub async fn new(
        config: KdeConnectConfig,
        connected_devices: Sender<ConnectedDevice>,
        action_rx: Receiver<KdeConnectAction>,
    ) -> Result<KdeConnectServer> {
        let (backend, new_device_rx) = Backend::new(config).await?;

        tokio::spawn(async move { backend.listen().await });

        Ok(KdeConnectServer {
            connected_devices,
            new_device_rx,
            action_rx,
        })
    }

    pub async fn send_action(&mut self) -> Result<()> {
        while let Some(mut device) = self.new_device_rx.recv().await {
            info!("[LIB] Add new device: {:#?}", device);
            let _ = self
                .connected_devices
                .send(ConnectedDevice {
                    id: device.config.device_id.clone(),
                    name: device.config.device_name.clone(),
                })
                .await;

            while let Some(action) = self.action_rx.recv().await {
                let _ = device.inner_task(&action).await;
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
        let config = KdeConnectConfig::default();

        let (action_tx, action_rx) = channel(1);
        let mut server = KdeConnectServer::new(config.clone(), tx, action_rx).await?;

        tokio::spawn(async move { server.send_action().await });

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
