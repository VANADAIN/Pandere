use std::time::Duration;

use anyhow::Result;
use crossterm::event::{self, Event, KeyCode};
use pandere_core::{ChatSummary, Message};
use ratatui::{Frame, Terminal, prelude::CrosstermBackend};

use crate::ui;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Screen {
    Main,
    Login,
    Messenger,
}

pub struct App {
    screen: Screen,
    chats: Vec<ChatSummary>,
    messages: Vec<Message>,
}

impl App {
    pub fn new(chats: Vec<ChatSummary>, messages: Vec<Message>) -> Self {
        Self {
            screen: Screen::Main,
            chats,
            messages,
        }
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
                KeyCode::Char('1') => self.screen = Screen::Main,
                KeyCode::Char('2') => self.screen = Screen::Login,
                KeyCode::Char('3') => self.screen = Screen::Messenger,
                _ => {}
            }
        }

        Ok(())
    }

    fn draw(&self, frame: &mut Frame) {
        ui::draw_app(frame, self.screen, &self.chats, &self.messages);
    }
}
