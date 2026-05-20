use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use std::{fs, io};

use anyhow::{Context, Result, anyhow};
use grammers_client::client::{LoginToken, PasswordToken};
use grammers_client::message::InputMessage;
use grammers_client::peer::{Dialog, Peer, User};
use grammers_client::tl;
use grammers_client::{Client, SignInError};
use grammers_mtsender::SenderPool;
use grammers_session::types::PeerRef;
use grammers_session::types::PeerId;
use grammers_session::storages::SqliteSession;
use pandere_core::paths::pandere_paths;
use pandere_core::{ChatId, ChatSummary, Message, MessageDeliveryState, MessageId, Service};
use tokio::sync::RwLock;
use tokio::task::JoinHandle;
use tracing::info;

pub fn crate_name() -> &'static str {
    "pandere-plugin-telegram"
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TelegramConfig {
    pub api_id: i32,
    pub api_hash: String,
    pub phone_number: String,
    pub session_path: PathBuf,
}

impl TelegramConfig {
    pub fn from_env() -> Result<Self> {
        let api_id = std::env::var("TELEGRAM_API_ID")
            .context("missing TELEGRAM_API_ID")?
            .parse()
            .context("invalid TELEGRAM_API_ID")?;
        let api_hash = std::env::var("TELEGRAM_API_HASH").context("missing TELEGRAM_API_HASH")?;
        let phone_number =
            std::env::var("TELEGRAM_PHONE").context("missing TELEGRAM_PHONE")?;
        let session_path = std::env::var("TELEGRAM_SESSION_PATH")
            .map(PathBuf::from)
            .unwrap_or_else(|_| default_session_path());

        Ok(Self {
            api_id,
            api_hash,
            phone_number,
            session_path,
        })
    }

    pub fn validate(&self) -> Result<()> {
        if self.api_id <= 0 {
            return Err(anyhow!("telegram api id must be positive"));
        }

        if self.api_hash.trim().is_empty() {
            return Err(anyhow!("telegram api hash must not be empty"));
        }

        if self.phone_number.trim().is_empty() {
            return Err(anyhow!("telegram phone number must not be empty"));
        }

        if self.session_path.as_os_str().is_empty() {
            return Err(anyhow!("telegram session path must not be empty"));
        }

        Ok(())
    }
}

pub fn default_session_path() -> PathBuf {
    pandere_paths().telegram_session_path()
}

pub fn migrate_legacy_session_file(destination: &Path) -> Result<bool> {
    let legacy_path = PathBuf::from("telegram.session");
    if destination.exists() || !legacy_path.exists() {
        return Ok(false);
    }

    create_parent_dir(destination)?;
    fs::rename(&legacy_path, destination).or_else(|rename_error| {
        fs::copy(&legacy_path, destination)
            .map(|_| ())
            .and_then(|_| fs::remove_file(&legacy_path))
            .map_err(|copy_error| {
                anyhow!(
                    "failed to move legacy telegram session file: rename error: {rename_error}; copy/remove error: {copy_error}"
                )
            })
    })?;

    Ok(true)
}

pub fn clear_session_file(path: &Path) -> Result<()> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(anyhow!(error))
            .with_context(|| format!("failed to remove telegram session file `{}`", path.display())),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoginPhase {
    Disconnected,
    Connected,
    CodeRequested,
    PasswordRequired,
    Authorized,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AuthStatus {
    Connected,
    NeedsLogin,
    Authorized {
        user_label: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LoginOutcome {
    CodeRequested,
    PasswordRequired {
        hint: Option<String>,
    },
    Authorized {
        user_label: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoginState {
    pub phase: LoginPhase,
    pub phone_number: String,
    pub session_path: PathBuf,
    pub has_saved_session: bool,
    pub password_hint: Option<String>,
    pub user_label: Option<String>,
}

#[derive(Clone)]
pub struct TelegramFetchClient {
    client: Client,
    peer_refs: Arc<RwLock<HashMap<i64, PeerRef>>>,
}

pub struct TelegramClient {
    service: Service,
    config: TelegramConfig,
    client: Client,
    runner_task: JoinHandle<()>,
    login_token: Option<LoginToken>,
    password_token: Option<PasswordToken>,
    peer_refs: Arc<RwLock<HashMap<i64, PeerRef>>>,
}

impl TelegramClient {
    pub async fn connect(config: TelegramConfig) -> Result<Self> {
        config.validate()?;
        let _ = migrate_legacy_session_file(&config.session_path)?;
        create_parent_dir(&config.session_path)?;

        let session = Arc::new(
            SqliteSession::open(&config.session_path)
                .await
                .with_context(|| {
                    format!(
                        "failed to open telegram session at `{}`",
                        config.session_path.display()
                    )
                })?,
        );

        let SenderPool { runner, handle, .. } = SenderPool::new(Arc::clone(&session), config.api_id);
        let client = Client::new(handle);
        let runner_task = tokio::spawn(async move { runner.run().await });

        Ok(Self {
            service: Service::Telegram,
            config,
            client,
            runner_task,
            login_token: None,
            password_token: None,
            peer_refs: Arc::new(RwLock::new(HashMap::new())),
        })
    }

    pub fn service(&self) -> Service {
        self.service
    }

    pub fn session_path(&self) -> &Path {
        &self.config.session_path
    }

    pub fn phone_number(&self) -> &str {
        &self.config.phone_number
    }

    pub fn set_phone_number(&mut self, phone_number: impl Into<String>) -> Result<()> {
        let phone_number = phone_number.into();
        if phone_number.trim().is_empty() {
            return Err(anyhow!("telegram phone number must not be empty"));
        }

        self.config.phone_number = phone_number;
        self.reset_login_flow();
        Ok(())
    }

    pub fn client(&self) -> &Client {
        &self.client
    }

    pub fn fetch_client(&self) -> TelegramFetchClient {
        TelegramFetchClient {
            client: self.client.clone(),
            peer_refs: Arc::clone(&self.peer_refs),
        }
    }

    pub fn login_plan(&self) -> [&'static str; 4] {
        [
            "Open persistent SqliteSession and spawn SenderPool runner.",
            "Check authorization state before requesting login code.",
            "Complete sign_in with code and optional password flow.",
            "After authorization, fetch dialogs to warm peer caches.",
        ]
    }

    pub fn has_saved_session(&self) -> bool {
        self.session_path().exists()
    }

    pub fn password_hint(&self) -> Option<&str> {
        self.password_token.as_ref().and_then(|token| token.hint())
    }

    pub fn reset_login_flow(&mut self) {
        self.login_token = None;
        self.password_token = None;
    }

    pub async fn auth_status(&self) -> Result<AuthStatus> {
        if !self.client.is_authorized().await? {
            return Ok(AuthStatus::NeedsLogin);
        }

        let me = self
            .client
            .get_me()
            .await
            .context("telegram session is authorized but get_me failed")?;

        Ok(AuthStatus::Authorized {
            user_label: user_label(&me),
        })
    }

    pub async fn login_phase(&self) -> Result<LoginPhase> {
        if self.runner_task.is_finished() {
            return Ok(LoginPhase::Disconnected);
        }

        if self.password_token.is_some() {
            return Ok(LoginPhase::PasswordRequired);
        }

        if self.login_token.is_some() {
            return Ok(LoginPhase::CodeRequested);
        }

        match self.auth_status().await? {
            AuthStatus::Connected => Ok(LoginPhase::Connected),
            AuthStatus::NeedsLogin => Ok(LoginPhase::Connected),
            AuthStatus::Authorized { .. } => Ok(LoginPhase::Authorized),
        }
    }

    pub async fn login_state(&self) -> Result<LoginState> {
        let phase = self.login_phase().await?;
        let user_label = match self.auth_status().await? {
            AuthStatus::Authorized { user_label } => Some(user_label),
            AuthStatus::Connected | AuthStatus::NeedsLogin => None,
        };

        Ok(LoginState {
            phase,
            phone_number: self.config.phone_number.clone(),
            session_path: self.config.session_path.clone(),
            has_saved_session: self.has_saved_session(),
            password_hint: self.password_hint().map(str::to_owned),
            user_label,
        })
    }

    pub async fn bootstrap_login(&mut self) -> Result<LoginState> {
        if self.client.is_authorized().await? {
            self.reset_login_flow();
        }

        self.login_state().await
    }

    pub async fn request_login_code(&mut self) -> Result<LoginOutcome> {
        if self.client.is_authorized().await? {
            self.reset_login_flow();
            let me = self.client.get_me().await?;
            return Ok(LoginOutcome::Authorized {
                user_label: user_label(&me),
            });
        }

        let token = self
            .client
            .request_login_code(&self.config.phone_number, &self.config.api_hash)
            .await
            .context("failed to request telegram login code")?;
        self.login_token = Some(token);
        self.password_token = None;

        Ok(LoginOutcome::CodeRequested)
    }

    pub async fn submit_login_code(&mut self, code: &str) -> Result<LoginOutcome> {
        let token = self
            .login_token
            .as_ref()
            .ok_or_else(|| anyhow!("login code was not requested"))?;

        match self.client.sign_in(token, code.trim()).await {
            Ok(user) => {
                self.login_token = None;
                self.password_token = None;
                Ok(LoginOutcome::Authorized {
                    user_label: user_label(&user),
                })
            }
            Err(SignInError::PasswordRequired(password_token)) => {
                let hint = password_token.hint().map(str::to_owned);
                self.login_token = None;
                self.password_token = Some(password_token);
                Ok(LoginOutcome::PasswordRequired { hint })
            }
            Err(SignInError::InvalidCode) => Err(anyhow!("telegram login code is invalid")),
            Err(SignInError::SignUpRequired) => Err(anyhow!(
                "telegram account must be created first with an official client"
            )),
            Err(error) => Err(anyhow!(error).context("telegram sign-in failed")),
        }
    }

    pub async fn submit_password(&mut self, password: &str) -> Result<LoginOutcome> {
        let password_token = self
            .password_token
            .take()
            .ok_or_else(|| anyhow!("telegram password is not currently required"))?;

        match self.client.check_password(password_token, password.trim()).await {
            Ok(user) => {
                self.reset_login_flow();
                Ok(LoginOutcome::Authorized {
                    user_label: user_label(&user),
                })
            }
            Err(SignInError::InvalidPassword(password_token)) => {
                let hint = password_token.hint().map(str::to_owned);
                self.password_token = Some(password_token);
                Err(anyhow!(
                    "telegram two-factor password is invalid{}",
                    hint.as_deref()
                        .map(|hint| format!(" (hint: {hint})"))
                        .unwrap_or_default()
                ))
            }
            Err(error) => Err(anyhow!(error).context("telegram password check failed")),
        }
    }

    pub async fn request_login_code_state(&mut self) -> Result<LoginState> {
        self.request_login_code().await?;
        self.login_state().await
    }

    pub async fn submit_login_code_state(&mut self, code: &str) -> Result<LoginState> {
        self.submit_login_code(code).await?;
        self.login_state().await
    }

    pub async fn submit_password_state(&mut self, password: &str) -> Result<LoginState> {
        self.submit_password(password).await?;
        self.login_state().await
    }

    pub async fn list_chats(&self, limit: usize) -> Result<Vec<ChatSummary>> {
        if !self.client.is_authorized().await? {
            return Err(anyhow!("telegram client is not authorized"));
        }

        let mut dialogs = self.client.iter_dialogs();
        let mut chats = Vec::with_capacity(limit.min(64));

        while chats.len() < limit {
            let Some(dialog) = dialogs
                .next()
                .await
                .context("failed to fetch telegram dialogs")?
            else {
                break;
            };

            self.cache_peer_ref(&dialog).await;
            chats.push(map_dialog(dialog));
        }

        Ok(chats)
    }

    pub async fn list_forum_topics(&self, chat_id: &ChatId, limit: usize) -> Result<Vec<ChatSummary>> {
        self.fetch_client().list_forum_topics(chat_id, limit).await
    }

    pub async fn fetch_messages(&self, chat_id: &ChatId, limit: usize) -> Result<Vec<Message>> {
        self.fetch_client().fetch_messages(chat_id, limit).await
    }

    pub async fn send_text(&self, chat_id: &ChatId, text: &str) -> Result<Message> {
        self.fetch_client().send_text(chat_id, text).await
    }

    async fn cache_peer_ref(&self, dialog: &Dialog) {
        self.peer_refs
            .write()
            .await
            .insert(dialog.peer().id().bot_api_dialog_id(), dialog.peer_ref());
    }
}

impl TelegramFetchClient {
    pub async fn list_chats(&self, limit: usize) -> Result<Vec<ChatSummary>> {
        let mut dialogs = self.client.iter_dialogs();
        let mut chats = Vec::with_capacity(limit.min(64));

        while chats.len() < limit {
            let Some(dialog) = dialogs
                .next()
                .await
                .context("failed to fetch telegram dialogs")?
            else {
                break;
            };

            self.peer_refs
                .write()
                .await
                .insert(dialog.peer().id().bot_api_dialog_id(), dialog.peer_ref());
            chats.push(map_dialog(dialog));
        }

        Ok(chats)
    }

    pub async fn list_forum_topics(&self, chat_id: &ChatId, limit: usize) -> Result<Vec<ChatSummary>> {
        if limit == 0 {
            return Ok(Vec::new());
        }

        let peer = self
            .find_peer_ref(chat_id)
            .await?
            .ok_or_else(|| anyhow!("telegram forum `{}` not found in cache", chat_id.as_str()))?;
        let parent_dialog_id = parse_dialog_id(chat_id.as_str())
            .ok_or_else(|| anyhow!("invalid telegram forum chat id `{}`", chat_id.as_str()))?;

        let forum_topics = self
            .client
            .invoke(&tl::functions::messages::GetForumTopics {
                peer: peer.into(),
                q: None,
                offset_date: 0,
                offset_id: 0,
                offset_topic: 0,
                limit: limit.min(i32::MAX as usize) as i32,
            })
            .await
            .context("failed to fetch telegram forum topics")?;

        let tl::enums::messages::ForumTopics::Topics(forum_topics) = forum_topics;
        let topic_messages = forum_topics.messages;

        Ok(forum_topics
            .topics
            .into_iter()
            .take(limit)
            .filter_map(|topic| map_forum_topic(parent_dialog_id, topic, &topic_messages))
            .collect())
    }

    pub async fn fetch_messages(&self, chat_id: &ChatId, limit: usize) -> Result<Vec<Message>> {
        let target = parse_chat_target(chat_id)?;
        let peer = self
            .find_peer_ref(chat_id)
            .await?
            .ok_or_else(|| anyhow!("telegram chat `{}` not found in current dialogs", chat_id.as_str()))?;
        match target {
            TelegramChatTarget::Dialog { .. } => self.fetch_dialog_messages(chat_id, peer, limit).await,
            TelegramChatTarget::ForumTopic { top_message, .. } => {
                info!("fetching telegram forum topic history for {}", chat_id.as_str());
                self.fetch_forum_topic_messages(chat_id, peer, top_message, limit)
                    .await
            }
        }
    }

    async fn find_peer_ref(&self, chat_id: &ChatId) -> Result<Option<PeerRef>> {
        let Some(dialog_id) = parse_dialog_id(chat_id.as_str()) else {
            return Ok(None);
        };

        if let Some(peer_ref) = self.peer_refs.read().await.get(&dialog_id).copied() {
            return Ok(Some(peer_ref));
        }

        Ok(self.peer_ref_from_dialog_id(dialog_id))
    }

    pub async fn send_text(&self, chat_id: &ChatId, text: &str) -> Result<Message> {
        let text = text.trim();
        if text.is_empty() {
            return Err(anyhow!("message text must not be empty"));
        }

        let target = parse_chat_target(chat_id)?;
        let peer = self
            .find_peer_ref(chat_id)
            .await?
            .ok_or_else(|| anyhow!("telegram chat `{}` not found in current dialogs", chat_id.as_str()))?;
        let sent = match target {
            TelegramChatTarget::Dialog { .. } => self
                .client
                .send_message(peer, text)
                .await
                .context("failed to send telegram text message")?,
            TelegramChatTarget::ForumTopic { top_message, .. } => {
                info!("sending telegram forum topic message to {}", chat_id.as_str());
                self.client
                    .send_message(peer, InputMessage::new().text(text).reply_to(Some(top_message)))
                    .await
                    .context("failed to send telegram forum topic message")?
            }
        };

        Ok(map_message(chat_id.clone(), sent))
    }

    async fn fetch_forum_topic_messages(
        &self,
        chat_id: &ChatId,
        peer: PeerRef,
        top_message: i32,
        limit: usize,
    ) -> Result<Vec<Message>> {
        let response = self
            .client
            .invoke(&tl::functions::messages::GetReplies {
                peer: peer.into(),
                msg_id: top_message,
                offset_id: 0,
                offset_date: 0,
                add_offset: 0,
                limit: limit.min(i32::MAX as usize) as i32,
                max_id: 0,
                min_id: 0,
                hash: 0,
            })
            .await
            .context("failed to fetch telegram forum topic replies")?;

        self.map_history_response(chat_id, response)
    }

    async fn fetch_dialog_messages(
        &self,
        chat_id: &ChatId,
        peer: PeerRef,
        limit: usize,
    ) -> Result<Vec<Message>> {
        let response = self
            .client
            .invoke(&tl::functions::messages::GetHistory {
                peer: peer.into(),
                offset_id: 0,
                offset_date: 0,
                add_offset: 0,
                limit: limit.min(i32::MAX as usize) as i32,
                max_id: 0,
                min_id: 0,
                hash: 0,
            })
            .await
            .context("failed to fetch telegram message history")?;

        self.map_history_response(chat_id, response)
    }

    fn map_history_response(
        &self,
        chat_id: &ChatId,
        response: tl::enums::messages::Messages,
    ) -> Result<Vec<Message>> {
        let (messages, users, chats) = unpack_messages_response(response)?;
        let author_names = build_author_names(users, chats);
        let mut messages = messages
            .into_iter()
            .filter(|message| !matches!(message, tl::enums::Message::Empty(_)))
            .collect::<Vec<_>>();
        messages.sort_by_key(raw_message_id);

        let oldest_message_id = messages.first().map(raw_message_id).unwrap_or_default();
        let newest_message_id = messages.last().map(raw_message_id).unwrap_or_default();
        info!(
            chat_id = %chat_id.as_str(),
            oldest_message_id,
            newest_message_id,
            message_count = messages.len(),
            "mapped telegram history slice"
        );

        let mapped = messages
            .into_iter()
            .map(|message| map_raw_message(chat_id.clone(), message, &author_names))
            .collect::<Vec<_>>();
        Ok(mapped)
    }

    fn peer_ref_from_dialog_id(&self, dialog_id: i64) -> Option<PeerRef> {
        let peer_id = peer_id_from_dialog_id(dialog_id)?;
        Some(PeerRef {
            id: peer_id,
            auth: grammers_session::types::PeerAuth::default(),
        })
    }
}

fn create_parent_dir(path: &Path) -> Result<()> {
    let Some(parent) = path.parent() else {
        return Ok(());
    };

    fs::create_dir_all(parent)
        .with_context(|| format!("failed to create telegram data directory `{}`", parent.display()))
}

impl Drop for TelegramClient {
    fn drop(&mut self) {
        self.runner_task.abort();
    }
}

fn user_label(user: &User) -> String {
    let full_name = user.full_name();
    if full_name.trim().is_empty() {
        "Telegram user".to_owned()
    } else {
        full_name
    }
}

fn map_dialog(dialog: Dialog) -> ChatSummary {
    let peer = dialog.peer();
    let title = peer
        .name()
        .map(str::to_owned)
        .unwrap_or_else(|| format!("Telegram {}", peer.id().bot_api_dialog_id()));
    let last_message_preview = dialog
        .last_message
        .as_ref()
        .map(|message| message.text().trim())
        .filter(|text| !text.is_empty())
        .map(str::to_owned);
    let last_activity_at = dialog
        .last_message
        .as_ref()
        .map(|message| timestamp_to_system_time(message.date().timestamp()));

    ChatSummary {
        id: dialog_chat_id(peer.id().bot_api_dialog_id()),
        service: Service::Telegram,
        title,
        last_message_preview,
        unread_count: dialog_unread_count(&dialog) as u32,
        last_activity_at,
        has_subchats: forum_enabled_group(peer),
    }
}

fn map_message(chat_id: ChatId, message: grammers_client::message::Message) -> Message {
    let author_name = message
        .sender()
        .and_then(peer_label)
        .or_else(|| message.peer().and_then(peer_label))
        .unwrap_or_else(|| {
            if message.outgoing() {
                "You".to_owned()
            } else {
                "Telegram".to_owned()
            }
        });

    Message {
        id: MessageId::new(format!("telegram:{}:{}", chat_id.as_str(), message.id())),
        chat_id,
        service: Service::Telegram,
        author_name,
        text: message.text().to_owned(),
        sent_at: timestamp_to_system_time(message.date().timestamp()),
        is_outgoing: message.outgoing(),
        delivery_state: MessageDeliveryState::Sent,
    }
}

fn map_forum_topic(
    parent_dialog_id: i64,
    topic: tl::enums::ForumTopic,
    messages: &[tl::enums::Message],
) -> Option<ChatSummary> {
    let tl::enums::ForumTopic::Topic(topic) = topic else {
        return None;
    };
    let topic_message = messages.iter().find(|message| message.id() == topic.top_message);
    let last_message_preview = topic_message
        .and_then(raw_message_text)
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .map(str::to_owned);
    let last_activity_at = topic_message
        .map(raw_message_timestamp)
        .map(|timestamp| timestamp_to_system_time(timestamp as i64))
        .or_else(|| Some(timestamp_to_system_time(topic.date as i64)));

    Some(ChatSummary {
        id: forum_topic_chat_id(parent_dialog_id, topic.id),
        service: Service::Telegram,
        title: topic.title,
        last_message_preview,
        unread_count: topic.unread_count.max(0) as u32,
        last_activity_at,
        has_subchats: false,
    })
}

fn map_raw_message(
    chat_id: ChatId,
    raw: tl::enums::Message,
    author_names: &std::collections::HashMap<String, String>,
) -> Message {
    let author_name = raw_message_sender_key(&raw)
        .and_then(|key| author_names.get(&key).cloned())
        .unwrap_or_else(|| {
            if raw_message_outgoing(&raw) {
                "You".to_owned()
            } else {
                "Telegram".to_owned()
            }
        });

    Message {
        id: MessageId::new(format!("telegram:{}:{}", chat_id.as_str(), raw.id())),
        chat_id,
        service: Service::Telegram,
        author_name,
        text: raw_message_text(&raw).unwrap_or_default().to_owned(),
        sent_at: timestamp_to_system_time(raw_message_timestamp(&raw) as i64),
        is_outgoing: raw_message_outgoing(&raw),
        delivery_state: MessageDeliveryState::Sent,
    }
}

fn build_author_names(
    users: Vec<tl::enums::User>,
    chats: Vec<tl::enums::Chat>,
) -> std::collections::HashMap<String, String> {
    let mut author_names = std::collections::HashMap::new();

    for user in users {
        let key = match &user {
            tl::enums::User::User(user) => format!("user:{}", user.id),
            tl::enums::User::Empty(user) => format!("user:{}", user.id),
        };
        let label = user_display_name(&user).unwrap_or_else(|| key.clone());
        author_names.insert(key, label);
    }

    for chat in chats {
        match &chat {
            tl::enums::Chat::Chat(chat) => {
                author_names.insert(format!("chat:{}", chat.id), chat.title.clone());
            }
            tl::enums::Chat::Forbidden(chat) => {
                author_names.insert(format!("chat:{}", chat.id), chat.title.clone());
            }
            tl::enums::Chat::Channel(channel) => {
                author_names.insert(format!("channel:{}", channel.id), channel.title.clone());
            }
            tl::enums::Chat::ChannelForbidden(channel) => {
                author_names.insert(format!("channel:{}", channel.id), channel.title.clone());
            }
            tl::enums::Chat::Empty(chat) => {
                author_names.insert(format!("chat:{}", chat.id), format!("Chat {}", chat.id));
            }
        }
    }

    author_names
}

fn unpack_messages_response(
    response: tl::enums::messages::Messages,
) -> Result<(Vec<tl::enums::Message>, Vec<tl::enums::User>, Vec<tl::enums::Chat>)> {
    let unpacked = match response {
        tl::enums::messages::Messages::Messages(messages) => {
            (messages.messages, messages.users, messages.chats)
        }
        tl::enums::messages::Messages::Slice(messages) => {
            (messages.messages, messages.users, messages.chats)
        }
        tl::enums::messages::Messages::ChannelMessages(messages) => {
            (messages.messages, messages.users, messages.chats)
        }
        tl::enums::messages::Messages::NotModified(_) => {
            return Err(anyhow!("telegram returned an unexpected not-modified message slice"));
        }
    };

    Ok(unpacked)
}

fn raw_message_sender_key(raw: &tl::enums::Message) -> Option<String> {
    match raw {
        tl::enums::Message::Message(message) => message
            .from_id
            .as_ref()
            .map(peer_key)
            .or_else(|| Some(peer_key(&message.peer_id))),
        tl::enums::Message::Service(message) => message
            .from_id
            .as_ref()
            .map(peer_key)
            .or_else(|| Some(peer_key(&message.peer_id))),
        tl::enums::Message::Empty(_) => None,
    }
}

fn raw_message_text(raw: &tl::enums::Message) -> Option<&str> {
    match raw {
        tl::enums::Message::Message(message) => Some(message.message.as_str()),
        tl::enums::Message::Service(_) | tl::enums::Message::Empty(_) => None,
    }
}

fn raw_message_id(raw: &tl::enums::Message) -> i32 {
    match raw {
        tl::enums::Message::Message(message) => message.id,
        tl::enums::Message::Service(message) => message.id,
        tl::enums::Message::Empty(message) => message.id,
    }
}

fn raw_message_timestamp(raw: &tl::enums::Message) -> i32 {
    match raw {
        tl::enums::Message::Message(message) => message.date,
        tl::enums::Message::Service(message) => message.date,
        tl::enums::Message::Empty(_) => 0,
    }
}

fn raw_message_outgoing(raw: &tl::enums::Message) -> bool {
    match raw {
        tl::enums::Message::Message(message) => message.out,
        tl::enums::Message::Service(message) => message.out,
        tl::enums::Message::Empty(_) => false,
    }
}

fn user_display_name(user: &tl::enums::User) -> Option<String> {
    match user {
        tl::enums::User::User(user) => {
            let first = user.first_name.as_deref().unwrap_or_default().trim();
            let last = user.last_name.as_deref().unwrap_or_default().trim();
            let joined = format!("{first} {last}").trim().to_owned();
            if !joined.is_empty() {
                Some(joined)
            } else {
                user.username.clone()
            }
        }
        tl::enums::User::Empty(_) => None,
    }
}

fn forum_enabled_group(peer: &Peer) -> bool {
    match peer {
        Peer::Group(group) => matches!(&group.raw, tl::enums::Chat::Channel(channel) if channel.forum),
        Peer::User(_) | Peer::Channel(_) => false,
    }
}

fn peer_key(peer: &tl::enums::Peer) -> String {
    match peer {
        tl::enums::Peer::User(user) => format!("user:{}", user.user_id),
        tl::enums::Peer::Chat(chat) => format!("chat:{}", chat.chat_id),
        tl::enums::Peer::Channel(channel) => format!("channel:{}", channel.channel_id),
    }
}

fn dialog_chat_id(dialog_id: i64) -> ChatId {
    ChatId::new(format!("telegram:{dialog_id}"))
}

fn forum_topic_chat_id(dialog_id: i64, top_message: i32) -> ChatId {
    ChatId::new(format!("telegram:{dialog_id}:topic:{top_message}"))
}

fn peer_id_from_dialog_id(dialog_id: i64) -> Option<PeerId> {
    if dialog_id == 0 {
        return None;
    }

    Some(match dialog_id {
        1..=0xffffffffff => PeerId::user_unchecked(dialog_id),
        -999999999999..=-1 => PeerId::chat_unchecked(-dialog_id),
        _ => {
            let bare_id = -dialog_id - 1_000_000_000_000;
            if bare_id <= 0 {
                return None;
            }
            PeerId::channel_unchecked(bare_id)
        }
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TelegramChatTarget {
    Dialog { dialog_id: i64 },
    ForumTopic { dialog_id: i64, top_message: i32 },
}

fn parse_chat_target(chat_id: &ChatId) -> Result<TelegramChatTarget> {
    let raw = chat_id.as_str();
    let Some(rest) = raw.strip_prefix("telegram:") else {
        return Err(anyhow!("unsupported telegram chat id `{raw}`"));
    };

    if let Some((dialog_id, top_message)) = rest.split_once(":topic:") {
        return Ok(TelegramChatTarget::ForumTopic {
            dialog_id: dialog_id
                .parse()
                .with_context(|| format!("invalid telegram dialog id in `{raw}`"))?,
            top_message: top_message
                .parse()
                .with_context(|| format!("invalid telegram topic id in `{raw}`"))?,
        });
    }

    Ok(TelegramChatTarget::Dialog {
        dialog_id: rest
            .parse()
            .with_context(|| format!("invalid telegram dialog id in `{raw}`"))?,
    })
}

fn peer_label(peer: &Peer) -> Option<String> {
    peer.name()
        .map(str::to_owned)
        .or_else(|| Some(peer.id().bot_api_dialog_id().to_string()))
}

fn dialog_unread_count(dialog: &Dialog) -> i32 {
    match &dialog.raw {
        grammers_client::tl::enums::Dialog::Dialog(raw) => raw.unread_count,
        grammers_client::tl::enums::Dialog::Folder(_) => 0,
    }
}

fn timestamp_to_system_time(timestamp: i64) -> SystemTime {
    if timestamp <= 0 {
        UNIX_EPOCH
    } else {
        UNIX_EPOCH + Duration::from_secs(timestamp as u64)
    }
}

fn parse_dialog_id(chat_id: &str) -> Option<i64> {
    let target = parse_chat_target(&ChatId::new(chat_id)).ok()?;
    Some(match target {
        TelegramChatTarget::Dialog { dialog_id } => dialog_id,
        TelegramChatTarget::ForumTopic { dialog_id, .. } => dialog_id,
    })
}
