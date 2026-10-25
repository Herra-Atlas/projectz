//! The vocabulary every messaging channel speaks.
//!
//! A channel is a transport, not a feature: it delivers text from somewhere and
//! takes text back. Everything that decides *what* is said -- the agent run, the
//! slash commands, the approval relay -- lives above this file, so adding Discord
//! later means one more adapter and nothing else.

use std::path::PathBuf;

use serde::Serialize;

/// What a channel can draw, so the core never asks for something it cannot do.
///
/// A capability is a fact about the transport, not a preference. Telegram has
/// inline buttons and edits; a channel that has only plain messages still works,
/// because the core asks rather than assumes.
#[derive(Clone, Copy, Debug)]
pub struct Capabilities {
    /// Whether a message can carry buttons.
    pub buttons: bool,
    /// Whether a sent message can be rewritten in place, so a long run draws one
    /// row that grows rather than a message per step.
    pub edit: bool,
    /// Whether a "typing" hint can be shown while a run is going.
    pub typing: bool,
    /// The longest text one message may carry. Splitting is the core's job because
    /// it is about the message, not about the transport's spelling of it.
    pub max_chars: usize,
    /// Whether files can be received and sent.
    pub files: bool,
    /// Whether a message the app sent can be removed again. Telegram can delete,
    /// which is how a finished run's working message stops being furniture.
    pub delete: bool,
}

/// A file named by an inbound message, before its bytes are fetched.
///
/// The id is the transport's handle (`file_id` on Telegram); the media type and
/// size are hints for deciding whether to fetch at all, not trusted metadata.
#[derive(Clone, Debug)]
pub struct InboundFile {
    pub id: String,
    pub name: String,
    pub mime_type: String,
    pub size: Option<u64>,
}

/// The bytes of one inbound file.
#[derive(Clone, Debug)]
pub struct DownloadedFile {
    pub name: String,
    pub mime_type: String,
    pub bytes: Vec<u8>,
}

/// A file the app wants to send back.
///
/// The path is already resolved inside the workspace by the core. A transport
/// reads it rather than being handed a copy, so a large PDF is streamed once
/// instead of living in memory through the channel trait.
#[derive(Clone, Debug)]
pub struct OutboundFile {
    pub path: PathBuf,
    pub name: String,
}

/// One button under a message.
///
/// `data` is opaque to the core beyond its prefixes (`ap:`, `m:`, `s:`, `x:`), and
/// each adapter must be able to carry it unchanged: Telegram caps callback data at
/// 64 bytes, which is why ids and indexes are used rather than sentences.
#[derive(Clone, Debug)]
pub struct Button {
    pub label: String,
    pub data: String,
}

impl Button {
    pub fn new(label: impl Into<String>, data: impl Into<String>) -> Self {
        Self {
            label: label.into(),
            data: data.into(),
        }
    }
}

/// Why a message arrived.
#[derive(Clone, Debug)]
pub enum InboundKind {
    /// A typed message.
    Message,
    /// A press on one of our own buttons.
    Action { id: String, data: String },
}

/// One thing the user did, as the core understands it.
///
/// `user_id` is the allowlist key and is the channel's own stable identifier --
/// Telegram's numeric user id, which unlike a username cannot be changed or handed
/// to someone else. `chat_id` is only the delivery address; for the private chats
/// this bot answers it happens to equal `user_id`, but it is kept separate because
/// it is the field a transport sends to.
#[derive(Clone, Debug)]
pub struct Inbound {
    pub user_id: String,
    pub chat_id: String,
    /// A readable name, used when the user is told their own id.
    pub user_name: String,
    pub text: String,
    pub kind: InboundKind,
    /// Files sent with this message. Empty for a plain text turn and for a press.
    pub files: Vec<InboundFile>,
}

impl Inbound {
    /// The pressed button's id and data, when this was a press.
    pub fn action(&self) -> Option<(&str, &str)> {
        match &self.kind {
            InboundKind::Action { id, data } => Some((id, data)),
            InboundKind::Message => None,
        }
    }
}

/// What a channel is doing, for the Settings page.
///
/// `identity` is the channel's own name for itself -- a bot's `@name` -- so the
/// page can say which account is connected without the core knowing what a bot is.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChannelStatus {
    pub id: &'static str,
    pub name: &'static str,
    pub enabled: bool,
    pub running: bool,
    pub identity: Option<String>,
    pub error: Option<String>,
}

/// The id a message was given, so a later edit can reach it.
#[derive(Clone, Debug)]
pub struct SentMessage {
    pub id: String,
}
