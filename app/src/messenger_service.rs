use std::sync::Arc;
use std::time::SystemTime;

use anyhow::Result;
use async_trait::async_trait;
use pandere_core::{ChatId, ChatSummary, Message, Service};
use pandere_plugin_slack::{SlackOAuthConfig, oauth_authorize_url};
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
    pub session_field_label: Option<String>,
    pub session_value: Option<String>,
    pub has_saved_session: bool,
    pub password_hint: Option<String>,
    pub account_label: Option<String>,
    pub help_lines: Vec<String>,
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
        session_field_label: Some("Session file".into()),
        session_value: Some(state.session_path.display().to_string()),
        has_saved_session: state.has_saved_session,
        password_hint: state.password_hint,
        account_label: state.user_label,
        help_lines: vec![
            "enter advances current login step".into(),
            "r request or refresh code".into(),
            "x logout and clear saved session".into(),
            "backspace edit input".into(),
            "esc clear input or notice".into(),
        ],
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

#[derive(Clone)]
struct SlackFetchHandle {
    chats: Vec<ChatSummary>,
    messages: Arc<std::collections::HashMap<ChatId, Vec<Message>>>,
}

#[async_trait]
impl MessengerFetchHandle for SlackFetchHandle {
    async fn list_chats(&self, limit: usize) -> Result<Vec<ChatSummary>> {
        Ok(self.chats.iter().take(limit).cloned().collect())
    }

    async fn list_subchats(&self, _chat_id: &ChatId, _limit: usize) -> Result<Vec<ChatSummary>> {
        Ok(Vec::new())
    }

    async fn fetch_messages(&self, chat_id: &ChatId, limit: usize) -> Result<Vec<Message>> {
        Ok(self
            .messages
            .get(chat_id)
            .into_iter()
            .flat_map(|messages| messages.iter().take(limit).cloned())
            .collect())
    }

    async fn send_text(&self, _chat_id: &ChatId, _text: &str) -> Result<Message> {
        Err(anyhow::anyhow!(
            "slack send is not implemented yet; install and history are the next step"
        ))
    }
}

pub struct SlackService {
    config: SlackOAuthConfig,
    authorize_url: String,
    fetch: Arc<SlackFetchHandle>,
}

impl SlackService {
    pub fn connect(config: SlackOAuthConfig) -> Result<Self> {
        let authorize_url = oauth_authorize_url(&config, "pandere-slack-install")?;
        let chats = sample_slack_chats();
        let messages = sample_slack_messages(&chats);
        Ok(Self {
            config,
            authorize_url,
            fetch: Arc::new(SlackFetchHandle { chats, messages }),
        })
    }

    fn login_state(&self) -> MessengerLoginState {
        MessengerLoginState {
            service: Service::Slack,
            phase: LoginPhase::Connected,
            input_mode: None,
            identifier_label: Some("Install URL".into()),
            identifier_value: Some(self.authorize_url.clone()),
            session_field_label: Some("Redirect URI".into()),
            session_value: Some(self.config.redirect_uri.clone()),
            has_saved_session: false,
            password_hint: None,
            account_label: None,
            help_lines: vec![
                "Slack uses OAuth install, not code entry in the TUI.".into(),
                "Open the install URL in a browser to authorize the app.".into(),
                format!(
                    "Configured bot scopes: {}",
                    self.config.bot_scopes.join(", ")
                ),
                "OAuth callback exchange and token persistence are the next Slack steps.".into(),
            ],
        }
    }
}

#[async_trait]
impl MessengerService for SlackService {
    fn service(&self) -> Service {
        Service::Slack
    }

    fn fetch_handle(&self) -> Arc<dyn MessengerFetchHandle> {
        self.fetch.clone()
    }

    async fn bootstrap_login(&mut self) -> Result<MessengerLoginState> {
        Ok(self.login_state())
    }

    async fn auth_status(&self) -> Result<MessengerAuthStatus> {
        Ok(MessengerAuthStatus::NeedsLogin)
    }

    async fn request_login_code(&mut self) -> Result<MessengerLoginState> {
        Ok(self.login_state())
    }

    async fn submit_login_input(
        &mut self,
        _input_mode: LoginInputMode,
        _input: &str,
    ) -> Result<MessengerLoginState> {
        Ok(self.login_state())
    }

    fn set_login_identifier(&mut self, _value: String) -> Result<()> {
        Ok(())
    }

    async fn list_chats(&self, limit: usize) -> Result<Vec<ChatSummary>> {
        self.fetch.list_chats(limit).await
    }

    async fn clear_session_and_reconnect(&mut self) -> Result<()> {
        Ok(())
    }
}

fn sample_slack_chats() -> Vec<ChatSummary> {
    vec![
        ChatSummary {
            id: ChatId::new("slack:C-general"),
            service: Service::Slack,
            title: "#general".into(),
            last_message_preview: Some("Finish OAuth callback helper next.".into()),
            unread_count: 0,
            last_activity_at: Some(SystemTime::now()),
            has_subchats: false,
        },
        ChatSummary {
            id: ChatId::new("slack:C-eng"),
            service: Service::Slack,
            title: "#engineering".into(),
            last_message_preview: Some("Service boundary is ready for Slack.".into()),
            unread_count: 2,
            last_activity_at: Some(SystemTime::now()),
            has_subchats: false,
        },
    ]
}

fn sample_slack_messages(
    chats: &[ChatSummary],
) -> Arc<std::collections::HashMap<ChatId, Vec<Message>>> {
    let mut messages = std::collections::HashMap::new();
    if let Some(first_chat) = chats.first() {
        messages.insert(
            first_chat.id.clone(),
            vec![
                Message {
                    id: pandere_core::MessageId::new("slack:C-general:1"),
                    chat_id: first_chat.id.clone(),
                    service: Service::Slack,
                    author_name: "Morgan".into(),
                    text: "Need the install link in the UI.".into(),
                    sent_at: SystemTime::now(),
                    is_outgoing: false,
                    delivery_state: pandere_core::MessageDeliveryState::Sent,
                },
                Message {
                    id: pandere_core::MessageId::new("slack:C-general:2"),
                    chat_id: first_chat.id.clone(),
                    service: Service::Slack,
                    author_name: "You".into(),
                    text: "Done. OAuth exchange is next.".into(),
                    sent_at: SystemTime::now(),
                    is_outgoing: true,
                    delivery_state: pandere_core::MessageDeliveryState::Sent,
                },
            ],
        );
    }
    Arc::new(messages)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn slack_service_exposes_install_metadata() {
        let config = SlackOAuthConfig {
            client_id: "123".into(),
            client_secret: "secret".into(),
            redirect_uri: "https://example.com/slack/callback".into(),
            bot_scopes: pandere_plugin_slack::default_bot_scopes()
                .iter()
                .map(|s| (*s).into())
                .collect(),
            user_scopes: vec![],
        };

        let mut service = SlackService::connect(config).expect("slack service should initialize");
        let login = service
            .bootstrap_login()
            .await
            .expect("bootstrap login should succeed");

        assert_eq!(login.service, Service::Slack);
        assert_eq!(login.identifier_label.as_deref(), Some("Install URL"));
        assert!(login
            .identifier_value
            .as_deref()
            .unwrap_or_default()
            .contains("slack.com/oauth/v2/authorize"));
        assert!(!login.help_lines.is_empty());
    }
}
