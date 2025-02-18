use device::Device;
use native_tls::{TlsAcceptor, TlsConnector};
use packets::{Identity, IdentityPacket};
use serde_json as json;
use std::{
    fs::File,
    io::{Read, Write},
    net::{Ipv4Addr, SocketAddrV4, TcpListener, TcpStream, UdpSocket},
    sync::{
        mpsc::{self, Receiver, Sender},
        Arc, Mutex,
    },
    thread::{self},
};

use config::KdeConnectConfig;

mod cert;
mod config;
mod device;
mod packets;
mod utils;

pub const KDECONNECT_PORT: u16 = 1716;

pub enum KdeAction {
    Idle,
    Pair,
}

pub struct KdeConnect {
    pub config: KdeConnectConfig,
    identity: Arc<Mutex<Identity>>,
    udp_socket: Arc<UdpSocket>,
    tcp_listener: Arc<TcpListener>,
    tls_acceptor: Arc<TlsAcceptor>,
    device_tx: Arc<Sender<Device>>,
    device_rx: Arc<Mutex<Receiver<Device>>>,
    action_rx: Option<Arc<Mutex<Receiver<KdeAction>>>>,
}

impl KdeConnect {
    pub fn new() -> anyhow::Result<KdeConnect> {
        let config = KdeConnectConfig::default();

        let socket_addr = SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, KDECONNECT_PORT);
        let udp_socket = UdpSocket::bind(socket_addr)?;
        udp_socket.set_broadcast(true)?;

        let identity = Identity::new(config.device_id.clone(), config.device_name.clone(), None);

        let mut cert_file = File::open(&config.root_ca)?;
        let mut certs = vec![];
        cert_file.read_to_end(&mut certs).unwrap();
        let mut key_file = File::open(&config.priv_key)?;
        let mut key = vec![];
        key_file.read_to_end(&mut key).unwrap();
        let pkcs8 = native_tls::Identity::from_pkcs8(&certs, &key)?;

        let acceptor = TlsAcceptor::new(pkcs8)?;

        let listener_addr = SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, KDECONNECT_PORT);
        let listener = TcpListener::bind(listener_addr)?;

        let (device_tx, device_rx) = mpsc::channel();

        Ok(KdeConnect {
            config,
            identity: Arc::new(Mutex::new(identity)),
            udp_socket: Arc::new(udp_socket),
            tcp_listener: Arc::new(listener),
            tls_acceptor: Arc::new(acceptor),
            device_tx: Arc::new(device_tx),
            device_rx: Arc::new(device_rx.into()),
            action_rx: None,
        })
    }

    fn handle_clients(
        device_rx: Arc<Mutex<Receiver<Device>>>,
        action_rx: Arc<Mutex<Receiver<KdeAction>>>,
    ) -> anyhow::Result<()> {
        while let Ok(mut device) = device_rx.lock().unwrap().recv() {
            device.inner_task(action_rx.clone())?;
        }

        Ok(())
    }

    fn udp_listener(
        device_tx: Arc<Sender<Device>>,
        udp_socket: Arc<UdpSocket>,
        this_identity: Arc<Mutex<Identity>>,
        acceptor: Arc<TlsAcceptor>,
    ) -> anyhow::Result<()> {
        println!("[UDP] Listening on socket");

        let mut buffer = vec![0; 8192];

        while let Ok((len, mut addr)) = udp_socket.recv_from(&mut buffer) {
            if let Ok(packet) = json::from_slice::<IdentityPacket>(&buffer[..len]) {
                let identity = packet.body;

                if identity.device_id == this_identity.lock().unwrap().device_id {
                    println!("[UDP] Dont respond to the same device");
                    continue;
                }

                println!("[UDP] New device found: {}", identity.device_name);

                if let Some(port) = identity.tcp_port {
                    addr.set_port(port);
                }

                let packet = this_identity.lock().unwrap().create_packet(None);
                let data = json::to_string(&packet).expect("Creating packet") + "\n";

                let stream = TcpStream::connect(addr);

                match stream {
                    Ok(mut stream) => {
                        stream
                            .write_all(data.as_bytes())
                            .expect("[TCP] Sending packet");

                        match acceptor.accept(stream) {
                            Ok(stream) => {
                                println!(
                                    "[TCP] Connected with device: {} and IP: {}",
                                    identity.device_name, addr
                                );

                                let device = Device::new(identity.clone(), stream)?;

                                device_tx.send(device).expect("[Device] Adding new device");
                            }
                            Err(err) => eprintln!("{}", err),
                        }
                    }
                    Err(err) => eprintln!("{}", err),
                }
            };
        }

        Ok(())
    }

    fn tcp_listener(
        listener: Arc<TcpListener>,
        _this_identity: Arc<Mutex<Identity>>,
    ) -> anyhow::Result<()> {
        println!("[TCP] Listening...");

        while let Ok((mut stream, addr)) = listener.accept() {
            let mut identity = String::new();
            stream.read_to_string(&mut identity)?;

            if let Ok(packet) = json::from_str::<IdentityPacket>(&identity) {
                let identity = packet.body;

                println!("[TCP] Found {}", identity.device_name);

                let connector = TlsConnector::new()?;
                let stream = TcpStream::connect(addr).expect("[TCP] Connecting to device");
                let stream = connector.connect(&addr.ip().to_string(), stream);

                println!("{:#?}", stream);
            }
        }

        Ok(())
    }

    pub fn start(&mut self, action_rx: Arc<Mutex<Receiver<KdeAction>>>) {
        self.action_rx = Some(action_rx.clone());

        thread::scope(|td| {
            let udp_socket = Arc::clone(&self.udp_socket);
            let udp_identity = Arc::clone(&self.identity);
            let acceptor = Arc::clone(&self.tls_acceptor);
            let tcp_listener = Arc::clone(&self.tcp_listener);
            let tcp_identity = Arc::clone(&self.identity);
            let device_tx = Arc::clone(&self.device_tx);
            let device_rx = Arc::clone(&self.device_rx);

            td.spawn(move || Self::handle_clients(device_rx, action_rx));
            td.spawn(move || Self::udp_listener(device_tx, udp_socket, udp_identity, acceptor));
            td.spawn(move || Self::tcp_listener(tcp_listener, tcp_identity));
        });
    }
}
