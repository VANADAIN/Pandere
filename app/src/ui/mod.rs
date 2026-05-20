mod messenger;

use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, Paragraph},
};

use crate::state::{AppState, ChatPreview, PluginCard};
use crate::{app::Screen, constants::APP_TITLE, logs::LogEntry};

pub(super) fn base_text_style() -> Style {
    Style::default().fg(Color::Gray)
}

fn emph_text_style() -> Style {
    base_text_style().add_modifier(Modifier::BOLD)
}

fn telegram_border_style() -> Style {
    Style::default().fg(Color::LightBlue)
}

fn default_border_style() -> Style {
    Style::default().fg(Color::DarkGray)
}

pub(super) fn muted_text_style() -> Style {
    Style::default().fg(Color::DarkGray)
}

fn block_for_screen(screen: Screen, title: impl Into<String>) -> Block<'static> {
    let title = title.into();
    let border_style = match screen {
        Screen::Login | Screen::Messenger => telegram_border_style(),
        _ => default_border_style(),
    };
    Block::default()
        .borders(Borders::ALL)
        .title(title)
        .border_style(border_style)
        .title_style(border_style.add_modifier(Modifier::BOLD))
}

pub(super) fn messenger_pane_block(
    title: impl Into<String>,
    is_active: bool,
    is_telegram: bool,
) -> Block<'static> {
    let border_style = if is_active {
        if is_telegram {
            telegram_border_style()
        } else {
            Style::default().fg(Color::Gray)
        }
    } else {
        default_border_style()
    };

    Block::default()
        .borders(Borders::ALL)
        .title(title.into())
        .border_style(border_style)
        .title_style(border_style.add_modifier(Modifier::BOLD))
}

fn log_level_style(level: tracing::Level) -> Style {
    match level {
        tracing::Level::ERROR => Style::default().fg(Color::LightRed),
        tracing::Level::WARN => Style::default().fg(Color::Yellow),
        tracing::Level::INFO => Style::default().fg(Color::LightGreen),
        tracing::Level::DEBUG => Style::default().fg(Color::LightCyan),
        tracing::Level::TRACE => Style::default().fg(Color::DarkGray),
    }
}

pub fn draw_app(frame: &mut Frame, state: &AppState, log_lines: &[LogEntry]) {
    let screen = state.screen;
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(1),
            Constraint::Length(2),
        ])
        .split(frame.area());

    let title = Paragraph::new(Line::from(APP_TITLE))
        .block(block_for_screen(screen, "App"))
        .style(emph_text_style());
    frame.render_widget(title, chunks[0]);

    match screen {
        Screen::Main => draw_main(
            frame,
            chunks[1],
            &state.plugin_cards(),
            &state.chat_previews(),
        ),
        Screen::Login => draw_login(frame, chunks[1], &state.login_lines()),
        Screen::Messenger => messenger::draw_messenger(frame, chunks[1], state),
        Screen::Logs => draw_logs(frame, chunks[1], log_lines),
    }

    let footer = Paragraph::new("0 Logs  1 Main  2 Login  3 Messenger  Left Back  Right Enter Chat  Up/Down Select or Scroll  c Compose  Enter Submit  Esc Cancel  q Quit")
        .block(block_for_screen(screen, "Keys"))
        .style(base_text_style());
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
        .block(block_for_screen(Screen::Main, "Plugin Registry"))
        .style(base_text_style());
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
        .block(block_for_screen(Screen::Main, "Chat Preview"))
        .style(base_text_style());
    frame.render_widget(chat_list, columns[1]);
}

fn draw_login(frame: &mut Frame, area: Rect, login_lines: &[String]) {
    let text = login_lines
        .iter()
        .cloned()
        .map(Line::from)
        .collect::<Vec<_>>();

    let paragraph = Paragraph::new(text)
        .block(block_for_screen(Screen::Login, "Login"))
        .style(base_text_style());
    frame.render_widget(paragraph, area);
}

fn draw_logs(frame: &mut Frame, area: Rect, log_lines: &[LogEntry]) {
    let lines = if log_lines.is_empty() {
        vec![Line::from("No logs yet")]
    } else {
        log_lines
            .iter()
            .map(|entry| {
                Line::from(Span::styled(
                    entry.line.clone(),
                    log_level_style(entry.level),
                ))
            })
            .collect::<Vec<_>>()
    };
    let visible_height = area.height.saturating_sub(2) as usize;
    let scroll = lines.len().saturating_sub(visible_height) as u16;
    let logs = Paragraph::new(lines)
        .block(block_for_screen(Screen::Logs, "Logs"))
        .style(base_text_style())
        .scroll((scroll, 0));
    frame.render_widget(logs, area);
}
