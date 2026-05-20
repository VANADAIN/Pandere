use std::collections::HashMap;

use pandere_core::{ChatId, ChatSummary, Message, MessageDeliveryState, MessageId};

use super::AppState;

impl AppState {
    pub fn apply_cached_thread(&mut self) -> bool {
        let Some(chat_id) = self.preview_chat_id() else {
            self.source.messages.clear();
            return false;
        };

        let Some(messages) = self.thread_cache.get(&chat_id).cloned() else {
            return false;
        };

        self.source.messages = messages;
        self.thread_status = super::ThreadStatus::Idle;
        self.reset_thread_scroll();
        true
    }

    pub fn cache_thread(&mut self, chat_id: ChatId, messages: Vec<Message>) {
        self.thread_cache.insert(chat_id.clone(), messages.clone());
        self.refresh_chat_preview_from_messages(&chat_id);
        if self.preview_chat_id().as_ref() == Some(&chat_id) {
            self.source.messages = messages;
            self.thread_status = super::ThreadStatus::Idle;
            self.reset_thread_scroll();
        }
    }

    pub fn apply_cached_forum_threads(&mut self) -> bool {
        let Some(root_chat_id) = self.selected_root_chat_id.as_ref() else {
            return false;
        };
        if self.forum_threads.contains_key(root_chat_id) {
            self.forum_threads_status = super::ThreadStatus::Idle;
            self.sync_selected_thread_to_root();
            return true;
        }
        false
    }

    pub fn cache_forum_threads(&mut self, root_chat_id: ChatId, threads: Vec<ChatSummary>) {
        self.forum_threads.insert(root_chat_id.clone(), threads);
        if self.selected_root_chat_id.as_ref() == Some(&root_chat_id) {
            self.forum_threads_status = super::ThreadStatus::Idle;
            self.sync_selected_thread_to_root();
        }
    }

    pub fn set_forum_threads_loading(&mut self) {
        self.forum_threads_status = super::ThreadStatus::Loading;
    }

    pub fn set_forum_threads_failed(&mut self, message: impl Into<String>) {
        self.forum_threads_status = super::ThreadStatus::Failed(message.into());
    }

    pub fn set_thread_loading(&mut self) {
        self.source.messages.clear();
        self.thread_status = super::ThreadStatus::Loading;
        self.reset_thread_scroll();
    }

    pub fn set_thread_failed(&mut self, message: impl Into<String>) {
        self.source.messages.clear();
        self.thread_status = super::ThreadStatus::Failed(message.into());
        self.reset_thread_scroll();
    }

    pub fn merge_message(
        &mut self,
        chat_id: &ChatId,
        message: Message,
        replacing_message_id: Option<&MessageId>,
    ) {
        {
            let thread = self.thread_cache.entry(chat_id.clone()).or_default();
            if let Some(message_id) = replacing_message_id {
                if let Some(existing) = thread.iter_mut().find(|item| &item.id == message_id) {
                    *existing = message.clone();
                } else {
                    thread.push(message.clone());
                }
            } else if let Some(existing) = thread.iter_mut().find(|item| item.id == message.id) {
                *existing = message.clone();
            } else {
                thread.push(message.clone());
            }

            sort_messages(thread);
        }

        let is_preview = self.preview_chat_id().as_ref() == Some(chat_id);
        self.refresh_chat_preview_from_messages(chat_id);

        if is_preview {
            if let Some(thread) = self.thread_cache.get(chat_id).cloned() {
                self.source.messages = thread;
            }
            self.thread_status = super::ThreadStatus::Idle;
            if self.follow_thread_bottom {
                self.reset_thread_scroll();
            }
        }
    }

    pub fn mark_message_delivery(
        &mut self,
        chat_id: &ChatId,
        message_id: &MessageId,
        delivery_state: MessageDeliveryState,
    ) {
        let is_preview = self.preview_chat_id().as_ref() == Some(chat_id);
        if let Some(thread) = self.thread_cache.get_mut(chat_id) {
            if let Some(message) = thread.iter_mut().find(|item| &item.id == message_id) {
                message.delivery_state = delivery_state;
            }
            if is_preview {
                self.source.messages = thread.clone();
            }
        }
    }

    pub fn thread_messages(&self, chat_id: &ChatId) -> Option<Vec<Message>> {
        self.thread_cache.get(chat_id).cloned()
    }

    fn refresh_chat_preview_from_messages(&mut self, chat_id: &ChatId) {
        let preview = self.thread_cache.get(chat_id).and_then(|messages| {
            messages
                .last()
                .map(|message| (message.text.clone(), Some(message.sent_at)))
        });

        let Some((text, sent_at)) = preview else {
            return;
        };

        if let Some(chat) = self
            .source
            .chats
            .iter_mut()
            .find(|chat| &chat.id == chat_id)
        {
            chat.last_message_preview = Some(text.clone());
            chat.last_activity_at = sent_at;
        }

        for threads in self.forum_threads.values_mut() {
            if let Some(chat) = threads.iter_mut().find(|chat| &chat.id == chat_id) {
                chat.last_message_preview = Some(text.clone());
                chat.last_activity_at = sent_at;
            }
        }
    }
}

pub(crate) fn build_thread_cache(messages: &[Message]) -> HashMap<ChatId, Vec<Message>> {
    let mut cache: HashMap<ChatId, Vec<Message>> = HashMap::new();
    for message in messages {
        cache
            .entry(message.chat_id.clone())
            .or_default()
            .push(message.clone());
    }
    for messages in cache.values_mut() {
        sort_messages(messages);
    }
    cache
}

fn sort_messages(messages: &mut [Message]) {
    messages.sort_by_key(|message| (message.sent_at, message.id.as_str().to_owned()));
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, SystemTime};

    use pandere_core::Service;

    use super::*;
    use crate::{fixtures::FixtureMessengerSource, plugin::PluginRegistry};

    fn test_state() -> AppState {
        AppState::new(FixtureMessengerSource, PluginRegistry::new(vec![]))
            .expect("state should initialize")
    }

    #[test]
    fn merge_message_replaces_optimistic_message() {
        let mut state = test_state();
        let chat_id = state
            .selected_root_chat_id
            .clone()
            .expect("fixture chat should exist");
        let optimistic_id = MessageId::new("local:telegram:team:1");

        let optimistic = Message {
            id: optimistic_id.clone(),
            chat_id: chat_id.clone(),
            service: Service::Telegram,
            author_name: "You".into(),
            text: "draft".into(),
            sent_at: SystemTime::now(),
            is_outgoing: true,
            delivery_state: MessageDeliveryState::Sending,
        };
        state.merge_message(&chat_id, optimistic, None);

        let sent = Message {
            id: MessageId::new("telegram:team:100"),
            chat_id: chat_id.clone(),
            service: Service::Telegram,
            author_name: "You".into(),
            text: "draft".into(),
            sent_at: SystemTime::now() + Duration::from_secs(1),
            is_outgoing: true,
            delivery_state: MessageDeliveryState::Sent,
        };
        state.merge_message(&chat_id, sent.clone(), Some(&optimistic_id));

        let messages = state
            .thread_messages(&chat_id)
            .expect("thread should exist after merge");
        assert!(messages.iter().any(|message| message.id == sent.id));
        assert!(!messages.iter().any(|message| message.id == optimistic_id));
    }

    #[test]
    fn mark_message_delivery_updates_existing_message() {
        let mut state = test_state();
        let chat_id = state
            .selected_root_chat_id
            .clone()
            .expect("fixture chat should exist");
        let message_id = MessageId::new("local:telegram:team:2");
        let message = Message {
            id: message_id.clone(),
            chat_id: chat_id.clone(),
            service: Service::Telegram,
            author_name: "You".into(),
            text: "retry me".into(),
            sent_at: SystemTime::now(),
            is_outgoing: true,
            delivery_state: MessageDeliveryState::Sending,
        };
        state.merge_message(&chat_id, message, None);

        state.mark_message_delivery(&chat_id, &message_id, MessageDeliveryState::Failed);

        let messages = state
            .thread_messages(&chat_id)
            .expect("thread should exist after merge");
        let message = messages
            .iter()
            .find(|message| message.id == message_id)
            .expect("message should remain in thread");
        assert_eq!(message.delivery_state, MessageDeliveryState::Failed);
    }

    #[test]
    fn set_chats_preserves_selection_when_chat_still_exists() {
        let mut state = test_state();
        let selected_chat_id = state
            .selected_root_chat_id
            .clone()
            .expect("fixture chat should exist");

        let mut chats = state.source.chats.clone();
        chats.reverse();
        state.set_chats(chats);

        assert_eq!(state.selected_root_chat_id, Some(selected_chat_id));
    }
}
