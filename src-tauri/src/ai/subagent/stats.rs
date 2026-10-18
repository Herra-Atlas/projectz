//! The figures a finished sub-agent reports to its parent.
//!
//! The parent's `sub_agent` row carries the agent's report once it is done, and
//! these are the numbers underneath it: what the run cost, how many calls it made,
//! how many rounds it took, and how long it ran. They are what the session
//! overview states for a conversation, stated here for one delegated run so it can
//! be judged from the row without opening it in the Sub agents panel.
//!
//! Derived here, from the run's own event stream, because that is the only place
//! the round and call counts exist: the stored transcript keeps the answer and the
//! steps, not the tally. The wire shape is deliberately small and snake-cased to
//! match the event metrics it rides in; the frontend folds it into the row.

use serde_json::{json, Value};

use crate::ai::types::ChatEvent;

use super::transcript::{elapsed_seconds, Stamped};

/// A run's spend, calls, rounds and duration, as the parent's row shows them.
///
/// Every figure is omitted when it is unknown rather than written as a zero: a
/// provider that reports no usage must leave the token line out, not claim the run
/// was free. `tool_uses`, `turns` and `seconds` are always known -- they are
/// counted from the events the run emitted, not reported by anyone.
pub fn summarize(metrics: &Value, events: &[Stamped]) -> Value {
    let field = |key: &str| metrics.get(key).and_then(Value::as_u64);
    // The `used_*` totals are the sum over every round the run made, which is the
    // real spend once a tool round re-sends the conversation. A provider that
    // reported none leaves only the final round to fall back on, exact for a run
    // that made a single request.
    let input = field("used_prompt_tokens").or_else(|| field("prompt_tokens"));
    let output = field("used_completion_tokens").or_else(|| field("completion_tokens"));

    let mut stats = serde_json::Map::new();
    if input.is_some() || output.is_some() {
        stats.insert(
            "used_tokens".into(),
            json!(input.unwrap_or(0) + output.unwrap_or(0)),
        );
    }
    // A call is one `tool_call`; a round is the `thinking` status the loop emits
    // once per request, which is the ceiling-aware definition of a turn.
    stats.insert(
        "tool_uses".into(),
        json!(count(events, |event| event.kind == "tool_call")),
    );
    stats.insert(
        "turns".into(),
        json!(count(events, |event| {
            event.kind == "status" && event.text.as_deref() == Some("thinking")
        })),
    );
    stats.insert("seconds".into(), json!(elapsed_seconds(events)));
    Value::Object(stats)
}

/// How many events match, so the two tallies above read the same way.
fn count(events: &[Stamped], matches: impl Fn(&ChatEvent) -> bool) -> usize {
    events
        .iter()
        .filter(|stamped| matches(&stamped.event))
        .count()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    fn stamped(kind: &str, text: Option<&str>) -> Stamped {
        Stamped {
            at: Instant::now(),
            event: ChatEvent {
                run_id: "agent-1".into(),
                session_id: None,
                sequence: 0,
                kind: kind.into(),
                text: text.map(str::to_string),
                error: None,
                metrics: None,
            },
        }
    }

    /// The spend is the run's `used_*` totals, which count every round, rather
    /// than the last round's `prompt_tokens` -- the whole point of the figure.
    #[test]
    fn the_spend_is_every_round_rather_than_the_last() {
        let events = vec![stamped("status", Some("thinking"))];
        let stats = summarize(
            &json!({
                "prompt_tokens": 7000,
                "completion_tokens": 60,
                "used_prompt_tokens": 12000,
                "used_completion_tokens": 100,
            }),
            &events,
        );
        assert_eq!(stats["used_tokens"], 12100);
    }

    /// A provider that reports no usage leaves the token line out entirely, rather
    /// than reporting a run that cost nothing.
    #[test]
    fn an_unreported_spend_is_omitted_not_zeroed() {
        let stats = summarize(&json!({}), &[]);
        assert!(stats.get("used_tokens").is_none(), "{stats}");
    }

    /// Calls and rounds are counted from the stream: a `tool_call` is one use and a
    /// `thinking` status is one round. Other statuses -- a tool running -- are not
    /// rounds and must not inflate the count.
    #[test]
    fn calls_and_rounds_are_counted_from_the_events() {
        let events = vec![
            stamped("status", Some("thinking")),
            stamped("tool_call", Some("read_file")),
            stamped("status", Some("using_tools")),
            stamped("tool_call", Some("grep")),
            stamped("status", Some("thinking")),
            stamped("delta", Some("the answer")),
        ];
        let stats = summarize(&json!({}), &events);
        assert_eq!(stats["tool_uses"], 2);
        assert_eq!(stats["turns"], 2);
    }

    /// A run that reported nothing still carries a duration of zero rather than
    /// nothing at all, so the row can always state how long it took.
    #[test]
    fn an_empty_run_reports_a_zero_duration() {
        let stats = summarize(&json!({}), &[]);
        assert_eq!(stats["seconds"], 0.0);
        assert_eq!(stats["turns"], 0);
        assert_eq!(stats["tool_uses"], 0);
    }

    /// The span is measured end to end, the same figure the transcript's own
    /// "Worked for" uses.
    #[test]
    fn the_duration_spans_the_whole_run() {
        let start = Instant::now();
        let events = vec![
            Stamped {
                at: start,
                event: stamped("status", Some("thinking")).event,
            },
            Stamped {
                at: start + Duration::from_secs(4),
                event: stamped("delta", Some("done")).event,
            },
        ];
        let stats = summarize(&json!({}), &events);
        assert_eq!(stats["seconds"], 4.0);
    }
}
