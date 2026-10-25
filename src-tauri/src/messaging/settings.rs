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
//! # Scheduling is a per-person decision
//!
//! `access.jobs` is a real switch on each id, off by default. A message can only
//! schedule work when that id has the Jobs capability, and the job it creates is
//! still one generation deep: scheduling from inside that job is taken away by
//! `jobs/runner.rs` as it is for every job.

use std::collections::BTreeMap;

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

/// The most channel instructions kept, in characters.
///
/// A bound rather than a judgement: this rides in every request from a phone, so
/// a pasted essay would cost tokens on every message.
const MAX_CHANNEL_INSTRUCTIONS: usize = 8_000;

/// What one Telegram id may do.
///
/// Kept per person rather than once per channel: an id is the identity the
/// transport trusts, so it is also the right place to say whether that person
/// gets tools, which mode a new conversation starts in, and what it may touch.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct MessagingUser {
    /// The mode a new conversation uses when `/new` does not choose.
    pub mode: ToolMode,
    /// Whether `/new` may offer Agent to this person at all.
    pub allow_agent: bool,
    /// The approval level for unattended work.
    pub permission: PermissionMode,
    /// The tools this person's conversations may use, before the installation
    /// ceiling narrows them further.
    pub access: AccessSet,
    /// Whether this person may load a local chat model when theirs is not loaded.
    pub allow_local_start: bool,
    /// Whether this person may start the speech-to-text engine with a voice message.
    pub allow_stt_start: bool,
}

impl Default for MessagingUser {
    fn default() -> Self {
        Self {
            mode: ToolMode::Agent,
            allow_agent: true,
            permission: PermissionMode::AutoWrites,
            // The capability whose effect outlives the run is off by default, as
            // it is for the channel fallback.
            access: AccessSet {
                jobs: false,
                ..AccessSet::ALL
            },
            // Loading a model or an audio engine replaces what the host has
            // loaded. That is a deliberate grant, not a default.
            allow_local_start: false,
            allow_stt_start: false,
        }
    }
}

impl MessagingUser {
    /// The mode for the next turn, after the per-person Agent ceiling.
    pub fn mode_for(&self, requested: Option<ToolMode>) -> ToolMode {
        if !self.allow_agent {
            return ToolMode::Chat;
        }
        requested.unwrap_or(self.mode)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct MessagingSettings {
    /// The master switch. Off until asked for, like every feature that reaches
    /// outside the app.
    pub enabled: bool,
    /// The user ids the bot obeys. Empty means nobody.
    pub allowed_user_ids: Vec<String>,
    /// The per-id policy. Every allowed id has a record after normalisation.
    pub users: BTreeMap<String, MessagingUser>,
    /// Which channels are switched on. The master switch gates all of them.
    pub channels: Vec<String>,
    /// **Legacy channel fallback only.** The page edits each id in `users`; these
    /// four fields are what a record written before per-user policy existed
    /// migrates from. New allowlist ids inherit them on normalisation.
    pub mode: ToolMode,
    /// Legacy fallback for whether `/new` may offer Agent.
    pub allow_agent: bool,
    /// Legacy fallback for the unattended approval level.
    pub permission: PermissionMode,
    /// Legacy fallback for a Telegram run's tool set. Jobs is off by default.
    pub access: AccessSet,
    /// Whether files may be received from and sent to the chat.
    ///
    /// On by default because a phone conversation without attachments is a
    /// conversation that cannot show the thing it is discussing. Off refuses both
    /// directions, so it is a real privacy switch rather than a receive-only one.
    pub allow_files: bool,
    /// Extra instructions for every message answered through a channel.
    ///
    /// Appended after the user's general instructions and before the sender's
    /// profile and the channel's own default note. Empty means only those apply.
    pub instructions: String,
    /// Whether a chat may start a local model that is not loaded.
    ///
    /// Off by default, and deliberately: the loader keeps one model resident, so
    /// starting one evicts whatever the window is using. That is a decision for the
    /// person at the machine, not for a message that arrived while they were away --
    /// when it is on, the chat still asks before replacing anything.
    pub allow_local_start: bool,
}

impl Default for MessagingSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            allowed_user_ids: Vec::new(),
            users: BTreeMap::new(),
            channels: vec!["telegram".to_string()],
            mode: ToolMode::Agent,
            allow_agent: true,
            permission: PermissionMode::AutoWrites,
            // Everything a normal conversation has, minus the capability whose
            // effect outlives the run. An id can turn it on deliberately.
            access: AccessSet {
                jobs: false,
                ..AccessSet::ALL
            },
            allow_files: true,
            instructions: String::new(),
            allow_local_start: false,
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
        // Every allowed id gets a record, and records for removed ids disappear.
        // The old channel-level fields are the fallback for a record written before
        // per-user policy existed, so an existing installation keeps the access it
        // had instead of silently gaining or losing tools.
        let fallback = MessagingUser {
            mode: self.mode,
            allow_agent: self.allow_agent,
            permission: self.permission,
            access: self.access,
            allow_local_start: self.allow_local_start,
            allow_stt_start: false,
        };
        let mut users = BTreeMap::new();
        for id in &self.allowed_user_ids {
            let mut user = self
                .users
                .get(id)
                .cloned()
                .unwrap_or_else(|| fallback.clone());
            if !user.allow_agent {
                user.mode = ToolMode::Chat;
            }
            users.insert(id.clone(), user);
        }
        self.users = users;
        // Trimmed and deduplicated, keeping the order the user gave.
        let mut channels: Vec<String> = Vec::new();
        for channel in &self.channels {
            let channel = channel.trim().to_lowercase();
            if !channel.is_empty() && !channels.contains(&channel) {
                channels.push(channel);
            }
        }
        self.channels = channels;
        self.instructions = self.instructions.trim().to_string();
        self.instructions = self
            .instructions
            .chars()
            .take(MAX_CHANNEL_INSTRUCTIONS)
            .collect();
        // The fallback ceiling is applied where the mode is chosen too, so a stale
        // Agent conversation cannot outlive it.
        if !self.allow_agent {
            self.mode = ToolMode::Chat;
        }
        self
    }

    /// What one sender may do. An unknown id gets the channel fallback, which is
    /// also what a new allowlist entry receives on the next normalisation.
    pub fn user(&self, user_id: &str) -> MessagingUser {
        self.users
            .get(user_id.trim())
            .cloned()
            .unwrap_or_else(|| MessagingUser {
                mode: if self.allow_agent {
                    self.mode
                } else {
                    ToolMode::Chat
                },
                allow_agent: self.allow_agent,
                permission: self.permission,
                access: self.access,
                allow_local_start: self.allow_local_start,
                allow_stt_start: false,
            })
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

    /// Local engine starts are per-person and off by default: both load something
    /// that can evict the host's current model.
    #[test]
    fn local_starts_are_per_id_and_off_by_default() {
        let settings = MessagingSettings {
            allowed_user_ids: vec!["1".into(), "2".into()],
            users: BTreeMap::from([(
                "2".to_string(),
                MessagingUser {
                    allow_local_start: true,
                    allow_stt_start: true,
                    ..MessagingUser::default()
                },
            )]),
            ..MessagingSettings::default()
        }
        .normalized();
        let first = settings.user("1");
        let second = settings.user("2");
        assert!(!first.allow_local_start && !first.allow_stt_start);
        assert!(second.allow_local_start && second.allow_stt_start);
    }

    /// **Scheduling is a per-person switch.** Off in the default record, and kept
    /// when that id is granted it deliberately.
    #[test]
    fn scheduling_is_off_by_default_and_can_be_granted() {
        assert!(!MessagingUser::default().access.jobs);
        let granted = MessagingUser {
            access: AccessSet::ALL,
            ..MessagingUser::default()
        };
        assert!(granted.access.jobs);
    }

    /// An old channel record that had Agent off keeps that ceiling for every
    /// existing id, and an old Agent session cannot bring tools back.
    #[test]
    fn a_legacy_agent_ceiling_reaches_every_allowed_id() {
        let settings = MessagingSettings {
            allowed_user_ids: vec!["1".into()],
            allow_agent: false,
            mode: ToolMode::Agent,
            ..MessagingSettings::default()
        }
        .normalized();
        let user = settings.user("1");
        assert!(!user.allow_agent);
        assert_eq!(user.mode, ToolMode::Chat);
        assert_eq!(user.mode_for(Some(ToolMode::Agent)), ToolMode::Chat);
        assert_eq!(user.mode_for(None), ToolMode::Chat);
    }

    /// Two ids are two permissions: one may run tools, the other may not.
    #[test]
    fn access_is_kept_per_telegram_id() {
        let restricted = MessagingUser {
            allow_agent: false,
            access: AccessSet {
                terminal: false,
                mcp: false,
                ..AccessSet::ALL
            },
            ..MessagingUser::default()
        };
        let settings = MessagingSettings {
            allowed_user_ids: vec!["1".into(), "2".into()],
            users: BTreeMap::from([("2".to_string(), restricted)]),
            ..MessagingSettings::default()
        }
        .normalized();

        let first = settings.user("1");
        let second = settings.user("2");
        assert!(first.access.terminal && first.access.mcp);
        assert!(!second.access.terminal && !second.access.mcp);
        assert!(!second.allow_agent);
        assert_eq!(second.mode, ToolMode::Chat);
        // A removed id loses its policy rather than leaving it available to a
        // future id that reuses the number.
        let trimmed = MessagingSettings {
            allowed_user_ids: vec!["1".into()],
            ..settings
        }
        .normalized();
        assert!(!trimmed.users.contains_key("2"));
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
