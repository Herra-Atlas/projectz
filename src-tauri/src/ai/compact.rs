//! Keeping a long conversation inside the model's window.
//!
//! A run can go on for a hundred tool rounds, and every round appends the
//! assistant's tool calls and their results -- a `read_file` of a large file, a
//! build log -- to the request. Left alone, the request grows until the provider
//! rejects it, and the failure arrives as an opaque error at the worst moment.
//!
//! # What it does, and what it deliberately does not
//!
//! The automatic pass is *mostly* elision, optionally a summary. Old tool results
//! -- the messages that carry the most tokens for the least value -- are replaced
//! with a short stub, and the system prompt and the protected recent tail are
//! left exactly as they are. This alone needs no model call and cannot invent a
//! fact. When that still does not reach the configured target there is a further
//! depth: the older turns are dropped outright, and -- only where a known window
//! and a model are available -- the dropped turns may be summarised by an
//! ordinary model call bounded by the user's timeout, and reinserted as one
//! message. The summary is a conservative addition: if it fails or times out,
//! the elided request is sent as it is.
//!
//! It is never a summary that rewrites history in place: nothing on the screen
//! changes, and the transcript in the database keeps every byte.
//!
//! # Why this is safe to do to the request, not the transcript
//!
//! It edits the array handed to the provider for one send. The transcript in the
//! database is untouched -- the user still sees every result -- and the model is
//! told, by a stub, that something was removed rather than being shown a gap.

use serde_json::{json, Value};

/// One knob pack for the whole feature, read from `app.preferences` once per run.
///
/// A struct rather than loose values because every depth below reads at least
/// three of them, and reading the same five fields four times out of a raw JSON
/// value is where disagreements about defaults are born.
#[derive(Clone, Copy, Debug)]
pub struct Compression {
    /// Whether the automatic pass runs at all. The manual `/compact` and a
    /// channel's `/compress` refuse by name when it is off, which is the same
    /// refusal reported before.
    pub enabled: bool,
    /// When to compress: percent of the measured context window. 80 means a
    /// measured prompt at 80% of the window triggers a pass.
    pub trigger_percent: u64,
    /// How much of the older end of a conversation the elision may rewrite, as
    /// a fraction (0-1). 0.5 makes the older half fair game.
    pub reach: f64,
    /// What a pass aims at: after compressing, the request should fit in this
    /// fraction of the window (0-1). Drives how deep the pass goes; it is a
    /// target, not a guarantee -- nothing is cut twice to reach it exactly.
    pub target: f64,
    /// The last N messages that are always sent exactly as they were. The
    /// model is working on these; a turn it is reading that arrives
    /// pre-mangled is a fact it cannot trust.
    pub protect_recent: usize,
    /// Seconds a summarising model gets before the pass gives up on a summary
    /// and keeps the elision-only result. A local model loading into cold VRAM
    /// pushes single-digit tokens per second, so an unbounded wait here would
    /// hang the reply that triggered the pass.
    pub model_timeout_secs: u64,
}

impl Default for Compression {
    fn default() -> Self {
        Self {
            enabled: true,
            trigger_percent: 80,
            reach: 0.5,
            target: 0.2,
            protect_recent: 20,
            model_timeout_secs: 120,
        }
    }
}

impl Compression {
    /// Parses the stored preference out of `app.preferences`.
    ///
    /// Missing or malformed numbers read as the defaults: a record that predates
    /// a field must not switch the whole feature off. The legacy `compaction`
    /// string is honoured once, as the switch only -- the old normal/fast split
    /// is gone, and depth is expressed by `reach`/`target` now.
    pub fn parse(stored: Option<&Value>) -> Self {
        let Some(object) = stored.filter(|value| value.is_object()) else {
            return Self::default();
        };
        let number = |key: &str, fallback: f64| -> f64 {
            object
                .get(key)
                .and_then(Value::as_f64)
                .filter(|value| value.is_finite())
                .unwrap_or(fallback)
        };
        let clamp = |value: f64, low: f64, high: f64| value.clamp(low, high);
        Self {
            enabled: object
                .get("autoCompact")
                .and_then(Value::as_bool)
                // The legacy flag: unknown or absent compacts, "off" does not.
                .unwrap_or(object.get("compaction").and_then(Value::as_str) != Some("off")),
            trigger_percent: clamp(
                number("compactAtPct", Self::default().trigger_percent as f64),
                5.0,
                99.0,
            ) as u64,
            reach: clamp(number("compactReach", Self::default().reach), 0.0, 1.0),
            target: clamp(number("compactTarget", Self::default().target), 0.0, 1.0),
            protect_recent: clamp(
                number("compactProtect", Self::default().protect_recent as f64),
                0.0,
                1000.0,
            ) as usize,
            model_timeout_secs: clamp(
                number("compactTimeout", Self::default().model_timeout_secs as f64),
                5.0,
                3600.0,
            ) as u64,
        }
    }
}

/// The stored preference, read from the one key that holds it.
///
/// Shared by the chat view's manual command and a channel's `/compress`, so the
/// two cannot act under different knobs.
pub fn stored(database: &crate::database::Database) -> Compression {
    Compression::parse(database.setting("app.preferences").as_ref())
}

/// What a tool result becomes once it is elided.
const TOOL_STUB: &str = "[tool output elided to keep the conversation within the context window]";

/// The note left where older turns were dropped, so the model knows the history
/// is not continuous rather than assuming it begins at the first message.
const TRIM_NOTICE: &str =
    "[earlier messages were dropped to fit the context window; ask if you need something from them]";

/// The heading of the message a summarising model's text lands in.
pub const SUMMARY_PREFIX: &str =
    "[Summary of the earlier conversation that was compressed to fit the window]:";

/// Whether a measured request is close enough to the model's limit to compress.
pub fn should_compress(
    compression: Compression,
    prompt_tokens: Option<u64>,
    context_window: Option<u64>,
) -> bool {
    if !compression.enabled {
        return false;
    }
    let (Some(prompt_tokens), Some(context_window)) = (prompt_tokens, context_window) else {
        return false;
    };
    context_window > 0
        && prompt_tokens
            >= context_window / 100 * compression.trigger_percent
                // A window that is not divisible by 100 must not let the
                // remainder swallow the tokens still below its percent.
                + (context_window % 100 * compression.trigger_percent).div_ceil(100)
}

/// The words the transcript receives when a pass ran. One wording now, because
/// the depth a run reached -- stubs, turn drops, possibly a summary -- is not
/// knowable when the notice is composed, and a user reading it wants to know
/// what changed: something was set aside, not rewritten behind their back.
pub fn notice() -> &'static str {
    "Context compressed — older tool output and turns were set aside from this request to keep the reply within the model's window."
}

/// Rough token cost of one message: its content bytes over four.
///
/// Deliberately crude. It is compared only against itself -- to order the depths
/// of a single pass and to know when the target is out of reach -- and it is
/// `pub` because the caller needs the same estimate to decide whether the
/// summary door is worth opening. The real figure that gates the whole pass,
/// the provider's own prompt count, is measured.
pub fn estimate_tokens(messages: &[Value]) -> usize {
    messages
        .iter()
        .map(|message| {
            message
                .get("content")
                .and_then(Value::as_str)
                .map(str::len)
                .unwrap_or(0)
                / 4
        })
        .sum()
}

/// The oldest end of what one pass may rewrite.
///
/// The protected tail always stays; `reach` says how far back elision may go,
/// measured from the oldest edge of the eligible range. This is what stops a
/// pass from touching a conversation back to its first turn over a target
/// nobody configured.
fn rewrite_start(messages: &[Value], compression: Compression) -> usize {
    let cut = messages
        .len()
        .saturating_sub(compression.protect_recent.max(1));
    cut - (compression.reach * cut as f64) as usize
}

/// What one pass over a request copy did.
pub struct PassOutcome {
    /// Approximate tokens saved. Zero means the transcript was already compact
    /// enough for the depths that ran.
    pub removed: usize,
    /// The old turns the drop depth removed, with their roles, as they were
    /// before the pass. Empty when elision sufficed: the caller summarises
    /// only these.
    pub dropped: Vec<(String, String)>,
}

/// Compacts the request in place, choosing depths until the target is met.
///
/// `messages` is the array about to be sent, in order, with any leading
/// `system` message first. The caller checks the measured prompt count before
/// invoking this; `target_tokens` is derived from the known window and the
/// stored target, and `None` -- a model with no window at all -- stops the pass
/// at elision, the one depth that cannot overshoot on a guess.
pub fn apply(
    messages: &mut Vec<Value>,
    compression: Compression,
    target_tokens: Option<usize>,
) -> PassOutcome {
    let before = estimate_tokens(messages);
    if !compression.enabled {
        return PassOutcome {
            removed: 0,
            dropped: Vec::new(),
        };
    }
    let cut = messages
        .len()
        .saturating_sub(compression.protect_recent.max(1));
    if cut == 0 {
        return PassOutcome {
            removed: 0,
            dropped: Vec::new(),
        };
    }

    // Depth 1: elide the old tool results inside the reach window. A body that
    // is **already** the stub is left alone rather than rewritten with the same
    // bytes: the stub is the state, not a step, so a second pass has nothing
    // left to do -- which is how `/compact` reports "nothing to compress"
    // instead of claiming a saving of zero.
    let from = rewrite_start(messages, compression);
    for message in messages.iter_mut().take(cut).skip(from) {
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

    // The target speaks in the estimate's currency, and until it is met the
    // deeper drop is on the table. An unknown window turns the pass off after
    // elision: there is no figure to aim at, only the one that triggered.
    let reached = target_tokens
        .map(|target| estimate_tokens(messages) <= target)
        .unwrap_or(true);
    let mut dropped = Vec::new();
    if !reached {
        // Depth 2: drop the old turns wholesale -- everything outside the
        // system block and the protected tail -- keeping the contiguous shape
        // so an `assistant` turn's tool results drop with the turn that called
        // them, and no provider sees an orphaned result or an unanswered call.
        let mut kept: Vec<Value> = Vec::with_capacity(messages.len() - cut + 1);
        for (index, message) in std::mem::take(messages).into_iter().enumerate() {
            let is_system = message.get("role").and_then(Value::as_str) == Some("system");
            if is_system || index >= cut {
                kept.push(message);
                continue;
            }
            if let (Some(role), Some(content)) = (
                message.get("role").and_then(Value::as_str),
                message.get("content").and_then(Value::as_str),
            ) {
                // An already-stubbed body has been given up once already; the
                // summary is for text that still says something.
                if content != TOOL_STUB {
                    dropped.push((role.to_string(), content.to_string()));
                }
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
    }
    PassOutcome {
        removed: before.saturating_sub(estimate_tokens(messages)),
        dropped,
    }
}

/// Compacts a stored conversation at the user's request and writes it back.
///
/// The other door into the same trim: the automatic pass trims what one request
/// sends and leaves the database alone, while this is the user asking for the
/// same elision to be written down. One implementation, because two would
/// eventually disagree about what "compressed" means.
///
/// Only the elision depth is reached from here: a transcript on disk offers no
/// measured window to aim at, and the summary door exists only inside a run.
///
/// `Ok(None)` means there was nothing old enough to compress.
pub fn compact_session(
    database: &crate::database::Database,
    session_id: &str,
    settings: Compression,
) -> Result<Option<Vec<serde_json::Value>>, String> {
    if !settings.enabled {
        return Err("Auto-compression is off. Turn it on in Settings → Model → Context.".into());
    }
    let mut messages = database.list_session_messages(session_id)?;
    let outcome = apply(&mut messages, settings, None);
    if outcome.removed == 0 {
        return Ok(None);
    }
    // The hidden user turn the transcript draws as a centred line, and the
    // model receives on the next request, so it knows history was trimmed. A
    // second pass has nothing left to stub, so it cannot stack a second notice.
    let insert_at = messages
        .iter()
        .take_while(|message| message.get("role").and_then(Value::as_str) == Some("system"))
        .count();
    messages.insert(
        insert_at,
        json!({ "role": "user", "content": notice(), "hidden": true }),
    );
    // Written through the session's own record, so the pin, the mode and the
    // rollups ride through untouched.
    let mut record = database.session_record(session_id)?;
    if let Some(object) = record.as_object_mut() {
        object.insert(
            "messages".into(),
            serde_json::Value::Array(messages.clone()),
        );
    }
    database.save_chat_session(&record)?;
    tracing::info!(%session_id, removed = outcome.removed, "conversation compressed");
    Ok(Some(messages))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn settings() -> Compression {
        Compression {
            enabled: true,
            ..Default::default()
        }
    }

    fn disabled() -> Compression {
        Compression {
            enabled: false,
            ..Default::default()
        }
    }

    fn conversation(len: usize) -> Vec<Value> {
        let mut messages = vec![json!({ "role": "system", "content": "system prompt" })];
        for index in 1..len {
            let role = if index % 2 == 0 { "tool" } else { "assistant" };
            messages.push(json!({ "role": role, "content": format!("message {index}") }));
        }
        messages
    }

    #[test]
    fn compression_waits_until_the_configured_percent_of_a_known_window() {
        let knobs = Compression {
            trigger_percent: 80,
            ..settings()
        };
        assert!(!should_compress(knobs, Some(7_999), Some(10_000)));
        assert!(should_compress(knobs, Some(8_000), Some(10_000)));
        assert!(should_compress(knobs, Some(1), Some(1)));
        assert!(!should_compress(knobs, Some(0), Some(1)));
        // No measured figure or no known window: the pass stays out of it.
        assert!(!should_compress(knobs, None, Some(10_000)));
        assert!(!should_compress(knobs, Some(8_000), None));
        assert!(!should_compress(knobs, Some(1), Some(0)));
        assert!(!should_compress(disabled(), Some(10_000), Some(10_000)));
    }

    /// The trigger honours its own stored value, not the default 80%.
    #[test]
    fn the_stored_trigger_percent_is_the_one_used() {
        let knobs = Compression {
            trigger_percent: 50,
            ..settings()
        };
        assert!(!should_compress(knobs, Some(4_999), Some(10_000)));
        assert!(should_compress(knobs, Some(5_000), Some(10_000)));
    }

    /// The knob pack parses from nothing and from a partial record.
    #[test]
    fn an_absent_record_reads_as_on_with_the_default_knobs() {
        let knobs = Compression::parse(None);
        assert!(knobs.enabled);
        assert_eq!(knobs.trigger_percent, 80);
        assert_eq!(knobs.protect_recent, 20);
        // The legacy stick on the switch is honoured once.
        assert!(!Compression::parse(Some(&json!({ "compaction": "off" }))).enabled);
        assert!(Compression::parse(Some(&json!({ "compaction": "normal" }))).enabled);
        // The stored switch wins over the legacy one.
        assert!(
            Compression::parse(Some(&json!({ "compaction": "off", "autoCompact": true }))).enabled
        );
    }

    #[test]
    fn off_changes_nothing() {
        let mut messages = conversation(80);
        let before = messages.clone();
        apply(&mut messages, disabled(), None);
        assert_eq!(messages, before);
    }

    #[test]
    fn a_conversation_at_or_below_its_protected_tail_is_left_alone() {
        let mut messages = conversation(10);
        let before = messages.clone();
        apply(&mut messages, settings(), None);
        assert_eq!(messages, before);
    }

    /// The system prompt and the protected tail survive; only the old tool
    /// bodies inside the reach window are stubbed.
    #[test]
    fn a_pass_elides_old_tool_output_and_keeps_the_protected_tail() {
        let knobs = Compression {
            protect_recent: 4,
            ..settings()
        };
        let mut messages = conversation(80);
        apply(&mut messages, knobs, None);
        assert_eq!(messages[0]["content"], "system prompt");
        assert_eq!(messages.len(), 80, "elision drops no messages");
        assert!(
            messages
                .iter()
                .any(|message| message["content"] == TOOL_STUB),
            "an old tool result should have been elided"
        );
        // The most recent tool result is inside the protected tail and untouched.
        let last_tool = messages
            .iter()
            .rposition(|message| message.get("role").and_then(Value::as_str) == Some("tool"))
            .expect("a recent tool message");
        assert_ne!(messages[last_tool]["content"], TOOL_STUB);
    }

    /// Reach bounds elision from the old end: a reach of zero is a pass that
    /// stubs nothing, whatever the target says.
    #[test]
    fn the_reach_setting_bounds_how_deep_elision_grabs() {
        let knobs = Compression {
            protect_recent: 20,
            reach: 0.0,
            ..settings()
        };
        let mut messages = conversation(80);
        apply(&mut messages, knobs, None);
        assert!(
            !messages
                .iter()
                .any(|message| message["content"] == TOOL_STUB),
            "a reach of zero touches nothing"
        );
    }

    /// Turn drops only happen when the elided estimate is still above the
    /// target, and only when the window is known.
    #[test]
    fn turns_are_dropped_only_when_the_target_is_unmet() {
        let knobs = Compression {
            protect_recent: 4,
            ..settings()
        };
        // A target of 50 tokens against an 80-message conversation: elision
        // alone cannot cross it, so the turns drop and a notice appears.
        let mut messages = conversation(80);
        apply(&mut messages, knobs, Some(50));
        assert_eq!(messages[0]["role"], "system");
        assert!(messages.len() < 80, "turns were dropped");
        assert!(messages
            .iter()
            .any(|message| message["content"] == TRIM_NOTICE));

        // A target the estimate already meets: nothing is dropped.
        let mut untouched = conversation(80);
        apply(&mut untouched, knobs, Some(1_000_000));
        assert_eq!(untouched.len(), 80);

        // No known window: depth 2 is out of reach, so nothing drops.
        let mut unknown_window = conversation(80);
        apply(&mut unknown_window, knobs, None);
        assert_eq!(unknown_window.len(), 80);
    }

    /// The dropped turns are handed back with their roles, so a caller can
    /// summarise them. Already-stubbed tool output is not collected twice.
    #[test]
    fn what_was_dropped_is_reported_for_summarising() {
        let mut messages = conversation(80);
        for message in messages.iter_mut() {
            if message.get("role").and_then(Value::as_str) == Some("tool") {
                message["content"] = Value::String(TOOL_STUB.to_string());
            }
        }
        let knobs = Compression {
            protect_recent: 4,
            ..settings()
        };
        let outcome = apply(&mut messages, knobs, Some(50));
        assert!(outcome
            .dropped
            .iter()
            .all(|(role, content)| role == "assistant" && !content.is_empty()));
    }

    /// The stub is the state, not a step: a second pass over an already-stubbed
    /// conversation saves nothing, which is why a repeated `/compact` reports
    /// zero instead of claiming a fresh saving.
    #[test]
    fn a_second_pass_has_nothing_left_to_elide() {
        // Real tool output is much longer than the stub it becomes; the short
        // bodies above would make "elided something" indistinguishable from
        // noise.
        let mut messages = conversation(80);
        for message in messages.iter_mut() {
            if message.get("role").and_then(Value::as_str) == Some("tool") {
                message["content"] = Value::String("x".repeat(200));
            }
        }
        let knobs = Compression {
            protect_recent: 4,
            ..settings()
        };
        let first = apply(&mut messages, knobs, None);
        assert!(first.removed > 0, "the first pass should elide something");

        let second = apply(&mut messages, knobs, None);
        assert_eq!(second.removed, 0, "nothing is left to elide");
    }
}
