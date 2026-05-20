use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent};

use super::{App, LeftPaneDirection, Screen};
use crate::state::MessengerFocus;

impl App {
    pub(super) fn handle_messenger_up(&mut self) -> Result<()> {
        if self.state.messenger_focus == MessengerFocus::Right {
            self.state.scroll_thread_up(3);
        } else {
            self.navigate_left_pane(LeftPaneDirection::Previous);
            self.schedule_preview_fetch();
            self.sync_draft_for_active_chat()?;
        }
        Ok(())
    }

    pub(super) fn handle_messenger_down(&mut self) -> Result<()> {
        if self.state.messenger_focus == MessengerFocus::Right {
            self.state.scroll_thread_down(3);
        } else {
            self.navigate_left_pane(LeftPaneDirection::Next);
            self.schedule_preview_fetch();
            self.sync_draft_for_active_chat()?;
        }
        Ok(())
    }

    pub(super) fn handle_messenger_right(&mut self) -> Result<()> {
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

    pub(super) fn handle_messenger_left(&mut self) -> Result<()> {
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

    pub(super) async fn handle_login_key(&mut self, key: KeyEvent) -> Result<bool> {
        if self.state.screen != Screen::Login {
            return Ok(false);
        }

        match key.code {
            KeyCode::Char('r') => self.request_login_code().await?,
            KeyCode::Char('x') => self.logout_telegram().await?,
            KeyCode::Enter => self.submit_login_input().await?,
            KeyCode::Backspace => self.state.pop_login_input(),
            KeyCode::Esc => {
                self.state.clear_login_input();
                self.state.clear_login_notice();
            }
            KeyCode::Char(ch) if self.state.login_input_mode.is_some() && !ch.is_control() => {
                self.state.push_login_input(ch);
            }
            _ => return Ok(false),
        }

        Ok(true)
    }

    pub(super) fn handle_composer_key(&mut self, key: KeyEvent) -> Result<bool> {
        if self.state.screen != Screen::Messenger || !self.state.composer_active {
            return Ok(false);
        }

        match key.code {
            KeyCode::Enter => self.send_composer_text()?,
            KeyCode::Backspace => {
                self.state.pop_composer_input();
                self.persist_active_draft()?;
            }
            KeyCode::Esc => {
                self.state.deactivate_composer();
                self.state.clear_composer_notice();
                self.persist_active_draft()?;
            }
            KeyCode::Char(ch) if !ch.is_control() => {
                self.state.push_composer_input(ch);
                self.persist_active_draft()?;
            }
            _ => return Ok(false),
        }

        Ok(true)
    }

    pub(super) async fn handle_global_key(&mut self, key: KeyEvent) -> Result<bool> {
        match key.code {
            KeyCode::Char('q') => return Ok(true),
            KeyCode::Char('1') => self.state.screen = Screen::Main,
            KeyCode::Char('2') => self.state.screen = Screen::Login,
            KeyCode::Char('3') => self.state.screen = Screen::Messenger,
            KeyCode::Char('0') => self.state.screen = Screen::Logs,
            KeyCode::Char('c') if self.state.screen == Screen::Messenger => {
                self.state.activate_composer();
                self.sync_draft_for_active_chat()?;
            }
            KeyCode::Up if self.state.screen == Screen::Messenger => self.handle_messenger_up()?,
            KeyCode::Down if self.state.screen == Screen::Messenger => {
                self.handle_messenger_down()?
            }
            KeyCode::Right if self.state.screen == Screen::Messenger => {
                self.handle_messenger_right()?
            }
            KeyCode::Left if self.state.screen == Screen::Messenger => {
                self.handle_messenger_left()?
            }
            _ => {}
        }

        Ok(false)
    }

    pub(super) fn navigate_left_pane(&mut self, direction: LeftPaneDirection) {
        match (self.state.is_inside_group_threads(), direction) {
            (true, LeftPaneDirection::Previous) => self.state.select_previous_thread_chat(),
            (true, LeftPaneDirection::Next) => self.state.select_next_thread_chat(),
            (false, LeftPaneDirection::Previous) => self.state.select_previous_root_chat(),
            (false, LeftPaneDirection::Next) => self.state.select_next_root_chat(),
        }
    }
}
