use std::time::SystemTime;

use anyhow::Result;
use pandere_core::{ChatId, ChatSummary, Message, MessageId, Service};

use crate::data_source::{AuthStatus, MessengerDataSource, MessengerSnapshot, SyncStatus};
use crate::plugin::{LoadedMessenger, PluginLoadStatus};

pub fn fixture_chats() -> Vec<ChatSummary> {
    vec![
        ChatSummary {
            id: ChatId::new("telegram:team"),
            service: Service::Telegram,
            title: "Pandere Core".into(),
            last_message_preview: Some("WIT draft small. Good.".into()),
            unread_count: 3,
            last_activity_at: Some(SystemTime::now()),
            has_subchats: false,
        },
        ChatSummary {
            id: ChatId::new("telegram:ops"),
            service: Service::Telegram,
            title: "Release Ops".into(),
            last_message_preview: Some("Need signed plugin artifacts next.".into()),
            unread_count: 0,
            last_activity_at: Some(SystemTime::now()),
            has_subchats: false,
        },
        ChatSummary {
            id: ChatId::new("telegram:personal"),
            service: Service::Telegram,
            title: "Saved Messages".into(),
            last_message_preview: Some("Telegram login spike after shell.".into()),
            unread_count: 1,
            last_activity_at: Some(SystemTime::now()),
            has_subchats: false,
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

pub struct FixtureMessengerSource;

impl MessengerDataSource for FixtureMessengerSource {
    fn snapshot(&self) -> Result<MessengerSnapshot> {
        let chats = fixture_chats();
        let messages = fixture_messages(&chats);

        Ok(MessengerSnapshot {
            service: Service::Telegram,
            auth_status: AuthStatus::NeedsLogin,
            sync_status: SyncStatus::Pending,
            chats,
            messages,
        })
    }
}

pub struct HostBackedFixtureSource {
    messenger: LoadedMessenger,
}

impl HostBackedFixtureSource {
    pub fn new(messenger: LoadedMessenger) -> Self {
        Self { messenger }
    }
}

impl MessengerDataSource for HostBackedFixtureSource {
    fn snapshot(&self) -> Result<MessengerSnapshot> {
        let chats = fixture_chats();
        let messages = fixture_messages(&chats);

        let (auth_status, sync_status) = match &self.messenger.status {
            PluginLoadStatus::Loaded => (
                AuthStatus::NeedsLogin,
                SyncStatus::Pending,
            ),
            PluginLoadStatus::Failed(message) => (
                AuthStatus::Unavailable(message.clone()),
                SyncStatus::Failed(message.clone()),
            ),
            PluginLoadStatus::Disabled => (
                AuthStatus::Unavailable("plugin disabled".into()),
                SyncStatus::Failed("plugin disabled".into()),
            ),
        };

        Ok(MessengerSnapshot {
            service: self.messenger.manifest.service,
            auth_status,
            sync_status,
            chats,
            messages,
        })
    }
}
