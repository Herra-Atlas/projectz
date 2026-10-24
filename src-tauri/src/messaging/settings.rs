//! What messaging is allowed to do, as one stored record.
//!
//! # One record, no table
//!
//! A channel is a handful of fields, and a field added tomorrow must not cost a
//! schema version. The record lives under one settings key, the way the MCP server
//! list and the assist record do. A channel's own secret is a *second* key
//! (`app.messaging.telegram`), so the core record never learns what a Telegram
//! token is -- that is the adapter's business, and a future Discord token lands in
//! its own key rather than here.
//!
//! # Why the allowlist is the security boundary
//!
//! A bot that can run tools on this machine is a remote shell with a nicer voice.
//! The only thing between a stranger and the workspace is the list of user ids, so
//! an empty list means **nobody**, not everybody: the feature switched on with no
//! ids configured must not be an open door. The poller refuses an unknown sender
//! before anything reaches the runtime, and the page says so in as many words.
//!
//! # Why scheduling is taken off unconditionally
//!
//! `access.jobs` is forced off here, on the way in and on the way out, for the same
//! reason `jobs::runner` does it: a message that can schedule a job can arrange to
//! do work later, when the user is no longer watching the conversation. One
//! generation, whatever the record says.

use serde::{Deserialize, Serialize};

use crate::ai::tools::{AccessSet, PermissionMode, ToolMode};

/// The core record: what every channel shares.
pub const KEY: &str = "app.messaging";

/// The most senders one installation may listen to.
///
/// A bound rather than a judgement: an allowlist with a thousand entries is a
/// configuration mistake, and a list that grows without limit is a list nobody
/// reads. One person, a phone and a tablet is the ordinary case.
const MAX_ALLOWED_IDS: usize = 20;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct MessagingSettings {
    /// The master switch. Off until asked for, like every feature that reaches
    /// outside the app.
    pub enabled: bool,
    /// The user ids the bot obeys. Empty means nobody.
    pub allowed_user_ids: Vec<String>,
    /// Which channels are switched on. The master switch gates all of them.
    pub channels: Vec<String>,
    /// Agent by default: a message is a request for work, not for a chat.
    pub mode: ToolMode,
    /// `auto_writes` by default. A run with nobody at the keyboard cannot use
    /// `ask` (it would park forever waiting for a prompt nobody sees), so the
    /// level has to be one that runs unattended -- and `full` is the wrong thing
    /// to hand a phone by default.
    pub permission: PermissionMode,
    /// What the run may touch. `jobs` is always forced off.
    pub access: AccessSet,
    /// Whether a chat may start a local model that is not loaded.
    ///
    /// Off by default, and deliberately: the loader keeps one model resident, so
    /// starting one evicts whatever the window is using. That is a decision for the
    /// person at the machine, not for a message that arrived while they were away --
    /// when it is on, the chat still asks before replacing anything.
    pub allow_local_start: bool,
    /// The folder the run works in. `None` uses the open workspace.
    pub workspace: Option<String>,
}

impl Default for MessagingSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            allowed_user_ids: Vec::new(),
            channels: vec!["telegram".to_string()],
            mode: ToolMode::Agent,
            permission: PermissionMode::AutoWrites,
            // Everything a normal conversation has, minus the one capability whose
            // effect outlives the run.
            access: AccessSet {
                jobs: false,
                ..AccessSet::ALL
            },
            allow_local_start: false,
            workspace: None,
        }
    }
}

impl MessagingSettings {
    pub fn load(database: &crate::database::Database) -> Self {
        database
            .setting::<Self>(KEY)
            .unwrap_or_default()
            .normalized()
    }

    /// Brings the record into a state that can run.
    ///
    /// Applied on the way in and on the way out, so what the page draws is what
    /// the poller will use.
    pub fn normalized(mut self) -> Self {
        self.allowed_user_ids = normalize_ids(&self.allowed_user_ids);
        // Trimmed and deduplicated, keeping the order the user gave.
        let mut channels: Vec<String> = Vec::new();
        for channel in &self.channels {
            let channel = channel.trim().to_lowercase();
            if !channel.is_empty() && !channels.contains(&channel) {
                channels.push(channel);
            }
        }
        self.channels = channels;
        if let Some(workspace) = self.workspace.as_mut() {
            *workspace = workspace.trim().to_string();
            if workspace.is_empty() {
                self.workspace = None;
            }
        }
        // Not a preference. A message that could schedule work could outlive the
        // conversation that made it, and there is nobody watching.
        self.access.jobs = false;
        self
    }

    /// Whether a sender may be obeyed.
    pub fn allows(&self, user_id: &str) -> bool {
        let user_id = user_id.trim();
        !user_id.is_empty() && self.allowed_user_ids.iter().any(|id| id == user_id)
    }

    /// Whether any sender is configured at all.
    pub fn has_allowlist(&self) -> bool {
        !self.allowed_user_ids.is_empty()
    }

    /// Whether a channel is switched on under the master switch.
    pub fn channel_enabled(&self, id: &str) -> bool {
        self.enabled && self.channels.iter().any(|channel| channel == id)
    }
}

/// Keeps the numeric ids, trimmed, without duplicates.
///
/// Numeric only on purpose: the allowlist is checked against an id the transport
/// reports as a number, and a username would be a second, mutable way to be
/// somebody. A record edited by hand cannot smuggle one in.
fn normalize_ids(ids: &[String]) -> Vec<String> {
    let mut kept: Vec<String> = Vec::new();
    for id in ids {
        let id = id.trim();
        if id.is_empty() || !id.chars().all(|character| character.is_ascii_digit()) {
            continue;
        }
        if !kept.iter().any(|existing| existing == id) {
            kept.push(id.to_string());
        }
    }
    kept.truncate(MAX_ALLOWED_IDS);
    kept
}

/// A channel's own secret, stored under its own key.
///
/// Telegram's only: a token. A channel that needs more than one field adds it here
/// rather than to the shared record, so two transports never argue over a name.
pub const TELEGRAM_KEY: &str = "app.messaging.telegram";

/// Which conversation each sender was last in.
///
/// A separate key rather than a field on [`MessagingSettings`], for one reason: the
/// Settings page writes the record back whole, and a page holding a copy from before
/// the last message would then revert the chat to an older conversation. This is
/// state the backend owns and the page never sees.
pub const CHATS_KEY: &str = "app.messaging.chats";

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct TelegramSettings {
    pub token: String,
}

impl TelegramSettings {
    pub fn load(database: &crate::database::Database) -> Self {
        database.setting::<Self>(TELEGRAM_KEY).unwrap_or_default()
    }

    pub fn save(&self, database: &crate::database::Database) -> Result<(), String> {
        let value = serde_json::to_value(self)
            .map_err(|error| format!("Could not store the Telegram settings: {error}"))?;
        database.set_setting(TELEGRAM_KEY, &value)
    }

    pub fn has_token(&self) -> bool {
        !self.token.trim().is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// **An empty allowlist is nobody, not everybody.** The one default that keeps
    /// a switched-on bot from being an open door.
    #[test]
    fn nobody_is_allowed_until_an_id_is_listed() {
        let settings = MessagingSettings::default();
        assert!(!settings.has_allowlist());
        assert!(!settings.allows("12345"));
        assert!(!settings.allows(""));
    }

    /// Telegram user ids are numeric; a username is mutable and re-registrable, so
    /// it is refused rather than stored as a second way to be somebody.
    #[test]
    fn only_numeric_ids_are_kept() {
        let settings = MessagingSettings {
            allowed_user_ids: vec![
                " 12345 ".into(),
                "12345".into(),
                "someone".into(),
                "@someone".into(),
                "".into(),
                "-100200".into(),
            ],
            ..MessagingSettings::default()
        }
        .normalized();
        assert_eq!(settings.allowed_user_ids, vec!["12345"]);
        assert!(settings.allows("12345"));
        assert!(!settings.allows("someone"));
    }

    /// **Scheduling is never available to a message.** A phone is the least
    /// watched surface there is, and a job would run long after the conversation
    /// that asked for it.
    #[test]
    fn a_message_can_never_schedule_work() {
        let wanted = MessagingSettings {
            access: AccessSet::ALL,
            ..MessagingSettings::default()
        }
        .normalized();
        assert!(!wanted.access.jobs);
        assert!(wanted.access.read && wanted.access.write && wanted.access.terminal);
    }

    /// The level has to run unattended: `ask` would park forever on the first
    /// write, because a chat cannot show a prompt nobody has thought to relay yet.
    #[test]
    fn the_default_level_can_run_without_anyone_watching() {
        let settings = MessagingSettings::default();
        assert_eq!(settings.permission, PermissionMode::AutoWrites);
        assert_eq!(settings.mode, ToolMode::Agent);
        assert!(!settings.enabled);
    }

    /// A channel is off unless both the master switch and its own name say so.
    #[test]
    fn both_switches_have_to_agree() {
        let mut settings = MessagingSettings {
            enabled: false,
            ..MessagingSettings::default()
        };
        assert!(!settings.channel_enabled("telegram"));
        settings.enabled = true;
        assert!(settings.channel_enabled("telegram"));
        assert!(!settings.channel_enabled("discord"));
    }

    /// A partial record -- written before a field existed, or by hand -- fills in
    /// the defaults rather than failing to load.
    #[test]
    fn a_partial_record_reads() {
        let settings: MessagingSettings =
            serde_json::from_value(serde_json::json!({ "enabled": true })).expect("partial");
        assert!(settings.enabled);
        assert!(!settings.access.jobs);
        assert_eq!(settings.channels, vec!["telegram"]);
    }

    /// A token is a secret, and the page is never told what it is -- only that one
    /// is saved. `has_token` is the single question the view asks.
    #[test]
    fn a_token_is_only_reported_as_present() {
        assert!(!TelegramSettings::default().has_token());
        let stored = TelegramSettings {
            token: " 123:abc ".into(),
        };
        assert!(stored.has_token());
    }
}
