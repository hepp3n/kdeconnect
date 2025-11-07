// SPDX-License-Identifier: GPL-3.0-only

use app::CosmicConnect;
use tracing::Level;
use tracing_subscriber::FmtSubscriber;
mod app;
mod config;
mod core;

pub const APP_ID: &str = "dev.heppen.CosmicExtConnect";
pub const CONFIG_VERSION: u64 = 1;

fn main() -> cosmic::iced::Result {
    let subscriber = FmtSubscriber::builder()
        .with_max_level(Level::DEBUG)
        .finish();

    tracing::subscriber::set_global_default(subscriber).expect("setting default subscriber failed");

    cosmic::applet::run::<CosmicConnect>(())
}
