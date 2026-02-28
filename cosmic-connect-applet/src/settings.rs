// cosmic-connect-applet/src/settings.rs

use cosmic_connect_applet::backend;
use cosmic_connect_applet::models::Device;

#[derive(Debug, Clone)]
pub struct DevicePermissions {
    pub battery: bool,
    pub clipboard: bool,
    pub connectivity_report: bool,
    pub contacts: bool,
    pub findmyphone: bool,
    pub lockdevice: bool,
    pub mousepad: bool,
    pub mpris: bool,
    pub notification: bool,
    pub photo: bool,
    pub ping: bool,
    pub presenter: bool,
    pub remotekeyboard: bool,
    pub remotecommands: bool,
    pub remotesystemvolume: bool,
    pub runcommand: bool,
    pub sendnotifications: bool,
    pub sftp: bool,
    pub share: bool,
    pub sms: bool,
    pub telephony: bool,
    pub virtualmonitor: bool,
}

pub async fn fetch_devices() -> Vec<Device> {
    backend::fetch_devices().await
}

pub async fn pair_device(device_id: String) {
    eprintln!("=== Requesting Pairing ===");
    eprintln!("Device: {}", device_id);
    
    match backend::pair_device(device_id).await {
        Ok(_) => eprintln!("✓ Pairing request sent successfully"),
        Err(e) => eprintln!("✗ Failed to send pairing request: {:?}", e),
    }
}

pub async fn unpair_device(device_id: String) {
    eprintln!("=== Unpairing Device ===");
    eprintln!("Device: {}", device_id);
    
    match backend::unpair_device(device_id).await {
        Ok(_) => eprintln!("✓ Device unpaired successfully"),
        Err(e) => eprintln!("✗ Failed to unpair device: {:?}", e),
    }
}

pub async fn ping_device(device_id: String) {
    eprintln!("=== Pinging Device ===");
    eprintln!("Device: {}", device_id);
    
    match backend::ping_device(device_id).await {
        Ok(_) => eprintln!("✓ Ping sent successfully"),
        Err(e) => eprintln!("✗ Failed to send ping: {:?}", e),
    }
}

pub async fn ring_device(device_id: String) {
    eprintln!("=== Ringing Device (Find My Phone) ===");
    eprintln!("Device: {}", device_id);
    
    match backend::ring_device(device_id).await {
        Ok(_) => eprintln!("✓ Ring command sent successfully"),
        Err(e) => eprintln!("✗ Failed to ring device: {:?}", e),
    }
}

pub async fn send_files(device_id: String, files: Vec<String>) {
    eprintln!("=== Sending Files ===");
    eprintln!("Device: {}", device_id);
    eprintln!("Files: {} file(s)", files.len());
    
    match backend::send_files(device_id, files).await {
        Ok(_) => eprintln!("✓ Files sent successfully"),
        Err(e) => eprintln!("✗ Failed to send files: {:?}", e),
    }
}

pub async fn browse_device(device_id: String) {
    eprintln!("=== Browsing Device Filesystem ===");
    eprintln!("Device: {}", device_id);
    
    match backend::browse_device_filesystem(device_id).await {
        Ok(_) => eprintln!("✓ Browse command sent successfully"),
        Err(e) => eprintln!("✗ Failed to browse device: {:?}", e),
    }
}

pub async fn set_plugin_enabled_internal(_device_id: String, plugin_name: String, enabled: bool) -> Result<(), String> {
    eprintln!("=== {} Plugin ===", if enabled { "Enabling" } else { "Disabling" });
    eprintln!("Plugin: {}", plugin_name);
    eprintln!("⚠️  Plugin configuration not yet implemented in backend");
    Ok(())
}

pub async fn load_device_permissions(_device_id: String) -> DevicePermissions {
    DevicePermissions {
        battery: true,
        clipboard: true,
        connectivity_report: true,
        contacts: false,
        findmyphone: true,
        lockdevice: false,
        mousepad: false,
        mpris: false,
        notification: true,
        photo: false,
        ping: true,
        presenter: false,
        remotekeyboard: false,
        remotecommands: false,
        remotesystemvolume: false,
        runcommand: false,
        sendnotifications: true,
        sftp: true,
        share: true,
        sms: true,
        telephony: false,
        virtualmonitor: false,
    }
}

// Placeholder main function for settings binary
fn main() {
    eprintln!("Settings app not yet implemented");
    eprintln!("This will be a full COSMIC settings application");
}