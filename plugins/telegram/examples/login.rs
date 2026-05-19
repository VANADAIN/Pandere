use std::io::{self, BufRead as _, Write as _};

use anyhow::Result;
use pandere_plugin_telegram::{LoginPhase, TelegramClient, TelegramConfig};

fn prompt(message: &str) -> Result<String> {
    let stdout = io::stdout();
    let mut stdout = stdout.lock();
    stdout.write_all(message.as_bytes())?;
    stdout.flush()?;

    let stdin = io::stdin();
    let mut stdin = stdin.lock();
    let mut line = String::new();
    stdin.read_line(&mut line)?;
    Ok(line.trim().to_owned())
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<()> {
    let config = TelegramConfig::from_env()?;
    let mut client = TelegramClient::connect(config).await?;

    loop {
        let state = client.bootstrap_login().await?;
        println!(
            "phase={:?} session={} saved_session={} phone={}",
            state.phase,
            state.session_path.display(),
            state.has_saved_session,
            state.phone_number
        );

        if let Some(user_label) = state.user_label.as_deref() {
            println!("authorized as {user_label}");
            break;
        }

        match state.phase {
            LoginPhase::Disconnected => {
                anyhow::bail!("telegram transport runner is not connected");
            }
            LoginPhase::Authorized => break,
            LoginPhase::Connected => {
                client.request_login_code_state().await?;
            }
            LoginPhase::CodeRequested => {
                let code = prompt("Telegram code: ")?;
                client.submit_login_code_state(&code).await?;
            }
            LoginPhase::PasswordRequired => {
                if let Some(hint) = state.password_hint.as_deref() {
                    println!("2FA hint: {hint}");
                }

                let password = prompt("Telegram password: ")?;
                client.submit_password_state(&password).await?;
            }
        }
    }

    let chats = client.list_chats(5).await?;
    println!("loaded {} chats", chats.len());
    for chat in chats {
        println!("- {} ({})", chat.title, chat.id.as_str());
    }

    Ok(())
}
