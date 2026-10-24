//! What a reply's tools returned, carried on that reply's own text.
//!
//! # Why this exists
//!
//! The provider is only ever sent `{role, content}` -- `prepare_messages` drops every
//! other field -- so the steps behind a reply never leave the app. For the panel that
//! is right; for the model it means it remembers what it *said* and has no record of
//! what it *saw*, so a conversation that used the browser cannot explain its own work
//! one turn later.
//!
//! # Why it is derived rather than stored
//!
//! The same activity always produces the same text, so the request prefix does not
//! change between turns and the provider's prompt cache still matches. A separately
//! stored summary would drift from the steps it summarises, and one that changed
//! would invalidate every token after it.
//!
//! # Why it lives in Rust
//!
//! Three callers need the identical bytes: the chat view (through `ai_recap_messages`),
//! a job, and a message from a channel. One implementation is the only way those three
//! cannot disagree -- and a disagreement is a cache miss on every single turn.
//!
//! # Why it is budgeted
//!
//! A page tree is large, and an old one is worth a glance rather than a re-read. Each
//! result keeps its opening -- where a tool's own summary lives -- and a reply stops
//! adding results once its budget is spent.

use serde_json::Value;

/// Characters kept from one tool result.
const RESULT_BUDGET: usize = 1_200;

/// Characters kept from one reply's whole set of results.
const REPLY_BUDGET: usize = 4_000;

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
    let mut spent = 0usize;
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

        // What the model was handed, if the call produced anything worth carrying.
        let body = ["output", "outcome"]
            .iter()
            .find_map(|key| {
                step.get(*key)
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
            })
            .map(str::to_string);
        let Some(body) = body else { continue };
        if spent >= REPLY_BUDGET {
            lines.push("  … the rest of this reply's output is omitted.".to_string());
            break;
        }
        let room = RESULT_BUDGET.min(REPLY_BUDGET - spent);
        for line in clip(&body, room).lines() {
            lines.push(format!("  {line}"));
        }
        spent += body.chars().count().min(room);
    }
    format!("{content}{}", lines.join("\n"))
}

/// Cuts on a character boundary, so a multi-byte character is never split.
fn clip(text: &str, limit: usize) -> String {
    if text.chars().count() <= limit {
        return text.to_string();
    }
    let mut clipped: String = text.chars().take(limit).collect();
    clipped.push('…');
    clipped
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

    /// A long result is clipped rather than sent whole, and the budget is spent in
    /// the order the calls happened.
    #[test]
    fn a_long_result_is_clipped_and_the_budget_stops_the_rest() {
        let long = "x".repeat(RESULT_BUDGET + 50);
        let steps = json!([
            { "kind": "tool", "label": "Read file", "detail": "a", "outcome": long },
            { "kind": "tool", "label": "Read file", "detail": "b", "outcome": long },
            { "kind": "tool", "label": "Read file", "detail": "c", "outcome": long },
            { "kind": "tool", "label": "Read file", "detail": "d", "outcome": long },
            // The budget is spent by now; this one is only named, and the next
            // triggers the omission notice.
            { "kind": "tool", "label": "Read file", "detail": "e", "outcome": long },
            { "kind": "tool", "label": "Read file", "detail": "f", "outcome": long },
        ]);
        let text = append("", &steps);
        assert!(text.contains('…'));
        assert!(text.contains("the rest of this reply's output is omitted"));
    }

    /// The clip never splits a character in half.
    #[test]
    fn clipping_counts_characters_not_bytes() {
        let clipped = clip(&"é".repeat(10), 4);
        assert_eq!(clipped, "éééé…");
    }
}
