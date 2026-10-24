//! Turning a run into something a chat window can read.
//!
//! Two problems, one file: a message that is too long for the transport, and a
//! stream of events that has to become a handful of readable lines. Both are about
//! *the message* rather than about Telegram, so both live here and a second channel
//! inherits them.

use serde_json::Value;

use crate::ai::types::ChatEvent;

/// Splits text into messages the transport will accept.
///
/// **On a character boundary and, when possible, on a line one.** Telegram rejects
/// anything over 4096 characters, and a reply that is one character over is a reply
/// the user never sees. Cutting mid-codepoint would produce an invalid string, and
/// cutting mid-word makes a two-part answer read like a broken one, so the split
/// walks back to the last newline within a reasonable distance when there is one.
pub fn split_message(text: &str, max_chars: usize) -> Vec<String> {
    let max_chars = max_chars.max(1);
    let mut parts = Vec::new();
    let mut rest = text;
    while !rest.is_empty() {
        if rest.chars().count() <= max_chars {
            parts.push(rest.to_string());
            break;
        }
        // The byte index of the character that would start the next message.
        let mut cut_bytes = 0;
        let mut cut_chars = 0;
        for (index, _) in rest.char_indices() {
            if cut_chars == max_chars {
                cut_bytes = index;
                break;
            }
            cut_chars += 1;
        }
        if cut_bytes == 0 {
            // `max_chars` was reached on the final character.
            parts.push(rest.to_string());
            break;
        }
        let head = &rest[..cut_bytes];
        // A newline in the last quarter is a better place to break than mid-word;
        // a newline near the start would make a message of two words and push the
        // rest again and again. The floor is snapped back onto a character boundary
        // before it is used, because a byte offset taken from a character count is
        // not necessarily one.
        let mut floor = cut_bytes.saturating_sub(max_chars / 4);
        while floor > 0 && !rest.is_char_boundary(floor) {
            floor -= 1;
        }
        let break_at = head[floor..]
            .rfind('\n')
            .map(|offset| floor + offset + 1)
            .unwrap_or(cut_bytes);
        let (head, tail) = rest.split_at(break_at);
        if !head.trim().is_empty() {
            parts.push(head.to_string());
        }
        rest = tail;
    }
    parts
}

/// One finished tool call as a line a reader can scan.
///
/// **Only the result, never the call.** A call that is still running has nothing
/// to say yet, and drawing it twice -- once as "reading", once as "read" -- is two
/// lines for one fact. The reasoning behind a step is deliberately absent: a chat
/// window is not the place to replay a model's private deliberation, and the final
/// answer is the part that was asked for.
pub fn tool_line(event: &ChatEvent) -> Option<String> {
    if event.kind != "tool_result" {
        return None;
    }
    let metrics = event.metrics.as_ref()?;
    let label = metrics.get("label").and_then(Value::as_str)?;
    let detail = metrics
        .get("detail")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .trim();
    let failed = metrics
        .get("failed")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let seconds = metrics.get("seconds").and_then(Value::as_f64);
    // A cached call did not take the time its figure would suggest, and saying so
    // is the difference between a fast tool and a repeated one.
    let cached = metrics
        .get("cached")
        .and_then(Value::as_bool)
        .unwrap_or(false);

    let mut line = if failed { "✗ " } else { "• " }.to_string();
    line.push_str(label);
    if !detail.is_empty() {
        line.push(' ');
        line.push_str(detail);
    }
    let mut suffix = Vec::new();
    if cached {
        suffix.push("cached".to_string());
    }
    if let Some(seconds) = seconds {
        suffix.push(duration(seconds));
    }
    if !suffix.is_empty() {
        line.push_str(" · ");
        line.push_str(&suffix.join(" · "));
    }
    Some(line)
}

/// A duration as a person reads it, in whole units.
pub fn duration(seconds: f64) -> String {
    if seconds < 1.0 {
        format!("{}ms", (seconds * 1000.0).round().max(1.0) as u64)
    } else if seconds < 60.0 {
        format!("{seconds:.1}s")
    } else {
        let whole = seconds.round() as u64;
        format!("{}m {}s", whole / 60, whole % 60)
    }
}

/// The working message: what the run has done so far.
///
/// Deliberately not the answer. A run that is still reading files has no answer
/// yet, and a window that shows a half-built one teaches the reader to distrust the
/// finished one.
pub fn working_text(lines: &[String]) -> String {
    if lines.is_empty() {
        "Working…".to_string()
    } else {
        format!("Working…\n\n{}", lines.join("\n"))
    }
}

/// The one-line summary left behind once the answer is sent.
pub fn worked_summary(lines: &[String]) -> String {
    match lines.len() {
        0 => "Worked.".to_string(),
        1 => format!("Worked · 1 step\n\n{}", lines[0]),
        count => format!("Worked · {count} steps"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn event(kind: &str, metrics: Value) -> ChatEvent {
        ChatEvent {
            run_id: "r".into(),
            session_id: None,
            sequence: 0,
            kind: kind.into(),
            text: None,
            error: None,
            metrics: Some(metrics),
        }
    }

    /// A message exactly at the limit is one message, not one and an empty one.
    #[test]
    fn text_at_the_limit_is_not_split() {
        let text = "a".repeat(10);
        assert_eq!(split_message(&text, 10), vec![text]);
    }

    /// The whole point: nothing is lost between parts, and no part is over the
    /// limit the transport enforces.
    #[test]
    fn every_part_is_within_the_limit_and_the_text_survives() {
        let text = (0..500)
            .map(|index| format!("line {index}\n"))
            .collect::<String>();
        let parts = split_message(&text, 100);
        assert!(parts.len() > 1);
        for part in &parts {
            assert!(part.chars().count() <= 100, "a part was too long");
        }
        assert_eq!(parts.concat(), text);
    }

    /// A multi-byte character is never cut in half, which would produce an invalid
    /// string and a message the transport refuses.
    #[test]
    fn a_split_never_breaks_a_character() {
        let text = "é".repeat(20);
        let parts = split_message(&text, 6);
        for part in &parts {
            assert!(part.chars().all(|character| character == 'é'));
        }
        assert_eq!(parts.concat(), text);
    }

    /// A long run's steps become lines, and a still-running call contributes
    /// nothing -- the result is the fact.
    #[test]
    fn only_finished_calls_become_lines() {
        let call = event(
            "tool_call",
            json!({ "label": "Read file", "detail": "a.rs" }),
        );
        assert!(tool_line(&call).is_none());
        let result = event(
            "tool_result",
            json!({ "label": "Read file", "detail": "a.rs", "seconds": 0.015 }),
        );
        assert_eq!(
            tool_line(&result).as_deref(),
            Some("• Read file a.rs · 15ms")
        );
    }

    /// A failure reads as one, and a cached call says so rather than claiming the
    /// time it did not spend.
    #[test]
    fn a_failure_and_a_cache_are_visible() {
        let failed = event(
            "tool_result",
            json!({ "label": "Run terminal", "detail": "cargo test", "failed": true }),
        );
        assert!(tool_line(&failed).unwrap().starts_with('✗'));
        let cached = event(
            "tool_result",
            json!({ "label": "Read file", "cached": true }),
        );
        assert!(tool_line(&cached).unwrap().contains("cached"));
    }

    /// Durations are whole units -- a step that took 15 milliseconds is not "0.0s".
    #[test]
    fn durations_read_in_whole_units() {
        assert_eq!(duration(0.015), "15ms");
        assert_eq!(duration(1.24), "1.2s");
        assert_eq!(duration(65.0), "1m 5s");
    }
}
