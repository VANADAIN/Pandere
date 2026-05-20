pub mod paths;

use std::time::{Duration, SystemTime};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Service {
    Telegram,
    Slack,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct AccountId(String);

impl AccountId {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ChatId(String);

impl ChatId {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct MessageId(String);

impl MessageId {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChatSummary {
    pub id: ChatId,
    pub service: Service,
    pub title: String,
    pub last_message_preview: Option<String>,
    pub unread_count: u32,
    pub last_activity_at: Option<SystemTime>,
    pub has_subchats: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MessageDeliveryState {
    Sent,
    Sending,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Message {
    pub id: MessageId,
    pub chat_id: ChatId,
    pub service: Service,
    pub author_name: String,
    pub text: String,
    pub sent_at: SystemTime,
    pub is_outgoing: bool,
    pub delivery_state: MessageDeliveryState,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyncCursor {
    pub account_id: AccountId,
    pub service: Service,
    pub position: String,
    pub updated_at: SystemTime,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RetentionPolicy {
    pub message_ttl: Option<Duration>,
    pub media_ttl: Option<Duration>,
    pub max_database_bytes: u64,
    pub max_media_bytes: u64,
    pub persist_cache: bool,
}
