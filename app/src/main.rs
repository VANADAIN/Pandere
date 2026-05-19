mod app;
mod fixtures;
mod terminal;
mod ui;

use anyhow::Result;
use tracing::info;
use tracing_subscriber::{EnvFilter, fmt};

use crate::{app::App, fixtures::{fixture_chats, fixture_messages}, terminal::TerminalGuard};

#[tokio::main]
async fn main() -> Result<()> {
    fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .compact()
        .init();

    info!("starting pandere fixture shell");

    let chats = fixture_chats();
    let messages = fixture_messages(&chats);
    let mut app = App::new(chats, messages);
    let mut terminal = TerminalGuard::setup()?;
    app.run(terminal.terminal())
}
