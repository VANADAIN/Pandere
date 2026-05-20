use std::{
    collections::HashSet,
    time::{Duration, Instant},
};

use anyhow::Result;
use crossterm::event::{self, Event, KeyCode};
use pandere_core::{ChatId, ChatSummary, Message, MessageDeliveryState, MessageId, Service};
use pandere_plugin_telegram::{
    AuthStatus as TelegramAuthStatus, LoginPhase, TelegramClient, TelegramConfig,
    clear_session_file,
};
use ratatui::{Frame, Terminal, prelude::CrosstermBackend};
use tokio::sync::mpsc;
use tracing::info;

use crate::{
    cache::CacheStore,
    data_source::MessengerDataSource,
    data_source::{AuthStatus, SyncStatus},
    fixtures::FixtureMessengerSource,
    logs::LogBuffer,
    state::{AppState, LoginInputMode, MessengerFocus},
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
    telegram_config: Option<TelegramConfig>,
    telegram: Option<TelegramClient>,
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
        telegram_config: Option<TelegramConfig>,
        telegram: Option<TelegramClient>,
        log_buffer: LogBuffer,
        cache: CacheStore,
    ) -> Self {
        let (fetch_tx, fetch_rx) = mpsc::unbounded_channel();
        let (forum_threads_tx, forum_threads_rx) = mpsc::unbounded_channel();
        let (dialogs_sync_tx, dialogs_sync_rx) = mpsc::unbounded_channel();
        let (send_tx, send_rx) = mpsc::unbounded_channel();
        Self {
            state,
            telegram_config,
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
            next_background_sync_at: Instant::now() + Duration::from_secs(15),
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

            if !event::poll(Duration::from_millis(50))? {
                continue;
            }

            let Event::Key(key) = event::read()? else {
                continue;
            };

            if self.state.screen == Screen::Login {
                match key.code {
                    KeyCode::Char('r') => {
                        self.request_login_code().await?;
                        continue;
                    }
                    KeyCode::Char('x') => {
                        self.logout_telegram().await?;
                        continue;
                    }
                    KeyCode::Enter => {
                        self.submit_login_input().await?;
                        continue;
                    }
                    KeyCode::Backspace => {
                        self.state.pop_login_input();
                        continue;
                    }
                    KeyCode::Esc => {
                        self.state.clear_login_input();
                        self.state.clear_login_notice();
                        continue;
                    }
                    KeyCode::Char(ch) if self.state.login_input_mode.is_some() && !ch.is_control() => {
                        self.state.push_login_input(ch);
                        continue;
                    }
                    _ => {}
                }
            }

            if self.state.screen == Screen::Messenger && self.state.composer_active {
                match key.code {
                    KeyCode::Enter => {
                        self.send_composer_text()?;
                        continue;
                    }
                    KeyCode::Backspace => {
                        self.state.pop_composer_input();
                        self.persist_active_draft()?;
                        continue;
                    }
                    KeyCode::Esc => {
                        self.state.deactivate_composer();
                        self.state.clear_composer_notice();
                        self.persist_active_draft()?;
                        continue;
                    }
                    KeyCode::Char(ch) if !ch.is_control() => {
                        self.state.push_composer_input(ch);
                        self.persist_active_draft()?;
                        continue;
                    }
                    _ => {}
                }
            }

            match key.code {
                KeyCode::Char('q') => break,
                KeyCode::Char('1') => self.state.screen = Screen::Main,
                KeyCode::Char('2') => self.state.screen = Screen::Login,
                KeyCode::Char('3') => self.state.screen = Screen::Messenger,
                KeyCode::Char('0') => self.state.screen = Screen::Logs,
                KeyCode::Char('c') if self.state.screen == Screen::Messenger => {
                    self.state.activate_composer();
                    self.sync_draft_for_active_chat()?;
                }
                KeyCode::Up if self.state.screen == Screen::Messenger => {
                    self.handle_messenger_up()?;
                }
                KeyCode::Down if self.state.screen == Screen::Messenger => {
                    self.handle_messenger_down()?;
                }
                KeyCode::Right if self.state.screen == Screen::Messenger => {
                    self.handle_messenger_right()?;
                }
                KeyCode::Left if self.state.screen == Screen::Messenger => {
                    self.handle_messenger_left()?;
                }
                _ => {}
            }
        }

        Ok(())
    }

    fn draw(&self, frame: &mut Frame) {
        ui::draw_app(frame, &self.state, &self.log_buffer.snapshot(500));
    }

    async fn refresh_telegram_state(&mut self) -> Result<()> {
        let Some(telegram) = self.telegram.as_mut() else {
            self.state.set_login_notice(
                "telegram env is not configured; set TELEGRAM_API_ID, TELEGRAM_API_HASH, TELEGRAM_PHONE",
            );
            return Ok(());
        };

        let login_state = telegram.bootstrap_login().await?;
        self.state.apply_login_state(login_state);

        match telegram.auth_status().await? {
            TelegramAuthStatus::Authorized { user_label } => {
                self.state.source.auth_status = AuthStatus::Authenticated(user_label);
                self.state.source.sync_status = SyncStatus::Idle;
                let chats = telegram.list_chats(500).await?;
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
        let Some(telegram) = self.telegram.as_mut() else {
            self.state.set_login_notice("telegram client is unavailable");
            return Ok(());
        };

        info!("requesting telegram login code");
        telegram.request_login_code_state().await?;
        self.state.clear_login_notice();
        self.refresh_telegram_state().await
    }

    async fn submit_login_input(&mut self) -> Result<()> {
        let Some(telegram) = self.telegram.as_mut() else {
            self.state.set_login_notice("telegram client is unavailable");
            return Ok(());
        };

        let input = self.state.login_input.trim().to_owned();
        if input.is_empty() {
            self.state.set_login_notice("input is empty");
            return Ok(());
        }

        let result = match self.state.login_input_mode {
            Some(LoginInputMode::Phone) => {
                telegram.set_phone_number(input)?;
                telegram.request_login_code_state().await
            }
            Some(LoginInputMode::Code) => telegram.submit_login_code_state(&input).await,
            Some(LoginInputMode::Password) => telegram.submit_password_state(&input).await,
            None => match self.state.login_phase {
                Some(LoginPhase::CodeRequested) => telegram.submit_login_code_state(&input).await,
                Some(LoginPhase::PasswordRequired) => telegram.submit_password_state(&input).await,
                _ => {
                    self.state.set_login_notice("nothing to submit in current login phase");
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
        let Some(config) = self.telegram_config.clone() else {
            self.state.set_login_notice("telegram config is unavailable");
            return Ok(());
        };

        info!("clearing telegram session");
        self.telegram = None;
        clear_session_file(&config.session_path)?;

        let telegram = TelegramClient::connect(config).await?;
        self.telegram = Some(telegram);
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
                requested_at: Instant::now() + Duration::from_millis(150),
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

        let used_cached_thread = self.state.apply_cached_thread() || self.load_thread_from_db(&chat_id);
        if used_cached_thread {
            info!(chat_id = %chat_id.as_str(), "using cached thread before refresh");
        } else {
            info!(chat_id = %chat_id.as_str(), "scheduled thread fetch");
            self.state.set_thread_loading();
        }

        self.pending_thread_fetch = Some(PendingThreadFetch {
            chat_id,
            requested_at: Instant::now() + Duration::from_millis(150),
        });
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
            let result = fetch_client.list_forum_topics(&root_chat_id, 500).await;
            let _ = tx.send(ForumThreadsFetchResult { root_chat_id, result });
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
            let result = fetch_client.fetch_messages(&chat_id, 50).await;
            let _ = tx.send(ThreadFetchResult { chat_id, result });
        });
    }

    fn maybe_start_background_sync(&mut self) {
        if Instant::now() < self.next_background_sync_at {
            return;
        }

        self.next_background_sync_at = Instant::now() + Duration::from_secs(20);

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
                let result = fetch_client.list_chats(500).await;
                let _ = tx.send(DialogsSyncResult { result });
            });
        }

        if let Some(chat_id) = self.state.preview_chat_id() {
            if !self.in_flight_threads.contains(&chat_id) {
                self.pending_thread_fetch = Some(PendingThreadFetch {
                    chat_id,
                    requested_at: Instant::now(),
                });
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
                    self.state.cache_thread(result.chat_id.clone(), messages.clone());
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
                    info!(chat_count = chats.len(), "background dialogs sync completed");
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
        let Some(telegram) = self.telegram.as_ref() else {
            self.state.set_composer_notice("telegram client is unavailable");
            return Ok(());
        };
        let fetch_client = telegram.fetch_client();

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
        self.state
            .merge_message(&chat_id, optimistic_message, None);
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
        self.pending_thread_fetch = Some(PendingThreadFetch {
            chat_id,
            requested_at: Instant::now(),
        });
    }

    fn handle_messenger_up(&mut self) -> Result<()> {
        if self.state.messenger_focus == MessengerFocus::Right {
            self.state.scroll_thread_up(3);
        } else {
            if self.state.is_inside_group_threads() {
                self.state.select_previous_thread_chat();
            } else {
                self.state.select_previous_root_chat();
            }
            self.schedule_preview_fetch();
            self.sync_draft_for_active_chat()?;
        }
        Ok(())
    }

    fn handle_messenger_down(&mut self) -> Result<()> {
        if self.state.messenger_focus == MessengerFocus::Right {
            self.state.scroll_thread_down(3);
        } else {
            if self.state.is_inside_group_threads() {
                self.state.select_next_thread_chat();
            } else {
                self.state.select_next_root_chat();
            }
            self.schedule_preview_fetch();
            self.sync_draft_for_active_chat()?;
        }
        Ok(())
    }

    fn handle_messenger_right(&mut self) -> Result<()> {
        if self.state.is_inside_group_threads() {
            if self.state.can_focus_right_pane() {
                self.state.focus_right_pane();
                if let Some(chat_id) = self.state.active_chat_id() {
                    self.in_flight_threads.remove(&chat_id);
                    self.schedule_force_thread_refresh(chat_id);
                }
            }
            self.sync_draft_for_active_chat()?;
            return Ok(());
        }

        if self.state.selected_root_has_threads() {
            self.state.enter_selected_root();
            self.schedule_preview_fetch();
        } else if self.state.can_focus_right_pane() {
            self.state.focus_right_pane();
            if let Some(chat_id) = self.state.active_chat_id() {
                self.in_flight_threads.remove(&chat_id);
                self.schedule_force_thread_refresh(chat_id);
            }
        }
        self.sync_draft_for_active_chat()?;
        Ok(())
    }

    fn handle_messenger_left(&mut self) -> Result<()> {
        if self.state.messenger_focus == MessengerFocus::Right {
            self.state.focus_left_pane();
            self.sync_draft_for_active_chat()?;
            return Ok(());
        }

        if self.state.is_inside_group_threads() {
            self.state.leave_group_threads();
            self.schedule_preview_fetch();
        }
        self.sync_draft_for_active_chat()?;
        Ok(())
    }

    fn load_thread_from_db(&mut self, chat_id: &ChatId) -> bool {
        match self.cache.load_messages(chat_id, 200) {
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
            self.cache.save_draft(&chat_id, &self.state.composer_input)?;
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
}

struct PendingThreadFetch {
    chat_id: ChatId,
    requested_at: Instant,
}

struct PendingForumThreadsFetch {
    root_chat_id: ChatId,
    requested_at: Instant,
}

struct ThreadFetchResult {
    chat_id: ChatId,
    result: Result<Vec<pandere_core::Message>>,
}

struct ForumThreadsFetchResult {
    root_chat_id: ChatId,
    result: Result<Vec<pandere_core::ChatSummary>>,
}

struct DialogsSyncResult {
    result: Result<Vec<ChatSummary>>,
}

struct SendMessageResult {
    chat_id: ChatId,
    optimistic_message_id: MessageId,
    result: Result<Message>,
}
