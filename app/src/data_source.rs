use anyhow::Result;
use pandere_core::{ChatSummary, Message, Service};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AuthStatus {
    Authenticated(String),
    NeedsLogin,
    Unavailable(String),
}

impl AuthStatus {
    pub fn label(&self) -> String {
        match self {
            Self::Authenticated(label) => format!("signed in: {label}"),
            Self::NeedsLogin => "needs login".into(),
            Self::Unavailable(message) => format!("unavailable: {message}"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SyncStatus {
    Idle,
    Pending,
    Failed(String),
}

impl SyncStatus {
    pub fn label(&self) -> String {
        match self {
            Self::Idle => "idle".into(),
            Self::Pending => "pending".into(),
            Self::Failed(message) => format!("failed: {message}"),
        }
    }
}

#[derive(Debug, Clone)]
pub struct MessengerSnapshot {
    pub service: Service,
    pub auth_status: AuthStatus,
    pub sync_status: SyncStatus,
    pub chats: Vec<ChatSummary>,
    pub messages: Vec<Message>,
}

pub trait MessengerDataSource {
    fn snapshot(&self) -> Result<MessengerSnapshot>;
}
