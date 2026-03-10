//! Binary entry point for the SMS window application.

use tracing::info;

fn main() -> cosmic::iced::Result {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("warn")),
        )
        .init();

    setup_signal_handlers();

    let args: Vec<String> = std::env::args().collect();

    let device_id = args
        .get(1)
        .cloned()
        .unwrap_or_else(|| "unknown".to_string());
    let device_name = args
        .get(2)
        .cloned()
        .unwrap_or_else(|| "Unknown Device".to_string());

    info!("KDE Connect SMS window starting: {} ({})", device_name, device_id);

    cosmic_ext_connect_applet::plugins::sms::run(device_id, device_name)
}

fn setup_signal_handlers() {
    use std::sync::atomic::{AtomicBool, Ordering};

    static SHUTDOWN_REQUESTED: AtomicBool = AtomicBool::new(false);

    ctrlc::set_handler(move || {
        if SHUTDOWN_REQUESTED.swap(true, Ordering::SeqCst) {
            std::process::exit(1);
        }
        std::process::exit(0);
    })
    .ok();
}
