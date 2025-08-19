use std::{io, sync::Arc};

use kdeconnect::{self, KdeConnect, device::DeviceAction};
use tokio::task;
use tracing::Level;
use tracing_subscriber::FmtSubscriber;

#[tokio::main]
async fn main() {
    let subscriber = FmtSubscriber::builder()
        .with_max_level(Level::DEBUG)
        .finish();

    tracing::subscriber::set_global_default(subscriber).expect("setting default subscriber failed");

    let kdeconnect = Arc::new(KdeConnect::default());

    task::spawn({
        let kdeconnect = Arc::clone(&kdeconnect);
        async move {
            kdeconnect.run_server().await;
        }
    });

    let stdin = io::stdin();
    let input = &mut String::new();

    loop {
        input.clear();
        let _ = stdin.read_line(input);

        match input.trim() {
            "exit" | "quit" => {
                println!("Exiting KDE Connect server...");
                break;
            }
            "pair" => {
                tracing::debug!("Pair action received. Trying to pair.");
                let pair_action = DeviceAction::Pair(true);
                kdeconnect.send_action(
                    String::from("aab467b729ae416f8ac7480afb27d6bc"),
                    pair_action,
                );
            }
            "unpair" => {
                tracing::debug!("UnPair action received. Trying to remove link.");
                let pair_action = DeviceAction::Pair(false);
                kdeconnect.send_action(
                    String::from("aab467b729ae416f8ac7480afb27d6bc"),
                    pair_action,
                );
            }
            "ping" => {
                tracing::debug!("Ping action received. Trying to ping.");
                let ping_action =
                    DeviceAction::Ping(String::from("Hello from KDE Connect and Rust!"));

                kdeconnect.send_action(
                    String::from("aab467b729ae416f8ac7480afb27d6bc"),
                    ping_action,
                );
            }
            _ => {
                println!("Unknown command. Type 'exit' or 'quit' to stop the server.");
            }
        }
    }
}
