use std::{
    io::{self, Stdout},
    time::{Duration, SystemTime},
};

use anyhow::Result;
use crossterm::{
    event::{self, Event, KeyCode},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use pandere_core::{ChatId, ChatSummary, Message, MessageId, Service};
use ratatui::{
    Frame, Terminal,
    layout::{Constraint, Direction, Layout},
    prelude::CrosstermBackend,
    style::{Modifier, Style},
    text::Line,
    widgets::{Block, Borders, List, ListItem, Paragraph},
};
use tracing::info;
use tracing_subscriber::{EnvFilter, fmt};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Screen {
    Main,
    Login,
    Messenger,
}

struct App {
    screen: Screen,
    chats: Vec<ChatSummary>,
    messages: Vec<Message>,
}

impl App {
    fn fixture() -> Self {
        let chats = vec![
            ChatSummary {
                id: ChatId::new("telegram:team"),
                service: Service::Telegram,
                title: "Pandere Core".into(),
                last_message_preview: Some("WIT draft small. Good.".into()),
                unread_count: 3,
                last_activity_at: Some(SystemTime::now()),
            },
            ChatSummary {
                id: ChatId::new("telegram:ops"),
                service: Service::Telegram,
                title: "Release Ops".into(),
                last_message_preview: Some("Need signed plugin artifacts next.".into()),
                unread_count: 0,
                last_activity_at: Some(SystemTime::now()),
            },
            ChatSummary {
                id: ChatId::new("telegram:personal"),
                service: Service::Telegram,
                title: "Saved Messages".into(),
                last_message_preview: Some("Telegram login spike after shell.".into()),
                unread_count: 1,
                last_activity_at: Some(SystemTime::now()),
            },
        ];

        let primary_chat = chats[0].id.clone();
        let messages = vec![
            Message {
                id: MessageId::new("m1"),
                chat_id: primary_chat.clone(),
                service: Service::Telegram,
                author_name: "Alex".into(),
                text: "Workspace scaffold landed.".into(),
                sent_at: SystemTime::now(),
                is_outgoing: false,
            },
            Message {
                id: MessageId::new("m2"),
                chat_id: primary_chat.clone(),
                service: Service::Telegram,
                author_name: "You".into(),
                text: "Next: core model, WIT, fixture shell.".into(),
                sent_at: SystemTime::now(),
                is_outgoing: true,
            },
            Message {
                id: MessageId::new("m3"),
                chat_id: primary_chat,
                service: Service::Telegram,
                author_name: "Nina".into(),
                text: "Keep host light. Push service logic into plugins.".into(),
                sent_at: SystemTime::now(),
                is_outgoing: false,
            },
        ];

        Self {
            screen: Screen::Main,
            chats,
            messages,
        }
    }

    fn run(&mut self, terminal: &mut Terminal<CrosstermBackend<Stdout>>) -> Result<()> {
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
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(3),
                Constraint::Min(1),
                Constraint::Length(2),
            ])
            .split(frame.area());

        let title = Paragraph::new(Line::from("Pandere Fixture Shell"))
            .block(Block::default().borders(Borders::ALL).title("App"))
            .style(Style::default().add_modifier(Modifier::BOLD));
        frame.render_widget(title, chunks[0]);

        match self.screen {
            Screen::Main => self.draw_main(frame, chunks[1]),
            Screen::Login => self.draw_login(frame, chunks[1]),
            Screen::Messenger => self.draw_messenger(frame, chunks[1]),
        }

        let footer = Paragraph::new("1 Main  2 Login  3 Messenger  q Quit")
            .block(Block::default().borders(Borders::ALL).title("Keys"));
        frame.render_widget(footer, chunks[2]);
    }

    fn draw_main(&self, frame: &mut Frame, area: ratatui::layout::Rect) {
        let items = self.chats.iter().map(|chat| {
            let preview = chat.last_message_preview.as_deref().unwrap_or("No messages yet");
            ListItem::new(format!(
                "{}  [{} unread]  {}",
                chat.title, chat.unread_count, preview
            ))
        });

        let list = List::new(items.collect::<Vec<_>>())
            .block(Block::default().borders(Borders::ALL).title("Main Screen"));
        frame.render_widget(list, area);
    }

    fn draw_login(&self, frame: &mut Frame, area: ratatui::layout::Rect) {
        let text = vec![
            Line::from("Telegram login placeholder"),
            Line::from(""),
            Line::from("Planned flow:"),
            Line::from("1. Enter phone number"),
            Line::from("2. Confirm code"),
            Line::from("3. Persist session via secure handle"),
        ];

        let paragraph =
            Paragraph::new(text).block(Block::default().borders(Borders::ALL).title("Login"));
        frame.render_widget(paragraph, area);
    }

    fn draw_messenger(&self, frame: &mut Frame, area: ratatui::layout::Rect) {
        let columns = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(35), Constraint::Percentage(65)])
            .split(area);

        let chats = self.chats.iter().map(|chat| {
            let marker = if chat.unread_count > 0 { "*" } else { " " };
            ListItem::new(format!("{marker} {}", chat.title))
        });
        let chat_list = List::new(chats.collect::<Vec<_>>())
            .block(Block::default().borders(Borders::ALL).title("Dialogs"));
        frame.render_widget(chat_list, columns[0]);

        let message_lines = self
            .messages
            .iter()
            .map(|message| format!("{}: {}", message.author_name, message.text))
            .collect::<Vec<_>>()
            .join("\n\n");
        let thread = Paragraph::new(message_lines)
            .block(Block::default().borders(Borders::ALL).title("Thread"));
        frame.render_widget(thread, columns[1]);
    }
}

struct TerminalGuard {
    terminal: Terminal<CrosstermBackend<Stdout>>,
}

impl TerminalGuard {
    fn setup() -> Result<Self> {
        enable_raw_mode()?;
        let mut stdout = io::stdout();
        execute!(stdout, EnterAlternateScreen)?;
        let backend = CrosstermBackend::new(stdout);
        let terminal = Terminal::new(backend)?;

        Ok(Self { terminal })
    }

    fn terminal(&mut self) -> &mut Terminal<CrosstermBackend<Stdout>> {
        &mut self.terminal
    }
}

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        let _ = disable_raw_mode();
        let _ = execute!(self.terminal.backend_mut(), LeaveAlternateScreen);
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .compact()
        .init();

    info!("starting pandere fixture shell");

    let mut app = App::fixture();
    let mut terminal = TerminalGuard::setup()?;
    app.run(terminal.terminal())
}
