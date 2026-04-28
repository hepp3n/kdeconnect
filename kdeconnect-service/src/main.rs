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

    // Single-instance guard: request the well-known D-Bus name before touching
    // any sockets or config files. DoNotQueue means a second instance exits
    // immediately rather than racing on port binding or cert/key generation.
    // The connection is scoped so it drops (releasing the name) before
    // KdeConnectService::new() acquires it on its own connection — otherwise
    // the two request_name calls on different connections would deadlock.
    {
        let guard_conn = zbus::Connection::session().await?;
        match guard_conn
            .request_name_with_flags(
                "io.github.hepp3n.kdeconnect",
                zbus::fdo::RequestNameFlags::DoNotQueue.into(),
            )
            .await
        {
            Ok(zbus::fdo::RequestNameReply::PrimaryOwner) => {
                info!("Single-instance guard passed");
            }
            Ok(_) => {
                info!("Another instance is already running — exiting");
                return Ok(());
            }
            Err(e) => {
                return Err(e.into());
            }
        }
    } // guard_conn drops here, name is released for KdeConnectService::new()

    let service = dbus_interface::KdeConnectService::new().await?;
    info!("D-Bus service started on io.github.hepp3n.kdeconnect");

    service.run().await?;

    std::process::exit(0);
}
