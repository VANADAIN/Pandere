use pandere_core::{ChatSummary, Message};
use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    text::Line,
    widgets::{Block, Borders, List, ListItem, Paragraph},
};

use crate::app::Screen;

pub fn draw_app(frame: &mut Frame, screen: Screen, chats: &[ChatSummary], messages: &[Message]) {
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

    match screen {
        Screen::Main => draw_main(frame, chunks[1], chats),
        Screen::Login => draw_login(frame, chunks[1]),
        Screen::Messenger => draw_messenger(frame, chunks[1], chats, messages),
    }

    let footer = Paragraph::new("1 Main  2 Login  3 Messenger  q Quit")
        .block(Block::default().borders(Borders::ALL).title("Keys"));
    frame.render_widget(footer, chunks[2]);
}

fn draw_main(frame: &mut Frame, area: Rect, chats: &[ChatSummary]) {
    let items = chats.iter().map(|chat| {
        let preview = chat.last_message_preview.as_deref().unwrap_or("No messages yet");
        ListItem::new(format!(
            "{}  [{} unread]  {}",
            chat.title, chat.unread_count, preview
        ))
    });

    let list =
        List::new(items.collect::<Vec<_>>()).block(Block::default().borders(Borders::ALL).title("Main Screen"));
    frame.render_widget(list, area);
}

fn draw_login(frame: &mut Frame, area: Rect) {
    let text = vec![
        Line::from("Telegram login placeholder"),
        Line::from(""),
        Line::from("Planned flow:"),
        Line::from("1. Enter phone number"),
        Line::from("2. Confirm code"),
        Line::from("3. Persist session via secure handle"),
    ];

    let paragraph = Paragraph::new(text).block(Block::default().borders(Borders::ALL).title("Login"));
    frame.render_widget(paragraph, area);
}

fn draw_messenger(frame: &mut Frame, area: Rect, chats: &[ChatSummary], messages: &[Message]) {
    let columns = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(35), Constraint::Percentage(65)])
        .split(area);

    let chats = chats.iter().map(|chat| {
        let marker = if chat.unread_count > 0 { "*" } else { " " };
        ListItem::new(format!("{marker} {}", chat.title))
    });
    let chat_list =
        List::new(chats.collect::<Vec<_>>()).block(Block::default().borders(Borders::ALL).title("Dialogs"));
    frame.render_widget(chat_list, columns[0]);

    let message_lines = messages
        .iter()
        .map(|message| format!("{}: {}", message.author_name, message.text))
        .collect::<Vec<_>>()
        .join("\n\n");
    let thread = Paragraph::new(message_lines).block(Block::default().borders(Borders::ALL).title("Thread"));
    frame.render_widget(thread, columns[1]);
}
