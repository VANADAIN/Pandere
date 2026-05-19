use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, anyhow};
use grammers_client::client::{LoginToken, PasswordToken};
use grammers_client::peer::Dialog;
use grammers_client::peer::User;
use grammers_client::{Client, SignInError};
use grammers_mtsender::SenderPool;
use grammers_session::storages::SqliteSession;
use pandere_core::{ChatId, ChatSummary, Service};
use tokio::task::JoinHandle;

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
            .unwrap_or_else(|_| PathBuf::from("telegram.session"));

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

pub struct TelegramClient {
    service: Service,
    config: TelegramConfig,
    client: Client,
    runner_task: JoinHandle<()>,
    login_token: Option<LoginToken>,
    password_token: Option<PasswordToken>,
}

impl TelegramClient {
    pub async fn connect(config: TelegramConfig) -> Result<Self> {
        config.validate()?;

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

            chats.push(map_dialog(dialog));
        }

        Ok(chats)
    }
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
        id: ChatId::new(format!("telegram:{}", peer.id().bot_api_dialog_id())),
        service: Service::Telegram,
        title,
        last_message_preview,
        unread_count: dialog_unread_count(&dialog) as u32,
        last_activity_at,
    }
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
