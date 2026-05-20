use std::sync::Arc;

use anyhow::Result;
use async_trait::async_trait;
use pandere_core::{ChatId, ChatSummary, Message, Service};
use pandere_plugin_telegram::{
    AuthStatus as TelegramAuthStatus, LoginPhase as TelegramLoginPhase,
    LoginState as TelegramLoginState, TelegramClient, TelegramConfig, TelegramFetchClient,
    clear_session_file,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoginInputMode {
    Identifier,
    Code,
    Password,
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
pub enum MessengerAuthStatus {
    Connected,
    NeedsLogin,
    Authorized { account_label: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MessengerLoginState {
    pub service: Service,
    pub phase: LoginPhase,
    pub input_mode: Option<LoginInputMode>,
    pub identifier_label: Option<String>,
    pub identifier_value: Option<String>,
    pub session_label: Option<String>,
    pub has_saved_session: bool,
    pub password_hint: Option<String>,
    pub account_label: Option<String>,
}

#[async_trait]
pub trait MessengerFetchHandle: Send + Sync {
    async fn list_chats(&self, limit: usize) -> Result<Vec<ChatSummary>>;
    async fn list_subchats(&self, chat_id: &ChatId, limit: usize) -> Result<Vec<ChatSummary>>;
    async fn fetch_messages(&self, chat_id: &ChatId, limit: usize) -> Result<Vec<Message>>;
    async fn send_text(&self, chat_id: &ChatId, text: &str) -> Result<Message>;

    async fn sync_tick(&self, limit: usize) -> Result<Vec<ChatSummary>> {
        self.list_chats(limit).await
    }
}

#[async_trait]
pub trait MessengerService: Send {
    fn service(&self) -> Service;
    fn fetch_handle(&self) -> Arc<dyn MessengerFetchHandle>;
    async fn bootstrap_login(&mut self) -> Result<MessengerLoginState>;
    async fn auth_status(&self) -> Result<MessengerAuthStatus>;
    async fn request_login_code(&mut self) -> Result<MessengerLoginState>;
    async fn submit_login_input(
        &mut self,
        input_mode: LoginInputMode,
        input: &str,
    ) -> Result<MessengerLoginState>;
    fn set_login_identifier(&mut self, value: String) -> Result<()>;
    async fn list_chats(&self, limit: usize) -> Result<Vec<ChatSummary>>;
    async fn clear_session_and_reconnect(&mut self) -> Result<()>;
}

#[derive(Clone)]
struct TelegramFetchHandle {
    client: TelegramFetchClient,
}

#[async_trait]
impl MessengerFetchHandle for TelegramFetchHandle {
    async fn list_chats(&self, limit: usize) -> Result<Vec<ChatSummary>> {
        self.client.list_chats(limit).await
    }

    async fn list_subchats(&self, chat_id: &ChatId, limit: usize) -> Result<Vec<ChatSummary>> {
        self.client.list_forum_topics(chat_id, limit).await
    }

    async fn fetch_messages(&self, chat_id: &ChatId, limit: usize) -> Result<Vec<Message>> {
        self.client.fetch_messages(chat_id, limit).await
    }

    async fn send_text(&self, chat_id: &ChatId, text: &str) -> Result<Message> {
        self.client.send_text(chat_id, text).await
    }
}

pub struct TelegramService {
    config: TelegramConfig,
    client: TelegramClient,
}

impl TelegramService {
    pub async fn connect(config: TelegramConfig) -> Result<Self> {
        let client = TelegramClient::connect(config.clone()).await?;
        Ok(Self { config, client })
    }
}

#[async_trait]
impl MessengerService for TelegramService {
    fn service(&self) -> Service {
        Service::Telegram
    }

    fn fetch_handle(&self) -> Arc<dyn MessengerFetchHandle> {
        Arc::new(TelegramFetchHandle {
            client: self.client.fetch_client(),
        })
    }

    async fn bootstrap_login(&mut self) -> Result<MessengerLoginState> {
        let state = self.client.bootstrap_login().await?;
        Ok(map_telegram_login_state(state))
    }

    async fn auth_status(&self) -> Result<MessengerAuthStatus> {
        let status = self.client.auth_status().await?;
        Ok(map_telegram_auth_status(status))
    }

    async fn request_login_code(&mut self) -> Result<MessengerLoginState> {
        let state = self.client.request_login_code_state().await?;
        Ok(map_telegram_login_state(state))
    }

    async fn submit_login_input(
        &mut self,
        input_mode: LoginInputMode,
        input: &str,
    ) -> Result<MessengerLoginState> {
        let state = match input_mode {
            LoginInputMode::Identifier => {
                self.client.set_phone_number(input.to_owned())?;
                self.client.request_login_code_state().await?
            }
            LoginInputMode::Code => self.client.submit_login_code_state(input).await?,
            LoginInputMode::Password => self.client.submit_password_state(input).await?,
        };

        Ok(map_telegram_login_state(state))
    }

    fn set_login_identifier(&mut self, value: String) -> Result<()> {
        self.client.set_phone_number(value)
    }

    async fn list_chats(&self, limit: usize) -> Result<Vec<ChatSummary>> {
        self.client.list_chats(limit).await
    }

    async fn clear_session_and_reconnect(&mut self) -> Result<()> {
        clear_session_file(&self.config.session_path)?;
        self.client = TelegramClient::connect(self.config.clone()).await?;
        Ok(())
    }
}

fn map_telegram_auth_status(status: TelegramAuthStatus) -> MessengerAuthStatus {
    match status {
        TelegramAuthStatus::Connected => MessengerAuthStatus::Connected,
        TelegramAuthStatus::NeedsLogin => MessengerAuthStatus::NeedsLogin,
        TelegramAuthStatus::Authorized { user_label } => MessengerAuthStatus::Authorized {
            account_label: user_label,
        },
    }
}

fn map_telegram_login_state(state: TelegramLoginState) -> MessengerLoginState {
    MessengerLoginState {
        service: Service::Telegram,
        phase: map_telegram_login_phase(state.phase),
        input_mode: match state.phase {
            TelegramLoginPhase::Connected => Some(LoginInputMode::Identifier),
            TelegramLoginPhase::CodeRequested => Some(LoginInputMode::Code),
            TelegramLoginPhase::PasswordRequired => Some(LoginInputMode::Password),
            TelegramLoginPhase::Disconnected | TelegramLoginPhase::Authorized => None,
        },
        identifier_label: Some("Phone".into()),
        identifier_value: Some(state.phone_number),
        session_label: Some(state.session_path.display().to_string()),
        has_saved_session: state.has_saved_session,
        password_hint: state.password_hint,
        account_label: state.user_label,
    }
}

fn map_telegram_login_phase(phase: TelegramLoginPhase) -> LoginPhase {
    match phase {
        TelegramLoginPhase::Disconnected => LoginPhase::Disconnected,
        TelegramLoginPhase::Connected => LoginPhase::Connected,
        TelegramLoginPhase::CodeRequested => LoginPhase::CodeRequested,
        TelegramLoginPhase::PasswordRequired => LoginPhase::PasswordRequired,
        TelegramLoginPhase::Authorized => LoginPhase::Authorized,
    }
}
