use std::collections::HashMap;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{Result, anyhow};
use grammers_client::peer::{Dialog, Peer, User};
use grammers_client::tl;
use pandere_core::{ChatId, ChatSummary, Message, MessageDeliveryState, MessageId, Service};

use crate::ids::{dialog_chat_id, forum_topic_chat_id};

pub(crate) fn user_label(user: &User) -> String {
    let full_name = user.full_name();
    if full_name.trim().is_empty() {
        "Telegram user".to_owned()
    } else {
        full_name
    }
}

pub(crate) fn map_dialog(dialog: Dialog) -> ChatSummary {
    let peer = dialog.peer();
    let title = peer
        .name()
        .map(str::to_owned)
        .unwrap_or_else(|| format!("Telegram {}", peer.id().bot_api_dialog_id()));
    let last_message_preview = dialog
        .last_message
        .as_ref()
        .map(|message| message.text().trim())
        .filter(|text| !text.is_empty())
        .map(str::to_owned);
    let last_activity_at = dialog
        .last_message
        .as_ref()
        .map(|message| timestamp_to_system_time(message.date().timestamp()));

    ChatSummary {
        id: dialog_chat_id(peer.id().bot_api_dialog_id()),
        service: Service::Telegram,
        title,
        last_message_preview,
        unread_count: dialog_unread_count(&dialog) as u32,
        last_activity_at,
        has_subchats: forum_enabled_group(peer),
    }
}

pub(crate) fn map_message(chat_id: ChatId, message: grammers_client::message::Message) -> Message {
    let author_name = message
        .sender()
        .and_then(peer_label)
        .or_else(|| message.peer().and_then(peer_label))
        .unwrap_or_else(|| {
            if message.outgoing() {
                "You".to_owned()
            } else {
                "Telegram".to_owned()
            }
        });

    Message {
        id: MessageId::new(format!("telegram:{}:{}", chat_id.as_str(), message.id())),
        chat_id,
        service: Service::Telegram,
        author_name,
        text: message.text().to_owned(),
        sent_at: timestamp_to_system_time(message.date().timestamp()),
        is_outgoing: message.outgoing(),
        delivery_state: MessageDeliveryState::Sent,
    }
}

pub(crate) fn map_forum_topic(
    parent_dialog_id: i64,
    topic: tl::enums::ForumTopic,
    messages: &[tl::enums::Message],
) -> Option<ChatSummary> {
    let tl::enums::ForumTopic::Topic(topic) = topic else {
        return None;
    };
    let topic_message = messages
        .iter()
        .find(|message| message.id() == topic.top_message);
    let last_message_preview = topic_message
        .and_then(raw_message_text)
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .map(str::to_owned);
    let last_activity_at = topic_message
        .map(raw_message_timestamp)
        .map(|timestamp| timestamp_to_system_time(timestamp as i64))
        .or_else(|| Some(timestamp_to_system_time(topic.date as i64)));

    Some(ChatSummary {
        id: forum_topic_chat_id(parent_dialog_id, topic.id),
        service: Service::Telegram,
        title: topic.title,
        last_message_preview,
        unread_count: topic.unread_count.max(0) as u32,
        last_activity_at,
        has_subchats: false,
    })
}

pub(crate) fn map_raw_message(
    chat_id: ChatId,
    raw: tl::enums::Message,
    author_names: &HashMap<String, String>,
) -> Message {
    let author_name = raw_message_sender_key(&raw)
        .and_then(|key| author_names.get(&key).cloned())
        .unwrap_or_else(|| {
            if raw_message_outgoing(&raw) {
                "You".to_owned()
            } else {
                "Telegram".to_owned()
            }
        });

    Message {
        id: MessageId::new(format!("telegram:{}:{}", chat_id.as_str(), raw.id())),
        chat_id,
        service: Service::Telegram,
        author_name,
        text: raw_message_text(&raw).unwrap_or_default().to_owned(),
        sent_at: timestamp_to_system_time(raw_message_timestamp(&raw) as i64),
        is_outgoing: raw_message_outgoing(&raw),
        delivery_state: MessageDeliveryState::Sent,
    }
}

pub(crate) fn build_author_names(
    users: Vec<tl::enums::User>,
    chats: Vec<tl::enums::Chat>,
) -> HashMap<String, String> {
    let mut author_names = HashMap::new();

    for user in users {
        let key = match &user {
            tl::enums::User::User(user) => format!("user:{}", user.id),
            tl::enums::User::Empty(user) => format!("user:{}", user.id),
        };
        let label = user_display_name(&user).unwrap_or_else(|| key.clone());
        author_names.insert(key, label);
    }

    for chat in chats {
        match &chat {
            tl::enums::Chat::Chat(chat) => {
                author_names.insert(format!("chat:{}", chat.id), chat.title.clone());
            }
            tl::enums::Chat::Forbidden(chat) => {
                author_names.insert(format!("chat:{}", chat.id), chat.title.clone());
            }
            tl::enums::Chat::Channel(channel) => {
                author_names.insert(format!("channel:{}", channel.id), channel.title.clone());
            }
            tl::enums::Chat::ChannelForbidden(channel) => {
                author_names.insert(format!("channel:{}", channel.id), channel.title.clone());
            }
            tl::enums::Chat::Empty(chat) => {
                author_names.insert(format!("chat:{}", chat.id), format!("Chat {}", chat.id));
            }
        }
    }

    author_names
}

pub(crate) fn unpack_messages_response(
    response: tl::enums::messages::Messages,
) -> Result<(
    Vec<tl::enums::Message>,
    Vec<tl::enums::User>,
    Vec<tl::enums::Chat>,
)> {
    let unpacked = match response {
        tl::enums::messages::Messages::Messages(messages) => {
            (messages.messages, messages.users, messages.chats)
        }
        tl::enums::messages::Messages::Slice(messages) => {
            (messages.messages, messages.users, messages.chats)
        }
        tl::enums::messages::Messages::ChannelMessages(messages) => {
            (messages.messages, messages.users, messages.chats)
        }
        tl::enums::messages::Messages::NotModified(_) => {
            return Err(anyhow!(
                "telegram returned an unexpected not-modified message slice"
            ));
        }
    };

    Ok(unpacked)
}

pub(crate) fn raw_message_id(raw: &tl::enums::Message) -> i32 {
    match raw {
        tl::enums::Message::Message(message) => message.id,
        tl::enums::Message::Service(message) => message.id,
        tl::enums::Message::Empty(message) => message.id,
    }
}

pub(crate) fn timestamp_to_system_time(timestamp: i64) -> SystemTime {
    if timestamp <= 0 {
        UNIX_EPOCH
    } else {
        UNIX_EPOCH + Duration::from_secs(timestamp as u64)
    }
}

fn raw_message_sender_key(raw: &tl::enums::Message) -> Option<String> {
    match raw {
        tl::enums::Message::Message(message) => message
            .from_id
            .as_ref()
            .map(peer_key)
            .or_else(|| Some(peer_key(&message.peer_id))),
        tl::enums::Message::Service(message) => message
            .from_id
            .as_ref()
            .map(peer_key)
            .or_else(|| Some(peer_key(&message.peer_id))),
        tl::enums::Message::Empty(_) => None,
    }
}

fn raw_message_text(raw: &tl::enums::Message) -> Option<&str> {
    match raw {
        tl::enums::Message::Message(message) => Some(message.message.as_str()),
        tl::enums::Message::Service(_) | tl::enums::Message::Empty(_) => None,
    }
}

fn raw_message_timestamp(raw: &tl::enums::Message) -> i32 {
    match raw {
        tl::enums::Message::Message(message) => message.date,
        tl::enums::Message::Service(message) => message.date,
        tl::enums::Message::Empty(_) => 0,
    }
}

fn raw_message_outgoing(raw: &tl::enums::Message) -> bool {
    match raw {
        tl::enums::Message::Message(message) => message.out,
        tl::enums::Message::Service(message) => message.out,
        tl::enums::Message::Empty(_) => false,
    }
}

fn user_display_name(user: &tl::enums::User) -> Option<String> {
    match user {
        tl::enums::User::User(user) => {
            let first = user.first_name.as_deref().unwrap_or_default().trim();
            let last = user.last_name.as_deref().unwrap_or_default().trim();
            let joined = format!("{first} {last}").trim().to_owned();
            if !joined.is_empty() {
                Some(joined)
            } else {
                user.username.clone()
            }
        }
        tl::enums::User::Empty(_) => None,
    }
}

fn forum_enabled_group(peer: &Peer) -> bool {
    match peer {
        Peer::Group(group) => {
            matches!(&group.raw, tl::enums::Chat::Channel(channel) if channel.forum)
        }
        Peer::User(_) | Peer::Channel(_) => false,
    }
}

fn peer_key(peer: &tl::enums::Peer) -> String {
    match peer {
        tl::enums::Peer::User(user) => format!("user:{}", user.user_id),
        tl::enums::Peer::Chat(chat) => format!("chat:{}", chat.chat_id),
        tl::enums::Peer::Channel(channel) => format!("channel:{}", channel.channel_id),
    }
}

fn peer_label(peer: &Peer) -> Option<String> {
    peer.name()
        .map(str::to_owned)
        .or_else(|| Some(peer.id().bot_api_dialog_id().to_string()))
}

fn dialog_unread_count(dialog: &Dialog) -> i32 {
    match &dialog.raw {
        grammers_client::tl::enums::Dialog::Dialog(raw) => raw.unread_count,
        grammers_client::tl::enums::Dialog::Folder(_) => 0,
    }
}
