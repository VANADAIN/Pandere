use std::{
    collections::HashSet,
    time::{Duration, Instant},
};

use anyhow::Result;
use crossterm::event::{self, Event, KeyCode};
use pandere_core::ChatId;
use pandere_plugin_telegram::{
    AuthStatus as TelegramAuthStatus, LoginPhase, TelegramClient, TelegramConfig,
    clear_session_file,
};
use ratatui::{Frame, Terminal, prelude::CrosstermBackend};
use tokio::sync::mpsc;

use crate::{
    data_source::MessengerDataSource,
    data_source::{AuthStatus, SyncStatus},
    fixtures::FixtureMessengerSource,
    state::{AppState, LoginInputMode},
    ui,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Screen {
    Main,
    Login,
    Messenger,
}

pub struct App {
    state: AppState,
    telegram_config: Option<TelegramConfig>,
    telegram: Option<TelegramClient>,
    fetch_tx: mpsc::UnboundedSender<ThreadFetchResult>,
    fetch_rx: mpsc::UnboundedReceiver<ThreadFetchResult>,
    pending_thread_fetch: Option<PendingThreadFetch>,
    in_flight_threads: HashSet<ChatId>,
}

impl App {
    pub fn new(
        state: AppState,
        telegram_config: Option<TelegramConfig>,
        telegram: Option<TelegramClient>,
    ) -> Self {
        let (fetch_tx, fetch_rx) = mpsc::unbounded_channel();
        Self {
            state,
            telegram_config,
            telegram,
            fetch_tx,
            fetch_rx,
            pending_thread_fetch: None,
            in_flight_threads: HashSet::new(),
        }
    }

    pub async fn initialize(&mut self) -> Result<()> {
        if self.telegram.is_some() {
            self.refresh_telegram_state().await?;
        }

        Ok(())
    }

    pub async fn run(
        &mut self,
        terminal: &mut Terminal<CrosstermBackend<std::io::Stdout>>,
    ) -> Result<()> {
        loop {
            self.process_thread_results();
            self.maybe_start_pending_thread_fetch();
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

            match key.code {
                KeyCode::Char('q') => break,
                KeyCode::Char('1') => self.state.screen = Screen::Main,
                KeyCode::Char('2') => self.state.screen = Screen::Login,
                KeyCode::Char('3') => self.state.screen = Screen::Messenger,
                KeyCode::Up if self.state.screen == Screen::Messenger => {
                    self.state.select_previous_chat();
                    self.schedule_selected_thread_fetch();
                }
                KeyCode::Down if self.state.screen == Screen::Messenger => {
                    self.state.select_next_chat();
                    self.schedule_selected_thread_fetch();
                }
                _ => {}
            }
        }

        Ok(())
    }

    fn draw(&self, frame: &mut Frame) {
        ui::draw_app(
            frame,
            self.state.screen,
            &self.state.plugin_cards(),
            &self.state.chat_previews(),
            self.state.chats(),
            &self.state.messages(),
            &self.state.login_lines(),
            self.state.selected_chat_index(),
            &self.state.thread_status_label(),
        );
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
                self.state.source.chats = telegram.list_chats(25).await?;
                self.state.selected_chat_id = self.state.source.chats.first().map(|chat| chat.id.clone());
                self.schedule_selected_thread_fetch();
            }
            TelegramAuthStatus::NeedsLogin | TelegramAuthStatus::Connected => {
                let fixture = FixtureMessengerSource.snapshot()?;
                self.state.source.auth_status = AuthStatus::NeedsLogin;
                self.state.source.sync_status = SyncStatus::Pending;
                self.state.source.chats = fixture.chats;
                self.state.thread_cache = crate::state::build_thread_cache(&fixture.messages);
                self.state.source.messages = fixture.messages;
                self.state.selected_chat_id = self.state.source.chats.first().map(|chat| chat.id.clone());
                self.state.thread_status = crate::state::ThreadStatus::Idle;
            }
        }

        Ok(())
    }

    async fn request_login_code(&mut self) -> Result<()> {
        let Some(telegram) = self.telegram.as_mut() else {
            self.state.set_login_notice("telegram client is unavailable");
            return Ok(());
        };

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

        self.telegram = None;
        clear_session_file(&config.session_path)?;

        let telegram = TelegramClient::connect(config).await?;
        self.telegram = Some(telegram);
        self.state.clear_login_input();
        self.state.set_login_notice("telegram session cleared");
        self.in_flight_threads.clear();
        self.pending_thread_fetch = None;
        self.refresh_telegram_state().await
    }

    fn schedule_selected_thread_fetch(&mut self) {
        let Some(chat_id) = self.state.selected_chat_id.clone() else {
            self.state.source.messages.clear();
            self.pending_thread_fetch = None;
            return;
        };

        if self.state.apply_cached_thread() {
            self.pending_thread_fetch = None;
            return;
        }

        self.state.set_thread_loading();
        self.pending_thread_fetch = Some(PendingThreadFetch {
            chat_id,
            requested_at: Instant::now() + Duration::from_millis(150),
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

        tokio::spawn(async move {
            let result = fetch_client.fetch_messages(&chat_id, 50).await;
            let _ = tx.send(ThreadFetchResult { chat_id, result });
        });
    }

    fn process_thread_results(&mut self) {
        while let Ok(result) = self.fetch_rx.try_recv() {
            self.in_flight_threads.remove(&result.chat_id);

            match result.result {
                Ok(messages) => {
                    self.state.cache_thread(result.chat_id, messages);
                }
                Err(error) => {
                    if self.state.selected_chat_id.as_ref() == Some(&result.chat_id) {
                        self.state.set_thread_failed(error.to_string());
                    }
                }
            }
        }
    }
}

struct PendingThreadFetch {
    chat_id: ChatId,
    requested_at: Instant,
}

struct ThreadFetchResult {
    chat_id: ChatId,
    result: Result<Vec<pandere_core::Message>>,
}
