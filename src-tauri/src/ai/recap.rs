//! What a reply's tools returned, carried on that reply's own text.
//!
//! # Why this exists
//!
//! The provider is only ever sent `{role, content}` -- `prepare_messages` drops every
//! other field -- so the steps behind a reply never leave the app. For the panel that
//! is right; for the model it means it remembers what it *said* and has no record of
//! what it *saw*, so a conversation that used tools cannot explain its own work one
//! turn later. This is that record.
//!
//! # Why it is whole, and why that is not a cache problem
//!
//! Nothing here clips a result. A bounded record was the bug it replaced: the model
//! read 80k tokens of files during one reply, and the next turn was handed a
//! thousand, so it re-read them and paid for the same work twice. Each result is
//! already bounded once, by [`crate::ai::tools::output::MAX_TOOL_OUTPUT`] at the
//! moment the model is given it, so carrying it whole costs exactly what the model
//! already read and no more.
//!
//! The provider caches a prefix, and this text is derived from stored steps, so the
//! bytes for a finished reply never move between turns: a bigger record is a larger
//! cached prefix, not a miss. What it must never become is a size computed at send
//! time -- from the window, or from how many turns have passed -- because then an
//! older reply's record would shrink as the conversation grew and every token after
//! it would be invalidated. A window that is genuinely full is handled by
//! [`crate::ai::compact`], which elides the oldest records on the request copy.
//!
//! # Why it is derived rather than stored
//!
//! The same activity always produces the same text, so the bytes are stable by
//! construction and nothing has to be migrated or kept in step with the steps it
//! summarises. Three callers need the identical bytes -- the chat view (through
//! `ai_recap_messages`), a job, and a message from a channel -- and one
//! implementation is the only way those three cannot disagree.

use serde_json::Value;

/// The marker that separates a reply's text from the record of its work.
pub const MARKER: &str = "[What I did in that reply]";

/// Appends the record of a reply's tools to its text.
///
/// Idempotent: a message that already carries the marker is returned untouched. Two
/// passes would otherwise grow the message on every turn, which is exactly the cache
/// invalidation this exists to avoid.
pub fn append(content: &str, activity: &Value) -> String {
    if content.contains(MARKER) {
        return content.to_string();
    }
    let Some(steps) = activity.as_array() else {
        return content.to_string();
    };
    let tools: Vec<&Value> = steps
        .iter()
        .filter(|step| step.get("kind").and_then(Value::as_str) == Some("tool"))
        .collect();
    if tools.is_empty() {
        return content.to_string();
    }

    let mut lines = vec![String::new(), String::new(), MARKER.to_string()];
    for step in tools {
        let label = step.get("label").and_then(Value::as_str).unwrap_or("Tool");
        let detail = step.get("detail").and_then(Value::as_str).unwrap_or("");
        let failed = step.get("failed").and_then(Value::as_bool).unwrap_or(false);
        let mut line = format!("- {label}");
        if !detail.trim().is_empty() {
            line.push_str(": ");
            line.push_str(detail.trim());
        }
        if failed {
            line.push_str(" (failed)");
        }
        lines.push(line);

        // What the model was handed, in the model's own copy of it. `context` is
        // the whole text for the cases the panel shortens; `output` is the panel's
        // copy when that is already everything; `outcome` is the last resort for a
        // result whose body was never worth showing a reader.
        let body = ["context", "output", "outcome"]
            .iter()
            .find_map(|key| {
                step.get(*key)
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
            })
            .map(str::to_string);
        let Some(body) = body else { continue };
        for line in body.lines() {
            lines.push(format!("  {line}"));
        }
    }
    format!("{content}{}", lines.join("\n"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn activity() -> Value {
        json!([
            { "kind": "thought", "text": "thinking", "running": false },
            { "kind": "tool", "tool": "read_file", "label": "Read file", "detail": "src/App.tsx", "outcome": "12 lines", "failed": false },
            { "kind": "tool", "tool": "run_terminal", "label": "Run terminal", "detail": "cargo test", "output": "ok\n1 passed", "failed": false },
            { "kind": "tool", "tool": "read_file", "label": "Read file", "detail": "gone.rs", "failed": true },
        ])
    }

    /// The record names each call and carries what it produced -- which is the whole
    /// point: the next turn can see what the last one did.
    #[test]
    fn a_reply_carries_its_tool_calls_and_results() {
        let text = append("It works.", &activity());
        assert!(text.starts_with("It works.\n\n[What I did in that reply]\n"));
        assert!(text.contains("- Read file: src/App.tsx"));
        assert!(text.contains("  12 lines"));
        assert!(text.contains("- Run terminal: cargo test"));
        assert!(text.contains("  ok\n  1 passed"));
        assert!(text.contains("- Read file: gone.rs (failed)"));
        // A thought is not a tool and is not carried.
        assert!(!text.contains("thinking"));
    }

    /// The model's own copy wins over the panel's, and is carried whole. This is the
    /// rule that stops a reply's tool output from vanishing on the next turn.
    #[test]
    fn the_models_copy_is_preferred_and_carried_whole() {
        let whole = format!("{}{}", "a".repeat(5_000), "z".repeat(5_000));
        let steps = json!([
            { "kind": "tool", "label": "Read file", "detail": "big.rs",
              "output": "short panel copy", "context": whole, "failed": false },
        ]);
        let text = append("", &steps);
        assert!(text.contains(&"a".repeat(5_000)), "the whole body is kept");
        assert!(text.contains(&"z".repeat(5_000)));
        assert!(!text.contains("short panel copy"), "{text}");
    }

    /// A failure's text travels too: the model was given the reason, and a record
    /// that drops it leaves the next turn unable to say why a call failed.
    #[test]
    fn a_failure_keeps_the_reason_the_model_read() {
        let steps = json!([
            { "kind": "tool", "label": "Read file", "detail": "gone.rs",
              "outcome": "no such file", "context": "no such file or directory", "failed": true },
        ]);
        let text = append("", &steps);
        assert!(text.contains("no such file or directory"), "{text}");
    }

    /// Running it twice must not grow the message: that would invalidate the cached
    /// prefix on every turn, which is the cost this module exists to avoid.
    #[test]
    fn the_record_is_added_once() {
        let once = append("answer", &activity());
        assert_eq!(append(&once, &activity()), once);
    }

    /// A reply with no tools is left exactly as it was, so a plain answer keeps the
    /// bytes it always had.
    #[test]
    fn a_reply_with_no_tools_is_untouched() {
        let text = append("answer", &json!([{ "kind": "thought", "text": "hmm" }]));
        assert_eq!(text, "answer");
        assert_eq!(append("answer", &Value::Null), "answer");
    }
}
