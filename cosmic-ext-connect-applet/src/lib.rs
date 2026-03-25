//! COSMIC KDE Connect Applet library.
//!
//! This library provides shared modules for the KDE Connect applet,
//! settings window, and SMS window binaries.

use i18n_embed::{
    fluent::{fluent_language_loader, FluentLanguageLoader},
    DesktopLanguageRequester,
};
use rust_embed::RustEmbed;

#[derive(RustEmbed)]
#[folder = "i18n/"]
struct Localizations;

pub static LANGUAGE_LOADER: std::sync::LazyLock<FluentLanguageLoader> =
    std::sync::LazyLock::new(|| {
        let loader = fluent_language_loader!();
        let requested = DesktopLanguageRequester::requested_languages();
        let _ = i18n_embed::select(&loader, &Localizations, &requested);
        loader
    });

#[macro_export]
macro_rules! fl {
    ($msg:literal) => {
        i18n_embed_fl::fl!($crate::LANGUAGE_LOADER, $msg)
    };
    ($msg:literal, $($arg:tt)*) => {
        i18n_embed_fl::fl!($crate::LANGUAGE_LOADER, $msg, $($arg)*)
    };
}

pub mod backend;
pub mod messages;
pub mod models;
pub mod notifications;
pub mod plugins;
pub mod portal;
pub mod ui;

// Re-export commonly used types
pub use notifications::{PairingNotification, start_notification_listener};
