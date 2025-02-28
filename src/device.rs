use serde_json as json;
use std::collections::{HashMap, HashSet};
use tokio::{
    io::{AsyncWriteExt, BufReader},
    net::TcpStream,
};
use tokio_native_tls::TlsStream;

use crate::{
    packets::{DeviceType, Identity, PacketType, Pair, Ping},
    KdeConnectAction,
};

pub type DeviceStream = HashMap<String, Device>;
pub type ConnectedDevices = HashSet<ConnectedDevice>;

#[derive(Debug)]
pub enum Message {
    Idle,
    Pair,
}

#[derive(Debug, Clone)]
pub struct DeviceConfig {
    pub device_id: String,
    pub device_name: String,
    pub device_type: DeviceType,
    // pub certificate: Option<Vec<u8>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ConnectedDevice {
    pub id: String,
    pub name: String,
}

#[derive(Debug)]
pub struct Device {
    pub config: DeviceConfig,
    pub stream: TlsStream<BufReader<TcpStream>>,
    // _peer_certificate: Vec<u8>,
}

impl Device {
    pub fn new(identity: Identity, stream: TlsStream<BufReader<TcpStream>>) -> Device {
        let config = DeviceConfig {
            device_id: identity.device_id,
            device_name: identity.device_name,
            device_type: identity.device_type,
        };

        // if let Some(cert) = stream.get_ref().peer_certificate()? {
        //     return Ok(Device {
        //         config,
        //         stream,
        //         _peer_certificate: cert.to_der()?,
        //     });
        // }

        Device {
            config,
            stream,
            // _peer_certificate: Vec::new(),
        }
    }

    pub async fn inner_task(&mut self, message: &KdeConnectAction) {
        match message {
            KdeConnectAction::PairDevice => {
                let pair_packet = Pair::create_packet(true);
                let data = json::to_string(&pair_packet).expect("Creating packet") + "\n";

                let _ = self
                    .stream
                    .write_all(PacketType::Pair(data).to_data())
                    .await;
            }
            KdeConnectAction::SendPing => {
                let ping_packet = Ping::create_packet("Hello COSMIC!".into());
                let data = json::to_string(&ping_packet).expect("Creating packet") + "\n";

                let _ = self
                    .stream
                    .write_all(PacketType::Ping(data).to_data())
                    .await;
            }
        }
    }
}
