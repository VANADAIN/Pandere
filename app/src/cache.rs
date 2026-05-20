use std::io;
use std::path::Path;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use pandere_core::paths::pandere_paths;
use pandere_core::{ChatId, ChatSummary, Message, MessageDeliveryState, MessageId, Service};
use rusqlite::{Connection, OptionalExtension, params};

use crate::data_source::{AuthStatus, MessengerSnapshot, SyncStatus};

const MESSAGE_CACHE_LIMIT_PER_CHAT: i64 = 200;

pub struct CacheStore {
    connection: Connection,
}

impl CacheStore {
    pub fn open_default() -> Result<Self> {
        let paths = pandere_paths();
        Self::open(paths.database_path())
    }

    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref().to_path_buf();
        let connection = Connection::open(&path)
            .with_context(|| format!("failed to open cache database `{}`", path.display()))?;
        let store = Self { connection };
        store.initialize()?;
        Ok(store)
    }

    pub fn load_snapshot(&self, service: Service) -> Result<Option<MessengerSnapshot>> {
        let chats = self.load_chats(service)?;
        if chats.is_empty() {
            return Ok(None);
        }

        let messages = match chats.first() {
            Some(first_chat) => self.load_messages(&first_chat.id, MESSAGE_CACHE_LIMIT_PER_CHAT as usize)?,
            None => Vec::new(),
        };

        Ok(Some(MessengerSnapshot {
            service,
            auth_status: AuthStatus::NeedsLogin,
            sync_status: SyncStatus::Idle,
            chats,
            messages,
        }))
    }

    pub fn load_chats(&self, service: Service) -> Result<Vec<ChatSummary>> {
        let mut statement = self.connection.prepare(
            "SELECT chat_id, title, last_message_preview, unread_count, last_activity_at, has_subchats
             FROM chat_cache
             WHERE service = ?1
             ORDER BY COALESCE(last_activity_at, 0) DESC, title ASC",
        )?;

        let rows = statement.query_map(params![service_key(service)], |row| {
            Ok(ChatSummary {
                id: ChatId::new(row.get::<_, String>(0)?),
                service,
                title: row.get(1)?,
                last_message_preview: row.get(2)?,
                unread_count: row.get::<_, i64>(3)? as u32,
                last_activity_at: row
                    .get::<_, Option<i64>>(4)?
                    .map(timestamp_to_system_time),
                has_subchats: row.get::<_, i64>(5)? != 0,
            })
        })?;

        rows.collect::<std::result::Result<Vec<_>, _>>()
            .context("failed to load cached chats")
    }

    pub fn save_chats(&mut self, service: Service, chats: &[ChatSummary]) -> Result<()> {
        let transaction = self.connection.transaction()?;
        let now = now_timestamp();
        for chat in chats {
            transaction.execute(
                "INSERT INTO chat_cache (
                    service, chat_id, title, last_message_preview, unread_count, last_activity_at, has_subchats, updated_at
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
                 ON CONFLICT(chat_id) DO UPDATE SET
                    service = excluded.service,
                    title = excluded.title,
                    last_message_preview = excluded.last_message_preview,
                    unread_count = excluded.unread_count,
                    last_activity_at = excluded.last_activity_at,
                    has_subchats = excluded.has_subchats,
                    updated_at = excluded.updated_at",
                params![
                    service_key(service),
                    chat.id.as_str(),
                    chat.title,
                    chat.last_message_preview,
                    i64::from(chat.unread_count),
                    chat.last_activity_at.map(system_time_to_timestamp),
                    if chat.has_subchats { 1 } else { 0 },
                    now,
                ],
            )?;
        }
        transaction.commit()?;
        self.store_sync_marker("dialogs:last_sync_at", now.to_string())?;
        Ok(())
    }

    pub fn load_messages(&self, chat_id: &ChatId, limit: usize) -> Result<Vec<Message>> {
        let mut statement = self.connection.prepare(
            "SELECT message_id, service, author_name, body, sent_at, is_outgoing, delivery_state
             FROM message_cache
             WHERE chat_id = ?1
             ORDER BY sent_at DESC, rowid DESC
             LIMIT ?2",
        )?;

        let mut messages = statement
            .query_map(params![chat_id.as_str(), limit as i64], |row| {
                Ok(Message {
                    id: MessageId::new(row.get::<_, String>(0)?),
                    chat_id: chat_id.clone(),
                    service: service_from_key(&row.get::<_, String>(1)?)?,
                    author_name: row.get(2)?,
                    text: row.get(3)?,
                    sent_at: timestamp_to_system_time(row.get(4)?),
                    is_outgoing: row.get::<_, i64>(5)? != 0,
                    delivery_state: delivery_state_from_key(&row.get::<_, String>(6)?)?,
                })
            })?
            .collect::<std::result::Result<Vec<_>, _>>()
            .context("failed to load cached messages")?;

        messages.reverse();
        Ok(messages)
    }

    pub fn save_messages(&mut self, chat_id: &ChatId, messages: &[Message]) -> Result<()> {
        let transaction = self.connection.transaction()?;
        let now = now_timestamp();

        for message in messages {
            if message.delivery_state != MessageDeliveryState::Sent {
                continue;
            }

            transaction.execute(
                "INSERT INTO message_cache (
                    message_id, chat_id, service, author_name, body, sent_at, is_outgoing, delivery_state, cached_at
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
                 ON CONFLICT(message_id) DO UPDATE SET
                    author_name = excluded.author_name,
                    body = excluded.body,
                    sent_at = excluded.sent_at,
                    is_outgoing = excluded.is_outgoing,
                    delivery_state = excluded.delivery_state,
                    cached_at = excluded.cached_at",
                params![
                    message.id.as_str(),
                    chat_id.as_str(),
                    service_key(message.service),
                    message.author_name,
                    message.text,
                    system_time_to_timestamp(message.sent_at),
                    if message.is_outgoing { 1 } else { 0 },
                    delivery_state_key(&message.delivery_state),
                    now,
                ],
            )?;
        }

        transaction.execute(
            "DELETE FROM message_cache
             WHERE chat_id = ?1
               AND message_id NOT IN (
                    SELECT message_id
                    FROM message_cache
                    WHERE chat_id = ?1
                    ORDER BY sent_at DESC, rowid DESC
                    LIMIT ?2
               )",
            params![chat_id.as_str(), MESSAGE_CACHE_LIMIT_PER_CHAT],
        )?;

        transaction.commit()?;
        Ok(())
    }

    pub fn load_draft(&self, chat_id: &ChatId) -> Result<Option<String>> {
        self.connection
            .query_row(
                "SELECT body FROM draft_cache WHERE chat_id = ?1",
                params![chat_id.as_str()],
                |row| row.get(0),
            )
            .optional()
            .context("failed to load cached draft")
    }

    pub fn save_draft(&mut self, chat_id: &ChatId, body: &str) -> Result<()> {
        let body = body.trim_end();
        if body.is_empty() {
            self.clear_draft(chat_id)?;
            return Ok(());
        }

        self.connection.execute(
            "INSERT INTO draft_cache (chat_id, body, updated_at)
             VALUES (?1, ?2, ?3)
             ON CONFLICT(chat_id) DO UPDATE SET
                body = excluded.body,
                updated_at = excluded.updated_at",
            params![chat_id.as_str(), body, now_timestamp()],
        )?;
        Ok(())
    }

    pub fn clear_draft(&mut self, chat_id: &ChatId) -> Result<()> {
        self.connection.execute(
            "DELETE FROM draft_cache WHERE chat_id = ?1",
            params![chat_id.as_str()],
        )?;
        Ok(())
    }

    pub fn store_sync_marker(&self, key: &str, value: String) -> Result<()> {
        self.connection.execute(
            "INSERT INTO sync_meta (key, value, updated_at)
             VALUES (?1, ?2, ?3)
             ON CONFLICT(key) DO UPDATE SET
                value = excluded.value,
                updated_at = excluded.updated_at",
            params![key, value, now_timestamp()],
        )?;
        Ok(())
    }

    fn initialize(&self) -> Result<()> {
        self.connection.execute_batch(
            "
            PRAGMA journal_mode = WAL;
            PRAGMA synchronous = NORMAL;
            CREATE TABLE IF NOT EXISTS chat_cache (
                service TEXT NOT NULL,
                chat_id TEXT PRIMARY KEY,
                title TEXT NOT NULL,
                last_message_preview TEXT,
                unread_count INTEGER NOT NULL,
                last_activity_at INTEGER,
                has_subchats INTEGER NOT NULL DEFAULT 0,
                updated_at INTEGER NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_chat_cache_service_activity
                ON chat_cache(service, last_activity_at DESC);

            CREATE TABLE IF NOT EXISTS message_cache (
                message_id TEXT PRIMARY KEY,
                chat_id TEXT NOT NULL,
                service TEXT NOT NULL,
                author_name TEXT NOT NULL,
                body TEXT NOT NULL,
                sent_at INTEGER NOT NULL,
                is_outgoing INTEGER NOT NULL,
                delivery_state TEXT NOT NULL,
                cached_at INTEGER NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_message_cache_chat_sent
                ON message_cache(chat_id, sent_at DESC);

            CREATE TABLE IF NOT EXISTS draft_cache (
                chat_id TEXT PRIMARY KEY,
                body TEXT NOT NULL,
                updated_at INTEGER NOT NULL
            );

            CREATE TABLE IF NOT EXISTS sync_meta (
                key TEXT PRIMARY KEY,
                value TEXT NOT NULL,
                updated_at INTEGER NOT NULL
            );
            ",
        )?;
        Ok(())
    }
}

fn service_key(service: Service) -> &'static str {
    match service {
        Service::Telegram => "telegram",
    }
}

fn service_from_key(value: &str) -> rusqlite::Result<Service> {
    match value {
        "telegram" => Ok(Service::Telegram),
        _ => Err(rusqlite::Error::FromSqlConversionFailure(
            0,
            rusqlite::types::Type::Text,
            Box::new(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("unknown service key `{value}`"),
            )),
        )),
    }
}

fn delivery_state_key(state: &MessageDeliveryState) -> &'static str {
    match state {
        MessageDeliveryState::Sent => "sent",
        MessageDeliveryState::Sending => "sending",
        MessageDeliveryState::Failed => "failed",
    }
}

fn delivery_state_from_key(value: &str) -> rusqlite::Result<MessageDeliveryState> {
    match value {
        "sent" => Ok(MessageDeliveryState::Sent),
        "sending" => Ok(MessageDeliveryState::Sending),
        "failed" => Ok(MessageDeliveryState::Failed),
        _ => Err(rusqlite::Error::FromSqlConversionFailure(
            0,
            rusqlite::types::Type::Text,
            Box::new(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("unknown delivery state `{value}`"),
            )),
        )),
    }
}

fn now_timestamp() -> i64 {
    system_time_to_timestamp(SystemTime::now())
}

fn system_time_to_timestamp(value: SystemTime) -> i64 {
    value
        .duration_since(UNIX_EPOCH)
        .unwrap_or_else(|_| Duration::from_secs(0))
        .as_secs() as i64
}

fn timestamp_to_system_time(timestamp: i64) -> SystemTime {
    if timestamp <= 0 {
        UNIX_EPOCH
    } else {
        UNIX_EPOCH + Duration::from_secs(timestamp as u64)
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    fn temp_db_path(name: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_else(|_| Duration::from_secs(0))
            .as_nanos();
        std::env::temp_dir().join(format!("pandere-{name}-{unique}.sqlite3"))
    }

    fn sample_chat() -> ChatSummary {
        ChatSummary {
            id: ChatId::new("telegram:1"),
            service: Service::Telegram,
            title: "Saved Messages".into(),
            last_message_preview: Some("hello".into()),
            unread_count: 1,
            last_activity_at: Some(SystemTime::now()),
            has_subchats: false,
        }
    }

    fn sample_message(delivery_state: MessageDeliveryState) -> Message {
        Message {
            id: MessageId::new("telegram:1:100"),
            chat_id: ChatId::new("telegram:1"),
            service: Service::Telegram,
            author_name: "You".into(),
            text: "hello".into(),
            sent_at: SystemTime::now(),
            is_outgoing: true,
            delivery_state,
        }
    }

    #[test]
    fn cache_round_trip_persists_chats_and_messages() {
        let path = temp_db_path("round-trip");
        let mut store = CacheStore::open(&path).expect("cache db should open");
        let chat = sample_chat();
        let message = sample_message(MessageDeliveryState::Sent);

        store
            .save_chats(Service::Telegram, std::slice::from_ref(&chat))
            .expect("chats should persist");
        store
            .save_messages(&chat.id, std::slice::from_ref(&message))
            .expect("messages should persist");

        let chats = store.load_chats(Service::Telegram).expect("chats should load");
        let messages = store.load_messages(&chat.id, 50).expect("messages should load");

        assert_eq!(chats.len(), 1);
        assert_eq!(chats[0].id.as_str(), chat.id.as_str());
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].id.as_str(), message.id.as_str());

        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn cache_ignores_non_sent_messages() {
        let path = temp_db_path("delivery-filter");
        let mut store = CacheStore::open(&path).expect("cache db should open");
        let chat = sample_chat();
        let message = sample_message(MessageDeliveryState::Sending);

        store
            .save_messages(&chat.id, std::slice::from_ref(&message))
            .expect("save should succeed");

        let messages = store.load_messages(&chat.id, 50).expect("messages should load");
        assert!(messages.is_empty());

        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn cache_round_trip_persists_drafts() {
        let path = temp_db_path("drafts");
        let mut store = CacheStore::open(&path).expect("cache db should open");
        let chat_id = ChatId::new("telegram:1");

        store
            .save_draft(&chat_id, "draft text")
            .expect("draft should persist");
        assert_eq!(
            store.load_draft(&chat_id).expect("draft should load"),
            Some("draft text".into())
        );

        store.clear_draft(&chat_id).expect("draft should clear");
        assert_eq!(store.load_draft(&chat_id).expect("draft should load"), None);

        let _ = std::fs::remove_file(path);
    }
}
