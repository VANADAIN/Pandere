mod runtime;
mod types;

use std::{collections::HashSet, time::Instant};

use anyhow::Result;
use crossterm::event::{self, Event};
use pandere_core::{ChatId, Message, MessageDeliveryState, MessageId, Service};
use pandere_plugin_telegram::{AuthStatus as TelegramAuthStatus, LoginPhase};
use ratatui::{Frame, Terminal, prelude::CrosstermBackend};
use tokio::sync::mpsc;
use tracing::info;

use self::types::{
    DialogsSyncResult, ForumThreadsFetchResult, LeftPaneDirection, PendingForumThreadsFetch,
    PendingThreadFetch, SendMessageResult, ThreadFetchResult,
};
use crate::{
    cache::CacheStore,
    constants::{
        BACKGROUND_SYNC_INTERVAL, DATABASE_THREAD_LOAD_LIMIT, EVENT_POLL_INTERVAL,
        INITIAL_BACKGROUND_SYNC_DELAY, LOG_SCREEN_BUFFER_LIMIT, PREVIEW_FETCH_DEBOUNCE,
        TELEGRAM_ENV_NOTICE, TELEGRAM_FETCH_DIALOG_LIMIT, TELEGRAM_FETCH_FORUM_TOPIC_LIMIT,
        TELEGRAM_FETCH_MESSAGE_LIMIT,
    },
    data_source::MessengerDataSource,
    data_source::{AuthStatus, SyncStatus},
    fixtures::FixtureMessengerSource,
    logs::LogBuffer,
    messenger_service::TelegramService,
    state::{AppState, LoginInputMode},
    ui,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Screen {
    Main,
    Login,
    Messenger,
    Logs,
}

pub struct App {
    state: AppState,
    telegram: Option<TelegramService>,
    log_buffer: LogBuffer,
    cache: CacheStore,
    fetch_tx: mpsc::UnboundedSender<ThreadFetchResult>,
    fetch_rx: mpsc::UnboundedReceiver<ThreadFetchResult>,
    forum_threads_tx: mpsc::UnboundedSender<ForumThreadsFetchResult>,
    forum_threads_rx: mpsc::UnboundedReceiver<ForumThreadsFetchResult>,
    dialogs_sync_tx: mpsc::UnboundedSender<DialogsSyncResult>,
    dialogs_sync_rx: mpsc::UnboundedReceiver<DialogsSyncResult>,
    send_tx: mpsc::UnboundedSender<SendMessageResult>,
    send_rx: mpsc::UnboundedReceiver<SendMessageResult>,
    pending_thread_fetch: Option<PendingThreadFetch>,
    pending_forum_threads_fetch: Option<PendingForumThreadsFetch>,
    in_flight_threads: HashSet<ChatId>,
    in_flight_forum_threads: HashSet<ChatId>,
    in_flight_dialogs_sync: bool,
    next_background_sync_at: Instant,
    loaded_draft_chat_id: Option<ChatId>,
    next_optimistic_message_id: u64,
}

impl App {
    pub fn new(
        state: AppState,
        telegram: Option<TelegramService>,
        log_buffer: LogBuffer,
        cache: CacheStore,
    ) -> Self {
        let (fetch_tx, fetch_rx) = mpsc::unbounded_channel();
        let (forum_threads_tx, forum_threads_rx) = mpsc::unbounded_channel();
        let (dialogs_sync_tx, dialogs_sync_rx) = mpsc::unbounded_channel();
        let (send_tx, send_rx) = mpsc::unbounded_channel();
        Self {
            state,
            telegram,
            log_buffer,
            cache,
            fetch_tx,
            fetch_rx,
            forum_threads_tx,
            forum_threads_rx,
            dialogs_sync_tx,
            dialogs_sync_rx,
            send_tx,
            send_rx,
            pending_thread_fetch: None,
            pending_forum_threads_fetch: None,
            in_flight_threads: HashSet::new(),
            in_flight_forum_threads: HashSet::new(),
            in_flight_dialogs_sync: false,
            next_background_sync_at: Instant::now() + INITIAL_BACKGROUND_SYNC_DELAY,
            loaded_draft_chat_id: None,
            next_optimistic_message_id: 0,
        }
    }

    pub async fn initialize(&mut self) -> Result<()> {
        self.load_cached_state()?;
        if self.telegram.is_some() {
            self.refresh_telegram_state().await?;
        }
        self.sync_draft_for_active_chat()?;

        Ok(())
    }

    pub async fn run(
        &mut self,
        terminal: &mut Terminal<CrosstermBackend<std::io::Stdout>>,
    ) -> Result<()> {
        loop {
            self.process_dialog_sync_results();
            self.process_forum_thread_results();
            self.process_thread_results();
            self.process_send_results();
            self.maybe_start_pending_forum_threads_fetch();
            self.maybe_start_pending_thread_fetch();
            self.maybe_start_background_sync();
            terminal.draw(|frame| self.draw(frame))?;

            if !event::poll(EVENT_POLL_INTERVAL)? {
                continue;
            }

            let Event::Key(key) = event::read()? else {
                continue;
            };

            if self.handle_login_key(key).await? {
                continue;
            }

            if self.handle_composer_key(key)? {
                continue;
            }

            if self.handle_global_key(key).await? {
                break;
            }
        }

        Ok(())
    }

    fn draw(&self, frame: &mut Frame) {
        ui::draw_app(
            frame,
            &self.state,
            &self.log_buffer.snapshot(LOG_SCREEN_BUFFER_LIMIT),
        );
    }

    async fn refresh_telegram_state(&mut self) -> Result<()> {
        if !self.ensure_telegram_login_available(TELEGRAM_ENV_NOTICE) {
            return Ok(());
        }

        let (login_state, auth_status, chats) = {
            let telegram = self
                .telegram
                .as_mut()
                .expect("telegram availability checked");
            let login_state = telegram.bootstrap_login().await?;
            let auth_status = telegram.auth_status().await?;
            let chats = match &auth_status {
                TelegramAuthStatus::Authorized { .. } => {
                    Some(telegram.list_chats(TELEGRAM_FETCH_DIALOG_LIMIT).await?)
                }
                TelegramAuthStatus::NeedsLogin | TelegramAuthStatus::Connected => None,
            };
            (login_state, auth_status, chats)
        };

        self.state.apply_login_state(login_state);

        match auth_status {
            TelegramAuthStatus::Authorized { user_label } => {
                self.state.source.auth_status = AuthStatus::Authenticated(user_label);
                self.state.source.sync_status = SyncStatus::Idle;
                let chats = chats.expect("authorized telegram state should include chat list");
                self.state.set_chats(chats.clone());
                self.cache.save_chats(Service::Telegram, &chats)?;
                self.schedule_preview_fetch();
            }
            TelegramAuthStatus::NeedsLogin | TelegramAuthStatus::Connected => {
                let fixture = FixtureMessengerSource.snapshot()?;
                self.state.source.auth_status = AuthStatus::NeedsLogin;
                self.state.source.sync_status = SyncStatus::Pending;
                self.state.source.chats = fixture.chats;
                self.state.thread_cache = crate::state::build_thread_cache(&fixture.messages);
                self.state.source.messages = fixture.messages;
                self.state.reset_messenger_selection();
                self.state.thread_status = crate::state::ThreadStatus::Idle;
            }
        }
        self.sync_draft_for_active_chat()?;

        Ok(())
    }

    async fn request_login_code(&mut self) -> Result<()> {
        if !self.ensure_telegram_login_available("telegram client is unavailable") {
            return Ok(());
        }

        info!("requesting telegram login code");
        self.telegram
            .as_mut()
            .expect("telegram availability checked")
            .request_login_code_state()
            .await?;
        self.state.clear_login_notice();
        self.refresh_telegram_state().await
    }

    async fn submit_login_input(&mut self) -> Result<()> {
        if !self.ensure_telegram_login_available("telegram client is unavailable") {
            return Ok(());
        }

        let input = self.state.login_input.trim().to_owned();
        if input.is_empty() {
            self.state.set_login_notice("input is empty");
            return Ok(());
        }

        let login_phase = self.state.login_phase;
        let result = match self.state.login_input_mode {
            Some(LoginInputMode::Phone) => {
                self.telegram
                    .as_mut()
                    .expect("telegram availability checked")
                    .set_phone_number(input)?;
                self.telegram
                    .as_mut()
                    .expect("telegram availability checked")
                    .request_login_code_state()
                    .await
            }
            Some(LoginInputMode::Code) => {
                self.telegram
                    .as_mut()
                    .expect("telegram availability checked")
                    .submit_login_code_state(&input)
                    .await
            }
            Some(LoginInputMode::Password) => {
                self.telegram
                    .as_mut()
                    .expect("telegram availability checked")
                    .submit_password_state(&input)
                    .await
            }
            None => match login_phase {
                Some(LoginPhase::CodeRequested) => {
                    self.telegram
                        .as_mut()
                        .expect("telegram availability checked")
                        .submit_login_code_state(&input)
                        .await
                }
                Some(LoginPhase::PasswordRequired) => {
                    self.telegram
                        .as_mut()
                        .expect("telegram availability checked")
                        .submit_password_state(&input)
                        .await
                }
                _ => {
                    self.state
                        .set_login_notice("nothing to submit in current login phase");
                    return Ok(());
                }
            },
        };

        match result {
            Ok(_) => {
                info!("telegram login step completed");
                self.state.clear_login_input();
                self.state.clear_login_notice();
                self.refresh_telegram_state().await
            }
            Err(error) => {
                self.state.set_login_notice(error.to_string());
                Ok(())
            }
        }
    }

    async fn logout_telegram(&mut self) -> Result<()> {
        if !self.ensure_telegram_login_available("telegram client is unavailable") {
            return Ok(());
        }

        info!("clearing telegram session");
        self.telegram
            .as_mut()
            .expect("telegram availability checked")
            .clear_session_and_reconnect()
            .await?;
        self.state.clear_login_input();
        self.state.set_login_notice("telegram session cleared");
        self.in_flight_threads.clear();
        self.in_flight_forum_threads.clear();
        self.pending_thread_fetch = None;
        self.pending_forum_threads_fetch = None;
        self.refresh_telegram_state().await
    }

    fn load_cached_state(&mut self) -> Result<()> {
        let Some(snapshot) = self.cache.load_snapshot(Service::Telegram)? else {
            return Ok(());
        };

        self.state.source.chats = snapshot.chats;
        self.state.thread_cache = crate::state::build_thread_cache(&snapshot.messages);
        self.state.reset_messenger_selection();
        let _ = self.state.apply_cached_thread();
        Ok(())
    }

    fn schedule_preview_fetch(&mut self) {
        if !self.state.is_inside_group_threads() && self.state.selected_root_has_threads() {
            let Some(root_chat_id) = self.state.selected_root_chat_id.clone() else {
                return;
            };

            if self.state.apply_cached_forum_threads() {
                self.pending_forum_threads_fetch = None;
                return;
            }

            self.state.set_forum_threads_loading();
            self.pending_forum_threads_fetch = Some(PendingForumThreadsFetch {
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

    fn maybe_start_pending_forum_threads_fetch(&mut self) {
        let Some(pending) = self.pending_forum_threads_fetch.as_ref() else {
            return;
        };

        if Instant::now() < pending.requested_at {
            return;
        }

        let Some(telegram) = self.telegram.as_ref() else {
            self.pending_forum_threads_fetch = None;
            return;
        };

        let root_chat_id = pending.root_chat_id.clone();
        if self.in_flight_forum_threads.contains(&root_chat_id) {
            self.pending_forum_threads_fetch = None;
            return;
        }

        let tx = self.forum_threads_tx.clone();
        let fetch_client = telegram.fetch_client();
        self.in_flight_forum_threads.insert(root_chat_id.clone());
        self.pending_forum_threads_fetch = None;
        info!(chat_id = %root_chat_id.as_str(), "starting forum thread list fetch");

        tokio::spawn(async move {
            let result = fetch_client
                .list_forum_topics(&root_chat_id, TELEGRAM_FETCH_FORUM_TOPIC_LIMIT)
                .await;
            let _ = tx.send(ForumThreadsFetchResult {
                root_chat_id,
                result,
            });
        });
    }

    fn maybe_start_pending_thread_fetch(&mut self) {
        let Some(pending) = self.pending_thread_fetch.as_ref() else {
            return;
        };

        if Instant::now() < pending.requested_at {
            return;
        }

        let Some(telegram) = self.telegram.as_ref() else {
            self.pending_thread_fetch = None;
            return;
        };

        let chat_id = pending.chat_id.clone();
        if self.in_flight_threads.contains(&chat_id) {
            self.pending_thread_fetch = None;
            return;
        }

        let fetch_client = telegram.fetch_client();
        let tx = self.fetch_tx.clone();
        self.in_flight_threads.insert(chat_id.clone());
        self.pending_thread_fetch = None;
        info!(chat_id = %chat_id.as_str(), "starting thread fetch");

        tokio::spawn(async move {
            let result = fetch_client
                .fetch_messages(&chat_id, TELEGRAM_FETCH_MESSAGE_LIMIT)
                .await;
            let _ = tx.send(ThreadFetchResult { chat_id, result });
        });
    }

    fn maybe_start_background_sync(&mut self) {
        if Instant::now() < self.next_background_sync_at {
            return;
        }

        self.next_background_sync_at = Instant::now() + BACKGROUND_SYNC_INTERVAL;

        let Some(telegram) = self.telegram.as_ref() else {
            return;
        };

        if !matches!(self.state.source.auth_status, AuthStatus::Authenticated(_)) {
            return;
        }

        if !self.in_flight_dialogs_sync {
            let tx = self.dialogs_sync_tx.clone();
            let fetch_client = telegram.fetch_client();
            self.in_flight_dialogs_sync = true;
            info!("starting background dialogs sync");
            tokio::spawn(async move {
                let result = fetch_client.list_chats(TELEGRAM_FETCH_DIALOG_LIMIT).await;
                let _ = tx.send(DialogsSyncResult { result });
            });
        }

        if let Some(chat_id) = self.state.preview_chat_id() {
            if !self.in_flight_threads.contains(&chat_id) {
                self.schedule_thread_fetch(chat_id, std::time::Duration::ZERO);
            }
        }
    }

    fn process_thread_results(&mut self) {
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

    fn process_dialog_sync_results(&mut self) {
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
                    if let Err(error) = self.cache.save_chats(Service::Telegram, &chats) {
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

    fn process_forum_thread_results(&mut self) {
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

    fn process_send_results(&mut self) {
        while let Ok(result) = self.send_rx.try_recv() {
            match result.result {
                Ok(mut message) => {
                    info!(chat_id = %result.chat_id.as_str(), "sent telegram text message");
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
                    info!(chat_id = %result.chat_id.as_str(), error = %error, "failed to send telegram text message");
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

    fn send_composer_text(&mut self) -> Result<()> {
        let Some(fetch_client) =
            self.telegram_fetch_client_or_composer_notice("telegram client is unavailable")
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
            service: Service::Telegram,
            author_name: "You".into(),
            text: text.clone(),
            sent_at: std::time::SystemTime::now(),
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
            let result = fetch_client.send_text(&chat_id, &text).await;
            let _ = tx.send(SendMessageResult {
                chat_id,
                optimistic_message_id,
                result,
            });
        });

        Ok(())
    }

    fn schedule_force_thread_refresh(&mut self, chat_id: ChatId) {
        self.state.set_thread_loading();
        self.schedule_thread_fetch(chat_id, std::time::Duration::ZERO);
    }

    fn load_thread_from_db(&mut self, chat_id: &ChatId) -> bool {
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

    fn sync_draft_for_active_chat(&mut self) -> Result<()> {
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

    fn persist_active_draft(&mut self) -> Result<()> {
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

    fn ensure_telegram_login_available(&mut self, notice: &str) -> bool {
        if self.telegram.is_none() {
            self.state.set_login_notice(notice);
            return false;
        }
        true
    }

    fn telegram_fetch_client_or_composer_notice(
        &mut self,
        notice: &str,
    ) -> Option<pandere_plugin_telegram::TelegramFetchClient> {
        let Some(telegram) = self.telegram.as_ref() else {
            self.state.set_composer_notice(notice);
            return None;
        };
        Some(telegram.fetch_client())
    }

    fn schedule_thread_fetch(&mut self, chat_id: ChatId, delay: std::time::Duration) {
        self.pending_thread_fetch = Some(PendingThreadFetch {
            chat_id,
            requested_at: Instant::now() + delay,
        });
    }
}
