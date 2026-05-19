use std::time::Duration;

use anyhow::Result;
use crossterm::event::{self, Event, KeyCode};
use pandere_plugin_telegram::{
    AuthStatus as TelegramAuthStatus, LoginPhase, TelegramClient, TelegramConfig,
    clear_session_file,
};
use ratatui::{Frame, Terminal, prelude::CrosstermBackend};

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
}

impl App {
    pub fn new(
        state: AppState,
        telegram_config: Option<TelegramConfig>,
        telegram: Option<TelegramClient>,
    ) -> Self {
        Self {
            state,
            telegram_config,
            telegram,
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
            terminal.draw(|frame| self.draw(frame))?;

            if !event::poll(Duration::from_millis(250))? {
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
                    self.refresh_selected_messages().await?;
                }
                KeyCode::Down if self.state.screen == Screen::Messenger => {
                    self.state.select_next_chat();
                    self.refresh_selected_messages().await?;
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
                self.refresh_selected_messages().await?;
            }
            TelegramAuthStatus::NeedsLogin | TelegramAuthStatus::Connected => {
                let fixture = FixtureMessengerSource.snapshot()?;
                self.state.source.auth_status = AuthStatus::NeedsLogin;
                self.state.source.sync_status = SyncStatus::Pending;
                self.state.source.chats = fixture.chats;
                self.state.source.messages = fixture.messages;
                self.state.selected_chat_id = self.state.source.chats.first().map(|chat| chat.id.clone());
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
        self.refresh_telegram_state().await
    }

    async fn refresh_selected_messages(&mut self) -> Result<()> {
        let Some(telegram) = self.telegram.as_mut() else {
            return Ok(());
        };

        if !matches!(telegram.auth_status().await?, TelegramAuthStatus::Authorized { .. }) {
            return Ok(());
        }

        let Some(chat_id) = self.state.selected_chat_id.clone() else {
            self.state.source.messages.clear();
            return Ok(());
        };

        self.state.source.messages = telegram.fetch_messages(&chat_id, 50).await?;
        Ok(())
    }
}
