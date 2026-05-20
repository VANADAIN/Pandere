use std::path::{Path, PathBuf};
use std::{fs, io};

use anyhow::{Context, Result, anyhow};
use pandere_core::paths::pandere_paths;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TelegramConfig {
    pub api_id: i32,
    pub api_hash: String,
    pub phone_number: String,
    pub session_path: PathBuf,
}

impl TelegramConfig {
    pub fn from_env() -> Result<Self> {
        let api_id = std::env::var("TELEGRAM_API_ID")
            .context("missing TELEGRAM_API_ID")?
            .parse()
            .context("invalid TELEGRAM_API_ID")?;
        let api_hash = std::env::var("TELEGRAM_API_HASH").context("missing TELEGRAM_API_HASH")?;
        let phone_number = std::env::var("TELEGRAM_PHONE").context("missing TELEGRAM_PHONE")?;
        let session_path = std::env::var("TELEGRAM_SESSION_PATH")
            .map(PathBuf::from)
            .unwrap_or_else(|_| default_session_path());

        Ok(Self {
            api_id,
            api_hash,
            phone_number,
            session_path,
        })
    }

    pub fn validate(&self) -> Result<()> {
        if self.api_id <= 0 {
            return Err(anyhow!("telegram api id must be positive"));
        }

        if self.api_hash.trim().is_empty() {
            return Err(anyhow!("telegram api hash must not be empty"));
        }

        if self.phone_number.trim().is_empty() {
            return Err(anyhow!("telegram phone number must not be empty"));
        }

        if self.session_path.as_os_str().is_empty() {
            return Err(anyhow!("telegram session path must not be empty"));
        }

        Ok(())
    }
}

pub fn default_session_path() -> PathBuf {
    pandere_paths().telegram_session_path()
}

pub fn migrate_legacy_session_file(destination: &Path) -> Result<bool> {
    let legacy_path = PathBuf::from("telegram.session");
    if destination.exists() || !legacy_path.exists() {
        return Ok(false);
    }

    create_parent_dir(destination)?;
    fs::rename(&legacy_path, destination).or_else(|rename_error| {
        fs::copy(&legacy_path, destination)
            .map(|_| ())
            .and_then(|_| fs::remove_file(&legacy_path))
            .map_err(|copy_error| {
                anyhow!(
                    "failed to move legacy telegram session file: rename error: {rename_error}; copy/remove error: {copy_error}"
                )
            })
    })?;

    Ok(true)
}

pub fn clear_session_file(path: &Path) -> Result<()> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(anyhow!(error)).with_context(|| {
            format!(
                "failed to remove telegram session file `{}`",
                path.display()
            )
        }),
    }
}

pub(crate) fn create_parent_dir(path: &Path) -> Result<()> {
    let Some(parent) = path.parent() else {
        return Ok(());
    };

    fs::create_dir_all(parent).with_context(|| {
        format!(
            "failed to create telegram data directory `{}`",
            parent.display()
        )
    })
}
