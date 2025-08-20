pub(crate) mod backends;
pub(crate) mod config;
pub mod device;
pub(crate) mod helpers;
pub(crate) mod packet;
pub(crate) mod plugins;
pub(crate) mod ssl;

use backends::start_backends;
use std::{collections::HashMap, sync::Arc};
use tokio::{
    sync::{
        Mutex,
        mpsc::{self, UnboundedSender},
    },
    task,
};
use tokio_stream::{StreamExt as _, wrappers::UnboundedReceiverStream};
use tracing::{debug, error};

use crate::{
    backends::DEFAULT_PORT,
    config::CONFIG,
    device::{ConnectedId, Linked, NewClient},
    packet::{Identity, PROTOCOL_VERSION, Packet, PacketType, Ping, RunCommand, RunCommandRequest},
};

#[derive(Debug, Clone)]
pub enum ClientAction {
    Broadcast,
}

#[derive(Debug, Clone)]
pub struct KdeConnect {
    devices: Arc<Mutex<HashMap<ConnectedId, NewClient>>>,
    device_tx: mpsc::UnboundedSender<Linked>,
    client_action: Arc<Mutex<mpsc::UnboundedReceiver<ClientAction>>>,
}

impl KdeConnect {
    pub fn new() -> (
        Self,
        UnboundedReceiverStream<Linked>,
        UnboundedSender<ClientAction>,
    ) {
        let (conn_tx, conn_rx) = mpsc::unbounded_channel::<Linked>();
        let (client_action_tx, client_action_rx) = mpsc::unbounded_channel::<ClientAction>();

        (
            Self {
                devices: Arc::new(Mutex::new(HashMap::new())),
                device_tx: conn_tx,
                client_action: Arc::new(Mutex::new(client_action_rx)),
            },
            conn_rx.into(),
            client_action_tx,
        )
    }
}

impl KdeConnect {
    pub async fn run_server(&self) {
        let (tx, rx) = mpsc::unbounded_channel();
        let identity_packet = self.make_identity(Some(DEFAULT_PORT));
        let client_action = Arc::clone(&self.client_action);

        debug!("Starting KDE Connect");

        // This is where the server will run, handling incoming devices.
        task::spawn(async move {
            start_backends(client_action, tx, identity_packet).await;
        });

        debug!("Server is running, waiting for devices...");

        let mut stream = UnboundedReceiverStream::new(rx);

        while let Some(mut data) = stream.next().await {
            self.device_tx.send(data.0.clone()).unwrap_or_else(|e| {
                error!("Failed to send device ID: {}", e);
            });

            self.devices
                .lock()
                .await
                .insert(data.0.0.clone(), data.clone());

            task::spawn(async move {
                data.1.process_stream().await;
            });
        }
    }

    pub fn send_action(&self, device_id: String, action: device::DeviceAction) {
        let devices = Arc::clone(&self.devices);

        tokio::spawn(async move {
            let guard = devices.lock().await;

            for (id, client) in guard.iter() {
                if id == &device_id {
                    client.1.send(action.clone())
                } else {
                    error!("Device with ID {} not found", device_id);
                }
            }
        });
    }

    fn make_identity(&self, tcp_port: Option<u16>) -> Packet {
        let device_id = CONFIG.device_uuid.clone();
        let device_name = CONFIG.device_name.clone();
        let device_type = CONFIG.device_type;

        let incoming_capabilities = vec![Ping::TYPE.to_string()];

        let outgoing_capabilities = vec![
            Ping::TYPE.to_string(),
            RunCommand::TYPE.to_string(),
            RunCommandRequest::TYPE.to_string(),
        ];

        let ident = Identity {
            device_id,
            device_name,
            device_type,
            incoming_capabilities,
            outgoing_capabilities,
            protocol_version: PROTOCOL_VERSION,
            tcp_port,
        };
        make_packet!(ident)
    }
}
