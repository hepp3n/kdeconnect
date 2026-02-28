// kdeconnect-service/src/main.rs
//! KDE Connect D-Bus Service Daemon

use anyhow::Result;
use tracing::info;

mod dbus_interface;

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("debug"))  // Changed to debug
        )
        .init();

    eprintln!("=== KDE Connect Service Starting ===");
    info!("=== KDE Connect Service Starting ===");

    let service = dbus_interface::KdeConnectService::new().await?;
    eprintln!("✓ D-Bus service started on org.cosmic.KdeConnect");
    info!("✓ D-Bus service started on org.cosmic.KdeConnect");

    service.run().await?;

    Ok(())
}
