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

    // Single-instance guard: check the bus name before touching sockets or
    // config files. The service connection below owns the name after its D-Bus
    // objects are registered, which avoids zbus' request-before-serve warning.
    {
        let guard_conn = zbus::Connection::session().await?;
        let dbus = zbus::fdo::DBusProxy::new(&guard_conn).await?;
        let service_name = zbus::names::BusName::try_from("io.github.hepp3n.kdeconnect")?;
        if dbus.name_has_owner(service_name).await? {
            info!("Another instance is already running — exiting");
            return Ok(());
        }
        info!("Single-instance guard passed");
    }

    let service = dbus_interface::KdeConnectService::new().await?;
    info!("D-Bus service started on io.github.hepp3n.kdeconnect");

    service.run().await?;

    std::process::exit(0);
}
