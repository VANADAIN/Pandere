use std::sync::Arc;

use anyhow::{Context, Result, anyhow};
use grammers_client::Client;
use grammers_mtsender::SenderPool;
use grammers_session::storages::MemorySession;
use pandere_core::Service;

pub fn crate_name() -> &'static str {
    "pandere-plugin-telegram"
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TelegramSpikeConfig {
    pub api_id: i32,
    pub api_hash: String,
    pub phone_number: String,
}

impl TelegramSpikeConfig {
    pub fn from_env() -> Result<Self> {
        let api_id = std::env::var("TELEGRAM_API_ID")
            .context("missing TELEGRAM_API_ID")?
            .parse()
            .context("invalid TELEGRAM_API_ID")?;
        let api_hash = std::env::var("TELEGRAM_API_HASH").context("missing TELEGRAM_API_HASH")?;
        let phone_number =
            std::env::var("TELEGRAM_PHONE").context("missing TELEGRAM_PHONE")?;

        Ok(Self {
            api_id,
            api_hash,
            phone_number,
        })
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

pub struct TelegramSpike {
    service: Service,
    config: TelegramSpikeConfig,
    sender_pool: SenderPool,
    client: Client,
}

impl TelegramSpike {
    pub fn new(config: TelegramSpikeConfig) -> Self {
        let session = Arc::new(MemorySession::default());
        let sender_pool = SenderPool::new(Arc::clone(&session), config.api_id);
        let client = Client::new(sender_pool.handle.clone());

        Self {
            service: Service::Telegram,
            config,
            sender_pool,
            client,
        }
    }

    pub fn service(&self) -> Service {
        self.service
    }

    pub fn api_hash(&self) -> &str {
        &self.config.api_hash
    }

    pub fn phone_number(&self) -> &str {
        &self.config.phone_number
    }

    pub fn client(&self) -> &Client {
        &self.client
    }

    pub fn pool(&self) -> &SenderPool {
        &self.sender_pool
    }

    pub fn login_plan(&self) -> [&'static str; 4] {
        [
            "Initialize a persistent session store instead of MemorySession.",
            "Call request_login_code(phone, api_hash) when the client is not authorized.",
            "Complete sign_in with code and optional password flow.",
            "Run dialogs fetch once so peer/update caches are warm before streaming updates.",
        ]
    }

    pub fn validate(&self) -> Result<()> {
        if self.config.api_hash.trim().is_empty() {
            return Err(anyhow!("telegram api hash must not be empty"));
        }

        if self.config.phone_number.trim().is_empty() {
            return Err(anyhow!("telegram phone number must not be empty"));
        }

        Ok(())
    }
}
