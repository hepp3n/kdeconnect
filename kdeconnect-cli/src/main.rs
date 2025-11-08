use std::collections::HashSet;
use std::sync::Arc;

use kdeconnect_core::KdeConnectCore;
use kdeconnect_core::device::{Device, DeviceId};
use kdeconnect_core::event::{AppEvent, ConnectionEvent};
use tokio::io::{self, AsyncBufReadExt as _, BufReader};
use tokio::sync::Mutex;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt().init();

    let connected_devices: Arc<Mutex<HashSet<Device>>> = Arc::new(Mutex::new(HashSet::new()));

    let (mut core, mut conn_rx) = KdeConnectCore::new().await?;
    let sender = core.take_events();

    tokio::spawn(async move {
        core.run_event_loop().await;
    });

    let connections = connected_devices.clone();

    tokio::spawn(async move {
        let stdin = io::stdin();
        let reader = BufReader::new(stdin);
        let mut lines = reader.lines();

        while let Some(line) = lines.next_line().await.expect("failed to read line") {
            match line.as_str() {
                "show" => {
                    println!("Currently Connected devices:\n{:?}\n", connections);
                }
                "pair" => {
                    let device_id = DeviceId(line);
                    let _ = sender.send(AppEvent::Pair(device_id));
                }
                _ => {}
            }
        }
    });

    while let Some(event) = conn_rx.recv().await {
        match event {
            ConnectionEvent::Connected(_) => todo!(),
            ConnectionEvent::Disconnected(_device_id) => todo!(),
            ConnectionEvent::StateUpdated(_state) => todo!(),
        }
    }

    Ok(())
}
