use anyhow::{Context, Result, anyhow};
use grammers_client::message::InputMessage;
use grammers_client::tl;
use grammers_session::types::{PeerAuth, PeerRef};
use pandere_core::{ChatId, ChatSummary, Message};
use tracing::info;

use crate::TelegramFetchClient;
use crate::ids::{TelegramChatTarget, parse_chat_target, parse_dialog_id, peer_id_from_dialog_id};
use crate::mapping::{
    build_author_names, map_dialog, map_forum_topic, map_message, map_raw_message, raw_message_id,
    unpack_messages_response,
};

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

    pub async fn list_forum_topics(
        &self,
        chat_id: &ChatId,
        limit: usize,
    ) -> Result<Vec<ChatSummary>> {
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
        let peer = self.find_peer_ref(chat_id).await?.ok_or_else(|| {
            anyhow!(
                "telegram chat `{}` not found in current dialogs",
                chat_id.as_str()
            )
        })?;
        match target {
            TelegramChatTarget::Dialog { .. } => {
                self.fetch_dialog_messages(chat_id, peer, limit).await
            }
            TelegramChatTarget::ForumTopic { top_message, .. } => {
                info!(
                    "fetching telegram forum topic history for {}",
                    chat_id.as_str()
                );
                self.fetch_forum_topic_messages(chat_id, peer, top_message, limit)
                    .await
            }
        }
    }

    pub async fn send_text(&self, chat_id: &ChatId, text: &str) -> Result<Message> {
        let text = text.trim();
        if text.is_empty() {
            return Err(anyhow!("message text must not be empty"));
        }

        let target = parse_chat_target(chat_id)?;
        let peer = self.find_peer_ref(chat_id).await?.ok_or_else(|| {
            anyhow!(
                "telegram chat `{}` not found in current dialogs",
                chat_id.as_str()
            )
        })?;
        let sent = match target {
            TelegramChatTarget::Dialog { .. } => self
                .client
                .send_message(peer, text)
                .await
                .context("failed to send telegram text message")?,
            TelegramChatTarget::ForumTopic { top_message, .. } => {
                info!(
                    "sending telegram forum topic message to {}",
                    chat_id.as_str()
                );
                self.client
                    .send_message(
                        peer,
                        InputMessage::new().text(text).reply_to(Some(top_message)),
                    )
                    .await
                    .context("failed to send telegram forum topic message")?
            }
        };

        Ok(map_message(chat_id.clone(), sent))
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
            auth: PeerAuth::default(),
        })
    }
}
