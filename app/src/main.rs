mod app;
mod cache;
mod constants;
mod data_source;
mod fixtures;
mod logs;
mod messenger_service;
mod plugin;
mod state;
mod terminal;
mod ui;

use anyhow::Result;
use pandere_core::paths::pandere_paths;
use pandere_plugin_telegram::TelegramConfig;
use tracing::info;
use tracing_subscriber::{EnvFilter, prelude::*};

use crate::{
    app::App,
    cache::CacheStore,
    constants::APP_TITLE,
    fixtures::HostBackedFixtureSource,
    logs::{LogBuffer, LogBufferLayer},
    messenger_service::TelegramService,
    plugin::bootstrap_dummy_registry,
    state::AppState,
    terminal::TerminalGuard,
};

#[tokio::main]
async fn main() -> Result<()> {
    let log_buffer = LogBuffer::default();
    let env_filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    tracing_subscriber::registry()
        .with(env_filter)
        .with(LogBufferLayer::new(log_buffer.clone()))
        .init();

    info!("starting {APP_TITLE}");
    let paths = pandere_paths();
    paths.ensure_exists()?;
    info!(
        config_dir = %paths.config_dir.display(),
        data_dir = %paths.data_dir.display(),
        cache_dir = %paths.cache_dir.display(),
        plugin_dir = %paths.plugin_install_dir().display(),
        "prepared pandere runtime directories"
    );

    let registry = bootstrap_dummy_registry();
    let cache = CacheStore::open_default()?;
    let telegram_config = maybe_load_telegram_config()?;
    let telegram = maybe_connect_telegram(telegram_config.clone()).await?;
    let source = HostBackedFixtureSource::new(
        registry
            .primary()
            .cloned()
            .expect("dummy registry should contain a messenger"),
    );
    let state = AppState::new(source, registry)?;
    let mut app = App::new(state, telegram, log_buffer, cache);
    app.initialize().await?;
    let mut terminal = TerminalGuard::setup()?;
    app.run(terminal.terminal()).await
}

fn maybe_load_telegram_config() -> Result<Option<TelegramConfig>> {
    match TelegramConfig::from_env() {
        Ok(config) => Ok(Some(config)),
        Err(error) => {
            info!(?error, "telegram config not available; using fixture login state");
            Ok(None)
        }
    }
}

async fn maybe_connect_telegram(config: Option<TelegramConfig>) -> Result<Option<TelegramService>> {
    let Some(config) = config else {
        return Ok(None);
    };

    Ok(Some(TelegramService::connect(config).await?))
}
