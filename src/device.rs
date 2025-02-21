use serde_json as json;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpStream,
};
use tokio_native_tls::TlsStream;

use crate::packets::{DeviceType, Identity, Pair};

#[derive(Debug)]
pub enum Message {
    Idle,
    Pair,
}

#[derive(Debug)]
pub struct DeviceConfig {
    pub device_id: String,
    pub device_name: String,
    pub device_type: DeviceType,
    // pub certificate: Option<Vec<u8>>,
}

#[derive(Debug)]
pub struct ConnectedDevices {
    pub id: String,
    pub name: String,
}

#[derive(Debug)]
pub struct Device {
    pub config: DeviceConfig,
    pub stream: TlsStream<TcpStream>,
    _peer_certificate: Vec<u8>,
}

impl Device {
    pub async fn new(identity: Identity, stream: TlsStream<TcpStream>) -> anyhow::Result<Device> {
        let device_id = identity.device_id;
        let device_name = identity.device_name;
        let device_type = identity.device_type;

        let config = DeviceConfig {
            device_id,
            device_name,
            device_type,
        };

        if let Some(cert) = stream.get_ref().peer_certificate()? {
            return Ok(Device {
                config,
                stream,
                _peer_certificate: cert.to_der()?,
            });
        }

        Ok(Device {
            config,
            stream,
            _peer_certificate: Vec::new(),
        })
    }

    pub async fn inner_task(&mut self, message: &Message) -> anyhow::Result<()> {
        match message {
            Message::Idle => {
                println!("Idling...");
            }
            Message::Pair => {
                let pair_packet = Pair::create_packet(true);
                let data = json::to_string(&pair_packet).expect("Creating packet") + "\n";

                self.stream
                    .write_all(data.as_bytes())
                    .await
                    .expect("[Packet] Sending");

                let mut buffer = String::new();

                self.stream
                    .read_to_string(&mut buffer)
                    .await
                    .expect("[Packet] Reading...");

                println!("{}", buffer);
            }
        };

        Ok(())
    }
}
