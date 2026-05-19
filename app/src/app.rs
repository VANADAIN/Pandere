use std::time::Duration;

use anyhow::Result;
use crossterm::event::{self, Event, KeyCode};
use ratatui::{Frame, Terminal, prelude::CrosstermBackend};

use crate::{state::AppState, ui};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Screen {
    Main,
    Login,
    Messenger,
}

pub struct App {
    state: AppState,
}

impl App {
    pub fn new(state: AppState) -> Self {
        Self { state }
    }

    pub fn run(
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

            match key.code {
                KeyCode::Char('q') => break,
                KeyCode::Char('1') => self.state.screen = Screen::Main,
                KeyCode::Char('2') => self.state.screen = Screen::Login,
                KeyCode::Char('3') => self.state.screen = Screen::Messenger,
                _ => {}
            }
        }

        Ok(())
    }

    fn draw(&self, frame: &mut Frame) {
        ui::draw_app(
            frame,
            self.state.screen,
            &self.state.messenger_overviews(),
            self.state.chats(),
            &self.state.messages(),
            &self.state.login_lines(),
        );
    }
}
