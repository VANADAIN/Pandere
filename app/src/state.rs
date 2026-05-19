use std::collections::HashMap;

use anyhow::Result;
use pandere_core::{ChatId, ChatSummary, Message, Service};
use pandere_plugin_telegram::{LoginPhase, LoginState};

use crate::{
    app::Screen,
    data_source::{MessengerDataSource, MessengerSnapshot},
    plugin::{LoadedMessenger, PluginRegistry},
};

#[derive(Debug, Clone)]
pub struct PluginCard {
    pub display_name: String,
    pub version: String,
    pub service: Service,
    pub enabled: bool,
    pub auth_label: String,
    pub sync_label: String,
    pub plugin_status_label: String,
    pub component_label: String,
}

#[derive(Debug, Clone)]
pub struct ChatPreview {
    pub title: String,
    pub unread_count: u32,
    pub last_message_preview: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoginInputMode {
    Phone,
    Code,
    Password,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ThreadStatus {
    Idle,
    Loading,
    Failed(String),
}

impl ThreadStatus {
    pub fn label(&self) -> String {
        match self {
            Self::Idle => "ready".into(),
            Self::Loading => "loading".into(),
            Self::Failed(message) => format!("failed: {message}"),
        }
    }
}

pub struct AppState {
    pub screen: Screen,
    pub source: MessengerSnapshot,
    pub registry: PluginRegistry,
    pub selected_chat_id: Option<ChatId>,
    pub login_phase: Option<LoginPhase>,
    pub login_input_mode: Option<LoginInputMode>,
    pub login_input: String,
    pub login_notice: Option<String>,
    pub login_phone_number: Option<String>,
    pub login_session_path: Option<String>,
    pub login_has_saved_session: bool,
    pub login_password_hint: Option<String>,
    pub login_user_label: Option<String>,
    pub thread_cache: HashMap<ChatId, Vec<Message>>,
    pub thread_status: ThreadStatus,
    pub composer_active: bool,
    pub composer_input: String,
    pub composer_notice: Option<String>,
}

impl AppState {
    pub fn new(
        source: impl MessengerDataSource,
        registry: PluginRegistry,
    ) -> Result<Self> {
        let source = source.snapshot()?;
        let selected_chat_id = source.chats.first().map(|chat| chat.id.clone());
        let thread_cache = build_thread_cache(&source.messages);
        let initial_messages = selected_chat_id
            .as_ref()
            .and_then(|chat_id| thread_cache.get(chat_id).cloned())
            .unwrap_or_else(|| source.messages.clone());

        Ok(Self {
            screen: Screen::Main,
            source: crate::data_source::MessengerSnapshot {
                messages: initial_messages,
                ..source
            },
            registry,
            selected_chat_id,
            login_phase: None,
            login_input_mode: None,
            login_input: String::new(),
            login_notice: None,
            login_phone_number: None,
            login_session_path: None,
            login_has_saved_session: false,
            login_password_hint: None,
            login_user_label: None,
            thread_cache,
            thread_status: ThreadStatus::Idle,
            composer_active: false,
            composer_input: String::new(),
            composer_notice: None,
        })
    }

    pub fn set_login_notice(&mut self, notice: impl Into<String>) {
        self.login_notice = Some(notice.into());
    }

    pub fn clear_login_notice(&mut self) {
        self.login_notice = None;
    }

    pub fn clear_login_input(&mut self) {
        self.login_input.clear();
    }

    pub fn push_login_input(&mut self, ch: char) {
        self.login_input.push(ch);
    }

    pub fn pop_login_input(&mut self) {
        self.login_input.pop();
    }

    pub fn masked_login_input(&self) -> String {
        match self.login_input_mode {
            Some(LoginInputMode::Password) => "*".repeat(self.login_input.chars().count()),
            Some(LoginInputMode::Phone) | Some(LoginInputMode::Code) | None => {
                self.login_input.clone()
            }
        }
    }

    pub fn apply_login_state(&mut self, state: LoginState) {
        let phone_number = state.phone_number.clone();
        self.login_phase = Some(state.phase);
        self.login_phone_number = Some(phone_number.clone());
        self.login_session_path = Some(state.session_path.display().to_string());
        self.login_has_saved_session = state.has_saved_session;
        self.login_password_hint = state.password_hint;
        self.login_user_label = state.user_label;
        self.login_input_mode = match state.phase {
            LoginPhase::Connected => Some(LoginInputMode::Phone),
            LoginPhase::CodeRequested => Some(LoginInputMode::Code),
            LoginPhase::PasswordRequired => Some(LoginInputMode::Password),
            LoginPhase::Disconnected | LoginPhase::Authorized => None,
        };

        match self.login_input_mode {
            Some(LoginInputMode::Phone) => self.login_input = phone_number,
            Some(LoginInputMode::Code) | Some(LoginInputMode::Password) => {}
            None => self.clear_login_input(),
        }
    }

    pub fn plugin_cards(&self) -> Vec<PluginCard> {
        let loaded = self.registry.primary().cloned().unwrap_or_else(fallback_messenger);
        let plugin_status_label = loaded.status_label();
        let component_label = loaded
            .manifest
            .component_path
            .as_ref()
            .map(|path| path.display().to_string())
            .unwrap_or_else(|| "embedded dummy component".into());

        vec![PluginCard {
            display_name: loaded.manifest.display_name,
            version: loaded.manifest.version,
            service: loaded.manifest.service,
            enabled: loaded.manifest.enabled,
            auth_label: self.source.auth_status.label(),
            sync_label: self.source.sync_status.label(),
            plugin_status_label,
            component_label,
        }]
    }

    pub fn chat_previews(&self) -> Vec<ChatPreview> {
        self.source
            .chats
            .iter()
            .map(|chat| ChatPreview {
                title: chat.title.clone(),
                unread_count: chat.unread_count,
                last_message_preview: chat.last_message_preview.clone(),
            })
            .collect()
    }

    pub fn chats(&self) -> &[ChatSummary] {
        &self.source.chats
    }

    pub fn messages(&self) -> Vec<Message> {
        self.source.messages.clone()
    }

    pub fn selected_chat_index(&self) -> Option<usize> {
        let selected = self.selected_chat_id.as_ref()?;
        self.source
            .chats
            .iter()
            .position(|chat| &chat.id == selected)
    }

    pub fn select_next_chat(&mut self) {
        if self.source.chats.is_empty() {
            self.selected_chat_id = None;
            return;
        }

        let current = self.selected_chat_index().unwrap_or(0);
        let next = (current + 1).min(self.source.chats.len() - 1);
        self.selected_chat_id = Some(self.source.chats[next].id.clone());
    }

    pub fn select_previous_chat(&mut self) {
        if self.source.chats.is_empty() {
            self.selected_chat_id = None;
            return;
        }

        let current = self.selected_chat_index().unwrap_or(0);
        let previous = current.saturating_sub(1);
        self.selected_chat_id = Some(self.source.chats[previous].id.clone());
    }

    pub fn apply_cached_thread(&mut self) -> bool {
        let Some(chat_id) = self.selected_chat_id.as_ref() else {
            self.source.messages.clear();
            return false;
        };

        let Some(messages) = self.thread_cache.get(chat_id).cloned() else {
            return false;
        };

        self.source.messages = messages;
        self.thread_status = ThreadStatus::Idle;
        true
    }

    pub fn cache_thread(&mut self, chat_id: ChatId, messages: Vec<Message>) {
        self.thread_cache.insert(chat_id.clone(), messages.clone());
        if self.selected_chat_id.as_ref() == Some(&chat_id) {
            self.source.messages = messages;
            self.thread_status = ThreadStatus::Idle;
        }
    }

    pub fn set_thread_loading(&mut self) {
        self.source.messages.clear();
        self.thread_status = ThreadStatus::Loading;
    }

    pub fn set_thread_failed(&mut self, message: impl Into<String>) {
        self.source.messages.clear();
        self.thread_status = ThreadStatus::Failed(message.into());
    }

    pub fn thread_status_label(&self) -> String {
        self.thread_status.label()
    }

    pub fn activate_composer(&mut self) {
        self.composer_active = true;
        self.composer_notice = None;
    }

    pub fn deactivate_composer(&mut self) {
        self.composer_active = false;
    }

    pub fn clear_composer(&mut self) {
        self.composer_input.clear();
    }

    pub fn push_composer_input(&mut self, ch: char) {
        self.composer_input.push(ch);
    }

    pub fn pop_composer_input(&mut self) {
        self.composer_input.pop();
    }

    pub fn set_composer_notice(&mut self, notice: impl Into<String>) {
        self.composer_notice = Some(notice.into());
    }

    pub fn clear_composer_notice(&mut self) {
        self.composer_notice = None;
    }

    pub fn login_lines(&self) -> Vec<String> {
        let plugin = self.registry.primary().cloned().unwrap_or_else(fallback_messenger);
        let component_path = plugin
            .manifest
            .component_path
            .as_ref()
            .map(|path| path.display().to_string())
            .unwrap_or_else(|| "embedded dummy component".into());
        let phase_label = self
            .login_phase
            .map(|phase| format!("{phase:?}"))
            .unwrap_or_else(|| "Unavailable".into());
        let phone_number = self
            .login_phone_number
            .as_deref()
            .unwrap_or("not configured");
        let session_path = self
            .login_session_path
            .as_deref()
            .unwrap_or("not configured");
        let user_label = self
            .login_user_label
            .as_deref()
            .unwrap_or("-");
        let input_label = match self.login_input_mode {
            Some(LoginInputMode::Phone) => "Phone input",
            Some(LoginInputMode::Code) => "Code input",
            Some(LoginInputMode::Password) => "Password input",
            None => "Input",
        };

        let mut lines = vec![
            format!("Plugin: {}", plugin.manifest.display_name),
            format!("Plugin ID: {}", plugin.manifest.id),
            format!("Version: {}", plugin.manifest.version),
            format!("Service: {:?}", self.source.service),
            format!("Enabled: {}", plugin.manifest.enabled),
            format!("Component: {component_path}"),
            format!("Login phase: {phase_label}"),
            format!("Phone: {phone_number}"),
            format!("Session: {session_path}"),
            format!("Saved session: {}", self.login_has_saved_session),
            format!("Authorized user: {user_label}"),
            format!("Auth: {}", self.source.auth_status.label()),
            format!("Sync: {}", self.source.sync_status.label()),
            format!("Plugin status: {}", plugin.status_label()),
            String::new(),
        ];

        if let Some(hint) = self.login_password_hint.as_deref() {
            lines.push(format!("2FA hint: {hint}"));
        }

        if self.login_input_mode.is_some() {
            lines.push(format!("{input_label}: {}", self.masked_login_input()));
        }

        if let Some(notice) = self.login_notice.as_deref() {
            lines.push(format!("Notice: {notice}"));
        }

        lines.push(String::new());
        lines.push("Login controls:".into());
        lines.push("enter advances current login step".into());
        lines.push("r request or refresh code".into());
        lines.push("x logout and clear saved session".into());
        lines.push("backspace edit input".into());
        lines.push("esc clear input/notice".into());

        lines
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

pub(crate) fn build_thread_cache(messages: &[Message]) -> HashMap<ChatId, Vec<Message>> {
    let mut cache: HashMap<ChatId, Vec<Message>> = HashMap::new();
    for message in messages {
        cache.entry(message.chat_id.clone()).or_default().push(message.clone());
    }
    cache
}
