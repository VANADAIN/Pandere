use chrono::{DateTime, Local};
use pandere_core::{Message, MessageDeliveryState};
use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, Paragraph},
};
use unicode_width::UnicodeWidthChar;

use crate::{app::Screen, logs::LogEntry};
use crate::state::{AppState, ChatPreview, MessengerFocus, MessengerView, PluginCard};

fn base_text_style() -> Style {
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

fn muted_text_style() -> Style {
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

fn messenger_pane_block(
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

    let title = Paragraph::new(Line::from("Pandere Host Bridge"))
        .block(block_for_screen(screen, "App"))
        .style(emph_text_style());
    frame.render_widget(title, chunks[0]);

    match screen {
        Screen::Main => draw_main(frame, chunks[1], &state.plugin_cards(), &state.chat_previews()),
        Screen::Login => draw_login(frame, chunks[1], &state.login_lines()),
        Screen::Messenger => {
            draw_messenger(
                frame,
                chunks[1],
                state,
            )
        }
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

fn draw_messenger(
    frame: &mut Frame,
    area: Rect,
    state: &AppState,
) {
    let columns = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(35), Constraint::Percentage(65)])
        .split(area);

    let (left_title, left_items): (String, Vec<ListItem>) = match &state.messenger_view {
        MessengerView::Root => {
            let root_chats = state.root_chats();
            let items = root_chats
                .iter()
                .enumerate()
                .map(|(index, chat)| {
                    let thread_marker = if chat.has_subchats { ">" } else { " " };
                    let unread_marker = if chat.unread_count > 0 { "*" } else { " " };
                    let selected_marker =
                        if Some(index) == state.selected_root_chat_index() { ">" } else { " " };
                    let kind_marker = root_chat_kind_marker(chat);
                    ListItem::new(format!(
                        "{selected_marker}{thread_marker}{unread_marker} [{kind_marker}] {}",
                        chat.title,
                    ))
                })
                .collect();
            ("Chats".into(), items)
        }
        MessengerView::GroupThreads { .. } => {
            let items = state
                .thread_chats()
                .iter()
                .enumerate()
                .map(|(index, chat)| {
                    let title = state
                        .selected_root_chat()
                        .map(|root| {
                            chat.title
                                .strip_prefix(&(root.title.clone() + " / "))
                                .unwrap_or(chat.title.as_str())
                                .to_owned()
                        })
                        .unwrap_or_else(|| chat.title.clone());
                    let unread_marker = if chat.unread_count > 0 { "*" } else { " " };
                    let selected_marker =
                        if Some(index) == state.selected_thread_chat_index() { ">" } else { " " };
                    ListItem::new(format!("{selected_marker}{unread_marker} {title}"))
                })
                .collect();
            (
                state
                    .selected_root_chat()
                    .map(|root| format!("Threads in {}", root.title))
                    .unwrap_or_else(|| "Threads".into()),
                items,
            )
        }
    };
    let left_list = List::new(left_items)
        .block(messenger_pane_block(
            left_title,
            state.messenger_focus == MessengerFocus::Left,
            true,
        ))
        .style(base_text_style());
    frame.render_widget(left_list, columns[0]);

    let messages = state.messages();
    let thread_status_label = state.thread_status_label();
    let right_block_title = if state.is_inside_group_threads() {
        state
            .selected_leaf_chat()
            .map(|chat| format!("Chat: {} ({thread_status_label})", chat.title))
            .unwrap_or_else(|| format!("Chat ({thread_status_label})"))
    } else if state.can_focus_right_pane() {
        state
            .selected_leaf_chat()
            .map(|chat| format!("Chat: {} ({thread_status_label})", chat.title))
            .unwrap_or_else(|| format!("Chat ({thread_status_label})"))
    } else {
        state.thread_column_title()
    };

    let content_width = columns[1].width.saturating_sub(2) as usize;
    let mut right_lines = if state.is_inside_group_threads() || !state.selected_root_has_threads() {
        if messages.is_empty() {
            let leaf_title = state
                .selected_leaf_chat()
                .map(|chat| chat.title.clone())
                .unwrap_or_else(|| "No conversation selected".into());
            vec![
                Line::from(leaf_title),
                Line::from(String::new()),
                Line::from(format!("Thread status: {thread_status_label}")),
            ]
        } else {
            messages
                .iter()
                .flat_map(|message| render_message_lines(message, content_width))
                .collect::<Vec<_>>()
        }
    } else {
        let thread_chats = state.thread_chats();
        if thread_chats.is_empty() {
            vec![Line::from(state.thread_placeholder())]
        } else {
            thread_chats
                .iter()
                .enumerate()
                .flat_map(|(index, chat)| {
                    let title = state
                        .selected_root_chat()
                        .map(|root| {
                            chat.title
                                .strip_prefix(&(root.title.clone() + " / "))
                                .unwrap_or(chat.title.as_str())
                                .to_owned()
                        })
                        .unwrap_or_else(|| chat.title.clone());
                    let selected_marker =
                        if Some(index) == state.selected_thread_chat_index() { ">" } else { " " };
                    let preview = chat
                        .last_message_preview
                        .as_deref()
                        .unwrap_or("No messages yet");
                    [
                        Line::from(format!("{selected_marker} {title}")),
                        Line::from(format!("  {preview}")),
                        Line::from(String::new()),
                    ]
                })
                .collect::<Vec<_>>()
        }
    };

    if state.is_inside_group_threads() || !state.selected_root_has_threads() {
        right_lines.push(Line::from(String::new()));
        right_lines.push(Line::from(if state.composer_active {
            format!("Compose: {}", state.composer_input)
        } else {
            "Compose: press c".to_owned()
        }));
        if let Some(notice) = state.composer_notice.as_deref() {
            right_lines.push(Line::from(format!("Notice: {notice}")));
        }
    }

    let visible_height = columns[1].height.saturating_sub(2) as usize;
    let auto_bottom_scroll = right_lines.len().saturating_sub(visible_height) as u16;
    let scroll = state.effective_thread_scroll(auto_bottom_scroll);
    let right = Paragraph::new(right_lines)
        .block(messenger_pane_block(
            right_block_title,
            state.messenger_focus == MessengerFocus::Right,
            true,
        ))
        .style(base_text_style())
        .scroll((scroll, 0));
    frame.render_widget(right, columns[1]);
}

fn render_message_lines(message: &Message, content_width: usize) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    let timestamp = format_message_timestamp(message);
    lines.push(Line::from(vec![
        Span::styled(
            message.author_name.clone(),
            base_text_style().add_modifier(Modifier::BOLD),
        ),
        Span::styled(format!("  {timestamp}"), muted_text_style()),
        Span::styled(delivery_suffix(message), delivery_style(message)),
    ]));

    if message.text.is_empty() {
        lines.push(Line::from("  "));
    } else {
        let body_width = content_width.saturating_sub(2).max(1);
        for line in message.text.lines() {
            let wrapped = wrap_plain_text(line, body_width);
            if wrapped.is_empty() {
                lines.push(Line::from("  "));
            } else {
                for wrapped_line in wrapped {
                    lines.push(Line::from(format!("  {wrapped_line}")));
                }
            }
        }
    }
    lines.push(Line::from(Span::styled(
        "  ┈┈┈",
        muted_text_style(),
    )));
    lines.push(Line::from(""));
    lines
}

fn delivery_suffix(message: &Message) -> String {
    match message.delivery_state {
        MessageDeliveryState::Sent => String::new(),
        MessageDeliveryState::Sending => "  [sending]".into(),
        MessageDeliveryState::Failed => "  [failed]".into(),
    }
}

fn delivery_style(message: &Message) -> Style {
    match message.delivery_state {
        MessageDeliveryState::Sent => muted_text_style(),
        MessageDeliveryState::Sending => Style::default().fg(Color::Yellow),
        MessageDeliveryState::Failed => Style::default().fg(Color::LightRed),
    }
}

fn format_message_timestamp(message: &Message) -> String {
    let local_time: DateTime<Local> = message.sent_at.into();
    let now = Local::now().date_naive();
    if local_time.date_naive() == now {
        local_time.format("%H:%M").to_string()
    } else {
        local_time.format("%Y-%m-%d %H:%M").to_string()
    }
}

fn wrap_plain_text(text: &str, width: usize) -> Vec<String> {
    if width == 0 {
        return vec![String::new()];
    }

    if text.is_empty() {
        return vec![String::new()];
    }

    let mut lines = Vec::new();
    let mut current = String::new();
    let mut current_width = 0usize;

    for ch in text.chars() {
        let ch_width = UnicodeWidthChar::width(ch).unwrap_or(0).max(1);
        if current_width > 0 && current_width + ch_width > width {
            lines.push(current);
            current = String::new();
            current_width = 0;
        }
        current.push(ch);
        current_width += ch_width;
    }

    if !current.is_empty() {
        lines.push(current);
    }

    if lines.is_empty() {
        vec![String::new()]
    } else {
        lines
    }
}

fn root_chat_kind_marker(chat: &pandere_core::ChatSummary) -> &'static str {
    if chat.has_subchats {
        return "FOR";
    }

    let Some(raw) = chat.id.as_str().strip_prefix("telegram:") else {
        return "CHT";
    };
    let Some(dialog_id) = raw.split(':').next().and_then(|id| id.parse::<i64>().ok()) else {
        return "CHT";
    };

    if dialog_id > 0 {
        "USR"
    } else if dialog_id <= -1_000_000_000_001 {
        "CHN"
    } else {
        "GRP"
    }
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
