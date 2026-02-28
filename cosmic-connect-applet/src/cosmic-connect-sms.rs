// cosmic-connect-applet/src/cosmic-connect-sms.rs
//! Binary entry point for the SMS window application.

fn main() -> cosmic::iced::Result {
    // Setup signal handlers for graceful shutdown
    setup_signal_handlers();
    
    let args: Vec<String> = std::env::args().collect();
    
    let device_id = args.get(1).cloned().unwrap_or_else(|| "unknown".to_string());
    let device_name = args.get(2).cloned().unwrap_or_else(|| "Unknown Device".to_string());
    
    eprintln!("=== KDE Connect SMS Window ===");
    eprintln!("Device: {} ({})", device_name, device_id);
    
    cosmic_connect_applet::plugins::sms::run(device_id, device_name)
}

fn setup_signal_handlers() {
    use std::sync::atomic::{AtomicBool, Ordering};
    
    static SHUTDOWN_REQUESTED: AtomicBool = AtomicBool::new(false);
    
    ctrlc::set_handler(move || {
        if SHUTDOWN_REQUESTED.swap(true, Ordering::SeqCst) {
            eprintln!("Force shutdown");
            std::process::exit(1);
        }
        
        eprintln!("Graceful shutdown requested...");
        std::process::exit(0);
    })
    .ok(); // Ignore error if already set
}