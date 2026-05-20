use anyhow::Result;
use tracing::info;

use super::App;
use crate::{
    constants::{TELEGRAM_ENV_NOTICE, TELEGRAM_FETCH_DIALOG_LIMIT},
    data_source::{AuthStatus, MessengerDataSource, SyncStatus},
    fixtures::FixtureMessengerSource,
    messenger_service::{LoginInputMode, LoginPhase, MessengerAuthStatus},
};

impl App {
    pub(super) async fn refresh_messenger_state(&mut self) -> Result<()> {
        if !self.ensure_messenger_available(TELEGRAM_ENV_NOTICE) {
            return Ok(());
        }

        let service = self
            .messenger
            .as_ref()
            .expect("messenger availability checked")
            .service();

        let (login_state, auth_status, chats) = {
            let messenger = self
                .messenger
                .as_mut()
                .expect("messenger availability checked");
            let login_state = messenger.bootstrap_login().await?;
            let auth_status = messenger.auth_status().await?;
            let chats = match &auth_status {
                MessengerAuthStatus::Authorized { .. } => {
                    Some(messenger.list_chats(TELEGRAM_FETCH_DIALOG_LIMIT).await?)
                }
                MessengerAuthStatus::NeedsLogin | MessengerAuthStatus::Connected => None,
            };
            (login_state, auth_status, chats)
        };

        self.state.apply_login_state(login_state);

        match auth_status {
            MessengerAuthStatus::Authorized { account_label } => {
                self.state.source.service = service;
                self.state.source.auth_status = AuthStatus::Authenticated(account_label);
                self.state.source.sync_status = SyncStatus::Idle;
                let chats = chats.expect("authorized messenger state should include chat list");
                self.state.set_chats(chats.clone());
                self.cache.save_chats(service, &chats)?;
                self.schedule_preview_fetch();
            }
            MessengerAuthStatus::NeedsLogin | MessengerAuthStatus::Connected => {
                let fixture = FixtureMessengerSource.snapshot()?;
                self.state.source.service = service;
                self.state.source.auth_status = AuthStatus::NeedsLogin;
                self.state.source.sync_status = SyncStatus::Pending;
                self.state.source.chats = fixture.chats;
                self.state.thread_cache = crate::state::build_thread_cache(&fixture.messages);
                self.state.source.messages = fixture.messages;
                self.state.reset_messenger_selection();
                self.state.thread_status = crate::state::ThreadStatus::Idle;
            }
        }
        self.sync_draft_for_active_chat()?;

        Ok(())
    }

    pub(super) async fn request_login_code(&mut self) -> Result<()> {
        if !self.ensure_messenger_available("messenger client is unavailable") {
            return Ok(());
        }

        info!("requesting messenger login code");
        self.messenger
            .as_mut()
            .expect("messenger availability checked")
            .request_login_code()
            .await?;
        self.state.clear_login_notice();
        self.refresh_messenger_state().await
    }

    pub(super) async fn submit_login_input(&mut self) -> Result<()> {
        if !self.ensure_messenger_available("messenger client is unavailable") {
            return Ok(());
        }

        let input = self.state.login_input.trim().to_owned();
        if input.is_empty() {
            self.state.set_login_notice("input is empty");
            return Ok(());
        }

        let login_phase = self.state.login_phase;
        let result = match self.state.login_input_mode {
            Some(LoginInputMode::Identifier) => {
                self.messenger
                    .as_mut()
                    .expect("messenger availability checked")
                    .set_login_identifier(input.clone())?;
                self.messenger
                    .as_mut()
                    .expect("messenger availability checked")
                    .submit_login_input(LoginInputMode::Identifier, &input)
                    .await
            }
            Some(LoginInputMode::Code) => {
                self.messenger
                    .as_mut()
                    .expect("messenger availability checked")
                    .submit_login_input(LoginInputMode::Code, &input)
                    .await
            }
            Some(LoginInputMode::Password) => {
                self.messenger
                    .as_mut()
                    .expect("messenger availability checked")
                    .submit_login_input(LoginInputMode::Password, &input)
                    .await
            }
            None => match login_phase {
                Some(LoginPhase::CodeRequested) => {
                    self.messenger
                        .as_mut()
                        .expect("messenger availability checked")
                        .submit_login_input(LoginInputMode::Code, &input)
                        .await
                }
                Some(LoginPhase::PasswordRequired) => {
                    self.messenger
                        .as_mut()
                        .expect("messenger availability checked")
                        .submit_login_input(LoginInputMode::Password, &input)
                        .await
                }
                _ => {
                    self.state
                        .set_login_notice("nothing to submit in current login phase");
                    return Ok(());
                }
            },
        };

        match result {
            Ok(_) => {
                info!("messenger login step completed");
                self.state.clear_login_input();
                self.state.clear_login_notice();
                self.refresh_messenger_state().await
            }
            Err(error) => {
                self.state.set_login_notice(error.to_string());
                Ok(())
            }
        }
    }

    pub(super) async fn logout_messenger(&mut self) -> Result<()> {
        if !self.ensure_messenger_available("messenger client is unavailable") {
            return Ok(());
        }

        info!("clearing messenger session");
        self.messenger
            .as_mut()
            .expect("messenger availability checked")
            .clear_session_and_reconnect()
            .await?;
        self.state.clear_login_input();
        self.state.set_login_notice("session cleared");
        self.in_flight_threads.clear();
        self.in_flight_forum_threads.clear();
        self.pending_thread_fetch = None;
        self.pending_forum_threads_fetch = None;
        self.refresh_messenger_state().await
    }
}
