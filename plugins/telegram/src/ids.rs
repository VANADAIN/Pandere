use anyhow::{Context, Result, anyhow};
use grammers_session::types::PeerId;
use pandere_core::ChatId;

pub(crate) fn dialog_chat_id(dialog_id: i64) -> ChatId {
    ChatId::new(format!("telegram:{dialog_id}"))
}

pub(crate) fn forum_topic_chat_id(dialog_id: i64, topic_id: i32) -> ChatId {
    ChatId::new(format!("telegram:{dialog_id}:topic:{topic_id}"))
}

pub(crate) fn peer_id_from_dialog_id(dialog_id: i64) -> Option<PeerId> {
    if dialog_id == 0 {
        return None;
    }

    Some(match dialog_id {
        1..=0xffffffffff => PeerId::user_unchecked(dialog_id),
        -999999999999..=-1 => PeerId::chat_unchecked(-dialog_id),
        _ => {
            let bare_id = -dialog_id - 1_000_000_000_000;
            if bare_id <= 0 {
                return None;
            }
            PeerId::channel_unchecked(bare_id)
        }
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TelegramChatTarget {
    Dialog { dialog_id: i64 },
    ForumTopic { dialog_id: i64, top_message: i32 },
}

pub(crate) fn parse_chat_target(chat_id: &ChatId) -> Result<TelegramChatTarget> {
    let raw = chat_id.as_str();
    let Some(rest) = raw.strip_prefix("telegram:") else {
        return Err(anyhow!("unsupported telegram chat id `{raw}`"));
    };

    if let Some((dialog_id, top_message)) = rest.split_once(":topic:") {
        return Ok(TelegramChatTarget::ForumTopic {
            dialog_id: dialog_id
                .parse()
                .with_context(|| format!("invalid telegram dialog id in `{raw}`"))?,
            top_message: top_message
                .parse()
                .with_context(|| format!("invalid telegram topic id in `{raw}`"))?,
        });
    }

    Ok(TelegramChatTarget::Dialog {
        dialog_id: rest
            .parse()
            .with_context(|| format!("invalid telegram dialog id in `{raw}`"))?,
    })
}

pub(crate) fn parse_dialog_id(chat_id: &str) -> Option<i64> {
    let target = parse_chat_target(&ChatId::new(chat_id)).ok()?;
    Some(match target {
        TelegramChatTarget::Dialog { dialog_id } => dialog_id,
        TelegramChatTarget::ForumTopic { dialog_id, .. } => dialog_id,
    })
}
