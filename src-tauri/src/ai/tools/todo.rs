//! The agent's checklist for the task in hand.
//!
//! A long task drifts: the model fixes one thing, gets pulled sideways, and
//! forgets the fourth item it meant to do. Writing the plan down where it can be
//! re-read -- and where the user can see it -- is what keeps a multi-step task on
//! the rails.
//!
//! # Where the list lives
//!
//! In the settings table under `session.<id>.todo`, which needs no migration and
//! rides the store that already exists. It is per session rather than per run:
//! the plan outlives the reply that wrote it, so the next turn in the same
//! conversation sees the same list and can pick up where it left off. Deleting a
//! conversation leaves the key behind, which is a few bytes in a settings row
//! nobody reads -- cheaper than a schema change to reclaim them.
//!
//! # Setting and reading are one tool
//!
//! `todo` with `items` replaces the list; `todo` without returns the current one.
//! Two tools would be two names for one piece of state, and the model would have
//! to remember which to call to see its own plan.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::ai::types::ChatEvent;

use super::{ToolContext, ToolSpec};

/// One item on the checklist.
#[derive(Clone, Debug, Serialize, Deserialize)]
struct TodoItem {
    text: String,
    #[serde(default)]
    status: Status,
}

/// Where an item is up to.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Status {
    Pending,
    InProgress,
    Completed,
}

impl Default for Status {
    fn default() -> Self {
        Status::Pending
    }
}

impl Status {
    /// The marker shown beside an item, and the wire name accepted for it.
    fn marker(&self) -> &'static str {
        match self {
            Status::Pending => "[ ]",
            Status::InProgress => "[~]",
            Status::Completed => "[x]",
        }
    }

    fn parse(value: &str) -> Option<Self> {
        match value {
            "pending" => Some(Status::Pending),
            "in_progress" => Some(Status::InProgress),
            "completed" => Some(Status::Completed),
            _ => None,
        }
    }
}

/// Read or replace the task checklist.
pub const TODO: ToolSpec = ToolSpec {
    name: "todo",
    description: "Keep a checklist of the steps in a multi-step task. Call it with `items` to set \
                  or update the whole list (mark what is done as you go), or with no arguments to \
                  see the current list. Use it for work with several distinct steps; do not use it \
                  for a one-step task. The list is shown to the user and persists for the \
                  conversation.",
    parameters: r#"{
        "type": "object",
        "properties": {
            "items": {
                "type": "array",
                "description": "The full checklist, replacing any previous one. Omit to read the current list.",
                "items": {
                    "type": "object",
                    "properties": {
                        "text": { "type": "string", "description": "One step, in the imperative." },
                        "status": {
                            "type": "string",
                            "enum": ["pending", "in_progress", "completed"],
                            "description": "Where this step is up to. Defaults to pending."
                        }
                    },
                    "required": ["text"],
                    "additionalProperties": false
                }
            }
        },
        "required": [],
        "additionalProperties": false
    }"#,
    // A write for caching: the list changes as work progresses, so a cached read
    // would hand back a stale plan.
    effect: super::Effect::Write,
    command_argument: None,
    execute: |arguments, context| Box::pin(std::future::ready(run(arguments, &context))),
};

fn run(arguments: Value, context: &ToolContext) -> Result<String, String> {
    let session = context
        .run
        .as_ref()
        .and_then(|run| run.session_id.clone())
        .ok_or("todo is only available in a conversation.")?;
    let database = context
        .database
        .as_ref()
        .ok_or("todo is only available in a conversation.")?;
    let key = format!("session.{session}.todo");

    let mut items: Vec<TodoItem> = database.setting(&key).unwrap_or_default();

    // With no `items` this is a read, so the stored list is returned untouched.
    let Some(list) = arguments.get("items") else {
        return Ok(render(&items, false));
    };
    items = parse_items(list)?;
    database
        .set_setting(&key, &items)
        .map_err(|error| format!("Could not save the checklist: {error}"))?;
    // Told to the frontend so the strip above the composer reflects the list the
    // model just set, without a round trip through storage.
    if let Some(run) = context.run.as_ref() {
        (run.emit)(ChatEvent {
            run_id: run.run_id.clone(),
            session_id: run.session_id.clone(),
            sequence: 0,
            kind: "todo".into(),
            text: None,
            error: None,
            metrics: Some(serde_json::json!({
                "items": items
                    .iter()
                    .map(|item| serde_json::json!({ "text": item.text, "status": status_name(item.status) }))
                    .collect::<Vec<_>>(),
            })),
        });
    }
    Ok(render(&items, true))
}

/// Parses the caller's `items` array.
///
/// An unknown status is refused rather than silently defaulted: a model that
/// spelled `done` and got a silent `pending` would believe an item was marked
/// when it was not, and that is exactly the confusion the checklist exists to
/// prevent.
fn parse_items(value: &Value) -> Result<Vec<TodoItem>, String> {
    let list = value.as_array().ok_or("`items` must be an array")?;
    list.iter()
        .map(|entry| {
            let text = entry
                .get("text")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|text| !text.is_empty())
                .ok_or("each item needs non-empty `text`")?
                .to_string();
            let status = match entry.get("status").and_then(Value::as_str) {
                None => Status::Pending,
                Some(name) => Status::parse(name).ok_or_else(|| {
                    format!("`{name}` is not a status. Use pending, in_progress or completed.")
                })?,
            };
            Ok(TodoItem { text, status })
        })
        .collect()
}

/// The name a status travels under on the wire.
fn status_name(status: Status) -> &'static str {
    match status {
        Status::Pending => "pending",
        Status::InProgress => "in_progress",
        Status::Completed => "completed",
    }
}

/// The list as the model reads it back, and the count that follows it.
fn render(items: &[TodoItem], changed: bool) -> String {
    if items.is_empty() {
        return if changed {
            "The checklist is now empty.".to_string()
        } else {
            "No checklist yet. Set one with `items`.".to_string()
        };
    }
    let mut out = String::new();
    if changed {
        out.push_str("Checklist updated:\n");
    }
    for (index, item) in items.iter().enumerate() {
        out.push_str(&format!(
            "{}. {} {}\n",
            index + 1,
            item.status.marker(),
            item.text
        ));
    }
    let done = items
        .iter()
        .filter(|item| item.status == Status::Completed)
        .count();
    out.push_str(&format!("({done} of {} done)", items.len()));
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn items_render_with_a_marker_and_a_count() {
        let items = vec![
            TodoItem {
                text: "Read the spec".into(),
                status: Status::Completed,
            },
            TodoItem {
                text: "Write the code".into(),
                status: Status::InProgress,
            },
            TodoItem {
                text: "Test it".into(),
                status: Status::Pending,
            },
        ];
        let text = render(&items, true);
        assert!(text.contains("1. [x] Read the spec"), "{text}");
        assert!(text.contains("2. [~] Write the code"), "{text}");
        assert!(text.contains("3. [ ] Test it"), "{text}");
        assert!(text.contains("(1 of 3 done)"), "{text}");
    }

    /// An item with no status is pending, which is what a first write of a plan
    /// looks like.
    #[test]
    fn a_status_may_be_omitted() {
        let items = parse_items(&json!([{ "text": "first" }])).expect("parse");
        assert_eq!(items[0].status, Status::Pending);
    }

    /// An unknown status is refused, not silently treated as pending -- otherwise
    /// a model that wrote `done` would believe the item was marked.
    #[test]
    fn an_unknown_status_is_refused() {
        let error =
            parse_items(&json!([{ "text": "x", "status": "done" }])).expect_err("bad status");
        assert!(error.contains("done"), "{error}");
    }

    #[test]
    fn a_blank_item_is_refused() {
        assert!(parse_items(&json!([{ "text": "  " }])).is_err());
    }

    /// With no conversation there is nowhere to store the list, so it reports
    /// rather than panicking.
    #[test]
    fn a_call_with_no_session_reports() {
        let error = run(
            json!({ "items": [{ "text": "x" }] }),
            &ToolContext::default(),
        )
        .expect_err("no session");
        assert!(error.contains("conversation"), "{error}");
    }
}
