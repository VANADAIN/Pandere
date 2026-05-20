use std::collections::HashMap;

use anyhow::Result;
use pandere_core::{ChatId, ChatSummary, Message, MessageDeliveryState, MessageId, Service};
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MessengerView {
    Root,
    GroupThreads { root_chat_id: ChatId },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MessengerFocus {
    Left,
    Right,
}

pub struct AppState {
    pub screen: Screen,
    pub source: MessengerSnapshot,
    pub registry: PluginRegistry,
    pub selected_root_chat_id: Option<ChatId>,
    pub selected_thread_chat_id: Option<ChatId>,
    pub messenger_view: MessengerView,
    pub messenger_focus: MessengerFocus,
    pub login_phase: Option<LoginPhase>,
    pub login_input_mode: Option<LoginInputMode>,
    pub login_input: String,
    pub login_notice: Option<String>,
    pub login_phone_number: Option<String>,
    pub login_session_path: Option<String>,
    pub login_has_saved_session: bool,
    pub login_password_hint: Option<String>,
    pub login_user_label: Option<String>,
    pub forum_threads: HashMap<ChatId, Vec<ChatSummary>>,
    pub forum_threads_status: ThreadStatus,
    pub thread_cache: HashMap<ChatId, Vec<Message>>,
    pub thread_status: ThreadStatus,
    pub thread_scroll: u16,
    pub follow_thread_bottom: bool,
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
        let thread_cache = build_thread_cache(&source.messages);

        let mut state = Self {
            screen: Screen::Main,
            source,
            registry,
            selected_root_chat_id: None,
            selected_thread_chat_id: None,
            messenger_view: MessengerView::Root,
            messenger_focus: MessengerFocus::Left,
            login_phase: None,
            login_input_mode: None,
            login_input: String::new(),
            login_notice: None,
            login_phone_number: None,
            login_session_path: None,
            login_has_saved_session: false,
            login_password_hint: None,
            login_user_label: None,
            forum_threads: HashMap::new(),
            forum_threads_status: ThreadStatus::Idle,
            thread_cache,
            thread_status: ThreadStatus::Idle,
            thread_scroll: 0,
            follow_thread_bottom: true,
            composer_active: false,
            composer_input: String::new(),
            composer_notice: None,
        };
        state.reset_messenger_selection();
        let _ = state.apply_cached_thread();
        Ok(state)
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
        if self.registry.all().is_empty() {
            let loaded = fallback_messenger();
            return vec![PluginCard {
                display_name: loaded.manifest.display_name.clone(),
                version: loaded.manifest.version.clone(),
                service: loaded.manifest.service,
                enabled: loaded.manifest.enabled,
                auth_label: self.source.auth_status.label(),
                sync_label: self.source.sync_status.label(),
                plugin_status_label: loaded.status_label(),
                component_label: "embedded dummy component".into(),
            }];
        }

        self.registry
            .all()
            .iter()
            .map(|loaded| PluginCard {
                display_name: loaded.manifest.display_name.clone(),
                version: loaded.manifest.version.clone(),
                service: loaded.manifest.service,
                enabled: loaded.manifest.enabled,
                auth_label: if loaded.manifest.service == self.source.service {
                    self.source.auth_status.label()
                } else {
                    "not connected".into()
                },
                sync_label: if loaded.manifest.service == self.source.service {
                    self.source.sync_status.label()
                } else {
                    "idle".into()
                },
                plugin_status_label: loaded.status_label(),
                component_label: loaded
                    .manifest
                    .component_path
                    .as_ref()
                    .map(|path| path.display().to_string())
                    .unwrap_or_else(|| "embedded dummy component".into()),
            })
            .collect()
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

    pub fn root_chats(&self) -> Vec<&ChatSummary> {
        self.source
            .chats
            .iter()
            .filter(|chat| !is_topic_chat_id(&chat.id))
            .collect()
    }

    pub fn thread_chats(&self) -> Vec<&ChatSummary> {
        let Some(root_chat_id) = self.selected_root_chat_id.as_ref() else {
            return Vec::new();
        };
        self.forum_threads
            .get(root_chat_id)
            .map(|threads| threads.iter().collect())
            .unwrap_or_default()
    }

    pub fn selected_root_chat(&self) -> Option<&ChatSummary> {
        let selected = self.selected_root_chat_id.as_ref()?;
        self.source.chats.iter().find(|chat| &chat.id == selected)
    }

    pub fn selected_leaf_chat(&self) -> Option<&ChatSummary> {
        let chat_id = self.active_chat_id()?;
        self.source
            .chats
            .iter()
            .find(|chat| chat.id == chat_id)
            .or_else(|| {
                self.forum_threads
                    .values()
                    .flat_map(|threads| threads.iter())
                    .find(|chat| chat.id == chat_id)
            })
    }

    pub fn messages(&self) -> Vec<Message> {
        self.source.messages.clone()
    }

    pub fn selected_root_chat_index(&self) -> Option<usize> {
        let selected = self.selected_root_chat_id.as_ref()?;
        self.root_chats()
            .iter()
            .position(|chat| &chat.id == selected)
    }

    pub fn selected_thread_chat_index(&self) -> Option<usize> {
        let selected = self.selected_thread_chat_id.as_ref()?;
        self.thread_chats()
            .iter()
            .position(|chat| &chat.id == selected)
    }

    pub fn selected_root_has_threads(&self) -> bool {
        self.selected_root_chat()
            .map(|chat| chat.has_subchats)
            .unwrap_or(false)
    }

    pub fn active_chat_id(&self) -> Option<ChatId> {
        match &self.messenger_view {
            MessengerView::Root => {
                if self.selected_root_has_threads() {
                    None
                } else {
                    self.selected_root_chat_id.clone()
                }
            }
            MessengerView::GroupThreads { .. } => self.selected_thread_chat_id.clone(),
        }
    }

    pub fn preview_chat_id(&self) -> Option<ChatId> {
        match &self.messenger_view {
            MessengerView::Root => {
                if self.selected_root_has_threads() {
                    self.selected_thread_chat_id.clone()
                } else {
                    self.selected_root_chat_id.clone()
                }
            }
            MessengerView::GroupThreads { .. } => self.selected_thread_chat_id.clone(),
        }
    }

    pub fn reset_messenger_selection(&mut self) {
        let first_root = self.root_chats().first().map(|chat| chat.id.clone());
        if self
            .selected_root_chat_id
            .as_ref()
            .is_none_or(|selected| !self.root_chats().iter().any(|chat| chat.id == *selected))
        {
            self.selected_root_chat_id = first_root;
        }
        self.messenger_view = MessengerView::Root;
        self.messenger_focus = MessengerFocus::Left;
        self.sync_selected_thread_to_root();
        self.source.messages.clear();
        self.forum_threads_status = ThreadStatus::Idle;
        self.thread_status = ThreadStatus::Idle;
        self.reset_thread_scroll();
    }

    pub fn set_chats(&mut self, chats: Vec<ChatSummary>) {
        let previously_selected = self.selected_root_chat_id.clone();
        self.source.chats = chats;
        let root_chats = self.root_chats();
        self.selected_root_chat_id = previously_selected
            .filter(|selected| root_chats.iter().any(|chat| chat.id == *selected))
            .or_else(|| root_chats.first().map(|chat| chat.id.clone()));
        self.sync_selected_thread_to_root();
    }

    pub fn select_next_root_chat(&mut self) {
        let root_chats = self.root_chats();
        if root_chats.is_empty() {
            self.selected_root_chat_id = None;
            return;
        }

        let current = self.selected_root_chat_index().unwrap_or(0);
        let next = (current + 1).min(root_chats.len() - 1);
        self.selected_root_chat_id = Some(root_chats[next].id.clone());
        self.sync_selected_thread_to_root();
    }

    pub fn select_previous_root_chat(&mut self) {
        let root_chats = self.root_chats();
        if root_chats.is_empty() {
            self.selected_root_chat_id = None;
            return;
        }

        let current = self.selected_root_chat_index().unwrap_or(0);
        let previous = current.saturating_sub(1);
        self.selected_root_chat_id = Some(root_chats[previous].id.clone());
        self.sync_selected_thread_to_root();
    }

    pub fn select_next_thread_chat(&mut self) {
        let thread_chats = self.thread_chats();
        if thread_chats.is_empty() {
            self.selected_thread_chat_id = None;
            return;
        }

        let current = self.selected_thread_chat_index().unwrap_or(0);
        let next = (current + 1).min(thread_chats.len() - 1);
        self.selected_thread_chat_id = Some(thread_chats[next].id.clone());
    }

    pub fn select_previous_thread_chat(&mut self) {
        let thread_chats = self.thread_chats();
        if thread_chats.is_empty() {
            self.selected_thread_chat_id = None;
            return;
        }

        let current = self.selected_thread_chat_index().unwrap_or(0);
        let previous = current.saturating_sub(1);
        self.selected_thread_chat_id = Some(thread_chats[previous].id.clone());
    }

    pub fn enter_selected_root(&mut self) {
        let Some(root_chat_id) = self.selected_root_chat_id.clone() else {
            return;
        };
        if self.selected_root_has_threads() {
            self.messenger_view = MessengerView::GroupThreads { root_chat_id };
            self.messenger_focus = MessengerFocus::Left;
        }
    }

    pub fn leave_group_threads(&mut self) {
        self.messenger_view = MessengerView::Root;
        self.messenger_focus = MessengerFocus::Left;
    }

    pub fn is_inside_group_threads(&self) -> bool {
        matches!(self.messenger_view, MessengerView::GroupThreads { .. })
    }

    pub fn focus_right_pane(&mut self) {
        self.messenger_focus = MessengerFocus::Right;
        self.follow_thread_bottom = true;
    }

    pub fn focus_left_pane(&mut self) {
        self.messenger_focus = MessengerFocus::Left;
    }

    pub fn can_focus_right_pane(&self) -> bool {
        !self.selected_root_has_threads() || self.is_inside_group_threads()
    }

    pub fn apply_cached_thread(&mut self) -> bool {
        let Some(chat_id) = self.preview_chat_id() else {
            self.source.messages.clear();
            return false;
        };

        let Some(messages) = self.thread_cache.get(&chat_id).cloned() else {
            return false;
        };

        self.source.messages = messages;
        self.thread_status = ThreadStatus::Idle;
        self.reset_thread_scroll();
        true
    }

    pub fn cache_thread(&mut self, chat_id: ChatId, messages: Vec<Message>) {
        self.thread_cache.insert(chat_id.clone(), messages.clone());
        self.refresh_chat_preview_from_messages(&chat_id);
        if self.preview_chat_id().as_ref() == Some(&chat_id) {
            self.source.messages = messages;
            self.thread_status = ThreadStatus::Idle;
            self.reset_thread_scroll();
        }
    }

    pub fn apply_cached_forum_threads(&mut self) -> bool {
        let Some(root_chat_id) = self.selected_root_chat_id.as_ref() else {
            return false;
        };
        if self.forum_threads.contains_key(root_chat_id) {
            self.forum_threads_status = ThreadStatus::Idle;
            self.sync_selected_thread_to_root();
            return true;
        }
        false
    }

    pub fn cache_forum_threads(&mut self, root_chat_id: ChatId, threads: Vec<ChatSummary>) {
        self.forum_threads.insert(root_chat_id.clone(), threads);
        if self.selected_root_chat_id.as_ref() == Some(&root_chat_id) {
            self.forum_threads_status = ThreadStatus::Idle;
            self.sync_selected_thread_to_root();
        }
    }

    pub fn set_forum_threads_loading(&mut self) {
        self.forum_threads_status = ThreadStatus::Loading;
    }

    pub fn set_forum_threads_failed(&mut self, message: impl Into<String>) {
        self.forum_threads_status = ThreadStatus::Failed(message.into());
    }

    pub fn set_thread_loading(&mut self) {
        self.source.messages.clear();
        self.thread_status = ThreadStatus::Loading;
        self.reset_thread_scroll();
    }

    pub fn set_thread_failed(&mut self, message: impl Into<String>) {
        self.source.messages.clear();
        self.thread_status = ThreadStatus::Failed(message.into());
        self.reset_thread_scroll();
    }

    pub fn thread_status_label(&self) -> String {
        self.thread_status.label()
    }

    pub fn thread_column_title(&self) -> String {
        if self.is_inside_group_threads() {
            self.selected_root_chat()
                .map(|root_chat| format!("Threads in {}", root_chat.title))
                .unwrap_or_else(|| "Threads".into())
        } else {
            "Preview".into()
        }
    }

    pub fn thread_placeholder(&self) -> String {
        match &self.messenger_view {
            MessengerView::Root => match self.selected_root_chat() {
                Some(_chat) if self.selected_root_has_threads() => {
                    match &self.forum_threads_status {
                        ThreadStatus::Idle => "Supergroup preview. Press Right to enter threads.".into(),
                        ThreadStatus::Loading => "Loading threads...".into(),
                        ThreadStatus::Failed(message) => format!("Failed to load threads: {message}"),
                    }
                }
                Some(chat) => format!("Opening {}", chat.title),
                None => "No chat selected".into(),
            },
            MessengerView::GroupThreads { .. } => {
                "Select a thread with Up/Down".into()
            }
        }
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

    pub fn set_composer_input(&mut self, value: String) {
        self.composer_input = value;
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

    pub fn scroll_thread_up(&mut self, lines: u16) {
        self.follow_thread_bottom = false;
        self.thread_scroll = self.thread_scroll.saturating_add(lines);
    }

    pub fn scroll_thread_down(&mut self, lines: u16) {
        if self.thread_scroll <= lines {
            self.thread_scroll = 0;
            self.follow_thread_bottom = true;
        } else {
            self.thread_scroll -= lines;
            self.follow_thread_bottom = false;
        }
    }

    pub fn effective_thread_scroll(&self, auto_bottom: u16) -> u16 {
        if self.follow_thread_bottom {
            auto_bottom
        } else {
            auto_bottom.saturating_sub(self.thread_scroll.min(auto_bottom))
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

    fn reset_thread_scroll(&mut self) {
        self.thread_scroll = 0;
        self.follow_thread_bottom = true;
    }

    pub fn merge_message(
        &mut self,
        chat_id: &ChatId,
        message: Message,
        replacing_message_id: Option<&MessageId>,
    ) {
        {
            let thread = self.thread_cache.entry(chat_id.clone()).or_default();
            if let Some(message_id) = replacing_message_id {
                if let Some(existing) = thread.iter_mut().find(|item| &item.id == message_id) {
                    *existing = message.clone();
                } else {
                    thread.push(message.clone());
                }
            } else if let Some(existing) = thread.iter_mut().find(|item| item.id == message.id) {
                *existing = message.clone();
            } else {
                thread.push(message.clone());
            }

            sort_messages(thread);
        }

        let is_preview = self.preview_chat_id().as_ref() == Some(chat_id);
        self.refresh_chat_preview_from_messages(chat_id);

        if is_preview {
            if let Some(thread) = self.thread_cache.get(chat_id).cloned() {
                self.source.messages = thread;
            }
            self.thread_status = ThreadStatus::Idle;
            if self.follow_thread_bottom {
                self.reset_thread_scroll();
            }
        }
    }

    pub fn mark_message_delivery(
        &mut self,
        chat_id: &ChatId,
        message_id: &MessageId,
        delivery_state: MessageDeliveryState,
    ) {
        let is_preview = self.preview_chat_id().as_ref() == Some(chat_id);
        if let Some(thread) = self.thread_cache.get_mut(chat_id) {
            if let Some(message) = thread.iter_mut().find(|item| &item.id == message_id) {
                message.delivery_state = delivery_state;
            }
            if is_preview {
                self.source.messages = thread.clone();
            }
        }
    }

    pub fn thread_messages(&self, chat_id: &ChatId) -> Option<Vec<Message>> {
        self.thread_cache.get(chat_id).cloned()
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
    for messages in cache.values_mut() {
        sort_messages(messages);
    }
    cache
}

fn is_topic_chat_id(chat_id: &ChatId) -> bool {
    chat_id.as_str().contains(":topic:")
}

impl AppState {
    fn sync_selected_thread_to_root(&mut self) {
        let thread_chats = self.thread_chats();
        if thread_chats.is_empty() {
            self.selected_thread_chat_id = None;
            return;
        }

        if self
            .selected_thread_chat_id
            .as_ref()
            .is_some_and(|selected| thread_chats.iter().any(|chat| chat.id == *selected))
        {
            return;
        }

        self.selected_thread_chat_id = thread_chats.first().map(|chat| chat.id.clone());
    }

    fn refresh_chat_preview_from_messages(&mut self, chat_id: &ChatId) {
        let preview = self.thread_cache.get(chat_id).and_then(|messages| {
            messages.last().map(|message| {
                (
                    message.text.clone(),
                    Some(message.sent_at),
                )
            })
        });

        let Some((text, sent_at)) = preview else {
            return;
        };

        if let Some(chat) = self.source.chats.iter_mut().find(|chat| &chat.id == chat_id) {
            chat.last_message_preview = Some(text.clone());
            chat.last_activity_at = sent_at;
        }

        for threads in self.forum_threads.values_mut() {
            if let Some(chat) = threads.iter_mut().find(|chat| &chat.id == chat_id) {
                chat.last_message_preview = Some(text.clone());
                chat.last_activity_at = sent_at;
            }
        }
    }
}

fn sort_messages(messages: &mut [Message]) {
    messages.sort_by_key(|message| {
        (
            message.sent_at,
            message.id.as_str().to_owned(),
        )
    });
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, SystemTime};

    use super::*;
    use crate::{fixtures::FixtureMessengerSource, plugin::PluginRegistry};

    fn test_state() -> AppState {
        AppState::new(FixtureMessengerSource, PluginRegistry::new(vec![]))
            .expect("state should initialize")
    }

    #[test]
    fn merge_message_replaces_optimistic_message() {
        let mut state = test_state();
        let chat_id = state
            .selected_root_chat_id
            .clone()
            .expect("fixture chat should exist");
        let optimistic_id = MessageId::new("local:telegram:team:1");

        let optimistic = Message {
            id: optimistic_id.clone(),
            chat_id: chat_id.clone(),
            service: Service::Telegram,
            author_name: "You".into(),
            text: "draft".into(),
            sent_at: SystemTime::now(),
            is_outgoing: true,
            delivery_state: MessageDeliveryState::Sending,
        };
        state.merge_message(&chat_id, optimistic, None);

        let sent = Message {
            id: MessageId::new("telegram:team:100"),
            chat_id: chat_id.clone(),
            service: Service::Telegram,
            author_name: "You".into(),
            text: "draft".into(),
            sent_at: SystemTime::now() + Duration::from_secs(1),
            is_outgoing: true,
            delivery_state: MessageDeliveryState::Sent,
        };
        state.merge_message(&chat_id, sent.clone(), Some(&optimistic_id));

        let messages = state
            .thread_messages(&chat_id)
            .expect("thread should exist after merge");
        assert!(messages.iter().any(|message| message.id == sent.id));
        assert!(!messages.iter().any(|message| message.id == optimistic_id));
    }

    #[test]
    fn mark_message_delivery_updates_existing_message() {
        let mut state = test_state();
        let chat_id = state
            .selected_root_chat_id
            .clone()
            .expect("fixture chat should exist");
        let message_id = MessageId::new("local:telegram:team:2");
        let message = Message {
            id: message_id.clone(),
            chat_id: chat_id.clone(),
            service: Service::Telegram,
            author_name: "You".into(),
            text: "retry me".into(),
            sent_at: SystemTime::now(),
            is_outgoing: true,
            delivery_state: MessageDeliveryState::Sending,
        };
        state.merge_message(&chat_id, message, None);

        state.mark_message_delivery(&chat_id, &message_id, MessageDeliveryState::Failed);

        let messages = state
            .thread_messages(&chat_id)
            .expect("thread should exist after merge");
        let message = messages
            .iter()
            .find(|message| message.id == message_id)
            .expect("message should remain in thread");
        assert_eq!(message.delivery_state, MessageDeliveryState::Failed);
    }

    #[test]
    fn set_chats_preserves_selection_when_chat_still_exists() {
        let mut state = test_state();
        let selected_chat_id = state
            .selected_root_chat_id
            .clone()
            .expect("fixture chat should exist");

        let mut chats = state.source.chats.clone();
        chats.reverse();
        state.set_chats(chats);

        assert_eq!(state.selected_root_chat_id, Some(selected_chat_id));
    }
}
