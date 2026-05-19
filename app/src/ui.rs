use pandere_core::{ChatSummary, Message};
use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    text::Line,
    widgets::{Block, Borders, List, ListItem, Paragraph},
};

use crate::app::Screen;
use crate::state::MessengerOverview;

pub fn draw_app(
    frame: &mut Frame,
    screen: Screen,
    messenger_overviews: &[MessengerOverview],
    chats: &[ChatSummary],
    messages: &[Message],
    login_lines: &[String],
) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(1),
            Constraint::Length(2),
        ])
        .split(frame.area());

    let title = Paragraph::new(Line::from("Pandere Host Bridge"))
        .block(Block::default().borders(Borders::ALL).title("App"))
        .style(Style::default().add_modifier(Modifier::BOLD));
    frame.render_widget(title, chunks[0]);

    match screen {
        Screen::Main => draw_main(frame, chunks[1], messenger_overviews),
        Screen::Login => draw_login(frame, chunks[1], login_lines),
        Screen::Messenger => draw_messenger(frame, chunks[1], chats, messages),
    }

    let footer = Paragraph::new("1 Main  2 Login  3 Messenger  q Quit")
        .block(Block::default().borders(Borders::ALL).title("Keys"));
    frame.render_widget(footer, chunks[2]);
}

fn draw_main(frame: &mut Frame, area: Rect, overviews: &[MessengerOverview]) {
    let items = overviews.iter().map(|messenger| {
        let preview = messenger
            .last_message_preview
            .as_deref()
            .unwrap_or("No messages yet");
        ListItem::new(format!(
            "{} ({:?})  [{} unread]  auth={}  sync={}  plugin={}  {}",
            messenger.display_name,
            messenger.service,
            messenger.unread_count,
            messenger.auth_label,
            messenger.sync_label,
            messenger.plugin_status_label,
            preview
        ))
    });

    let list =
        List::new(items.collect::<Vec<_>>()).block(Block::default().borders(Borders::ALL).title("Main Screen"));
    frame.render_widget(list, area);
}

fn draw_login(frame: &mut Frame, area: Rect, login_lines: &[String]) {
    let text = login_lines
        .iter()
        .cloned()
        .map(Line::from)
        .collect::<Vec<_>>();

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
