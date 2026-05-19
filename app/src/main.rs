mod app;
mod data_source;
mod fixtures;
mod plugin;
mod state;
mod terminal;
mod ui;

use anyhow::Result;
use pandere_plugin_telegram::{TelegramClient, TelegramConfig};
use tracing::info;
use tracing_subscriber::{EnvFilter, fmt};

use crate::{
    app::App,
    fixtures::HostBackedFixtureSource,
    plugin::bootstrap_dummy_registry,
    state::AppState,
    terminal::TerminalGuard,
};

#[tokio::main]
async fn main() -> Result<()> {
    fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .compact()
        .init();

    info!("starting pandere host-backed shell");

    let registry = bootstrap_dummy_registry();
    let telegram_config = maybe_load_telegram_config()?;
    let telegram = maybe_connect_telegram(telegram_config.clone()).await?;
    let source = HostBackedFixtureSource::new(
        registry
            .primary()
            .cloned()
            .expect("dummy registry should contain a messenger"),
    );
    let state = AppState::new(source, registry)?;
    let mut app = App::new(state, telegram_config, telegram);
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

async fn maybe_connect_telegram(config: Option<TelegramConfig>) -> Result<Option<TelegramClient>> {
    let Some(config) = config else {
        return Ok(None);
    };

    Ok(Some(TelegramClient::connect(config).await?))
}
