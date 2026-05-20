use std::time::Instant;

use anyhow::Result;
use pandere_core::{ChatId, ChatSummary, Message, MessageId};

pub(super) struct PendingThreadFetch {
    pub chat_id: ChatId,
    pub requested_at: Instant,
}

pub(super) struct PendingForumThreadsFetch {
    pub root_chat_id: ChatId,
    pub requested_at: Instant,
}

pub(super) struct ThreadFetchResult {
    pub chat_id: ChatId,
    pub result: Result<Vec<Message>>,
}

pub(super) struct ForumThreadsFetchResult {
    pub root_chat_id: ChatId,
    pub result: Result<Vec<ChatSummary>>,
}

pub(super) struct DialogsSyncResult {
    pub result: Result<Vec<ChatSummary>>,
}

pub(super) struct SendMessageResult {
    pub chat_id: ChatId,
    pub optimistic_message_id: MessageId,
    pub result: Result<Message>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum LeftPaneDirection {
    Previous,
    Next,
}
