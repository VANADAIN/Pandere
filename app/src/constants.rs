use std::time::Duration;

pub const APP_TITLE: &str = "Pandere Host Bridge";
pub const FALLBACK_COMPONENT_LABEL: &str = "embedded dummy component";

pub const TELEGRAM_FETCH_DIALOG_LIMIT: usize = 500;
pub const TELEGRAM_FETCH_MESSAGE_LIMIT: usize = 50;
pub const TELEGRAM_FETCH_FORUM_TOPIC_LIMIT: usize = 500;
pub const DATABASE_THREAD_LOAD_LIMIT: usize = 200;
pub const LOG_SCREEN_BUFFER_LIMIT: usize = 500;

pub const EVENT_POLL_INTERVAL: Duration = Duration::from_millis(50);
pub const PREVIEW_FETCH_DEBOUNCE: Duration = Duration::from_millis(150);
pub const INITIAL_BACKGROUND_SYNC_DELAY: Duration = Duration::from_secs(15);
pub const BACKGROUND_SYNC_INTERVAL: Duration = Duration::from_secs(20);

pub const TELEGRAM_ENV_NOTICE: &str =
    "telegram env is not configured; set TELEGRAM_API_ID, TELEGRAM_API_HASH, TELEGRAM_PHONE";

pub const COMPOSER_PLACEHOLDER: &str = "Compose: press c";
pub const NO_CONVERSATION_SELECTED: &str = "No conversation selected";

pub const TELEGRAM_PLUGIN_VERSION: &str = "0.1.0-spike";
pub const SLACK_PLUGIN_VERSION: &str = "0.1.0-plan";
