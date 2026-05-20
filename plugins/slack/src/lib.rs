use std::time::SystemTime;

use anyhow::{Context, Result, anyhow};

pub fn crate_name() -> &'static str {
    "pandere-plugin-slack"
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SlackOAuthConfig {
    pub client_id: String,
    pub client_secret: String,
    pub redirect_uri: String,
    pub bot_scopes: Vec<String>,
    pub user_scopes: Vec<String>,
}

impl SlackOAuthConfig {
    pub fn from_env() -> Result<Self> {
        let client_id = std::env::var("SLACK_CLIENT_ID").context("missing SLACK_CLIENT_ID")?;
        let client_secret =
            std::env::var("SLACK_CLIENT_SECRET").context("missing SLACK_CLIENT_SECRET")?;
        let redirect_uri =
            std::env::var("SLACK_REDIRECT_URI").context("missing SLACK_REDIRECT_URI")?;
        let bot_scopes = parse_scopes_from_env("SLACK_BOT_SCOPES", default_bot_scopes());
        let user_scopes = parse_scopes_from_env("SLACK_USER_SCOPES", default_user_scopes());

        let config = Self {
            client_id,
            client_secret,
            redirect_uri,
            bot_scopes,
            user_scopes,
        };
        config.validate()?;
        Ok(config)
    }

    pub fn validate(&self) -> Result<()> {
        if self.client_id.trim().is_empty() {
            return Err(anyhow!("slack client id must not be empty"));
        }
        if self.client_secret.trim().is_empty() {
            return Err(anyhow!("slack client secret must not be empty"));
        }
        if self.redirect_uri.trim().is_empty() {
            return Err(anyhow!("slack redirect uri must not be empty"));
        }
        if self.bot_scopes.is_empty() && self.user_scopes.is_empty() {
            return Err(anyhow!("at least one slack scope must be configured"));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SlackOAuthCallback {
    pub code: String,
    pub state: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SlackInstallation {
    pub team_id: String,
    pub team_name: String,
    pub installer_user_id: String,
    pub bot_access_token: Option<String>,
    pub user_access_token: Option<String>,
    pub bot_refresh_token: Option<String>,
    pub user_refresh_token: Option<String>,
    pub installed_at: SystemTime,
}

impl SlackInstallation {
    pub fn validate(&self) -> Result<()> {
        if self.team_id.trim().is_empty() {
            return Err(anyhow!("slack team id must not be empty"));
        }
        if self.installer_user_id.trim().is_empty() {
            return Err(anyhow!("slack installer user id must not be empty"));
        }
        if self.bot_access_token.is_none() && self.user_access_token.is_none() {
            return Err(anyhow!(
                "slack installation must contain at least one access token"
            ));
        }
        Ok(())
    }

    pub fn installation_key(&self) -> String {
        format!("slack:{}:{}", self.team_id, self.installer_user_id)
    }
}

pub fn oauth_authorize_url(config: &SlackOAuthConfig, state: &str) -> Result<String> {
    config.validate()?;
    if state.trim().is_empty() {
        return Err(anyhow!("oauth state must not be empty"));
    }

    let mut params = vec![
        ("client_id", config.client_id.as_str()),
        ("redirect_uri", config.redirect_uri.as_str()),
        ("state", state),
    ];

    let bot_scopes = join_scopes(&config.bot_scopes);
    if !bot_scopes.is_empty() {
        params.push(("scope", bot_scopes.as_str()));
    }

    let user_scopes = join_scopes(&config.user_scopes);
    if !user_scopes.is_empty() {
        params.push(("user_scope", user_scopes.as_str()));
    }

    let query = params
        .into_iter()
        .map(|(key, value)| format!("{}={}", percent_encode(key), percent_encode(value)))
        .collect::<Vec<_>>()
        .join("&");

    Ok(format!("https://slack.com/oauth/v2/authorize?{query}"))
}

pub fn default_bot_scopes() -> &'static [&'static str] {
    &[
        "chat:write",
        "channels:history",
        "groups:history",
        "im:history",
        "mpim:history",
    ]
}

pub fn default_user_scopes() -> &'static [&'static str] {
    &["chat:write"]
}

fn parse_scopes_from_env(name: &str, fallback: &[&str]) -> Vec<String> {
    match std::env::var(name) {
        Ok(value) => value
            .split(',')
            .map(str::trim)
            .filter(|scope| !scope.is_empty())
            .map(str::to_owned)
            .collect(),
        Err(_) => fallback.iter().map(|scope| (*scope).to_owned()).collect(),
    }
}

fn join_scopes(scopes: &[String]) -> String {
    scopes.join(",")
}

fn percent_encode(value: &str) -> String {
    let mut encoded = String::with_capacity(value.len());
    for byte in value.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                encoded.push(byte as char)
            }
            _ => encoded.push_str(&format!("%{:02X}", byte)),
        }
    }
    encoded
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn authorize_url_contains_required_params() {
        let config = SlackOAuthConfig {
            client_id: "123".into(),
            client_secret: "secret".into(),
            redirect_uri: "https://example.com/slack/callback".into(),
            bot_scopes: vec!["chat:write".into(), "channels:history".into()],
            user_scopes: vec!["chat:write".into()],
        };

        let url = oauth_authorize_url(&config, "nonce-123").expect("url should build");

        assert!(url.starts_with("https://slack.com/oauth/v2/authorize?"));
        assert!(url.contains("client_id=123"));
        assert!(url.contains("state=nonce-123"));
        assert!(url.contains("scope=chat%3Awrite%2Cchannels%3Ahistory"));
        assert!(url.contains("user_scope=chat%3Awrite"));
        assert!(url.contains("redirect_uri=https%3A%2F%2Fexample.com%2Fslack%2Fcallback"));
    }

    #[test]
    fn installation_key_is_stable() {
        let installation = SlackInstallation {
            team_id: "T123".into(),
            team_name: "Acme".into(),
            installer_user_id: "U456".into(),
            bot_access_token: Some("xoxb-1".into()),
            user_access_token: None,
            bot_refresh_token: None,
            user_refresh_token: None,
            installed_at: SystemTime::now(),
        };

        assert_eq!(installation.installation_key(), "slack:T123:U456");
    }
}
