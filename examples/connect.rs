use std::{io::stdin, sync::Arc};

use kdeconnect::{device::ConnectedDevice, KdeConnectAction, KdeConnectClient};
use tokio::sync::mpsc;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let (tx, mut rx) = mpsc::unbounded_channel::<ConnectedDevice>();

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
