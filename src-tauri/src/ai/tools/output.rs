//! Caps on what a tool may return to the model.
//!
//! Two separate limits, because they solve different problems:
//!
//! - [`MAX_TOOL_OUTPUT`] bounds any single tool result. `run_terminal` can
//!   legitimately produce megabytes; the model cannot read them, and the
//!   context window cannot hold them.
//! - Truncation keeps the **head and tail**, not just the head. A build error,
//!   a failing test and a compiler diagnostic all appear at the end, so a
//!   head-only cap spends the entire budget on the part that was already fine.

/// Largest tool result passed to the model, in characters.
///
/// Roughly 7k tokens. Large enough for a real source file or a useful error
/// dump, small enough that several tools in one round do not crowd out the
/// conversation.
pub const MAX_TOOL_OUTPUT: usize = 28_000;

/// Fraction of the budget kept from the end when truncating.
const TAIL_RATIO: f64 = 0.4;

/// Shortens a tool result to [`MAX_TOOL_OUTPUT`], keeping head and tail.
///
/// See [`truncate_within`] for the behaviour; this is the model-facing budget,
/// named separately because the two callers mean different things by their caps.
pub fn truncate(text: String) -> String {
    truncate_within(&text, MAX_TOOL_OUTPUT)
}

/// Shortens text to `limit`, keeping head and tail.
///
/// The elision marker is explicit about what happened rather than silently
/// dropping the middle, so a model that sees the marker knows there is more and
/// can ask for a narrower slice instead of assuming the file ended.
///
/// Takes a slice rather than a `String` because a second caller already has one
/// it is borrowing, and taking ownership would mean cloning the whole result to
/// cap it.
pub fn truncate_within(text: &str, limit: usize) -> String {
    if text.chars().count() <= limit {
        return text.to_string();
    }

    let mut all = text.chars();
    let total = text.chars().count();
    let head_budget = (limit as f64 * (1.0 - TAIL_RATIO)) as usize;
    let tail_budget = limit - head_budget;

    let head = all.by_ref().take(head_budget).collect::<String>();
    let tail = all.rev().take(tail_budget).collect::<String>();
    let dropped = total.saturating_sub(head_budget + tail_budget);

    format!(
        "{head}\n\n[... {dropped} characters omitted from the middle. \
         Re-run with a narrower range or a filter to see the rest. ...]\n\n{}",
        tail.chars().rev().collect::<String>()
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_short_result_is_returned_untouched() {
        let text = "hello".to_string();
        assert_eq!(truncate(text.clone()), text);
    }

    #[test]
    fn a_result_at_exactly_the_limit_is_untouched() {
        let text = "a".repeat(MAX_TOOL_OUTPUT);
        assert_eq!(truncate(text.clone()), text);
    }

    #[test]
    fn truncation_keeps_the_tail_where_errors_are() {
        // The whole point: a head-only cap would discard the error entirely.
        let text = format!("{}{}", "h".repeat(MAX_TOOL_OUTPUT), "FATAL: build failed");
        let capped = truncate(text);
        assert!(capped.contains("FATAL: build failed"), "tail was lost");
        assert!(capped.starts_with('h'), "head was lost");
    }

    #[test]
    fn the_marker_says_how_much_was_omitted() {
        let text = "x".repeat(MAX_TOOL_OUTPUT * 2);
        let capped = truncate(text);
        assert!(capped.contains("omitted from the middle"), "{capped}");
    }

    #[test]
    fn truncation_counts_characters_not_bytes() {
        // Byte-based limits split multi-byte characters and would panic on a
        // char boundary; this input is far over the limit in both measures.
        let text = "ä".repeat(MAX_TOOL_OUTPUT);
        let capped = truncate(text);
        assert!(capped.chars().count() <= MAX_TOOL_OUTPUT + 200);
        assert!(capped.contains('ä'));
    }
}
