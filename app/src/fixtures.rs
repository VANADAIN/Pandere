use std::time::SystemTime;

use pandere_core::{ChatId, ChatSummary, Message, MessageId, Service};

pub fn fixture_chats() -> Vec<ChatSummary> {
    vec![
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
    ]
}

pub fn fixture_messages(chats: &[ChatSummary]) -> Vec<Message> {
    let primary_chat = chats[0].id.clone();

    vec![
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
    ]
}
