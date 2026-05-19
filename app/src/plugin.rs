use std::path::PathBuf;

use anyhow::Result;
use pandere_core::Service;
use pandere_plugin_host::{PluginHost, RuntimeHost, component_engine, dummy_component_bytes};

#[derive(Debug, Clone)]
pub struct PluginManifest {
    pub id: String,
    pub display_name: String,
    pub version: String,
    pub service: Service,
    pub component_path: Option<PathBuf>,
    pub enabled: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PluginLoadStatus {
    Loaded,
    Failed(String),
    Disabled,
}

impl PluginLoadStatus {
    pub fn label(&self) -> String {
        match self {
            Self::Loaded => "loaded".into(),
            Self::Failed(message) => format!("failed: {message}"),
            Self::Disabled => "disabled".into(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct LoadedMessenger {
    pub manifest: PluginManifest,
    pub status: PluginLoadStatus,
}

impl LoadedMessenger {
    pub fn status_label(&self) -> String {
        self.status.label()
    }
}

#[derive(Debug, Default, Clone)]
pub struct PluginRegistry {
    messengers: Vec<LoadedMessenger>,
}

impl PluginRegistry {
    pub fn new(messengers: Vec<LoadedMessenger>) -> Self {
        Self { messengers }
    }

    pub fn primary(&self) -> Option<&LoadedMessenger> {
        self.messengers.first()
    }
}

pub fn bootstrap_dummy_registry() -> PluginRegistry {
    PluginRegistry::new(vec![load_dummy_telegram()])
}

fn load_dummy_telegram() -> LoadedMessenger {
    let manifest = PluginManifest {
        id: "telegram".into(),
        display_name: "Telegram".into(),
        version: "0.1.0-spike".into(),
        service: Service::Telegram,
        component_path: None,
        enabled: true,
    };

    let status = match instantiate_dummy_component() {
        Ok(load_status) => load_status,
        Err(error) => PluginLoadStatus::Failed(error.to_string()),
    };

    LoadedMessenger { manifest, status }
}

fn instantiate_dummy_component() -> Result<PluginLoadStatus> {
    let engine = component_engine()?;
    let host = PluginHost::new(engine)?;
    let component_bytes = dummy_component_bytes()?;
    let component = host.load_component_from_bytes(&component_bytes)?;
    let runtime = RuntimeHost::default();
    host.probe_component(&component, runtime)?;
    Ok(PluginLoadStatus::Loaded)
}
