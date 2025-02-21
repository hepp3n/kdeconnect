use anyhow::Result;
use config::KdeConnectConfig;
use device::{ConnectedDevices, Device};
use std::{cell::RefCell, sync::Arc};
use tokio::{
    sync::{mpsc, oneshot, Mutex},
    task::JoinHandle,
};
use udp_listener::UdpListener;

mod cert;
mod config;
pub mod device;
mod packets;
mod udp_listener;
mod utils;

#[derive(Debug)]
pub enum KdeConnectAction {
    Stop { tx: oneshot::Sender<()> },

    StartListener { config: KdeConnectConfig },
}

#[derive(Debug)]
enum KdeConnectState {
    Ready,
    Running,
    Stopped,
}

#[derive(Debug)]
pub struct KdeConnectServer {
    tx: mpsc::Sender<KdeConnectAction>,
    rx: mpsc::Receiver<KdeConnectAction>,
    device_tx: mpsc::Sender<Device>,
    device_rx: mpsc::Receiver<Device>,
    pub state: Arc<Mutex<RefCell<KdeConnectState>>>,
}

impl KdeConnectServer {
    pub async fn stop(&self) -> JoinHandle<Result<(), mpsc::error::SendError<KdeConnectAction>>> {
        let (tx, rx) = oneshot::channel();
        let message = KdeConnectAction::Stop { tx };
        let handle = self.send(message);

        let _ = rx.await;
        handle
    }

    async fn update(&mut self, message: KdeConnectAction) {
        match message {
            KdeConnectAction::Stop { tx } => {
                self.state.lock().await.replace(KdeConnectState::Stopped);
                let _ = tx.send(());
                self.rx.close();
            }
            KdeConnectAction::StartListener { config } => {
                let device_tx = Arc::clone(&Arc::new(self.device_tx.clone()));
                let config = Arc::clone(&Arc::new(config));

                tokio::task::spawn(async move {
                    let udp = UdpListener::new(
                        config.device_id.clone(),
                        config.device_name.clone(),
                        config.root_ca.clone(),
                        config.priv_key.clone(),
                        device_tx,
                    );

                    let _ = udp.await.unwrap().listen().await;
                });
            }
        }
    }

    pub fn send(
        &self,
        message: KdeConnectAction,
    ) -> JoinHandle<Result<(), mpsc::error::SendError<KdeConnectAction>>> {
        let tx = self.tx.clone();
        tokio::spawn(async move { tx.send(message).await })
    }
}

#[derive(Debug)]
pub struct KdeConnectClient {
    pub server: Arc<Mutex<RefCell<KdeConnectServer>>>,
    pub config: KdeConnectConfig,
    pub connected_devices: Vec<ConnectedDevices>,
}

impl KdeConnectClient {
    pub fn new(buffer: usize) -> (Self, JoinHandle<()>) {
        let (tx, rx) = mpsc::channel(buffer);

        let config = KdeConnectConfig::default();

        let state = Arc::new(Mutex::new(RefCell::new(KdeConnectState::Ready)));
        let (device_tx, device_rx) = mpsc::channel(4);

        let server = KdeConnectServer {
            tx,
            rx,
            state,
            device_tx,
            device_rx,
        };

        let arc_server = Arc::new(Mutex::new(RefCell::new(server)));

        let client = KdeConnectClient {
            server: arc_server.clone(),
            config,
            connected_devices: Vec::new(),
        };

        let server = arc_server.clone();

        let handle = tokio::spawn(async move {
            server
                .lock()
                .await
                .get_mut()
                .state
                .lock()
                .await
                .replace(KdeConnectState::Running);

            while let Some(message) = server.lock().await.get_mut().rx.recv().await {
                server.lock().await.get_mut().update(message).await;
            }

            server
                .lock()
                .await
                .get_mut()
                .state
                .lock()
                .await
                .replace(KdeConnectState::Stopped);
        });

        (client, handle)
    }
}
