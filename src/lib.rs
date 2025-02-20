use anyhow::Result;
use config::KdeConnectConfig;
use std::{cell::RefCell, sync::Arc};
use tokio::{
    sync::{mpsc, oneshot, Mutex},
    task::JoinHandle,
};
use udp_listener::UdpListener;

use device::Device;

mod cert;
mod config;
pub mod device;
mod packets;
mod udp_listener;
mod utils;

#[derive(Debug)]
pub enum KdeConnectAction {
    StartUdpListener,
    HandleDevices((device::Message, String)),

    Stop { tx: oneshot::Sender<()> },
}

#[derive(Debug)]
enum KdeConnectState {
    Ready,
    Running,
    Stopped,
}

#[derive(Debug)]
struct KdeConnect {
    _id: uuid::Uuid,
    rx: mpsc::Receiver<KdeConnectAction>,
    state: Arc<Mutex<RefCell<KdeConnectState>>>,
    config: KdeConnectConfig,
    connected_devices: Vec<(String, String)>,
    device_rx: Option<Arc<Mutex<mpsc::Receiver<Device>>>>,
}

#[derive(Debug, Clone)]
pub struct KdeConnectHandler {
    _id: uuid::Uuid,
    tx: mpsc::Sender<KdeConnectAction>,
    _state: Arc<Mutex<RefCell<KdeConnectState>>>,
}

impl KdeConnect {
    async fn run(&mut self) {
        self.state.lock().await.replace(KdeConnectState::Running);

        while let Some(message) = self.rx.recv().await {
            self.update(message).await;
        }

        self.state.lock().await.replace(KdeConnectState::Stopped);
    }

    async fn update(&mut self, message: KdeConnectAction) {
        match message {
            KdeConnectAction::Stop { tx } => {
                self.state.lock().await.replace(KdeConnectState::Stopped);
                let _ = tx.send(());
                self.rx.close();
            }
            KdeConnectAction::StartUdpListener => {
                let device_id = self.config.device_id.clone();
                let device_name = self.config.device_name.clone();
                let root_ca = self.config.root_ca.clone();
                let key = self.config.priv_key.clone();

                let udp_listener = UdpListener::new(device_id, device_name, root_ca, key)
                    .await
                    .unwrap();

                self.device_rx = Some(udp_listener.1.clone());

                tokio::spawn(async move {
                    let _ = udp_listener.0.lock().await.listen().await;
                });

                if let Some(device_rx) = &self.device_rx {
                    while let Some(device) = device_rx.lock().await.recv().await {
                        self.connected_devices
                            .push((device.config.device_id, device.config.device_name));
                    }
                }
            }
            KdeConnectAction::HandleDevices((action, device_id)) => {
                if let Some(device_rx) = &self.device_rx {
                    while let Some(mut device) = device_rx.lock().await.recv().await {
                        if device.config.device_id == device_id {
                            let _ = device.inner_task(&action).await;
                        }
                    }
                }
            }
        }
    }
}

impl KdeConnectHandler {
    pub fn new(buffer: usize) -> (Self, JoinHandle<()>) {
        let (tx, rx) = mpsc::channel(buffer);
        let actor_id = uuid::Uuid::new_v4();
        let state = Arc::new(Mutex::new(RefCell::new(KdeConnectState::Ready)));
        let actor_state = state.clone();

        let actor_handler = KdeConnectHandler {
            _id: actor_id,
            tx,
            _state: state,
        };

        let handle = tokio::spawn(async move {
            let config = KdeConnectConfig::default();

            let mut actor = KdeConnect {
                _id: actor_id,
                rx,
                state: actor_state,
                config,
                connected_devices: Vec::new(),
                device_rx: None,
            };
            actor.run().await;
        });

        (actor_handler, handle)
    }

    pub fn send(
        &self,
        message: KdeConnectAction,
    ) -> JoinHandle<Result<(), mpsc::error::SendError<KdeConnectAction>>> {
        let tx = self.tx.clone();
        tokio::spawn(async move { tx.send(message).await })
    }

    pub async fn stop(&self) -> JoinHandle<Result<(), mpsc::error::SendError<KdeConnectAction>>> {
        let (tx, rx) = oneshot::channel();
        let message = KdeConnectAction::Stop { tx };
        let handle = self.send(message);

        let _ = rx.await;
        handle
    }
}
