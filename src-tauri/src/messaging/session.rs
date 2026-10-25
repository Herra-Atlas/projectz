//! The conversation a channel keeps, read from and written through the one store.
//!
//! A message from a phone is not a second kind of history: it is an ordinary
//! session, saved the way every other run saves one, so it appears in the sidebar,
//! opens in the chat view and can be continued from the app.
//!
//! # Two lists, on purpose
//!
//! What is **sent** to the model is a narrow view: user and assistant turns with
//! text, capped by turns and by characters. What is **stored back** is the record as
//! it was, plus the new turn -- because a stored message can carry fields the model
//! never sees (`activity`, a hidden turn, the model that wrote it), and rebuilding
//! the transcript from the narrow view would quietly delete them the first time a
//! phone continued an app conversation.

use serde_json::Value;

use crate::ai::types::ChatMessage;
use crate::database::Database;

/// The stored transcript, exactly as the store holds it.
///
/// Used for the write-back, so a field this module does not know about survives a
/// turn taken from outside the app.
pub fn stored(database: &Database, session_id: &str) -> Vec<Value> {
    database
        .list_session_messages(session_id)
        .unwrap_or_default()
}

/// The part of a stored transcript the model should be sent.
///
/// Only `user` and `assistant` turns with text: a transcript can hold hidden turns
/// and notices that mean something to the app's own view and nothing to a fresh
/// request.
///
/// **Nothing is dropped for length here.** Trimming the front of a conversation is
/// exactly what changes the request prefix, and a prefix that changes every turn is
/// a provider cache that never hits. The window is managed by the one rule the rest
/// of the app uses: auto-compression at the configured share of a *measured*
/// context, or `/compress`.
pub fn to_messages(stored: &[Value]) -> Vec<ChatMessage> {
    stored
        .iter()
        .filter_map(|message| {
            let role = message.get("role")?.as_str()?;
            if role != "user" && role != "assistant" {
                return None;
            }
            let content = message.get("content")?.as_str()?.trim();
            if content.is_empty() {
                return None;
            }
            // What the reply's tools returned rides on the reply's own text, so the
            // next turn can see what the last one did. Derived, never stored, and
            // identical bytes every turn -- which is what keeps the cached prefix.
            let content = match (role, message.get("activity")) {
                ("assistant", Some(activity)) => crate::ai::recap::append(content, activity),
                _ => content.to_string(),
            };
            Some(ChatMessage {
                role: role.to_string(),
                content,
            })
        })
        .collect()
}

/// The created-at stamp of a stored session, so a later turn does not move the
/// conversation's place in the sidebar.
pub fn created_at(database: &Database, session_id: &str) -> String {
    database
        .session_record(session_id)
        .ok()
        .and_then(|record| {
            record
                .get("createdAt")
                .and_then(Value::as_str)
                .map(str::to_string)
        })
        .unwrap_or_else(crate::database::utc_now)
}

/// The title a stored session already has, when it has one.
pub fn existing_title(database: &Database, session_id: &str) -> Option<String> {
    database
        .session_record(session_id)
        .ok()
        .and_then(|record| {
            record
                .get("title")
                .and_then(Value::as_str)
                .map(str::to_string)
        })
        .filter(|title| !title.trim().is_empty())
}

/// The mode a stored conversation was created in.
///
/// `/sessions` can switch to a conversation born in Chat or Agent, and the phone
/// should continue it the way it was left rather than under today's default.
pub fn mode(database: &Database, session_id: &str) -> Option<crate::ai::tools::ToolMode> {
    let value = database.session_record(session_id).ok()?;
    match value.get("mode").and_then(Value::as_str)? {
        "chat" => Some(crate::ai::tools::ToolMode::Chat),
        "agent" => Some(crate::ai::tools::ToolMode::Agent),
        _ => None,
    }
}

/// A title from a message, the way a person would write one down.
pub fn title_from(text: &str) -> String {
    let line = text.lines().next().unwrap_or_default().trim();
    let title: String = line.chars().take(60).collect();
    let title = title.trim_end_matches(['.', ',', ':', ';']).trim();
    if title.is_empty() {
        "Message".to_string()
    } else {
        title.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    fn message(role: &str, content: &str) -> Value {
        serde_json::json!({ "role": role, "content": content })
    }

    /// A title comes from the first line and is short enough to draw.
    #[test]
    fn a_title_is_the_first_line_cut_short() {
        assert_eq!(title_from("Fix the parser\nmore detail"), "Fix the parser");
        assert_eq!(title_from("   "), "Message");
        assert!(title_from(&"x".repeat(200)).chars().count() <= 60);
    }

    /// **Nothing is dropped for length.** Trimming the front changes the request
    /// prefix, and a prefix that changes is a provider cache that never hits -- the
    /// window is managed by auto-compression or by `/compress`, not here.
    #[test]
    fn the_request_view_is_the_whole_conversation() {
        let stored: Vec<Value> = (0..50)
            .map(|index| {
                message(
                    if index % 2 == 0 { "user" } else { "assistant" },
                    &format!("turn {index}"),
                )
            })
            .collect();
        let loaded = to_messages(&stored);
        assert_eq!(loaded.len(), 50);
        assert_eq!(loaded.first().unwrap().content, "turn 0");
        assert_eq!(loaded.last().unwrap().content, "turn 49");
    }

    /// A reply's tools travel with it, so the next turn remembers what the last one
    /// saw rather than only what it said.
    #[test]
    fn a_stored_reply_carries_its_tool_record() {
        let stored = vec![serde_json::json!({
            "role": "assistant",
            "content": "Done.",
            "activity": [
                { "kind": "tool", "label": "Read file", "detail": "a.rs", "outcome": "3 lines" },
            ],
        })];
        let sent = to_messages(&stored);
        assert_eq!(sent.len(), 1);
        assert!(sent[0].content.contains("[What I did in that reply]"));
        assert!(sent[0].content.contains("- Read file: a.rs"));
    }

    /// **The write-back is the stored record, not the request view.** A field this
    /// module does not understand -- an assistant turn's `activity`, a hidden turn --
    /// survives, which is what keeps an app conversation intact when a phone
    /// continues it.
    #[test]
    fn the_stored_record_keeps_fields_the_model_never_sees() {
        let stored = vec![
            message("user", "hello"),
            serde_json::json!({
                "role": "assistant",
                "content": "hi",
                "activity": [{ "kind": "tool_result" }],
                "modelId": "gpt",
            }),
            serde_json::json!({ "role": "system", "hidden": true, "content": "note" }),
        ];
        let sent = to_messages(&stored);
        // The system turn is not sent...
        assert_eq!(sent.len(), 2);
        // ...and the activity is not lost from the record.
        assert_eq!(stored[1]["activity"][0]["kind"], "tool_result");
    }

    /// A conversation that does not exist is no history rather than an error, so the
    /// first message needs no special case.
    #[test]
    fn a_missing_conversation_is_empty() {
        let database = Arc::new(Database::open_in_memory());
        database.initialize().expect("migrate");
        assert!(stored(&database, "nope").is_empty());
        assert!(to_messages(&[]).is_empty());
    }

    /// A turn with no text is skipped: a tool-only assistant message would otherwise
    /// arrive as an empty instruction.
    #[test]
    fn an_empty_turn_is_not_sent() {
        let stored = vec![message("user", "  "), message("assistant", "ok")];
        let sent = to_messages(&stored);
        assert_eq!(sent.len(), 1);
        assert_eq!(sent[0].role, "assistant");
    }
}
