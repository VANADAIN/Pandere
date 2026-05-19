use anyhow::Result;
use pandere_core::{ChatId, ChatSummary, Message, Service};

use crate::{
    app::Screen,
    data_source::{MessengerDataSource, MessengerSnapshot},
    plugin::{LoadedMessenger, PluginRegistry},
};

#[derive(Debug, Clone)]
pub struct MessengerOverview {
    pub display_name: String,
    pub service: Service,
    pub auth_label: String,
    pub sync_label: String,
    pub plugin_status_label: String,
    pub unread_count: u32,
    pub last_message_preview: Option<String>,
}

pub struct AppState {
    pub screen: Screen,
    pub source: MessengerSnapshot,
    pub registry: PluginRegistry,
    pub selected_chat_id: Option<ChatId>,
}

impl AppState {
    pub fn new(
        source: impl MessengerDataSource,
        registry: PluginRegistry,
    ) -> Result<Self> {
        let source = source.snapshot()?;
        let selected_chat_id = source.chats.first().map(|chat| chat.id.clone());

        Ok(Self {
            screen: Screen::Main,
            source,
            registry,
            selected_chat_id,
        })
    }

    pub fn messenger_overviews(&self) -> Vec<MessengerOverview> {
        let loaded = self.registry.primary().cloned().unwrap_or_else(fallback_messenger);

        self.source
            .chats
            .iter()
            .map(|chat| MessengerOverview {
                display_name: format!("{} / {}", self.source.display_name, chat.title),
                service: self.source.service,
                auth_label: self.source.auth_status.label(),
                sync_label: self.source.sync_status.label(),
                plugin_status_label: loaded.status_label(),
                unread_count: chat.unread_count,
                last_message_preview: chat.last_message_preview.clone(),
            })
            .collect()
    }

    pub fn chats(&self) -> &[ChatSummary] {
        &self.source.chats
    }

    pub fn messages(&self) -> Vec<Message> {
        match &self.selected_chat_id {
            Some(chat_id) => self
                .source
                .messages
                .iter()
                .filter(|message| message.chat_id == *chat_id)
                .cloned()
                .collect(),
            None => self.source.messages.clone(),
        }
    }

    pub fn login_lines(&self) -> Vec<String> {
        let plugin = self.registry.primary().cloned().unwrap_or_else(fallback_messenger);
        let component_path = plugin
            .manifest
            .component_path
            .as_ref()
            .map(|path| path.display().to_string())
            .unwrap_or_else(|| "embedded dummy component".into());

        vec![
            format!("Plugin: {}", plugin.manifest.display_name),
            format!("Plugin ID: {}", plugin.manifest.id),
            format!("Version: {}", plugin.manifest.version),
            format!("Service: {:?}", self.source.service),
            format!("Enabled: {}", plugin.manifest.enabled),
            format!("Component: {component_path}"),
            format!("Auth: {}", self.source.auth_status.label()),
            format!("Sync: {}", self.source.sync_status.label()),
            format!("Plugin status: {}", plugin.status_label()),
            String::new(),
            "Planned flow:".into(),
            "1. Enter phone number".into(),
            "2. Confirm code".into(),
            "3. Persist session via secure handle".into(),
        ]
    }
}

fn fallback_messenger() -> LoadedMessenger {
        LoadedMessenger {
            manifest: crate::plugin::PluginManifest {
                id: "unknown".into(),
                display_name: "Unknown".into(),
                version: "0.0.0".into(),
            service: Service::Telegram,
            component_path: None,
            enabled: false,
            },
            status: crate::plugin::PluginLoadStatus::Disabled,
        }
}
