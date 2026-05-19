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
    let telegram = maybe_connect_telegram().await?;
    let source = HostBackedFixtureSource::new(
        registry
            .primary()
            .cloned()
            .expect("dummy registry should contain a messenger"),
    );
    let state = AppState::new(source, registry)?;
    let mut app = App::new(state, telegram);
    app.initialize().await?;
    let mut terminal = TerminalGuard::setup()?;
    app.run(terminal.terminal()).await
}

async fn maybe_connect_telegram() -> Result<Option<TelegramClient>> {
    let config = match TelegramConfig::from_env() {
        Ok(config) => config,
        Err(error) => {
            info!(?error, "telegram config not available; using fixture login state");
            return Ok(None);
        }
    };

    Ok(Some(TelegramClient::connect(config).await?))
}
