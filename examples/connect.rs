use std::{io::stdin, sync::Arc};

use kdeconnect::{KdeAction, KdeConnect};
use tokio::sync::{mpsc, Mutex};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let (action_tx, action_rx) = mpsc::channel(4);
    let action_rx = Arc::new(Mutex::new(action_rx));

    tokio::spawn(async move {
        let kdeconnect = KdeConnect::new().await.unwrap();

        kdeconnect.start(action_rx).await
    });

    let mut action = String::new();

    loop {
        print!("Pair? [1/2/0]: ");
        action.clear();
        stdin().read_line(&mut action).unwrap();

        match action.trim().parse::<usize>().unwrap() {
            1 => action_tx.send(KdeAction::Idle).await?,
            2 => action_tx.send(KdeAction::Pair).await?,
            0 => break,
            _ => continue,
        };
    }

    Ok(())
}
