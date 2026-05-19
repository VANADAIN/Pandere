use std::collections::HashMap;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::Result;
use wasmtime::{Config, Engine};

pub mod bindings {
    wasmtime::component::bindgen!({
        path: "../wit",
        world: "messenger-plugin",
        trappable_imports: true,
    });
}

use bindings::pandere::messenger::types::{PluginError, SecretRef};

pub fn crate_name() -> &'static str {
    "pandere-plugin-host"
}

pub fn component_engine() -> Result<Engine> {
    let mut config = Config::new();
    config.wasm_component_model(true);

    Ok(Engine::new(&config)?)
}

#[derive(Debug, Default)]
pub struct HostState {
    sessions: HashMap<String, String>,
    secrets: HashMap<String, String>,
}

impl HostState {
    pub fn load_session(&self, key: &str) -> Option<String> {
        self.sessions.get(key).cloned()
    }

    pub fn store_session(&mut self, key: impl Into<String>, value: impl Into<String>) {
        self.sessions.insert(key.into(), value.into());
    }

    pub fn load_secret(&self, secret: &SecretRef) -> std::result::Result<String, PluginError> {
        self.secrets
            .get(&secret.handle)
            .cloned()
            .ok_or_else(|| PluginError::Unsupported("unknown secret handle".into()))
    }

    pub fn store_secret(
        &mut self,
        label: impl Into<String>,
        value: impl Into<String>,
    ) -> std::result::Result<SecretRef, PluginError> {
        let label = label.into();
        let handle = format!("secret://{label}");
        self.secrets.insert(handle.clone(), value.into());
        Ok(SecretRef { handle })
    }

    pub fn now_unix_secs(&self) -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock should be after unix epoch")
            .as_secs()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn component_engine_enables_component_model() {
        let _engine = component_engine().expect("component engine should initialize");
    }

    #[test]
    fn host_state_round_trips_session_and_secret_values() {
        let mut host = HostState::default();
        host.store_session("telegram/session", "serialized-session");
        assert_eq!(
            host.load_session("telegram/session").as_deref(),
            Some("serialized-session")
        );

        let secret = host
            .store_secret("telegram-auth", "top-secret")
            .expect("secret should store");
        let loaded = host.load_secret(&secret).expect("secret should load");
        assert_eq!(loaded, "top-secret");
    }
}
