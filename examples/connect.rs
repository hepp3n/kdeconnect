use std::io::stdin;

use kdeconnect::{device::ConnectedDevice, run_server, KdeConnectAction, KdeConnectClient};
use tokio::sync::mpsc;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let (client_tx, server_rx) = mpsc::unbounded_channel::<KdeConnectAction>();
    let client = KdeConnectClient::new(client_tx);

    tokio::spawn(async move {
        run_server(server_rx).await;
    });

    let (tx, mut rx) = mpsc::unbounded_channel::<ConnectedDevice>();

    let config = client.config.clone();
    let _ = client
        .send(KdeConnectAction::StartListener { config, tx })
        .await?;

    let mut action = String::new();

    while let Some(device) = rx.recv().await {
        println!("Connected device: {}", device.id);

        action.clear();
        stdin().read_line(&mut action).unwrap();
        println!("Action: {}", action);
    }

    Ok(())
}
