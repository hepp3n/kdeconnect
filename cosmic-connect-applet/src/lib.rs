// cosmic-connect-applet/src/lib.rs
//! COSMIC KDE Connect Applet library.
//!
//! This library provides shared modules for the KDE Connect applet,
//! settings window, and SMS window binaries.

pub mod backend;
pub mod messages;
pub mod models;
pub mod notifications;
pub mod plugins;
pub mod portal;
pub mod ui;

// Re-export commonly used types
pub use notifications::{PairingNotification, start_notification_listener};