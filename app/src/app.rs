mod composer;
mod login;
mod runtime;
mod sync;
mod types;

use std::{collections::HashSet, time::Instant};

use anyhow::Result;
use crossterm::event::{self, Event};
use pandere_core::ChatId;
use ratatui::{Frame, Terminal, prelude::CrosstermBackend};
use tokio::sync::mpsc;

use self::types::{
    DialogsSyncResult, ForumThreadsFetchResult, LeftPaneDirection, PendingForumThreadsFetch,
    PendingThreadFetch, SendMessageResult, ThreadFetchResult,
};
use crate::{
    cache::CacheStore,
    constants::{EVENT_POLL_INTERVAL, INITIAL_BACKGROUND_SYNC_DELAY, LOG_SCREEN_BUFFER_LIMIT},
    logs::LogBuffer,
    messenger_service::{MessengerFetchHandle, MessengerService},
    state::AppState,
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
    messenger: Option<Box<dyn MessengerService>>,
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
        messenger: Option<Box<dyn MessengerService>>,
        log_buffer: LogBuffer,
        cache: CacheStore,
    ) -> Self {
        let (fetch_tx, fetch_rx) = mpsc::unbounded_channel();
        let (forum_threads_tx, forum_threads_rx) = mpsc::unbounded_channel();
        let (dialogs_sync_tx, dialogs_sync_rx) = mpsc::unbounded_channel();
        let (send_tx, send_rx) = mpsc::unbounded_channel();
        Self {
            state,
            messenger,
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
        if self.messenger.is_some() {
            self.refresh_messenger_state().await?;
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

    fn ensure_messenger_available(&mut self, notice: &str) -> bool {
        if self.messenger.is_none() {
            self.state.set_login_notice(notice);
            return false;
        }
        true
    }

    fn messenger_fetch_handle(&self) -> Option<std::sync::Arc<dyn MessengerFetchHandle>> {
        self.messenger
            .as_ref()
            .map(|messenger| messenger.fetch_handle())
    }
}
