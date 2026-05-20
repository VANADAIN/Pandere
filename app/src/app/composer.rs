use std::{sync::Arc, time::SystemTime};

use anyhow::Result;
use pandere_core::{ChatId, Message, MessageDeliveryState, MessageId};
use tracing::info;

use super::App;
use crate::messenger_service::MessengerFetchHandle;

impl App {
    pub(super) fn process_send_results(&mut self) {
        while let Ok(result) = self.send_rx.try_recv() {
            match result.result {
                Ok(mut message) => {
                    info!(chat_id = %result.chat_id.as_str(), "sent text message");
                    message.delivery_state = MessageDeliveryState::Sent;
                    self.state.merge_message(
                        &result.chat_id,
                        message,
                        Some(&result.optimistic_message_id),
                    );
                    if let Some(messages) = self.state.thread_messages(&result.chat_id) {
                        let _ = self.cache.save_messages(&result.chat_id, &messages);
                    }
                    self.in_flight_threads.remove(&result.chat_id);
                    self.schedule_force_thread_refresh(result.chat_id);
                }
                Err(error) => {
                    info!(chat_id = %result.chat_id.as_str(), error = %error, "failed to send text message");
                    self.state.mark_message_delivery(
                        &result.chat_id,
                        &result.optimistic_message_id,
                        MessageDeliveryState::Failed,
                    );
                    self.state.set_composer_notice(error.to_string());
                }
            }
        }
    }

    pub(super) fn send_composer_text(&mut self) -> Result<()> {
        let Some(fetch_handle) =
            self.messenger_fetch_handle_or_composer_notice("messenger client is unavailable")
        else {
            return Ok(());
        };

        let Some(chat_id) = self.state.active_chat_id() else {
            self.state.set_composer_notice("no chat selected");
            return Ok(());
        };

        let text = self.state.composer_input.trim().to_owned();
        if text.is_empty() {
            self.state.set_composer_notice("message is empty");
            return Ok(());
        }

        let optimistic_message_id = self.next_optimistic_message_id(&chat_id);
        let optimistic_message = Message {
            id: optimistic_message_id.clone(),
            chat_id: chat_id.clone(),
            service: self.primary_service(),
            author_name: "You".into(),
            text: text.clone(),
            sent_at: SystemTime::now(),
            is_outgoing: true,
            delivery_state: MessageDeliveryState::Sending,
        };
        self.state.merge_message(&chat_id, optimistic_message, None);
        self.state.clear_composer();
        self.state.clear_composer_notice();
        self.state.deactivate_composer();
        self.cache.clear_draft(&chat_id)?;
        self.loaded_draft_chat_id = Some(chat_id.clone());

        let tx = self.send_tx.clone();
        tokio::spawn(async move {
            let result = fetch_handle.send_text(&chat_id, &text).await;
            let _ = tx.send(super::SendMessageResult {
                chat_id,
                optimistic_message_id,
                result,
            });
        });

        Ok(())
    }

    pub(super) fn sync_draft_for_active_chat(&mut self) -> Result<()> {
        let active_chat_id = self.state.active_chat_id();
        if self.loaded_draft_chat_id == active_chat_id {
            return Ok(());
        }

        if let Some(previous_chat_id) = self.loaded_draft_chat_id.take() {
            self.cache
                .save_draft(&previous_chat_id, &self.state.composer_input)?;
        }

        if let Some(chat_id) = active_chat_id.clone() {
            let draft = self.cache.load_draft(&chat_id)?.unwrap_or_default();
            self.state.set_composer_input(draft);
            self.loaded_draft_chat_id = Some(chat_id);
        } else {
            self.state.clear_composer();
        }

        Ok(())
    }

    pub(super) fn persist_active_draft(&mut self) -> Result<()> {
        if let Some(chat_id) = self.state.active_chat_id() {
            self.cache
                .save_draft(&chat_id, &self.state.composer_input)?;
            self.loaded_draft_chat_id = Some(chat_id);
        }
        Ok(())
    }

    fn next_optimistic_message_id(&mut self, chat_id: &ChatId) -> MessageId {
        self.next_optimistic_message_id += 1;
        MessageId::new(format!(
            "local:{}:{}",
            chat_id.as_str(),
            self.next_optimistic_message_id
        ))
    }

    fn messenger_fetch_handle_or_composer_notice(
        &mut self,
        notice: &str,
    ) -> Option<Arc<dyn MessengerFetchHandle>> {
        let Some(messenger) = self.messenger.as_ref() else {
            self.state.set_composer_notice(notice);
            return None;
        };
        Some(messenger.fetch_handle())
    }
}
