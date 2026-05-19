mod app;
mod data_source;
mod fixtures;
mod plugin;
mod state;
mod terminal;
mod ui;

use anyhow::Result;
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
    let source = HostBackedFixtureSource::new(
        registry
            .primary()
            .cloned()
            .expect("dummy registry should contain a messenger"),
    );
    let state = AppState::new(source, registry)?;
    let mut app = App::new(state);
    let mut terminal = TerminalGuard::setup()?;
    app.run(terminal.terminal())
}
