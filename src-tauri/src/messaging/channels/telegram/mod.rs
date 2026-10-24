//! Telegram as a channel: the adapter, its own secret, and its state.
//!
//! # The token lives here, not in the core record
//!
//! `app.messaging` is what every channel shares; `app.messaging.telegram` is this
//! adapter's own key. The core never sees a token, which is what lets a second
//! transport keep its own credential without the shared record growing a field per
//! service.
//!
//! # The page is told whether a token is saved, never what it is
//!
//! The same rule the provider list follows for an API key. A secret that can be
//! read back out of the settings window is a secret shown to anyone who walks past
//! a screen, and an empty field on save means "keep the one you have" rather than
//! "erase it by accident".
//!
//! # Why the client is validated before it is stored
//!
//! A token is checked with `getMe` first. Saving first and discovering it is wrong
//! on the next poll leaves the page showing a connected-looking record and an error
//! that arrives a minute later; refusing the save puts the mistake where it was
//! made.

pub mod client;
pub mod poller;
pub mod update;

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, RwLock};

use serde::Deserialize;
use serde_json::{json, Value};

use crate::database::Database;
use crate::messaging::adapter::{Channel, ChannelFuture};
use crate::messaging::settings::TelegramSettings;
use crate::messaging::types::{Button, Capabilities, ChannelStatus, SentMessage};
use crate::messaging::Messaging;
use client::TelegramClient;

/// Telegram refuses anything longer than this, and a reply that is one character
/// over is a reply nobody sees.
const MAX_MESSAGE_CHARS: usize = 4_096;

/// What the Settings page is told about this channel's configuration.
#[derive(Default, Clone)]
struct Live {
    running: bool,
    identity: Option<String>,
    error: Option<String>,
}

pub struct Telegram {
    database: Arc<Database>,
    config: RwLock<TelegramSettings>,
    client: RwLock<Option<Arc<TelegramClient>>>,
    enabled: AtomicBool,
    /// Bumped whenever the token changes, so the poller throws away a cursor that
    /// belongs to a different bot.
    epoch: AtomicU64,
    live: RwLock<Live>,
}

impl Telegram {
    pub fn new(database: Arc<Database>) -> Self {
        let config = TelegramSettings::load(&database);
        let client = TelegramClient::new(&config.token).ok().map(Arc::new);
        Self {
            database,
            config: RwLock::new(config),
            client: RwLock::new(client),
            enabled: AtomicBool::new(false),
            epoch: AtomicU64::new(0),
            live: RwLock::new(Live::default()),
        }
    }

    fn client(&self) -> Option<Arc<TelegramClient>> {
        self.client
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }

    fn epoch(&self) -> u64 {
        self.epoch.load(Ordering::SeqCst)
    }

    fn enabled(&self) -> bool {
        self.enabled.load(Ordering::SeqCst)
    }

    fn set_identity(&self, identity: Option<String>) {
        let mut live = self
            .live
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        live.identity = identity;
        live.error = None;
    }

    fn mark_running(&self) {
        let mut live = self
            .live
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        live.running = true;
        live.error = None;
    }

    fn mark_parked(&self) {
        let mut live = self
            .live
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        live.running = false;
    }

    fn set_error(&self, error: &str) {
        let mut live = self
            .live
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        live.running = false;
        live.error = Some(error.to_string());
    }

    /// The redacted configuration the page draws.
    fn config_view(&self) -> Value {
        let saved = self
            .config
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .has_token();
        json!({ "tokenSaved": saved })
    }
}

impl Channel for Telegram {
    fn id(&self) -> &'static str {
        "telegram"
    }

    fn name(&self) -> &'static str {
        "Telegram"
    }

    fn capabilities(&self) -> Capabilities {
        Capabilities {
            buttons: true,
            edit: true,
            typing: true,
            max_chars: MAX_MESSAGE_CHARS,
        }
    }

    fn set_enabled(&self, enabled: bool) {
        self.enabled.store(enabled, Ordering::SeqCst);
        if !enabled {
            self.mark_parked();
        }
    }

    fn listen(self: Arc<Self>, hub: Arc<Messaging>) -> ChannelFuture<'static, ()> {
        Box::pin(poller::run(self, hub))
    }

    fn send<'a>(
        &'a self,
        to: &'a str,
        text: &'a str,
        buttons: &'a [Button],
    ) -> ChannelFuture<'a, Result<SentMessage, String>> {
        Box::pin(async move {
            let Some(client) = self.client() else {
                return Err("The Telegram bot token is not set.".to_string());
            };
            client.send_message(to, text, buttons).await
        })
    }

    fn edit<'a>(
        &'a self,
        to: &'a str,
        message_id: &'a str,
        text: &'a str,
        buttons: &'a [Button],
    ) -> ChannelFuture<'a, Result<(), String>> {
        Box::pin(async move {
            let Some(client) = self.client() else {
                return Err("The Telegram bot token is not set.".to_string());
            };
            client.edit_message(to, message_id, text, buttons).await
        })
    }

    fn typing<'a>(&'a self, to: &'a str) -> ChannelFuture<'a, Result<(), String>> {
        Box::pin(async move {
            let Some(client) = self.client() else {
                return Err("The Telegram bot token is not set.".to_string());
            };
            client.send_typing(to).await
        })
    }

    fn answer<'a>(
        &'a self,
        action_id: &'a str,
        text: Option<&'a str>,
    ) -> ChannelFuture<'a, Result<(), String>> {
        Box::pin(async move {
            let Some(client) = self.client() else {
                return Err("The Telegram bot token is not set.".to_string());
            };
            client.answer_callback(action_id, text).await
        })
    }

    fn status(&self) -> ChannelStatus {
        let live = self
            .live
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone();
        ChannelStatus {
            id: self.id(),
            name: self.name(),
            enabled: self.enabled(),
            running: live.running,
            identity: live.identity,
            error: live.error,
        }
    }

    fn config(&self) -> Value {
        self.config_view()
    }

    fn configure<'a>(&'a self, config: Value) -> ChannelFuture<'a, Result<Value, String>> {
        Box::pin(async move {
            #[derive(Deserialize, Default)]
            #[serde(rename_all = "camelCase", default)]
            struct Input {
                token: String,
                clear: bool,
            }
            let input: Input = serde_json::from_value(config)
                .map_err(|error| format!("That is not a Telegram configuration: {error}"))?;

            if input.clear {
                let settings = TelegramSettings::default();
                settings.save(&self.database)?;
                *self
                    .config
                    .write()
                    .unwrap_or_else(|poisoned| poisoned.into_inner()) = settings;
                *self
                    .client
                    .write()
                    .unwrap_or_else(|poisoned| poisoned.into_inner()) = None;
                *self
                    .live
                    .write()
                    .unwrap_or_else(|poisoned| poisoned.into_inner()) = Live::default();
                self.epoch.fetch_add(1, Ordering::SeqCst);
                return Ok(self.config_view());
            }

            let token = input.token.trim().to_string();
            // An empty field means "keep the saved token", which is how a page can
            // redraw the form without erasing a working credential.
            if token.is_empty() {
                return Ok(self.config_view());
            }

            let client = TelegramClient::new(&token)?;
            // Checked before it is stored, so a typo is reported at the field rather
            // than as a poll that fails a minute later.
            let user = client.get_me().await?;
            let settings = TelegramSettings { token };
            settings.save(&self.database)?;
            *self
                .config
                .write()
                .unwrap_or_else(|poisoned| poisoned.into_inner()) = settings;
            *self
                .client
                .write()
                .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(Arc::new(client));
            {
                let mut live = self
                    .live
                    .write()
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
                live.identity = Some(user.label());
                live.error = None;
                live.running = false;
            }
            self.epoch.fetch_add(1, Ordering::SeqCst);
            Ok(self.config_view())
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn channel() -> Telegram {
        let database = Arc::new(Database::open_in_memory());
        database.initialize().expect("migrate");
        Telegram::new(database)
    }

    /// The page is told a token exists, never what it is -- even though the channel
    /// itself holds one.
    #[test]
    fn the_config_view_hides_the_token() {
        let channel = channel();
        assert_eq!(channel.config()["tokenSaved"], false);
        let stored = TelegramSettings {
            token: "123:abc".into(),
        };
        stored.save(channel.database.as_ref()).expect("save");
        let channel = Telegram::new(Arc::clone(&channel.database));
        assert_eq!(channel.config()["tokenSaved"], true);
        // The value itself is nowhere in the answer.
        assert!(!channel.config().to_string().contains("123:abc"));
    }

    /// A channel starts off and out of the loop; the hub is the only thing that can
    /// switch it on.
    #[test]
    fn a_channel_starts_disabled() {
        let status = channel().status();
        assert!(!status.enabled);
        assert!(!status.running);
        assert_eq!(status.id, "telegram");
    }

    /// The transport's limits are facts the core reads rather than assumes.
    #[test]
    fn telegram_can_draw_buttons_and_edit() {
        let capabilities = channel().capabilities();
        assert!(capabilities.buttons && capabilities.edit && capabilities.typing);
        assert_eq!(capabilities.max_chars, 4_096);
    }

    /// Switching a channel off stops it claiming to be connected, so the page
    /// cannot show "running" over a switch that is off.
    #[test]
    fn disabling_clears_the_running_flag() {
        let channel = channel();
        channel.set_enabled(true);
        channel.mark_running();
        assert!(channel.status().running);
        channel.set_enabled(false);
        assert!(!channel.status().running);
    }
}
