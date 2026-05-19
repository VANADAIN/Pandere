use std::collections::HashMap;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::Result;

use crate::bindings::pandere::messenger::{
    host::{
        Host as GuestHostTrait, HttpRequest, HttpResponse, PluginError as HostPluginError,
        SecretRef as HostSecretRef,
    },
    types::{Host as TypesHostTrait, PluginError, SecretRef},
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LogEntry {
    pub level: String,
    pub message: String,
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

#[derive(Debug, Default)]
pub struct RuntimeHost {
    state: HostState,
    logs: Vec<LogEntry>,
}

impl RuntimeHost {
    pub fn new(state: HostState) -> Self {
        Self {
            state,
            logs: Vec::new(),
        }
    }

    pub fn state(&self) -> &HostState {
        &self.state
    }

    pub fn state_mut(&mut self) -> &mut HostState {
        &mut self.state
    }

    pub fn logs(&self) -> &[LogEntry] {
        &self.logs
    }
}

impl GuestHostTrait for RuntimeHost {
    fn now_unix_secs(&mut self) -> Result<u64> {
        Ok(self.state.now_unix_secs())
    }

    fn log(&mut self, level: String, message: String) -> Result<()> {
        self.logs.push(LogEntry { level, message });
        Ok(())
    }

    fn load_session(&mut self, key: String) -> Result<Option<String>> {
        Ok(self.state.load_session(&key))
    }

    fn store_session(&mut self, key: String, value: String) -> Result<()> {
        self.state.store_session(key, value);
        Ok(())
    }

    fn load_secret(
        &mut self,
        handle: HostSecretRef,
    ) -> Result<std::result::Result<String, HostPluginError>> {
        Ok(self.state.load_secret(&SecretRef {
            handle: handle.handle,
        }))
    }

    fn store_secret(
        &mut self,
        label: String,
        value: String,
    ) -> Result<std::result::Result<HostSecretRef, HostPluginError>> {
        Ok(self.state.store_secret(label, value).map(|secret| HostSecretRef {
            handle: secret.handle,
        }))
    }

    fn send_http(
        &mut self,
        request: HttpRequest,
    ) -> Result<std::result::Result<HttpResponse, HostPluginError>> {
        let response = HttpResponse {
            status: 200,
            headers: request.headers,
            body: request.body.unwrap_or_default(),
        };
        Ok(Ok(response))
    }
}

impl TypesHostTrait for RuntimeHost {}
