use std::time::{Duration, Instant};

use anyhow::Result;
use pandere_core::{ChatId, Service};
use tracing::info;

use super::App;
use crate::{
    constants::{
        BACKGROUND_SYNC_INTERVAL, DATABASE_THREAD_LOAD_LIMIT, PREVIEW_FETCH_DEBOUNCE,
        TELEGRAM_FETCH_DIALOG_LIMIT, TELEGRAM_FETCH_FORUM_TOPIC_LIMIT,
        TELEGRAM_FETCH_MESSAGE_LIMIT,
    },
    data_source::SyncStatus,
};

impl App {
    pub(super) fn load_cached_state(&mut self) -> Result<()> {
        let service = self.primary_service();
        let Some(snapshot) = self.cache.load_snapshot(service)? else {
            return Ok(());
        };

        self.state.source.chats = snapshot.chats;
        self.state.thread_cache = crate::state::build_thread_cache(&snapshot.messages);
        self.state.reset_messenger_selection();
        let _ = self.state.apply_cached_thread();
        Ok(())
    }

    pub(super) fn schedule_preview_fetch(&mut self) {
        if !self.state.is_inside_group_threads() && self.state.selected_root_has_threads() {
            let Some(root_chat_id) = self.state.selected_root_chat_id.clone() else {
                return;
            };

            if self.state.apply_cached_forum_threads() {
                self.pending_forum_threads_fetch = None;
                return;
            }

            self.state.set_forum_threads_loading();
            self.pending_forum_threads_fetch = Some(super::PendingForumThreadsFetch {
                root_chat_id,
                requested_at: Instant::now() + PREVIEW_FETCH_DEBOUNCE,
            });
            self.pending_thread_fetch = None;
            self.state.source.messages.clear();
            return;
        };

        let Some(chat_id) = self.state.preview_chat_id() else {
            self.state.source.messages.clear();
            self.pending_thread_fetch = None;
            return;
        };

        let used_cached_thread =
            self.state.apply_cached_thread() || self.load_thread_from_db(&chat_id);
        if used_cached_thread {
            info!(chat_id = %chat_id.as_str(), "using cached thread before refresh");
        } else {
            info!(chat_id = %chat_id.as_str(), "scheduled thread fetch");
            self.state.set_thread_loading();
        }

        self.schedule_thread_fetch(chat_id, PREVIEW_FETCH_DEBOUNCE);
    }

    pub(super) fn maybe_start_pending_forum_threads_fetch(&mut self) {
        let Some(pending) = self.pending_forum_threads_fetch.as_ref() else {
            return;
        };

        if Instant::now() < pending.requested_at {
            return;
        }

        let Some(fetch_handle) = self.messenger_fetch_handle() else {
            self.pending_forum_threads_fetch = None;
            return;
        };

        let root_chat_id = pending.root_chat_id.clone();
        if self.in_flight_forum_threads.contains(&root_chat_id) {
            self.pending_forum_threads_fetch = None;
            return;
        }

        let tx = self.forum_threads_tx.clone();
        self.in_flight_forum_threads.insert(root_chat_id.clone());
        self.pending_forum_threads_fetch = None;
        info!(chat_id = %root_chat_id.as_str(), "starting forum thread list fetch");

        tokio::spawn(async move {
            let result = fetch_handle
                .list_subchats(&root_chat_id, TELEGRAM_FETCH_FORUM_TOPIC_LIMIT)
                .await;
            let _ = tx.send(super::ForumThreadsFetchResult {
                root_chat_id,
                result,
            });
        });
    }

    pub(super) fn maybe_start_pending_thread_fetch(&mut self) {
        let Some(pending) = self.pending_thread_fetch.as_ref() else {
            return;
        };

        if Instant::now() < pending.requested_at {
            return;
        }

        let Some(fetch_handle) = self.messenger_fetch_handle() else {
            self.pending_thread_fetch = None;
            return;
        };

        let chat_id = pending.chat_id.clone();
        if self.in_flight_threads.contains(&chat_id) {
            self.pending_thread_fetch = None;
            return;
        }

        let tx = self.fetch_tx.clone();
        self.in_flight_threads.insert(chat_id.clone());
        self.pending_thread_fetch = None;
        info!(chat_id = %chat_id.as_str(), "starting thread fetch");

        tokio::spawn(async move {
            let result = fetch_handle
                .fetch_messages(&chat_id, TELEGRAM_FETCH_MESSAGE_LIMIT)
                .await;
            let _ = tx.send(super::ThreadFetchResult { chat_id, result });
        });
    }

    pub(super) fn maybe_start_background_sync(&mut self) {
        if Instant::now() < self.next_background_sync_at {
            return;
        }

        self.next_background_sync_at = Instant::now() + BACKGROUND_SYNC_INTERVAL;

        let Some(fetch_handle) = self.messenger_fetch_handle() else {
            return;
        };

        if !matches!(
            self.state.source.auth_status,
            crate::data_source::AuthStatus::Authenticated(_)
        ) {
            return;
        }

        if !self.in_flight_dialogs_sync {
            let tx = self.dialogs_sync_tx.clone();
            self.in_flight_dialogs_sync = true;
            info!("starting background dialogs sync");
            tokio::spawn(async move {
                let result = fetch_handle.sync_tick(TELEGRAM_FETCH_DIALOG_LIMIT).await;
                let _ = tx.send(super::DialogsSyncResult { result });
            });
        }

        if let Some(chat_id) = self.state.preview_chat_id() {
            if !self.in_flight_threads.contains(&chat_id) {
                self.schedule_thread_fetch(chat_id, Duration::ZERO);
            }
        }
    }

    pub(super) fn process_thread_results(&mut self) {
        while let Ok(result) = self.fetch_rx.try_recv() {
            self.in_flight_threads.remove(&result.chat_id);

            match result.result {
                Ok(messages) => {
                    info!(
                        chat_id = %result.chat_id.as_str(),
                        message_count = messages.len(),
                        "thread fetch completed"
                    );
                    self.state
                        .cache_thread(result.chat_id.clone(), messages.clone());
                    if let Err(error) = self.cache.save_messages(&result.chat_id, &messages) {
                        info!(chat_id = %result.chat_id.as_str(), error = %error, "failed to persist message cache");
                    }
                }
                Err(error) => {
                    info!(
                        chat_id = %result.chat_id.as_str(),
                        error = %error,
                        "thread fetch failed"
                    );
                    if self.state.preview_chat_id().as_ref() == Some(&result.chat_id) {
                        self.state.set_thread_failed(error.to_string());
                    }
                }
            }
        }
    }

    pub(super) fn process_dialog_sync_results(&mut self) {
        while let Ok(result) = self.dialogs_sync_rx.try_recv() {
            self.in_flight_dialogs_sync = false;
            match result.result {
                Ok(chats) => {
                    info!(
                        chat_count = chats.len(),
                        "background dialogs sync completed"
                    );
                    self.state.source.sync_status = SyncStatus::Idle;
                    self.state.set_chats(chats.clone());
                    if let Err(error) = self.cache.save_chats(self.primary_service(), &chats) {
                        info!(error = %error, "failed to persist dialog cache");
                    }
                }
                Err(error) => {
                    info!(error = %error, "background dialogs sync failed");
                    self.state.source.sync_status = SyncStatus::Failed(error.to_string());
                }
            }
        }
    }

    pub(super) fn process_forum_thread_results(&mut self) {
        while let Ok(result) = self.forum_threads_rx.try_recv() {
            self.in_flight_forum_threads.remove(&result.root_chat_id);

            match result.result {
                Ok(threads) => {
                    info!(
                        chat_id = %result.root_chat_id.as_str(),
                        thread_count = threads.len(),
                        "forum thread list fetch completed"
                    );
                    self.state.cache_forum_threads(result.root_chat_id, threads);
                    if !self.state.is_inside_group_threads() {
                        let _ = self.state.apply_cached_thread();
                    }
                }
                Err(error) => {
                    info!(
                        chat_id = %result.root_chat_id.as_str(),
                        error = %error,
                        "forum thread list fetch failed"
                    );
                    if self.state.selected_root_chat_id.as_ref() == Some(&result.root_chat_id) {
                        self.state.set_forum_threads_failed(error.to_string());
                    }
                }
            }
        }
    }

    pub(super) fn load_thread_from_db(&mut self, chat_id: &ChatId) -> bool {
        match self
            .cache
            .load_messages(chat_id, DATABASE_THREAD_LOAD_LIMIT)
        {
            Ok(messages) if !messages.is_empty() => {
                self.state.cache_thread(chat_id.clone(), messages);
                true
            }
            Ok(_) => false,
            Err(error) => {
                info!(chat_id = %chat_id.as_str(), error = %error, "failed to load cached thread from database");
                false
            }
        }
    }

    pub(super) fn schedule_force_thread_refresh(&mut self, chat_id: ChatId) {
        self.state.set_thread_loading();
        self.schedule_thread_fetch(chat_id, Duration::ZERO);
    }

    pub(super) fn schedule_thread_fetch(&mut self, chat_id: ChatId, delay: Duration) {
        self.pending_thread_fetch = Some(super::PendingThreadFetch {
            chat_id,
            requested_at: Instant::now() + delay,
        });
    }

    pub(super) fn primary_service(&self) -> Service {
        self.messenger
            .as_ref()
            .map(|messenger| messenger.service())
            .unwrap_or(self.state.source.service)
    }
}
