use kdeconnect::KdeConnectClient;
use std::io::stdin;
use tokio::sync::mpsc;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let (tx, mut rx) = mpsc::channel(4);

    let _client = KdeConnectClient::new(tx);

    let mut action = String::new();

    while let Some(device) = rx.recv().await {
        println!("Connected device: {}", device.id);

        action.clear();
        stdin().read_line(&mut action).unwrap();
        println!("Action: {}", action);
    }

    Ok(())
}
