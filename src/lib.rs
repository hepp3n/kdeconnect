pub(crate) mod backends;
pub(crate) mod config;
pub mod device;
pub(crate) mod helpers;
pub(crate) mod packet;
pub(crate) mod pairing_handler;
pub(crate) mod plugins;
pub(crate) mod ssl;

use backends::start_backends;
use std::sync::Arc;
use tokio::{
    sync::{
        Mutex,
        mpsc::{self, UnboundedSender},
    },
    task,
};
use tokio_stream::{StreamExt as _, wrappers::UnboundedReceiverStream};
use tracing::debug;

use crate::{
    backends::DEFAULT_PORT,
    config::CONFIG,
    device::DeviceResponse,
    packet::{Identity, PROTOCOL_VERSION, Packet, PacketType, Ping, RunCommand, RunCommandRequest},
};

#[derive(Debug, Clone)]
pub enum ClientAction {
    Broadcast,
}

#[derive(Debug, Clone)]
pub struct KdeConnect {
    client_action: Arc<Mutex<mpsc::UnboundedReceiver<ClientAction>>>,
    device_response: mpsc::UnboundedSender<DeviceResponse>,
}

impl KdeConnect {
    pub fn new() -> (
        Self,
        UnboundedSender<ClientAction>,
        UnboundedReceiverStream<DeviceResponse>,
    ) {
        let (client_action_tx, client_action_rx) = mpsc::unbounded_channel::<ClientAction>();
        let (device_response_tx, device_response_rx) = mpsc::unbounded_channel::<DeviceResponse>();

        (
            Self {
                client_action: Arc::new(Mutex::new(client_action_rx)),
                device_response: device_response_tx,
            },
            client_action_tx,
            device_response_rx.into(),
        )
    }
}

impl KdeConnect {
    pub async fn run_server(&mut self) {
        let (tx, rx) = mpsc::unbounded_channel();
        let identity_packet = self.make_identity(Some(DEFAULT_PORT)).await;
        let client_action = Arc::clone(&self.client_action);

        debug!("Starting KDE Connect");

        // This is where the server will run, handling incoming devices.
        task::spawn(async move {
            start_backends(client_action, tx, identity_packet).await;
        });

        debug!("Server is running, waiting for devices...");

        let mut stream = UnboundedReceiverStream::new(rx);

        while let Some(device) = stream.next().await {
            let device_response = self.device_response.clone();

            task::spawn(device.handler(device_response));
        }
    }

    async fn make_identity(&self, tcp_port: Option<u16>) -> Packet {
        let device_id = CONFIG.lock().await.device_uuid.clone();
        let device_name = CONFIG.lock().await.device_name.clone();
        let device_type = CONFIG.lock().await.device_type;

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
