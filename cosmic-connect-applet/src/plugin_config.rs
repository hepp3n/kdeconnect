// cosmic-connect-applet/src/plugin_config.rs
//! Plugin configuration management for KDE Connect plugins.
//!
//! This module handles reading and writing plugin-specific configuration
//! settings for each device, stored in ~/.config/kdeconnect/{device_id}/{plugin_name}/config

use std::path::PathBuf;
use std::fs;
use std::io::{self, Write};

/// Configuration for the Share plugin (file transfer)
#[derive(Debug, Clone)]
pub struct SharePluginConfig {
    /// Directory where received files are saved
    pub destination_path: String,
}

impl Default for SharePluginConfig {
    fn default() -> Self {
        // Default to Downloads folder
        let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".to_string());
        let default_path = format!("{}/Downloads", home);
        
        Self {
            destination_path: default_path,
        }
    }
}

impl SharePluginConfig {
    /// Load configuration from file
    pub fn load(device_id: &str) -> io::Result<Self> {
        let config_path = Self::get_config_path(device_id);
        
        if !config_path.exists() {
            eprintln!("Share plugin config not found for device {}, using defaults", device_id);
            return Ok(Self::default());
        }
        
        let content = fs::read_to_string(&config_path)?;
        
        // Parse the KDE config file format (key=value)
        let mut config = Self::default();
        
        for line in content.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') || line.starts_with('[') {
                continue;
            }
            
            if let Some((key, value)) = line.split_once('=') {
                let key = key.trim();
                let value = value.trim();
                
                match key {
                    "incomingPath" | "destinationPath" => {
                        config.destination_path = value.to_string();
                    }
                    _ => {}
                }
            }
        }
        
        Ok(config)
    }
    
    /// Save configuration to file
    pub fn save(&self, device_id: &str) -> io::Result<()> {
        let config_path = Self::get_config_path(device_id);
        
        // Ensure directory exists
        if let Some(parent) = config_path.parent() {
            fs::create_dir_all(parent)?;
        }
        
        // Write config file in KDE config format
        let mut file = fs::File::create(&config_path)?;
        writeln!(file, "[General]")?;
        writeln!(file, "incomingPath={}", self.destination_path)?;
        
        eprintln!("✓ Saved share plugin config for device {}", device_id);
        eprintln!("  Path: {}", config_path.display());
        eprintln!("  Destination: {}", self.destination_path);
        
        Ok(())
    }
    
    /// Get the config file path for a device's share plugin
    fn get_config_path(device_id: &str) -> PathBuf {
        let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".to_string());
        PathBuf::from(format!("{}/.config/kdeconnect/{}/kdeconnect_share/config", home, device_id))
    }
    
    /// Check if a config file exists for the device
    pub fn exists(device_id: &str) -> bool {
        Self::get_config_path(device_id).exists()
    }
}

/// Configuration for the Clipboard plugin
#[derive(Debug, Clone)]
pub struct ClipboardPluginConfig {
    /// Automatically synchronize clipboard content
    pub auto_share: bool,
    /// Share password content from password managers
    pub send_password: bool,
}

impl Default for ClipboardPluginConfig {
    fn default() -> Self {
        Self {
            auto_share: true,      // Auto-sync enabled by default
            send_password: false,  // Don't share passwords by default (security)
        }
    }
}

impl ClipboardPluginConfig {
    /// Load configuration from file
    pub fn load(device_id: &str) -> io::Result<Self> {
        let config_path = Self::get_config_path(device_id);
        
        if !config_path.exists() {
            eprintln!("Clipboard plugin config not found for device {}, using defaults", device_id);
            return Ok(Self::default());
        }
        
        let content = fs::read_to_string(&config_path)?;
        
        // Parse the KDE config file format (key=value)
        let mut config = Self::default();
        
        for line in content.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') || line.starts_with('[') {
                continue;
            }
            
            if let Some((key, value)) = line.split_once('=') {
                let key = key.trim();
                let value = value.trim();
                
                match key {
                    "autoShare" => {
                        config.auto_share = value.parse::<bool>().unwrap_or(true);
                    }
                    "sendPassword" => {
                        config.send_password = value.parse::<bool>().unwrap_or(false);
                    }
                    _ => {}
                }
            }
        }
        
        Ok(config)
    }
    
    /// Save configuration to file
    pub fn save(&self, device_id: &str) -> io::Result<()> {
        let config_path = Self::get_config_path(device_id);
        
        // Ensure directory exists
        if let Some(parent) = config_path.parent() {
            fs::create_dir_all(parent)?;
        }
        
        // Write config file in KDE config format
        let mut file = fs::File::create(&config_path)?;
        writeln!(file, "[General]")?;
        writeln!(file, "autoShare={}", self.auto_share)?;
        writeln!(file, "sendPassword={}", self.send_password)?;
        
        eprintln!("✓ Saved clipboard plugin config for device {}", device_id);
        eprintln!("  Path: {}", config_path.display());
        eprintln!("  Auto share: {}", self.auto_share);
        eprintln!("  Send passwords: {}", self.send_password);
        
        Ok(())
    }
    
    /// Get the config file path for a device's clipboard plugin
    fn get_config_path(device_id: &str) -> PathBuf {
        let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".to_string());
        PathBuf::from(format!("{}/.config/kdeconnect/{}/kdeconnect_clipboard/config", home, device_id))
    }
    
    /// Check if a config file exists for the device
    pub fn exists(device_id: &str) -> bool {
        Self::get_config_path(device_id).exists()
    }
}

/// A single command that can be executed remotely
#[derive(Debug, Clone)]
pub struct RemoteCommand {
    /// Unique identifier for this command
    pub id: String,
    /// Name shown on the remote device
    pub name: String,
    /// The shell command to execute
    pub command: String,
}

/// Configuration for the RunCommand plugin (host remote commands)
#[derive(Debug, Clone)]
pub struct RunCommandPluginConfig {
    /// List of commands that can be executed remotely
    pub commands: Vec<RemoteCommand>,
}

impl Default for RunCommandPluginConfig {
    fn default() -> Self {
        Self {
            commands: vec![
                // Example default commands
                RemoteCommand {
                    id: "lock-screen".to_string(),
                    name: "Lock Screen".to_string(),
                    command: "loginctl lock-session".to_string(),
                },
            ],
        }
    }
}

impl RunCommandPluginConfig {
    /// Load configuration from file
    pub fn load(device_id: &str) -> io::Result<Self> {
        let config_path = Self::get_config_path(device_id);
        
        if !config_path.exists() {
            eprintln!("RunCommand plugin config not found for device {}, using defaults", device_id);
            return Ok(Self::default());
        }
        
        let content = fs::read_to_string(&config_path)?;
        
        // Parse the KDE config file format
        let mut commands = Vec::new();
        let mut current_group: Option<String> = None;
        let mut current_name: Option<String> = None;
        let mut current_command: Option<String> = None;
        
        for line in content.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            
            // Group header like [command_0]
            if line.starts_with('[') && line.ends_with(']') {
                // Save previous command if complete
                if let (Some(id), Some(name), Some(cmd)) = (&current_group, &current_name, &current_command) {
                    commands.push(RemoteCommand {
                        id: id.clone(),
                        name: name.clone(),
                        command: cmd.clone(),
                    });
                }
                
                // Start new command group
                current_group = Some(line[1..line.len()-1].to_string());
                current_name = None;
                current_command = None;
                continue;
            }
            
            if let Some((key, value)) = line.split_once('=') {
                let key = key.trim();
                let value = value.trim();
                
                match key {
                    "name" => {
                        current_name = Some(value.to_string());
                    }
                    "command" => {
                        current_command = Some(value.to_string());
                    }
                    _ => {}
                }
            }
        }
        
        // Save last command if complete
        if let (Some(id), Some(name), Some(cmd)) = (current_group, current_name, current_command) {
            commands.push(RemoteCommand {
                id,
                name,
                command: cmd,
            });
        }
        
        Ok(Self { commands })
    }
    
    /// Save configuration to file
    pub fn save(&self, device_id: &str) -> io::Result<()> {
        let config_path = Self::get_config_path(device_id);
        
        // Ensure directory exists
        if let Some(parent) = config_path.parent() {
            fs::create_dir_all(parent)?;
        }
        
        // Write config file in KDE config format
        let mut file = fs::File::create(&config_path)?;
        
        // Write each command as a group
        for (index, cmd) in self.commands.iter().enumerate() {
            writeln!(file, "[command_{}]", index)?;
            writeln!(file, "name={}", cmd.name)?;
            writeln!(file, "command={}", cmd.command)?;
            writeln!(file)?; // Empty line between groups
        }
        
        eprintln!("✓ Saved runcommand plugin config for device {}", device_id);
        eprintln!("  Path: {}", config_path.display());
        eprintln!("  Commands: {}", self.commands.len());
        
        Ok(())
    }
    
    /// Get the config file path for a device's runcommand plugin
    fn get_config_path(device_id: &str) -> PathBuf {
        let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".to_string());
        PathBuf::from(format!("{}/.config/kdeconnect/{}/kdeconnect_runcommand/config", home, device_id))
    }
    
    /// Check if a config file exists for the device
    pub fn exists(device_id: &str) -> bool {
        Self::get_config_path(device_id).exists()
    }
}

/// Configuration for the Pause media during calls plugin
#[derive(Debug, Clone)]
pub struct PauseMusicPluginConfig {
    /// When to pause media
    pub pause_on_ringing: bool,      // Pause as soon as phone rings
    pub pause_only_on_talking: bool, // Pause only while talking
    
    /// What to pause/mute
    pub pause_media: bool,            // Pause media players
    pub mute_system_sound: bool,      // Mute system sound
    
    /// Auto-resume
    pub resume_after_call: bool,      // Automatically resume media when call finished
}

impl Default for PauseMusicPluginConfig {
    fn default() -> Self {
        Self {
            // Conditions - pause as soon as phone rings by default
            pause_on_ringing: true,
            pause_only_on_talking: false,
            
            // Actions - pause media by default, don't mute system
            pause_media: true,
            mute_system_sound: false,
            
            // Auto-resume enabled by default
            resume_after_call: true,
        }
    }
}

impl PauseMusicPluginConfig {
    /// Load configuration from file
    pub fn load(device_id: &str) -> io::Result<Self> {
        let config_path = Self::get_config_path(device_id);
        
        if !config_path.exists() {
            eprintln!("PauseMusic plugin config not found for device {}, using defaults", device_id);
            return Ok(Self::default());
        }
        
        let content = fs::read_to_string(&config_path)?;
        
        // Parse the KDE config file format (key=value)
        let mut config = Self::default();
        
        for line in content.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') || line.starts_with('[') {
                continue;
            }
            
            if let Some((key, value)) = line.split_once('=') {
                let key = key.trim();
                let value = value.trim();
                
                match key {
                    "pause_on_ringing" | "pauseOnRinging" => {
                        config.pause_on_ringing = value.parse::<bool>().unwrap_or(true);
                    }
                    "pause_only_on_talking" | "pauseOnlyOnTalking" => {
                        config.pause_only_on_talking = value.parse::<bool>().unwrap_or(false);
                    }
                    "pause_media" | "pauseMedia" => {
                        config.pause_media = value.parse::<bool>().unwrap_or(true);
                    }
                    "mute_system_sound" | "muteSystemSound" => {
                        config.mute_system_sound = value.parse::<bool>().unwrap_or(false);
                    }
                    "resume_after_call" | "resumeAfterCall" => {
                        config.resume_after_call = value.parse::<bool>().unwrap_or(true);
                    }
                    _ => {}
                }
            }
        }
        
        Ok(config)
    }
    
    /// Save configuration to file
    pub fn save(&self, device_id: &str) -> io::Result<()> {
        let config_path = Self::get_config_path(device_id);
        
        // Ensure directory exists
        if let Some(parent) = config_path.parent() {
            fs::create_dir_all(parent)?;
        }
        
        // Write config file in KDE config format
        let mut file = fs::File::create(&config_path)?;
        writeln!(file, "[General]")?;
        writeln!(file, "pauseOnRinging={}", self.pause_on_ringing)?;
        writeln!(file, "pauseOnlyOnTalking={}", self.pause_only_on_talking)?;
        writeln!(file, "pauseMedia={}", self.pause_media)?;
        writeln!(file, "muteSystemSound={}", self.mute_system_sound)?;
        writeln!(file, "resumeAfterCall={}", self.resume_after_call)?;
        
        eprintln!("✓ Saved pausemusic plugin config for device {}", device_id);
        eprintln!("  Path: {}", config_path.display());
        eprintln!("  Pause on ringing: {}", self.pause_on_ringing);
        eprintln!("  Pause only on talking: {}", self.pause_only_on_talking);
        eprintln!("  Pause media: {}", self.pause_media);
        eprintln!("  Mute system: {}", self.mute_system_sound);
        eprintln!("  Resume after call: {}", self.resume_after_call);
        
        Ok(())
    }
    
    /// Get the config file path for a device's pausemusic plugin
    fn get_config_path(device_id: &str) -> PathBuf {
        let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".to_string());
        PathBuf::from(format!("{}/.config/kdeconnect/{}/kdeconnect_pausemusic/config", home, device_id))
    }
    
    /// Check if a config file exists for the device
    pub fn exists(device_id: &str) -> bool {
        Self::get_config_path(device_id).exists()
    }
}

/// Configuration for the Find this device plugin (findmyphone)
#[derive(Debug, Clone)]
pub struct FindMyPhonePluginConfig {
    /// Path to the sound file to play when finding device
    pub ringtone_path: String,
}

impl Default for FindMyPhonePluginConfig {
    fn default() -> Self {
        // Default to system bell sound or a common ringtone path
        let default_sound = if std::path::Path::new("/usr/share/sounds/freedesktop/stereo/phone-incoming-call.oga").exists() {
            "/usr/share/sounds/freedesktop/stereo/phone-incoming-call.oga".to_string()
        } else if std::path::Path::new("/usr/share/sounds/freedesktop/stereo/bell.oga").exists() {
            "/usr/share/sounds/freedesktop/stereo/bell.oga".to_string()
        } else if std::path::Path::new("/usr/share/sounds/freedesktop/stereo/alarm-clock-elapsed.oga").exists() {
            "/usr/share/sounds/freedesktop/stereo/alarm-clock-elapsed.oga".to_string()
        } else {
            // Fallback to empty string, plugin will use system default
            String::new()
        };
        
        Self {
            ringtone_path: default_sound,
        }
    }
}

impl FindMyPhonePluginConfig {
    /// Load configuration from file
    pub fn load(device_id: &str) -> io::Result<Self> {
        let config_path = Self::get_config_path(device_id);
        
        if !config_path.exists() {
            eprintln!("FindMyPhone plugin config not found for device {}, using defaults", device_id);
            return Ok(Self::default());
        }
        
        let content = fs::read_to_string(&config_path)?;
        
        // Parse the KDE config file format (key=value)
        let mut config = Self::default();
        
        for line in content.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') || line.starts_with('[') {
                continue;
            }
            
            if let Some((key, value)) = line.split_once('=') {
                let key = key.trim();
                let value = value.trim();
                
                match key {
                    "ringtone" | "ringtonePath" => {
                        config.ringtone_path = value.to_string();
                    }
                    _ => {}
                }
            }
        }
        
        Ok(config)
    }
    
    /// Save configuration to file
    pub fn save(&self, device_id: &str) -> io::Result<()> {
        let config_path = Self::get_config_path(device_id);
        
        // Ensure directory exists
        if let Some(parent) = config_path.parent() {
            fs::create_dir_all(parent)?;
        }
        
        // Write config file in KDE config format
        let mut file = fs::File::create(&config_path)?;
        writeln!(file, "[General]")?;
        writeln!(file, "ringtone={}", self.ringtone_path)?;
        
        eprintln!("✓ Saved findmyphone plugin config for device {}", device_id);
        eprintln!("  Path: {}", config_path.display());
        eprintln!("  Ringtone: {}", self.ringtone_path);
        
        Ok(())
    }
    
    /// Get the config file path for a device's findmyphone plugin
    fn get_config_path(device_id: &str) -> PathBuf {
        let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".to_string());
        PathBuf::from(format!("{}/.config/kdeconnect/{}/kdeconnect_findmyphone/config", home, device_id))
    }
    
    /// Check if a config file exists for the device
    pub fn exists(device_id: &str) -> bool {
        Self::get_config_path(device_id).exists()
    }
}

/// Notification urgency level
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UrgencyLevel {
    Low = 0,
    Normal = 1,
    Critical = 2,
}

impl UrgencyLevel {
    pub fn from_i32(value: i32) -> Self {
        match value {
            0 => UrgencyLevel::Low,
            2 => UrgencyLevel::Critical,
            _ => UrgencyLevel::Normal,
        }
    }
    
    pub fn to_string(&self) -> &'static str {
        match self {
            UrgencyLevel::Low => "Low",
            UrgencyLevel::Normal => "Normal",
            UrgencyLevel::Critical => "Critical",
        }
    }
}

/// Per-application notification settings
#[derive(Debug, Clone)]
pub struct AppNotificationSetting {
    pub app_name: String,
    pub enabled: bool,
}

/// Configuration for the Send notifications plugin (sendnotifications)
#[derive(Debug, Clone)]
pub struct SendNotificationsPluginConfig {
    /// Only send persistent notifications
    pub persistent_only: bool,
    
    /// Include notification body text
    pub include_body: bool,
    
    /// Sync notification icons
    pub sync_icons: bool,
    
    /// Minimum urgency level to send (0=Low, 1=Normal, 2=Critical)
    pub min_urgency: UrgencyLevel,
    
    /// List of apps with specific settings (blocklist or allowlist)
    pub app_settings: Vec<AppNotificationSetting>,
    
    /// If true, app_settings is a blocklist (block these apps)
    /// If false, app_settings is an allowlist (only allow these apps)
    pub use_blocklist: bool,
}

impl Default for SendNotificationsPluginConfig {
    fn default() -> Self {
        Self {
            persistent_only: false,  // Send all notifications
            include_body: true,      // Include body text
            sync_icons: true,        // Sync icons
            min_urgency: UrgencyLevel::Low,  // Send all urgency levels
            app_settings: Vec::new(), // No app restrictions
            use_blocklist: true,     // Default to blocklist mode
        }
    }
}

impl SendNotificationsPluginConfig {
    /// Load configuration from file
    pub fn load(device_id: &str) -> io::Result<Self> {
        let config_path = Self::get_config_path(device_id);
        
        if !config_path.exists() {
            eprintln!("SendNotifications plugin config not found for device {}, using defaults", device_id);
            return Ok(Self::default());
        }
        
        let content = fs::read_to_string(&config_path)?;
        
        // Parse the KDE config file format
        let mut config = Self::default();
        let mut in_general = false;
        let mut in_applications = false;
        
        for line in content.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            
            // Section headers
            if line.starts_with('[') && line.ends_with(']') {
                let section = &line[1..line.len()-1];
                in_general = section == "General";
                in_applications = section == "Applications";
                continue;
            }
            
            if let Some((key, value)) = line.split_once('=') {
                let key = key.trim();
                let value = value.trim();
                
                if in_general {
                    match key {
                        "persistentOnly" => {
                            config.persistent_only = value.parse::<bool>().unwrap_or(false);
                        }
                        "includeBody" => {
                            config.include_body = value.parse::<bool>().unwrap_or(true);
                        }
                        "syncIcons" => {
                            config.sync_icons = value.parse::<bool>().unwrap_or(true);
                        }
                        "minUrgency" => {
                            let urgency_val = value.parse::<i32>().unwrap_or(0);
                            config.min_urgency = UrgencyLevel::from_i32(urgency_val);
                        }
                        "useBlocklist" | "blacklistApps" => {
                            config.use_blocklist = value.parse::<bool>().unwrap_or(true);
                        }
                        _ => {}
                    }
                } else if in_applications {
                    // Application-specific settings: app_name=true/false
                    let enabled = value.parse::<bool>().unwrap_or(true);
                    config.app_settings.push(AppNotificationSetting {
                        app_name: key.to_string(),
                        enabled,
                    });
                }
            }
        }
        
        Ok(config)
    }
    
    /// Save configuration to file
    pub fn save(&self, device_id: &str) -> io::Result<()> {
        let config_path = Self::get_config_path(device_id);
        
        // Ensure directory exists
        if let Some(parent) = config_path.parent() {
            fs::create_dir_all(parent)?;
        }
        
        // Write config file in KDE config format
        let mut file = fs::File::create(&config_path)?;
        
        // General section
        writeln!(file, "[General]")?;
        writeln!(file, "persistentOnly={}", self.persistent_only)?;
        writeln!(file, "includeBody={}", self.include_body)?;
        writeln!(file, "syncIcons={}", self.sync_icons)?;
        writeln!(file, "minUrgency={}", self.min_urgency as i32)?;
        writeln!(file, "useBlocklist={}", self.use_blocklist)?;
        writeln!(file)?;
        
        // Applications section (if any app settings exist)
        if !self.app_settings.is_empty() {
            writeln!(file, "[Applications]")?;
            for app in &self.app_settings {
                writeln!(file, "{}={}", app.app_name, app.enabled)?;
            }
        }
        
        eprintln!("✓ Saved sendnotifications plugin config for device {}", device_id);
        eprintln!("  Path: {}", config_path.display());
        eprintln!("  Persistent only: {}", self.persistent_only);
        eprintln!("  Include body: {}", self.include_body);
        eprintln!("  Sync icons: {}", self.sync_icons);
        eprintln!("  Min urgency: {:?}", self.min_urgency);
        eprintln!("  Mode: {}", if self.use_blocklist { "Blocklist" } else { "Allowlist" });
        eprintln!("  App rules: {}", self.app_settings.len());
        
        Ok(())
    }
    
    /// Get the config file path for a device's sendnotifications plugin
    fn get_config_path(device_id: &str) -> PathBuf {
        let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".to_string());
        PathBuf::from(format!("{}/.config/kdeconnect/{}/kdeconnect_sendnotifications/config", home, device_id))
    }
    
    /// Check if a config file exists for the device
    pub fn exists(device_id: &str) -> bool {
        Self::get_config_path(device_id).exists()
    }
}

/// All plugin-specific configurations
#[derive(Debug, Clone)]
pub struct PluginConfigs {
    pub share: SharePluginConfig,
    pub clipboard: ClipboardPluginConfig,
    pub runcommand: RunCommandPluginConfig,
    pub pausemusic: PauseMusicPluginConfig,
    pub findmyphone: FindMyPhonePluginConfig,
    pub sendnotifications: SendNotificationsPluginConfig,
}

impl PluginConfigs {
    /// Load all plugin configurations for a device
    pub fn load(device_id: &str) -> Self {
        Self {
            share: SharePluginConfig::load(device_id).unwrap_or_default(),
            clipboard: ClipboardPluginConfig::load(device_id).unwrap_or_default(),
            runcommand: RunCommandPluginConfig::load(device_id).unwrap_or_default(),
            pausemusic: PauseMusicPluginConfig::load(device_id).unwrap_or_default(),
            findmyphone: FindMyPhonePluginConfig::load(device_id).unwrap_or_default(),
            sendnotifications: SendNotificationsPluginConfig::load(device_id).unwrap_or_default(),
        }
    }
    
    /// Save all plugin configurations for a device
    pub fn save(&self, device_id: &str) -> io::Result<()> {
        self.share.save(device_id)?;
        self.clipboard.save(device_id)?;
        self.runcommand.save(device_id)?;
        self.pausemusic.save(device_id)?;
        self.findmyphone.save(device_id)?;
        self.sendnotifications.save(device_id)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_default_config() {
        let config = SharePluginConfig::default();
        assert!(config.destination_path.ends_with("/Downloads"));
    }
}