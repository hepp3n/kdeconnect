// cosmic-connect-applet/src/messages.rs
// #[allow(dead_code)] = Placeholder for code that will be used once features are fully integrated

use crate::models::Device;

#[derive(Debug, Clone)]
pub enum Message {
    TogglePopup,
    PopupClosed(cosmic::iced::window::Id),
    RefreshDevices,
    DevicesUpdated(Vec<Device>),
    ToggleDeviceMenu(String),
    
    // Device actions
    PingDevice(String),
    PairDevice(String),
    UnpairDevice(String),
    RingDevice(String),
    BrowseDevice(String),
    SendFiles(String),
    SendSMS(String),
    ShareClipboard(String),
    ShareText(String),
    ShareUrl(String),
    
    // Advanced features
    RemoteInput(String),
    LockDevice(String),
    PresenterMode(String),
    UseAsMonitor(String),
    OpenSettings,
    
    // Pairing
    AcceptPairing(String),
    RejectPairing(String),
    PairingRequestReceived(String, String, String), // device_id, device_name, device_type
    
    // Delayed refresh for post-pairing updates
    DelayedRefresh,
    
    // MPRIS events from phone - store as JSON value to avoid direct dependency
    MprisReceived(String, serde_json::Value), // device_id, mpris_data
}