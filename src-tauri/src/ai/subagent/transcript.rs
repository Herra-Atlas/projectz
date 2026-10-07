//! Turning one sub-agent's event stream into a stored conversation.
//!
//! A sub-agent is an ordinary run: the same loop, the same tool rows and the same
//! reasoning segments its parent produces. The panel that shows it is the chat
//! transcript with the composer removed, so the job here is to fold the flat
//! event stream back into the message shape that panel already renders — an
//! assistant turn carrying an `activity` list of steps.
//!
//! **The folding mirrors the frontend's own listener, deliberately.** That
//! listener builds the same list while a reply streams; this builds it once, from
//! the events a finished run left behind. They are two implementations of one
//! shape, kept next to each other rather than merged, because the live one needs
//! React state and a clock and this one needs neither. The shape they share is
//! `ActivityStep` in `src/features/chat/types.ts`, and a change to it belongs in
//! both.
//!
//! **Timing comes from arrival, not from the model.** A tool's duration is the
//! figure the loop already measured and put in the result's metrics. A thought's
//! duration is not reported anywhere, so it is measured here as the gap between
//! the first delta and the segment that closed it.

use std::time::Instant;

use serde_json::{json, Value};

use crate::ai::types::ChatEvent;
use crate::database::utc_now;
use crate::websearch::WebSearchOutput;

/// One event with the moment it arrived.
///
/// The instant is what lets a thought be timed: nothing in the event stream says
/// how long a stretch of reasoning ran, only when its deltas arrived.
pub struct Stamped {
    pub at: Instant,
    pub event: ChatEvent,
}

/// Everything needed to write one sub-agent run.
pub struct RunRecord<'a> {
    pub id: &'a str,
    pub label: &'a str,
    pub prompt: &'a str,
    /// The agent's final message, or empty when it failed before answering.
    pub content: &'a str,
    /// What the provider reported for the final round.
    pub metrics: &'a Value,
    /// Set when the run failed, so the stored reply says why.
    pub error: Option<&'a str>,
    pub parent_session: Option<&'a str>,
    pub model: &'a str,
    pub provider: &'a str,
    pub started_at: &'a str,
    pub events: &'a [Stamped],
    pub searches: &'a [WebSearchOutput],
}

/// The stored conversation for one sub-agent run.
///
/// `kind` and `parentSessionId` ride in the session metadata beside the pin and
/// the mode, which is where every other per-conversation flag already lives. That
/// is also what keeps a sub-agent out of the sidebar: the list query excludes
/// anything whose metadata says it is one, so no schema change is needed to
/// introduce the type.
pub fn build_run(record: RunRecord<'_>) -> Value {
    let mut activity = steps_from(record.events);
    attach_searches(&mut activity, record.searches);

    // The reply the panel draws. `metrics` is written only when there is
    // something to say, so a failed run does not carry a wall of zeroes.
    let elapsed = elapsed_seconds(record.events);
    let mut assistant = json!({
        "role": "assistant",
        "content": record.content,
        "modelId": record.model,
        "providerId": record.provider,
    });
    if !activity.is_empty() {
        assistant["activity"] = Value::Array(activity);
    }
    let metrics = metrics_json(record.metrics, elapsed, record.error.is_some());
    if !metrics
        .as_object()
        .map(|object| object.is_empty())
        .unwrap_or(true)
    {
        assistant["metrics"] = metrics;
    }

    json!({
        "id": record.id,
        // The label is the title the user sees in the Sub agents list. When none
        // was given the prompt stands in, trimmed, like a chat's own first message.
        "title": if record.label.is_empty() { title_from(record.prompt) } else { record.label.to_string() },
        "createdAt": record.started_at,
        "updatedAt": utc_now(),
        "kind": "subagent",
        "parentSessionId": record.parent_session,
        "modelId": record.model,
        "providerId": record.provider,
        // Deliberately present even though a sub-agent is never edited or renamed:
        // `save_chat_session_tx` rebuilds the metadata blob from these keys, and a
        // missing one reads back as the default. `mode` is Agent because that is
        // what a sub-agent runs as.
        "mode": "agent",
        "messages": [
            { "role": "user", "content": record.prompt },
            assistant,
        ],
    })
}

/// A short title from a task when no label was given.
fn title_from(prompt: &str) -> String {
    let first = prompt.lines().next().unwrap_or_default().trim();
    let trimmed: String = first.chars().take(48).collect();
    if trimmed.is_empty() {
        "Sub-agent".to_string()
    } else {
        trimmed
    }
}

/// Folds the event stream into the activity steps the panel renders.
fn steps_from(events: &[Stamped]) -> Vec<Value> {
    let mut steps: Vec<Value> = Vec::new();
    // When the currently-open thought began, so its duration can be measured
    // against the moment it closed. `None` when no thought is open.
    let mut thought_started: Option<Instant> = None;

    for stamped in events {
        let event = &stamped.event;
        match event.kind.as_str() {
            "reasoning_delta" => {
                let Some(text) = event.text.as_deref() else {
                    continue;
                };
                let open = steps.last_mut().filter(|step| {
                    step["kind"] == "thought" && step["running"] == Value::Bool(true)
                });
                match open {
                    Some(step) => {
                        let merged =
                            format!("{}{}", step["text"].as_str().unwrap_or_default(), text);
                        step["text"] = Value::String(merged);
                    }
                    None => {
                        thought_started = Some(stamped.at);
                        steps.push(json!({
                            "kind": "thought",
                            "text": text,
                            "running": true,
                        }));
                    }
                }
            }
            // A closed stretch of thinking. Its text arrives whole, replacing
            // whatever streamed in its place, and this is the only moment its
            // duration is knowable.
            "reasoning_segment" => {
                let Some(text) = event.text.as_deref() else {
                    continue;
                };
                let seconds = thought_started
                    .map(|started| stamped.at.duration_since(started).as_secs_f64())
                    .unwrap_or_default();
                thought_started = None;
                let closed = json!({
                    "kind": "thought",
                    "text": text,
                    "seconds": seconds,
                    "running": false,
                });
                let running = steps.last().is_some_and(|step| {
                    step["kind"] == "thought" && step["running"] == Value::Bool(true)
                });
                if running {
                    let last = steps.len() - 1;
                    steps[last] = closed;
                } else {
                    steps.push(closed);
                }
            }
            "tool_call" => {
                let metrics = event.metrics.clone().unwrap_or(Value::Null);
                steps.push(json!({
                    "kind": "tool",
                    "tool": event.text.clone().unwrap_or_default(),
                    "label": metrics["label"].clone(),
                    "detail": metrics["detail"].clone(),
                    "outcome": "",
                    "failed": false,
                    "running": true,
                }));
            }
            "tool_result" => {
                let metrics = event.metrics.clone().unwrap_or(Value::Null);
                let name = event.text.clone().unwrap_or_default();
                // Matched on the tool name rather than the row's index: indices
                // restart every round, so an index would let a result land on a
                // row from a round that had already closed.
                let open = steps.iter_mut().rev().find(|step| {
                    step["kind"] == "tool"
                        && step["running"] == Value::Bool(true)
                        && step["tool"] == Value::String(name.clone())
                });
                match open {
                    Some(step) => {
                        step["running"] = Value::Bool(false);
                        copy(&metrics, step, "seconds");
                        step["outcome"] = metrics["outcome"].clone();
                        step["failed"] = Value::Bool(metrics["failed"] == Value::Bool(true));
                        if metrics["cached"] == Value::Bool(true) {
                            step["cached"] = Value::Bool(true);
                        }
                        copy(&metrics, step, "output");
                        copy(&metrics, step, "diff");
                        if metrics["created"] == Value::Bool(true) {
                            step["created"] = Value::Bool(true);
                        }
                    }
                    // A result with no open row is still shown, for the same
                    // reason the live panel shows it: a call the user cannot see
                    // is one they cannot account for.
                    None => steps.push(json!({
                        "kind": "tool",
                        "tool": name,
                        "label": metrics["label"].clone(),
                        "detail": metrics["detail"].clone(),
                        "outcome": metrics["outcome"].clone(),
                        "failed": metrics["failed"] == Value::Bool(true),
                        "seconds": metrics["seconds"].clone(),
                        "running": false,
                    })),
                }
            }
            // Everything else -- status, deltas of the answer, the completion
            // marker -- is not a step. The answer itself travels in the reply.
            _ => {}
        }
    }
    steps
}

/// Moves one field across when it is actually present.
fn copy(from: &Value, to: &mut Value, field: &str) {
    if let Some(value) = from.get(field) {
        if !value.is_null() {
            to[field] = value.clone();
        }
    }
}

/// Hangs each search's results on the call that ran it.
///
/// A search's results arrive as their own event, after the tool row that asked
/// for them, so they are attached rather than pushed as a separate step — the
/// panel draws them behind the row, which is where every other tool's output
/// lives. A search with no matching open row still gets one, because a search
/// that ran and cannot be seen is the one outcome worse than an ugly row.
fn attach_searches(steps: &mut Vec<Value>, searches: &[WebSearchOutput]) {
    for search in searches {
        let payload = json!({ "query": search.query, "results": search.results });
        let open = steps.iter_mut().rev().find(|step| {
            step["kind"] == "tool" && step["tool"] == Value::String("search_web".into())
        });
        match open {
            Some(step) => step["search"] = payload,
            None => steps.push(json!({
                "kind": "tool",
                "tool": "search_web",
                "label": "Search the web",
                "detail": search.query,
                "outcome": format!("{} results", search.results.len()),
                "failed": false,
                "running": false,
                "search": payload,
            })),
        }
    }
}

/// Seconds from the first event to the last, as the panel's "Worked for".
fn elapsed_seconds(events: &[Stamped]) -> f64 {
    let Some(first) = events.first() else {
        return 0.0;
    };
    let last = events.last().unwrap_or(first);
    last.at.duration_since(first.at).as_secs_f64()
}

/// The reply's metrics, in the shape the panel and the overview already read.
///
/// Token figures are copied only when the provider reported them, so a provider
/// that says nothing leaves them out rather than reporting zero — the same rule
/// the live path follows.
fn metrics_json(reported: &Value, elapsed: f64, failed: bool) -> Value {
    let mut metrics = serde_json::Map::new();
    metrics.insert("elapsed_seconds".into(), json!(elapsed));
    for (key, field) in [
        ("prompt_tokens", "prompt_tokens"),
        ("completion_tokens", "completion_tokens"),
        ("cached_tokens", "cached_tokens"),
    ] {
        if let Some(value) = reported.get(field) {
            if !value.is_null() {
                metrics.insert(key.into(), value.clone());
            }
        }
    }
    if failed {
        metrics.insert("failed".into(), Value::Bool(true));
    }
    Value::Object(metrics)
}
