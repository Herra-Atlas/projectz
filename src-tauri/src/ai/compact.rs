//! Keeping a long conversation inside the model's window.
//!
//! A run can go on for a hundred tool rounds, and every round appends the
//! assistant's tool calls and their results -- a `read_file` of a large file, a
//! build log -- to the request. Left alone, the request grows until the provider
//! rejects it, and the failure arrives as an opaque error at the worst moment.
//!
//! # What it does, and what it deliberately does not
//!
//! Compaction here is *elision*, not summarisation. The messages that carry the
//! most tokens for the least value -- old tool results, which the model has
//! already read and acted on -- are replaced with a short stub, and the system
//! prompt and the recent turns are left exactly as they are. This needs no model
//! call, cannot invent a fact, and costs nothing but the tokens it saves.
//!
//! It is not a summary that rewrites history: that would need a second model call
//! per compaction, would itself spend context, and could state something the
//! conversation never said. When a summary is worth it, it is a deliberate feature
//! and not something a background pass should do silently.
//!
//! # Why this is safe to do to the request, not the transcript
//!
//! It edits the array handed to the provider for one send. The transcript in the
//! database is untouched -- the user still sees every result -- and the model is
//! told, by a stub, that something was removed rather than being shown a gap.

use serde_json::{json, Value};

/// How aggressively to compact, from `app.preferences.compaction`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Compaction {
    /// Never compact. The run fails on the provider's limit, which is what a user
    /// who wants the full context accepts.
    Off,
    /// Elide old tool results, keeping the system prompt and recent turns.
    Normal,
    /// Elide old tool results and drop older turns entirely.
    Fast,
}

impl Compaction {
    /// Parses the stored preference, defaulting to `Normal`.
    ///
    /// An absent or unknown value compacts rather than doing nothing: a
    /// conversation that dies on its own length is worse than one that has lost
    /// some old tool output, and `Normal` is the setting most users want.
    pub fn parse(value: Option<&str>) -> Self {
        match value.map(str::trim) {
            Some("off") => Compaction::Off,
            Some("fast") => Compaction::Fast,
            _ => Compaction::Normal,
        }
    }
}

/// What a tool result becomes once it is elided.
const TOOL_STUB: &str = "[tool output elided to keep the conversation within the context window]";

/// The note left where older turns were dropped, so the model knows the history
/// is not continuous rather than assuming it begins at the first message.
const TRIM_NOTICE: &str =
    "[earlier messages were dropped to fit the context window; ask if you need something from them]";

/// Compacts the request in place.
///
/// `messages` is the array about to be sent, in order, with any leading `system`
/// message first. A conversation short enough to send unchanged is returned
/// untouched, so this costs nothing on the common case.
pub fn apply(messages: &mut Vec<Value>, mode: Compaction) {
    let (limit, recent) = match mode {
        Compaction::Off => return,
        // The limits are in messages, not tokens, because counting tokens here
        // would mean a tokenizer per provider. A round is a handful of messages,
        // so forty is roughly the length at which a request starts to hurt.
        Compaction::Normal => (40, 12),
        Compaction::Fast => (24, 8),
    };
    if messages.len() <= limit {
        return;
    }
    let keep_from = messages.len() - recent;

    if mode == Compaction::Fast {
        // Drop the older turns outright, keeping every system message wherever it
        // sits and the whole recent window.
        let mut kept: Vec<Value> = Vec::with_capacity(recent + 1);
        for (index, message) in std::mem::take(messages).into_iter().enumerate() {
            let is_system = message.get("role").and_then(Value::as_str) == Some("system");
            if is_system || index >= keep_from {
                kept.push(message);
            }
        }
        // The notice goes after the leading system block, because a `system`
        // message has to stay first for llama.cpp's templates.
        let insert_at = kept
            .iter()
            .take_while(|message| message.get("role").and_then(Value::as_str) == Some("system"))
            .count();
        kept.insert(insert_at, json!({ "role": "user", "content": TRIM_NOTICE }));
        *messages = kept;
        return;
    }

    // Normal: keep every message but blank the body of the older tool results.
    for message in messages.iter_mut().take(keep_from) {
        if message.get("role").and_then(Value::as_str) != Some("tool") {
            continue;
        }
        if let Some(object) = message.as_object_mut() {
            object.insert("content".into(), Value::String(TOOL_STUB.to_string()));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn conversation(len: usize) -> Vec<Value> {
        let mut messages = vec![json!({ "role": "system", "content": "system prompt" })];
        for index in 1..len {
            let role = if index % 2 == 0 { "tool" } else { "assistant" };
            messages.push(json!({ "role": role, "content": format!("message {index}") }));
        }
        messages
    }

    #[test]
    fn off_changes_nothing() {
        let mut messages = conversation(80);
        let before = messages.clone();
        apply(&mut messages, Compaction::Off);
        assert_eq!(messages, before);
    }

    #[test]
    fn a_short_conversation_is_left_alone() {
        let mut messages = conversation(10);
        let before = messages.clone();
        apply(&mut messages, Compaction::Normal);
        assert_eq!(messages, before);
    }

    /// The system prompt and the recent window survive; only the old tool bodies
    /// are stubbed.
    #[test]
    fn normal_elides_old_tool_output_and_keeps_the_system_prompt() {
        let mut messages = conversation(80);
        apply(&mut messages, Compaction::Normal);
        assert_eq!(messages[0]["content"], "system prompt");
        assert_eq!(messages.len(), 80, "no message should be dropped in Normal");
        assert!(
            messages
                .iter()
                .any(|message| message["content"] == TOOL_STUB),
            "an old tool result should have been elided"
        );
        // The most recent tool result is inside the recent window and untouched.
        let last_tool = messages
            .iter()
            .rposition(|message| message.get("role").and_then(Value::as_str) == Some("tool"))
            .expect("a recent tool message");
        assert_ne!(messages[last_tool]["content"], TOOL_STUB);
    }

    /// Fast drops the old turns and says so, and the system prompt stays first.
    #[test]
    fn fast_drops_old_turns_and_leaves_a_notice() {
        let mut messages = conversation(80);
        apply(&mut messages, Compaction::Fast);
        assert_eq!(messages[0]["role"], "system");
        assert_eq!(messages[0]["content"], "system prompt");
        assert!(messages.len() < 80, "Fast should drop messages");
        assert!(
            messages
                .iter()
                .any(|message| message["content"] == TRIM_NOTICE),
            "the model must be told the history was trimmed"
        );
        // The notice sits after the system message, not before it.
        assert_eq!(messages[1]["content"], TRIM_NOTICE);
    }

    /// A value nobody recognises compacts rather than doing nothing, because a
    /// conversation that dies on its own length is the worse failure.
    #[test]
    fn an_unknown_preference_defaults_to_normal() {
        assert_eq!(Compaction::parse(None), Compaction::Normal);
        assert_eq!(Compaction::parse(Some("nonsense")), Compaction::Normal);
        assert_eq!(Compaction::parse(Some("off")), Compaction::Off);
        assert_eq!(Compaction::parse(Some("fast")), Compaction::Fast);
    }
}
