// SPDX-License-Identifier: GPL-3.0-only

pub mod app;
pub mod details;
pub mod failure;
pub mod icon_cache;
pub mod key_binds;
pub mod localize;
pub mod menu;
pub mod message;
pub mod watcher;

use app::CosmicViewer;
use cosmic::{
    app::{Settings, run},
    iced::Limits,
    widget::icon,
};
use std::sync::{Mutex, OnceLock};

use crate::icon_cache::IconCache;

static ICON_CACHE: OnceLock<Mutex<IconCache>> = OnceLock::new();

pub fn icon_cache_get(name: &'static str) -> icon::Handle {
    let mut icon_cache = ICON_CACHE.get().unwrap().lock().unwrap();
    icon_cache.get(name)
}

fn main() -> cosmic::iced::Result {
    // WMDE: without a subscriber every `tracing::error!` went nowhere. RUST_LOG picks the
    // level; unset, errors only.
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();
    ICON_CACHE.get_or_init(|| Mutex::new(IconCache::new()));

    let settings = Settings::default()
        .exit_on_close(false)
        .size_limits(Limits::NONE.min_width(360.0).min_height(300.0));

    // WMDE: every path given, or the pictures directory; see viewer_core::args.
    let paths = viewer_core::args::start_paths(std::env::args_os().skip(1));

    run::<CosmicViewer>(settings, paths)
}
