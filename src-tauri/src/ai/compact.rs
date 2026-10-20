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

/// The fraction of a known context window that must be occupied before elision.
const COMPACTION_THRESHOLD_PERCENT: u64 = 80;

/// Whether a measured request is close enough to the model's limit to compact.
pub fn should_compact(
    mode: Compaction,
    prompt_tokens: Option<u64>,
    context_window: Option<u64>,
) -> bool {
    if mode == Compaction::Off {
        return false;
    }
    let (Some(prompt_tokens), Some(context_window)) = (prompt_tokens, context_window) else {
        return false;
    };
    context_window > 0
        && prompt_tokens
            >= context_window / 100 * COMPACTION_THRESHOLD_PERCENT
                + (context_window % 100 * COMPACTION_THRESHOLD_PERCENT).div_ceil(100)
}

/// What the transcript says when this mode is applied.
///
/// The words name what actually happened rather than saying "context compacted",
/// because "old tool output was elided" and "older turns were dropped" are
/// different losses and the reader is entitled to know which one they got.
pub fn notice(mode: Compaction) -> &'static str {
    match mode {
        Compaction::Off => "",
        Compaction::Normal => {
            "Context compacted — older tool output was elided to keep this reply within the model's window."
        }
        Compaction::Fast => {
            "Context compacted — older turns were dropped and old tool output was elided to keep this reply within the model's window."
        }
    }
}

/// Compacts the request in place once usage reaches the configured threshold.
///
/// `messages` is the array about to be sent, in order, with any leading `system`
/// message first. The caller checks measured token usage before invoking this.
pub fn apply(messages: &mut Vec<Value>, mode: Compaction) {
    let recent = match mode {
        Compaction::Off => return,
        Compaction::Normal => 12,
        Compaction::Fast => 8,
    };
    if messages.len() <= recent {
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
    //
    // A body that is **already** the stub is left alone rather than rewritten with the
    // same bytes. That is what makes a second pass a no-op: the stub is the state, not a
    // step, so re-applying the mode nothing left to do -- which is what lets the manual
    // `/compact` report "nothing to elide" instead of claiming a saving of zero.
    for message in messages.iter_mut().take(keep_from) {
        if message.get("role").and_then(Value::as_str) != Some("tool") {
            continue;
        }
        if let Some(object) = message.as_object_mut() {
            let already = object.get("content").and_then(Value::as_str) == Some(TOOL_STUB);
            if !already {
                object.insert("content".into(), Value::String(TOOL_STUB.to_string()));
            }
        }
    }
}

/// Compacts a stored transcript that no run is sending right now.
///
/// **The manual counterpart to `apply`**, for the composer's `/compact`. `apply` edits a
/// request copy and is only ever reached from inside a run; this is reached from a button,
/// so it has to decide for itself whether the conversation is long enough to be worth
/// trimming *without* a measured token count -- there is no provider figure for a
/// transcript sitting in the database.
///
/// The mode is the user's stored preference, because the command is not a second setting:
/// it runs what the next reply would run. `Off` is therefore a refusal rather than a silent
/// no-op, and a transcript with nothing to elide (a short one, or no tool results at all)
/// reports zero bytes saved so the composer can say there was nothing to do instead of
/// claiming success.
///
/// The notice is left as a **hidden user turn** in the stored messages, the same shape the
/// run's own notice takes: the transcript on screen draws it as a centred line, and the
/// model receives it on the next request, so it knows history was trimmed.
///
/// **Idempotent.** A conversation that was just compacted has nothing left for a second
/// pass to elide, so a second `/compact` reports zero and writes nothing -- which is what
/// stops repeated use from burying the transcript in notices.
pub fn apply_to_history(messages: &mut Vec<Value>, mode: Compaction) -> CompactOutcome {
    let before = messages_size(messages);
    if mode == Compaction::Off {
        return CompactOutcome {
            removed: 0,
            mode: "off".into(),
        };
    }
    // `apply` leaves a conversation of `recent` messages or fewer alone; this is checked
    // too so that a short conversation is not stamped with a notice about nothing.
    if messages.len() <= matching_recent(mode) {
        return CompactOutcome {
            removed: 0,
            mode: name_of(mode).into(),
        };
    }
    // "Is there anything left to do?" is asked of the messages that would actually change,
    // not of their count. Both modes only elide **un-elided tool output** in the old
    // window -- `Fast` drops the turns around it, but a tool result already stubbed is not
    // a result any more -- and both need one at least once before either is worth running
    // again. This is what stops a second `/compact` from writing a second notice over the
    // first.
    let keep_from = messages.len() - matching_recent(mode);
    let untouched_tool_output = messages.iter().take(keep_from).any(|message| {
        message.get("role").and_then(Value::as_str) == Some("tool")
            && message.get("content").and_then(Value::as_str) != Some(TOOL_STUB)
    });
    if !untouched_tool_output {
        return CompactOutcome {
            removed: 0,
            mode: name_of(mode).into(),
        };
    }
    apply(messages, mode);
    let removed = before.saturating_sub(messages_size(messages));
    if removed == 0 {
        return CompactOutcome {
            removed: 0,
            mode: name_of(mode).into(),
        };
    }
    let notice = notice(mode).to_string();
    // After the leading system block: a `system` message has to stay first for llama.cpp's
    // templates, and the notice is a turn like any other from there on.
    let insert_at = messages
        .iter()
        .take_while(|message| message.get("role").and_then(Value::as_str) == Some("system"))
        .count();
    messages.insert(
        insert_at,
        json!({ "role": "user", "content": notice, "hidden": true }),
    );
    CompactOutcome {
        removed,
        mode: name_of(mode).into(),
    }
}

/// What one manual compaction did.
#[derive(Clone, Debug)]
pub struct CompactOutcome {
    /// Bytes of message content saved. Zero means the transcript was already compact.
    pub removed: usize,
    /// The mode that ran, as its stored wire name -- logged, and reported by the command.
    pub mode: String,
}

/// The message count `apply` treats as "recent" for a mode.
fn matching_recent(mode: Compaction) -> usize {
    match mode {
        Compaction::Off => usize::MAX,
        Compaction::Normal => 12,
        Compaction::Fast => 8,
    }
}

fn name_of(mode: Compaction) -> &'static str {
    match mode {
        Compaction::Off => "off",
        Compaction::Normal => "normal",
        Compaction::Fast => "fast",
    }
}

/// The characters in a transcript's content, read through the same protocol `apply` edits.
///
/// Counted from the stored messages rather than from a token estimate: the number is meant
/// to tell a user "this saved something", not to predict a bill, and a character count is
/// exact where an estimate would be a second opinion about someone else's tokeniser.
fn messages_size(messages: &[Value]) -> usize {
    messages
        .iter()
        .map(|message| {
            message
                .get("content")
                .and_then(Value::as_str)
                .map(str::len)
                .unwrap_or(0)
        })
        .sum()
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
    fn compaction_waits_until_eighty_percent_of_a_known_window() {
        assert!(!should_compact(
            Compaction::Normal,
            Some(7_999),
            Some(10_000)
        ));
        assert!(should_compact(
            Compaction::Normal,
            Some(8_000),
            Some(10_000)
        ));
        assert!(should_compact(Compaction::Fast, Some(8_000), Some(10_000)));
        assert!(should_compact(Compaction::Normal, Some(1), Some(1)));
        assert!(!should_compact(Compaction::Normal, Some(0), Some(1)));
        assert!(!should_compact(Compaction::Normal, None, Some(10_000)));
        assert!(!should_compact(Compaction::Normal, Some(8_000), None));
        assert!(!should_compact(Compaction::Normal, Some(1), Some(0)));
        assert!(!should_compact(Compaction::Off, Some(10_000), Some(10_000)));
    }

    /// The notice describes the loss the mode actually causes. A user who chose
    /// Fast is told turns were dropped, not just that something was "compacted".
    #[test]
    fn the_notice_names_what_the_mode_removed() {
        assert!(notice(Compaction::Normal).contains("tool output"));
        assert!(!notice(Compaction::Normal).contains("turns were dropped"));
        assert!(notice(Compaction::Fast).contains("turns were dropped"));
        assert!(notice(Compaction::Off).is_empty());
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

    /// The manual path leaves a hidden notice the transcript draws, and a second pass has
    /// nothing left to do -- so `/compact` twice does not stack notices on a conversation.
    #[test]
    fn a_manual_compaction_is_idempotent() {
        // Real tool output is much longer than the stub it becomes; `conversation`'s short
        // bodies would make the "elided something" check indistinguishable from noise.
        let mut messages = conversation(80);
        for message in messages.iter_mut() {
            if message.get("role").and_then(Value::as_str) == Some("tool") {
                message["content"] = Value::String("x".repeat(200));
            }
        }
        let first = apply_to_history(&mut messages, Compaction::Normal);
        assert!(first.removed > 0, "the first pass should elide something");
        assert_eq!(messages[0]["role"], "system");
        assert!(messages.iter().any(|message| message["hidden"] == true));

        let length_after_first = messages.len();
        let second = apply_to_history(&mut messages, Compaction::Normal);
        assert_eq!(second.removed, 0, "nothing is left to elide");
        assert_eq!(messages.len(), length_after_first, "no second notice");
    }

    /// `Off` is a refusal the caller reports, not a silent success.
    #[test]
    fn a_manual_compaction_does_nothing_when_off() {
        let mut messages = conversation(80);
        let before = messages.clone();
        let outcome = apply_to_history(&mut messages, Compaction::Off);
        assert_eq!(outcome.removed, 0);
        assert_eq!(messages, before);
    }

    /// A conversation with no tool output has nothing `Normal` can elide, and says so
    /// rather than claiming it saved something.
    #[test]
    fn a_manual_compaction_with_nothing_to_elide_reports_zero() {
        let mut messages = vec![json!({ "role": "system", "content": "s" })];
        for index in 1..40 {
            messages.push(json!({ "role": if index % 2 == 0 { "user" } else { "assistant" }, "content": "text" }));
        }
        let outcome = apply_to_history(&mut messages, Compaction::Normal);
        assert_eq!(outcome.removed, 0);
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
