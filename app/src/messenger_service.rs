use anyhow::Result;
use pandere_core::ChatSummary;
use pandere_plugin_telegram::{
    AuthStatus as TelegramAuthStatus, LoginState, TelegramClient, TelegramConfig, TelegramFetchClient,
    clear_session_file,
};

pub struct TelegramService {
    config: TelegramConfig,
    client: TelegramClient,
}

impl TelegramService {
    pub async fn connect(config: TelegramConfig) -> Result<Self> {
        let client = TelegramClient::connect(config.clone()).await?;
        Ok(Self { config, client })
    }

    pub fn fetch_client(&self) -> TelegramFetchClient {
        self.client.fetch_client()
    }

    pub async fn bootstrap_login(&mut self) -> Result<LoginState> {
        self.client.bootstrap_login().await
    }

    pub async fn auth_status(&self) -> Result<TelegramAuthStatus> {
        self.client.auth_status().await
    }

    pub async fn request_login_code_state(&mut self) -> Result<LoginState> {
        self.client.request_login_code_state().await
    }

    pub async fn submit_login_code_state(&mut self, code: &str) -> Result<LoginState> {
        self.client.submit_login_code_state(code).await
    }

    pub async fn submit_password_state(&mut self, password: &str) -> Result<LoginState> {
        self.client.submit_password_state(password).await
    }

    pub fn set_phone_number(&mut self, phone_number: impl Into<String>) -> Result<()> {
        self.client.set_phone_number(phone_number)
    }

    pub async fn list_chats(&self, limit: usize) -> Result<Vec<ChatSummary>> {
        self.client.list_chats(limit).await
    }

    pub async fn reconnect(&mut self) -> Result<()> {
        self.client = TelegramClient::connect(self.config.clone()).await?;
        Ok(())
    }

    pub async fn clear_session_and_reconnect(&mut self) -> Result<()> {
        clear_session_file(&self.config.session_path)?;
        self.reconnect().await
    }
}
