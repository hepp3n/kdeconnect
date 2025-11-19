// SPDX-License-Identifier: GPL-3.0-only

use app::CosmicConnect;
use tracing::{Level, level_filters::LevelFilter};
use tracing_subscriber::{
    FmtSubscriber, Layer, Registry, filter,
    fmt::{self, FormatEvent},
    layer::SubscriberExt as _,
    util::SubscriberInitExt as _,
};
mod app;
mod config;
mod core;

pub const APP_ID: &str = "dev.heppen.CosmicExtConnect";
pub const CONFIG_VERSION: u64 = 1;

fn main() -> cosmic::iced::Result {
    let log_file = std::fs::File::create("/tmp/cosmic_ext_connect.log").unwrap();
    let subscriber = Registry::default()
        .with(
            // stdout layer, to view everything in the console
            fmt::layer().compact().with_ansi(true),
        )
        .with(
            // log-debug file, to log the debug
            fmt::layer()
                .with_writer(log_file)
                .with_filter(filter::LevelFilter::from_level(Level::DEBUG)),
        );

    tracing::subscriber::set_global_default(subscriber).unwrap();

    cosmic::applet::run::<CosmicConnect>(())
}
