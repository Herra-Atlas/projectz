//! Messages from outside the app, answered by the same agent the window uses.
//!
//! # What this is
//!
//! A hub, not a bot. The hub owns the settings, the per-sender conversation state
//! and the permission model; a *channel* is a transport that can deliver a message
//! and take one back. Telegram is the first, and the reason every decision that is
//! not about Telegram lives here: a second transport must be a folder plus one line
//! in [`adapters`], not a second run path.
//!
//! # Why a message is an ordinary run
//!
//! [`Messaging::handle`] calls `ai::run::run_prompt`, the same function a scheduled
//! job uses, so tools, permission gates, sub-agents, compaction and the transcript
//! are all the ones the rest of the app already has. A message from a phone is not a
//! reduced experience and not a second agent.
//!
//! # What is not here
//!
//! No polling, no HTTP, no token. Those are a channel's, and a channel is only
//! reachable through [`adapter::Channel`].

pub mod adapter;
pub mod channels;
pub mod commands;
pub mod delivery;
pub mod files;
pub mod format;
pub mod local;
pub mod local_menu;
pub mod menu;
pub mod profile_menu;
pub mod profiles;
pub mod prompt;
pub mod relay;
pub mod run;
pub mod session;
pub mod settings;
pub mod storage;
pub mod types;

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::time::Duration;

use serde::Serialize;
use serde_json::Value;
use tauri::{AppHandle, Emitter};

use crate::ai::runtime::AiRuntime;
use crate::database::Database;
use adapter::Channel;
use settings::MessagingSettings;
use types::ChannelStatus;

/// One sender's conversation, held for the life of the process.
///
/// Deliberately not a table. It is a cursor: which session this chat is in, which
/// model it last chose, which run is going and what the last round cost. Losing it
/// on restart costs the sender one `/sessions` or one `/new` -- and the transcript
/// itself is in the database, where every other conversation is.
#[derive(Clone, Debug, Default)]
pub struct ChatState {
    /// Where replies are delivered. Kept because a transport's user id and chat id
    /// are the same only for a private chat.
    pub chat_id: String,
    /// The session this chat is currently in. `None` means the next message starts
    /// one, which is what `/new` sets.
    pub session_id: Option<String>,
    /// The mode chosen for this conversation with `/new`.
    ///
    /// `None` means "not chosen here": the stored session's own mode is read next,
    /// then this sender's per-person default. Kept on the chat rather than only in the session
    /// row so a mode picked before the first message survives until that row exists.
    pub mode: Option<crate::ai::tools::ToolMode>,
    /// A model chosen with `/model`, overriding the stored default.
    pub model: Option<Value>,
    /// The run in flight, so `/stop` and the Stop button have something to cancel.
    pub last_run: Option<String>,
    /// The context the provider last reported, for `/context`.
    pub prompt_tokens: Option<u64>,
    /// The models behind the buttons of the last `/model`, in order.
    pub choices: Vec<Value>,
    /// The categories the last `/model` drew, so a button can reopen one without
    /// asking the registry again.
    pub menu: Vec<menu::Category>,
    /// A message waiting for a local model to finish loading.
    pub pending: Option<String>,
    /// The local model that message was waiting for.
    pub pending_model: Option<Value>,
    /// An `ask_user` question waiting for a typed answer.
    pub question: Option<String>,
}

/// What the Settings page draws.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChannelView {
    pub id: String,
    pub name: String,
    pub enabled: bool,
    pub running: bool,
    pub identity: Option<String>,
    pub error: Option<String>,
    /// The channel's own record, redacted by the channel itself.
    pub config: Value,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MessagingStatus {
    pub settings: MessagingSettings,
    pub channels: Vec<ChannelView>,
}

/// The hub. One instance, managed by Tauri.
pub struct Messaging {
    app: AppHandle,
    runtime: AiRuntime,
    database: Arc<Database>,
    settings: RwLock<MessagingSettings>,
    channels: Vec<Arc<dyn Channel>>,
    chats: Mutex<HashMap<String, ChatState>>,
    /// Which session each sender is in, persisted.
    ///
    /// Kept beside the in-memory state so a restart does not silently start a new
    /// conversation: a message that arrives on Tuesday continues Monday's, which is
    /// what a messaging client is expected to do. Written when the binding changes,
    /// not on every message.
    bindings: Mutex<HashMap<String, String>>,
    /// One async lock per sender, so two messages from the same phone are answered
    /// in order and the second does not load a transcript the first is writing.
    locks: Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>,
    /// Wakes every parked poller: a settings change, or a switch moving.
    wake: tokio::sync::Notify,
    started: AtomicBool,
}

impl Messaging {
    pub fn new(app: AppHandle, runtime: AiRuntime, database: Arc<Database>) -> Arc<Self> {
        let settings = MessagingSettings::load(&database);
        let channels: Vec<Arc<dyn Channel>> = channels::registry(&database);
        for channel in &channels {
            channel.set_enabled(settings.channel_enabled(channel.id()));
        }
        // A stored binding is the only conversation state that survives a restart:
        // the run in flight and the last context size describe a process, not a
        // conversation, so they start empty.
        let bindings: HashMap<String, String> =
            database.setting(settings::CHATS_KEY).unwrap_or_default();
        let chats = bindings
            .iter()
            .map(|(key, session_id)| {
                (
                    key.clone(),
                    ChatState {
                        session_id: Some(session_id.clone()),
                        ..ChatState::default()
                    },
                )
            })
            .collect();
        Arc::new(Self {
            app,
            runtime,
            database,
            settings: RwLock::new(settings),
            channels,
            chats: Mutex::new(chats),
            bindings: Mutex::new(bindings),
            locks: Mutex::new(HashMap::new()),
            wake: tokio::sync::Notify::new(),
            started: AtomicBool::new(false),
        })
    }

    /// Starts every channel's loop, once.
    ///
    /// Each loop runs for the life of the process and parks itself while its
    /// channel is off. That is what makes a token edit or a switch take effect
    /// without restarting the app, and it is why nothing here holds a `JoinHandle`:
    /// there is no state to stop and start, only a flag to read.
    pub fn start(self: &Arc<Self>) {
        if self.started.swap(true, Ordering::SeqCst) {
            return;
        }
        for channel in &self.channels {
            let hub = Arc::clone(self);
            let channel = Arc::clone(channel);
            tauri::async_runtime::spawn(async move {
                channel.listen(hub).await;
            });
        }
    }

    pub fn app(&self) -> &AppHandle {
        &self.app
    }

    pub fn runtime(&self) -> &AiRuntime {
        &self.runtime
    }

    pub fn database(&self) -> &Arc<Database> {
        &self.database
    }

    pub fn settings(&self) -> MessagingSettings {
        self.settings
            .read()
            .map(|settings| settings.clone())
            .unwrap_or_default()
    }

    pub fn enabled(&self) -> bool {
        self.settings().enabled
    }

    /// Writes the record, brings every channel into step and wakes the pollers.
    pub fn save(&self, settings: MessagingSettings) -> Result<MessagingSettings, String> {
        let settings = settings.normalized();
        let value = serde_json::to_value(&settings)
            .map_err(|error| format!("Could not store the messaging settings: {error}"))?;
        self.database.set_setting(settings::KEY, &value)?;
        if let Ok(mut current) = self.settings.write() {
            *current = settings.clone();
        }
        for channel in &self.channels {
            channel.set_enabled(settings.channel_enabled(channel.id()));
        }
        self.wake();
        self.emit();
        Ok(settings)
    }

    /// Moves the master switch.
    pub fn set_enabled(&self, enabled: bool) -> Result<MessagingSettings, String> {
        let mut settings = self.settings();
        settings.enabled = enabled;
        self.save(settings)
    }

    pub fn status(&self) -> MessagingStatus {
        MessagingStatus {
            settings: self.settings(),
            channels: self
                .channels
                .iter()
                .map(|channel| view(channel.as_ref()))
                .collect(),
        }
    }

    pub fn channel(&self, id: &str) -> Option<Arc<dyn Channel>> {
        self.channels
            .iter()
            .find(|channel| channel.id() == id)
            .cloned()
    }

    /// Sends a finished job's answer back to the sender who scheduled it.
    ///
    /// Only a job created from a channel has a target. The allowlist and the
    /// channel's own switch are checked again here: a target recorded yesterday
    /// does not outrank an id removed or a channel switched off today.
    pub async fn notify_job(
        &self,
        job_id: &str,
        name: &str,
        status: &str,
        reason: Option<&str>,
        content: &str,
    ) {
        let Some(target) = delivery::target_for(self.database.as_ref(), job_id) else {
            return;
        };
        let settings = self.settings();
        if !settings.allows(&target.user_id) || !settings.channel_enabled(&target.channel) {
            return;
        }
        let Some(channel) = self.channel(&target.channel) else {
            return;
        };

        let text = match status {
            "completed" => {
                let answer = content.trim();
                if answer.is_empty() {
                    format!("Job finished: {name}\n\nCompleted.")
                } else {
                    format!("Job finished: {name}\n\n{answer}")
                }
            }
            "stopped" => format!(
                "Job stopped: {name}\n\n{}",
                reason.unwrap_or("It was stopped before finishing.")
            ),
            _ => format!(
                "Job failed: {name}\n\n{}",
                reason.unwrap_or("The run did not finish.")
            ),
        };
        for part in format::split_message(&text, channel.capabilities().max_chars) {
            if channel.send(&target.chat_id, &part, &[]).await.is_err() {
                break;
            }
        }
    }

    /// This sender's state, copied out.
    ///
    /// A copy rather than a guard: a caller that held the map across an await
    /// would serialise every sender behind one network round trip.
    pub fn state(&self, key: &str) -> ChatState {
        self.chats
            .lock()
            .ok()
            .and_then(|chats| chats.get(key).cloned())
            .unwrap_or_default()
    }

    pub fn update_state(&self, key: &str, update: impl FnOnce(&mut ChatState)) {
        let Ok(mut chats) = self.chats.lock() else {
            return;
        };
        update(chats.entry(key.to_string()).or_default());
    }

    /// Points a sender at a conversation, or at none for `/new`.
    ///
    /// The in-memory state and the stored binding move together, so a restart, a
    /// `/sessions` switch and the first message of a chat cannot end up disagreeing
    /// about which conversation is current.
    pub fn bind_session(&self, key: &str, session_id: Option<String>) {
        self.update_state(key, |state| state.session_id = session_id.clone());
        let Ok(mut bindings) = self.bindings.lock() else {
            return;
        };
        match session_id {
            Some(id) => {
                bindings.insert(key.to_string(), id);
            }
            None => {
                bindings.remove(key);
            }
        }
        if let Err(error) = self.database.set_setting(settings::CHATS_KEY, &*bindings) {
            // The conversation continues either way; only a restart's continuity is
            // lost, which is not worth failing the message for.
            tracing::warn!(%error, "a messaging conversation binding could not be stored");
        }
    }

    /// The lock that serialises one sender's messages.
    pub fn chat_lock(&self, key: &str) -> Arc<tokio::sync::Mutex<()>> {
        let mut locks = self
            .locks
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        Arc::clone(
            locks
                .entry(key.to_string())
                .or_insert_with(|| Arc::new(tokio::sync::Mutex::new(()))),
        )
    }

    /// Tells every parked channel to look at its settings again.
    pub fn wake(&self) {
        self.wake.notify_waiters();
    }

    /// Waits for a wake, or for half a minute, whichever is first.
    ///
    /// The timeout is a floor rather than a poll interval: it exists so a loop that
    /// missed a wakeup still re-reads its settings eventually, and costs nothing
    /// when the notification arrives.
    pub async fn parked(&self) {
        tokio::select! {
            _ = self.wake.notified() => {}
            _ = tokio::time::sleep(Duration::from_secs(30)) => {}
        }
    }

    /// Tells the app's own window that the settings or a conversation changed.
    pub fn emit(&self) {
        let _ = self
            .app
            .emit("messaging-event", serde_json::json!({ "kind": "state" }));
    }
}

fn view(channel: &dyn Channel) -> ChannelView {
    let ChannelStatus {
        id,
        name,
        enabled,
        running,
        identity,
        error,
    } = channel.status();
    ChannelView {
        id: id.to_string(),
        name: name.to_string(),
        enabled,
        running,
        identity,
        error,
        config: channel.config(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The state map hands back a default for a sender that has not spoken yet, so
    /// the first message needs no initialization step.
    #[test]
    fn a_sender_with_no_history_has_empty_state() {
        let state = ChatState::default();
        assert!(state.session_id.is_none());
        assert!(state.model.is_none());
        assert_eq!(state.chat_id, "");
    }
}
