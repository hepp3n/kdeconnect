//! KDE Connect D-Bus Service Daemon

use anyhow::Result;
use tracing::info;

mod dbus_interface;

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("warn")),
        )
        .init();

    info!("KDE Connect service starting");

    let service = dbus_interface::KdeConnectService::new().await?;
    info!("D-Bus service started on io.github.hepp3n.kdeconnect");

    service.run().await?;

    // Spawned tasks (event handler, core loop) keep the tokio runtime alive
    // after run() returns — force exit to ensure clean shutdown.
    std::process::exit(0);
}
