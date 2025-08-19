use std::io;

use kdeconnect::{self, KdeConnect, device::DeviceAction};
use tokio_stream::StreamExt;

use tokio::task;
use tracing::{Level, info};
use tracing_subscriber::FmtSubscriber;

#[tokio::main]
async fn main() {
    let subscriber = FmtSubscriber::builder()
        .with_max_level(Level::DEBUG)
        .finish();

    tracing::subscriber::set_global_default(subscriber).expect("setting default subscriber failed");

    let (kdeconnect, mut devices) = KdeConnect::new();
    let kconnect = kdeconnect.clone();

    task::spawn(async move {
        kconnect.run_server().await;
    });

    while let Some((device_id, connection)) = devices.next().await {
        let stdin = io::stdin();
        let input = &mut String::new();

        input.clear();
        let _ = stdin.read_line(input);

        let action = input.trim();

        info!("Sendind action {} to via {}", action, connection);

        match action {
            "exit" | "quit" => {
                println!("Exiting KDE Connect server...");
                break;
            }
            "pair" => {
                tracing::debug!("Pair action received. Trying to pair.");
                let pair_action = DeviceAction::Pair(true);
                kdeconnect.send_action(device_id, pair_action);
            }
            "unpair" => {
                tracing::debug!("UnPair action received. Trying to remove link.");
                let pair_action = DeviceAction::Pair(false);
                kdeconnect.send_action(device_id, pair_action);
            }
            "ping" => {
                tracing::debug!("Ping action received. Trying to ping.");
                let ping_action =
                    DeviceAction::Ping(String::from("Hello from KDE Connect and Rust!"));

                kdeconnect.send_action(device_id, ping_action);
            }
            _ => {
                println!("Unknown command. Type 'exit' or 'quit' to stop the server.");
            }
        };
    }
}
