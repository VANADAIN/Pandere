use anyhow::Result;
use wasmtime::{Config, Engine};

pub fn component_engine() -> Result<Engine> {
    let mut config = Config::new();
    config.wasm_component_model(true);

    Ok(Engine::new(&config)?)
}
