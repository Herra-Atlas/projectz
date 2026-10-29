//! Talking to a sub-agent after it has finished.
//!
//! A sub-agent is an ordinary session row, so a further turn is an ordinary run:
//! [`run_prompt`] loads the conversation, streams a reply under a new run id and
//! writes the result back. This file only turns the panel's request into that
//! spec -- which model, which permission, and the metadata the row must keep.
//!
//! **The run id is handed back before the reply starts.** The panel needs it to
//! watch the stream and to stop the run, and neither is possible if the command
//! only answers once the agent has finished.
//!
//! **What the agent may do.** A finished sub-agent has no live parent gate to
//! inherit, so the level stored on its row is used when there is one and
//! `auto_writes` otherwise: reads and workspace writes run without a prompt, while
//! the terminal family still asks. The installation-wide host switches are
//! applied by the runtime exactly as they are for every other door.

use serde_json::{json, Value};
use tauri::{AppHandle, Manager, State};

use crate::ai::run::{run_prompt, PromptRun};
use crate::ai::runtime::AiRuntime;
use crate::ai::tools::{AccessSet, PermissionMode, ToolMode};
use crate::ai::types::ChatMessage;
use crate::database::utc_now;

/// Sends one message to a finished sub-agent and watches it under `run_id`.
///
/// The run id comes from the caller for the same reason [`run_prompt`]'s callers
/// supply one: the panel registers the run before the first event can arrive, so
/// no token is streamed into a run it does not yet know about.
#[tauri::command]
pub async fn ai_subagent_send(
    app: AppHandle,
    runtime: State<'_, AiRuntime>,
    agent_id: String,
    text: String,
    run_id: String,
) -> Result<(), String> {
    let text = text.trim().to_string();
    if text.is_empty() {
        return Err("The message is empty.".to_string());
    }
    if run_id.trim().is_empty() {
        return Err("The run id is missing.".to_string());
    }

    let database = runtime.database();
    let record = database.session_record(&agent_id)?;
    if record.get("kind").and_then(Value::as_str) != Some("subagent") {
        return Err("That conversation is not a sub agent run.".to_string());
    }
    let stored = database.list_session_messages(&agent_id)?;
    if stored.is_empty() {
        return Err("The run has no transcript to continue.".to_string());
    }

    let run_id = run_id.trim().to_string();
    let spec = spec_for(&run_id, &agent_id, &record, &stored, &text);
    // The turn is started in the background: the command must answer now, because
    // the panel is already drawing the run and needs to be able to stop it.
    tauri::async_runtime::spawn(async move {
        let runtime = app.state::<AiRuntime>();
        let outcome = run_prompt(&app, runtime.inner(), spec).await;
        if let Some(error) = outcome.error {
            tracing::warn!(agent = %agent_id, %error, "a sub-agent follow-up failed");
        }
    });
    Ok(())
}

/// One follow-up run, built from the row it continues.
fn spec_for(
    run_id: &str,
    agent_id: &str,
    record: &Value,
    stored: &[Value],
    text: &str,
) -> PromptRun {
    let permission = record
        .get("permission")
        .cloned()
        .and_then(|value| serde_json::from_value::<PermissionMode>(value).ok())
        .unwrap_or(PermissionMode::AutoWrites);
    let provider_id = string_field(record, "providerId");
    let local_model = provider_id.as_deref() == Some(crate::ai::local::LOCAL_ENDPOINT_ID);
    let model_id = string_field(record, "modelId");

    // What the model reads: the turns it already had, then the new one. A stored
    // assistant turn's `activity` is not sent -- the recap the runtime builds is
    // what tells the model what it did.
    let mut messages = wire_messages(stored);
    messages.push(ChatMessage {
        role: "user".to_string(),
        content: text.to_string(),
    });
    // What is written back: the records as stored (activity, hidden turns and
    // all), then the same user turn. `run_prompt` appends the reply.
    let mut stored_messages = stored.to_vec();
    stored_messages.push(json!({ "role": "user", "content": text }));

    PromptRun {
        run_id: run_id.to_string(),
        session_id: agent_id.to_string(),
        title: string_field(record, "title").unwrap_or_else(|| "Sub-agent".to_string()),
        kind: "subagent".to_string(),
        messages,
        stored_messages,
        attachments: Vec::new(),
        additional_instructions: None,
        suppress_general_instructions: false,
        delivery: None,
        endpoint_id: provider_id,
        local_model,
        model: if local_model {
            Some("projectz-local".to_string())
        } else {
            model_id.clone()
        },
        provider_label: model_id.unwrap_or_default(),
        metadata: Some(json!({
            "kind": "subagent",
            "parentSessionId": record.get("parentSessionId").cloned().unwrap_or(Value::Null),
            "pinned": record.get("pinned").cloned().unwrap_or_else(|| json!(false)),
            "renamed": record.get("renamed").cloned().unwrap_or_else(|| json!(false)),
            "renamedByUser": record.get("renamedByUser").cloned().unwrap_or_else(|| json!(false)),
            "dotColor": record.get("dotColor").cloned().unwrap_or(Value::Null),
            "permission": serde_json::to_value(permission).unwrap_or(Value::Null),
        })),
        mode: ToolMode::Agent,
        permission,
        access: AccessSet::default(),
        web_search: true,
        created_at: string_field(record, "createdAt").unwrap_or_else(utc_now),
        on_event: None,
    }
}

/// The stored turns as the request view: role and text, nothing else.
///
/// An assistant turn carries its record of what it did, built from the steps the
/// transcript stored -- the same derivation a channel uses, so a follow-up reads the
/// same history a continuing chat would. Without it a resumed agent begins by
/// re-reading files it already read.
fn wire_messages(stored: &[Value]) -> Vec<ChatMessage> {
    stored
        .iter()
        .filter_map(|message| {
            let role = message.get("role")?.as_str()?;
            let content = message.get("content")?.as_str()?;
            let content = match message.get("activity") {
                Some(activity) => crate::ai::recap::append(content, activity),
                None => content.to_string(),
            };
            Some(ChatMessage {
                role: role.to_string(),
                content,
            })
        })
        .collect()
}

fn string_field(value: &Value, key: &str) -> Option<String> {
    value.get(key).and_then(Value::as_str).map(str::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record() -> Value {
        json!({
            "id": "agent-1",
            "title": "Find the parser",
            "createdAt": "2026-01-01T00:00:00Z",
            "modelId": "gpt-5",
            "providerId": "openai",
            "kind": "subagent",
            "parentSessionId": "chat-1",
            "permission": "auto_writes",
        })
    }

    fn stored() -> Vec<Value> {
        vec![
            json!({ "role": "user", "content": "Where is the parser?" }),
            json!({ "role": "assistant", "content": "In ai/local/parser." }),
        ]
    }

    /// The conversation the model sees is the stored turns plus the new message.
    #[test]
    fn the_request_is_the_transcript_plus_the_new_turn() {
        let spec = spec_for("run-1", "agent-1", &record(), &stored(), "And the tests?");
        assert_eq!(spec.messages.len(), 3);
        assert_eq!(spec.messages[2].content, "And the tests?");
        assert_eq!(spec.messages[0].content, "Where is the parser?");
    }

    /// The stored metadata is carried, so the follow-up does not detach the run
    /// from the chat that owns it.
    #[test]
    fn the_row_keeps_its_kind_and_parent() {
        let spec = spec_for("run-1", "agent-1", &record(), &stored(), "More");
        let metadata = spec.metadata.expect("metadata");
        assert_eq!(metadata["kind"], "subagent");
        assert_eq!(metadata["parentSessionId"], "chat-1");
        assert_eq!(metadata["permission"], "auto_writes");
    }

    /// A row with no stored level still runs somewhere sensible rather than
    /// parking on a prompt the panel cannot answer.
    #[test]
    fn an_unstored_level_defaults_to_auto_writes() {
        let mut record = record();
        record["permission"] = Value::Null;
        let spec = spec_for("run-1", "agent-1", &record, &stored(), "More");
        assert_eq!(spec.permission, PermissionMode::AutoWrites);
    }

    /// A local run is addressed the way the runtime expects it, not with the
    /// local engine's id as a model name.
    #[test]
    fn a_local_run_uses_the_local_alias() {
        let mut record = record();
        record["providerId"] = json!(crate::ai::local::LOCAL_ENDPOINT_ID);
        record["modelId"] = json!("qwen3.5-4b");
        let spec = spec_for("run-1", "agent-1", &record, &stored(), "More");
        assert!(spec.local_model);
        assert_eq!(spec.model.as_deref(), Some("projectz-local"));
        assert_eq!(
            spec.endpoint_id.as_deref(),
            Some(crate::ai::local::LOCAL_ENDPOINT_ID)
        );
    }
}
