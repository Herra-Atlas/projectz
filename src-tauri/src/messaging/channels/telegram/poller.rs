//! The long-poll loop: one request held open, one message in at a time.
//!
//! # Why the loop never ends
//!
//! A switch, a token and a master setting can all change while it runs. Restarting
//! a task on every save would mean a task handle to keep, a cancel token, and a
//! race where a message arrives mid-restart. A flag read at the top of the loop is
//! smaller and has one state: the loop is either working or parked on
//! [`Messaging::parked`], and every save wakes it.
//!
//! # Why the offset starts where the last update ended
//!
//! Telegram re-delivers anything not acknowledged. That is what makes polling
//! reliable and it is also what would replay every message sent while the app was
//! closed -- so on start (and after a token change) the backlog is skipped with a
//! single `offset: -1` read. A message sent while the app was down was not answered
//! then, and answering it hours later, out of order, would be worse than silence.

use std::sync::Arc;
use std::time::Duration;

use serde_json::Value;

use crate::messaging::adapter::Channel;
use crate::messaging::channels::telegram::client::LONG_POLL_SECONDS;
use crate::messaging::channels::telegram::{update, Telegram};
use crate::messaging::types::{Inbound, InboundKind};
use crate::messaging::Messaging;

/// How long to wait after a failure that may be transient.
const INITIAL_BACKOFF: Duration = Duration::from_secs(1);
/// The longest wait between retries. Telegram's own limits are far below this, so
/// a wait this long means something is wrong that retrying will not fix.
const MAX_BACKOFF: Duration = Duration::from_secs(60);

pub async fn run(channel: Arc<Telegram>, hub: Arc<Messaging>) {
    let mut offset = 0i64;
    let mut bootstrapped: Option<u64> = None;
    let mut backoff = INITIAL_BACKOFF;

    loop {
        if !channel.enabled() || !hub.enabled() {
            channel.mark_parked();
            hub.parked().await;
            continue;
        }
        let Some(client) = channel.client() else {
            channel.set_error("Add a bot token to connect Telegram.");
            hub.parked().await;
            continue;
        };

        // A new token means a different bot, with its own update ids, so the
        // cursor and the identity are both re-read.
        let epoch = channel.epoch();
        if bootstrapped != Some(epoch) {
            bootstrapped = Some(epoch);
            offset = 0;
            if let Ok(updates) = client.get_updates(-1, 0).await {
                if let Some(id) = last_update_id(&updates) {
                    offset = id + 1;
                }
            }
            match client.get_me().await {
                Ok(user) => channel.set_identity(Some(user.label())),
                Err(error) => {
                    channel.set_error(&error);
                    backoff = wait(backoff).await;
                    continue;
                }
            }
        }

        match client.get_updates(offset, LONG_POLL_SECONDS).await {
            Ok(updates) => {
                backoff = INITIAL_BACKOFF;
                channel.mark_running();
                for update in &updates {
                    let Some(parsed) = update::parse(update) else {
                        continue;
                    };
                    // Advanced before the message is handled: a run that panics or
                    // takes a minute must not be delivered again on the next poll.
                    offset = offset.max(parsed.id + 1);
                    dispatch(&hub, &channel, parsed);
                }
            }
            Err(error) => {
                channel.set_error(&error);
                backoff = wait(backoff).await;
            }
        }
    }
}

fn last_update_id(updates: &[Value]) -> Option<i64> {
    updates
        .iter()
        .filter_map(|update| update.get("update_id").and_then(Value::as_i64))
        .max()
}

/// Waits, then doubles the wait for next time.
async fn wait(backoff: Duration) -> Duration {
    tokio::time::sleep(backoff).await;
    (backoff * 2).min(MAX_BACKOFF)
}

/// Decides what one update means, and hands it to the hub.
fn dispatch(hub: &Arc<Messaging>, channel: &Arc<Telegram>, parsed: update::Update) {
    if parsed.is_bot {
        return;
    }
    let settings = hub.settings();
    let allowed = parsed
        .user_id
        .as_deref()
        .is_some_and(|id| settings.allows(id));

    if !allowed {
        let Some(chat_id) = parsed.chat_id else {
            return;
        };
        let Some(user_id) = parsed.user_id else {
            return;
        };
        // A press is always acknowledged, or the button spins forever.
        if let Some(action) = parsed.action {
            let channel = Arc::clone(channel);
            tauri::async_runtime::spawn(async move {
                let _ = channel.answer(&action.id, Some("Not linked.")).await;
            });
            return;
        }
        // The one thing an unknown sender is ever told: their own id, and only
        // while nobody is configured. With an allowlist in place, silence -- a bot
        // that explains itself to strangers is a bot that can be probed.
        if settings.has_allowlist() {
            return;
        }
        let channel = Arc::clone(channel);
        tauri::async_runtime::spawn(async move {
            let text = format!(
                "Your Telegram user id is {user_id}.\n\n\
                 Add it in ProjectZ → Settings → Communications → Telegram, \
                 then send /start."
            );
            let _ = channel.send(&chat_id, &text, &[]).await;
        });
        return;
    }

    let Some(chat_id) = parsed.chat_id else {
        return;
    };
    let Some(user_id) = parsed.user_id else {
        return;
    };
    let kind = match parsed.action {
        Some(action) => InboundKind::Action {
            id: action.id,
            data: action.data,
        },
        None => {
            // A photo with no caption, a sticker, a voice note: nothing to answer
            // and nothing to say about it.
            if parsed.text.trim().is_empty() {
                return;
            }
            InboundKind::Message
        }
    };

    let inbound = Inbound {
        user_id,
        chat_id,
        user_name: parsed.user_name,
        text: parsed.text,
        kind,
    };
    let hub = Arc::clone(hub);
    let channel: Arc<dyn crate::messaging::adapter::Channel> = channel.clone();
    tauri::async_runtime::spawn(async move {
        if let Err(error) = crate::messaging::run::handle(hub, channel, inbound).await {
            tracing::warn!(%error, "a messaging conversation failed");
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// The cursor is the highest id seen, so a batch that arrives out of order
    /// still cannot make the next poll replay one.
    #[test]
    fn the_cursor_is_the_highest_id() {
        let updates = vec![
            json!({ "update_id": 5 }),
            json!({ "update_id": 9 }),
            json!({ "update_id": 7 }),
        ];
        assert_eq!(last_update_id(&updates), Some(9));
        assert_eq!(last_update_id(&[]), None);
    }

    /// Backoff grows and stops growing, so a persistent failure does not hammer
    /// Telegram and does not stop retrying either.
    #[test]
    fn backoff_is_bounded() {
        assert_eq!(INITIAL_BACKOFF, Duration::from_secs(1));
        assert_eq!(MAX_BACKOFF, Duration::from_secs(60));
    }
}
