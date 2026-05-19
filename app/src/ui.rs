use pandere_core::{ChatSummary, Message};
use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    text::Line,
    widgets::{Block, Borders, List, ListItem, Paragraph},
};

use crate::app::Screen;
use crate::state::{ChatPreview, PluginCard};

pub fn draw_app(
    frame: &mut Frame,
    screen: Screen,
    plugin_cards: &[PluginCard],
    chat_previews: &[ChatPreview],
    chats: &[ChatSummary],
    messages: &[Message],
    login_lines: &[String],
    selected_chat_index: Option<usize>,
    thread_status_label: &str,
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
        Screen::Main => draw_main(frame, chunks[1], plugin_cards, chat_previews),
        Screen::Login => draw_login(frame, chunks[1], login_lines),
        Screen::Messenger => {
            draw_messenger(
                frame,
                chunks[1],
                chats,
                messages,
                selected_chat_index,
                thread_status_label,
            )
        }
    }

    let footer = Paragraph::new("1 Main  2 Login  3 Messenger  Enter Next/Submit  r Refresh Code  x Logout  Up/Down Chat  q Quit")
        .block(Block::default().borders(Borders::ALL).title("Keys"));
    frame.render_widget(footer, chunks[2]);
}

fn draw_main(
    frame: &mut Frame,
    area: Rect,
    plugin_cards: &[PluginCard],
    chat_previews: &[ChatPreview],
) {
    let columns = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(45), Constraint::Percentage(55)])
        .split(area);

    let plugin_items = plugin_cards.iter().map(|plugin| {
        ListItem::new(vec![
            Line::from(format!(
                "{} v{} ({:?})",
                plugin.display_name, plugin.version, plugin.service
            )),
            Line::from(format!(
                "enabled={}  auth={}  sync={}",
                plugin.enabled, plugin.auth_label, plugin.sync_label
            )),
            Line::from(format!(
                "plugin={}  component={}",
                plugin.plugin_status_label, plugin.component_label
            )),
        ])
    });
    let plugin_list = List::new(plugin_items.collect::<Vec<_>>())
        .block(Block::default().borders(Borders::ALL).title("Plugin Registry"));
    frame.render_widget(plugin_list, columns[0]);

    let chat_items = chat_previews.iter().map(|chat| {
        let preview = chat
            .last_message_preview
            .as_deref()
            .unwrap_or("No messages yet");
        ListItem::new(format!(
            "{}  [{} unread]  {}",
            chat.title, chat.unread_count, preview
        ))
    });
    let chat_list = List::new(chat_items.collect::<Vec<_>>())
        .block(Block::default().borders(Borders::ALL).title("Chat Preview"));
    frame.render_widget(chat_list, columns[1]);
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

fn draw_messenger(
    frame: &mut Frame,
    area: Rect,
    chats: &[ChatSummary],
    messages: &[Message],
    selected_chat_index: Option<usize>,
    thread_status_label: &str,
) {
    let columns = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(35), Constraint::Percentage(65)])
        .split(area);

    let chats = chats.iter().enumerate().map(|(index, chat)| {
        let unread_marker = if chat.unread_count > 0 { "*" } else { " " };
        let selected_marker = if Some(index) == selected_chat_index { ">" } else { " " };
        ListItem::new(format!("{selected_marker}{unread_marker} {}", chat.title))
    });
    let chat_list =
        List::new(chats.collect::<Vec<_>>()).block(Block::default().borders(Borders::ALL).title("Dialogs"));
    frame.render_widget(chat_list, columns[0]);

    let message_lines = if messages.is_empty() {
        vec![Line::from(format!("Thread status: {thread_status_label}"))]
    } else {
        messages
            .iter()
            .flat_map(|message| {
                [
                    Line::from(format!("{}: {}", message.author_name, message.text)),
                    Line::from(String::new()),
                ]
            })
            .collect::<Vec<_>>()
    };
    let visible_height = columns[1].height.saturating_sub(2) as usize;
    let scroll = message_lines.len().saturating_sub(visible_height) as u16;
    let thread = Paragraph::new(message_lines).block(
        Block::default()
            .borders(Borders::ALL)
            .title(format!("Thread ({thread_status_label})")),
    )
    .scroll((scroll, 0));
    frame.render_widget(thread, columns[1]);
}
