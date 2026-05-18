use anyhow::Result;
use tracing::info;
use tracing_subscriber::{EnvFilter, fmt};

#[tokio::main]
async fn main() -> Result<()> {
    fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .compact()
        .init();

    info!("starting pandere");
    println!("Pandere initialized.");

    Ok(())
}
