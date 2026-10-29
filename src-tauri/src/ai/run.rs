//! One prompt through the agent loop, written down as a session.
//!
//! # Why this is not in the job runner
//!
//! A scheduled job and a message from another channel are the same thing to the
//! model: a conversation's history, a model, a permission level, an access set and
//! nobody watching. The second caller used to mean a second copy of the request,
//! the event collection and the transcript write -- three places for the two paths
//! to drift. This is that copy, once.
//!
//! What is *not* here is the caller's own business: loading a local engine (a job
//! decides whether it may unload it afterwards), choosing a workspace, and deciding
//! what a failure means to the user.
//!
//! # Why the caller supplies the run id
//!
//! A channel that can be told to stop needs the id *before* the stream starts, not
//! after it ends, and it is the same id every event is filtered by. Minting it here
//! would leave the caller unable to cancel a run it had just started.

use std::sync::{Arc, Mutex};
use std::time::Instant;

use serde_json::json;
use tauri::{AppHandle, Emitter, Listener};

use crate::ai::runtime::AiRuntime;
use crate::ai::subagent::transcript::{steps_from, Stamped};
use crate::ai::tools::{AccessSet, PermissionMode, ToolMode};
use crate::ai::types::{ChatEvent, ChatMessage, ChatRequest, ReasoningEffort};
use crate::database::Database;

/// Everything one unattended run needs, as the caller knows it.
pub struct PromptRun {
    /// The chat run id. The caller keeps it so it can cancel the run.
    pub run_id: String,
    /// The session this run belongs to. The caller normally makes it, because the
    /// same conversation is reused by the next message.
    pub session_id: String,
    /// Shown in the sidebar and in a completion notification.
    pub title: String,
    /// `job`, `telegram`, or `chat` for a caller with no better word.
    pub kind: String,
    /// The conversation so far, ending with the new turn. Stored back whole, so a
    /// caller that keeps history by loading it gets it back unchanged.
    pub messages: Vec<ChatMessage>,
    /// The stored transcript to write back, when the caller has one.
    ///
    /// A stored message can carry fields the model never sees -- an assistant
    /// turn's `activity`, a hidden notice -- so a caller continuing an existing
    /// conversation passes the records rather than letting [`Self::messages`] be
    /// flattened back into text. Empty means "save the messages as given", which is
    /// what a caller with no history does.
    pub stored_messages: Vec<serde_json::Value>,
    /// Files attached to this turn. Only a channel has them today.
    pub attachments: Vec<crate::ai::types::ChatAttachment>,
    /// Extra prompt text this door needs, after the shared instructions.
    pub additional_instructions: Option<String>,
    /// Set when the door's own instructions replace the app's general ones.
    pub suppress_general_instructions: bool,
    /// Where the run came from, for a job notification that outlives it.
    pub delivery: Option<crate::ai::types::DeliveryTarget>,
    pub endpoint_id: Option<String>,
    pub local_model: bool,
    pub model: Option<String>,
    /// The label stored on the session row. A local run uses the model id; a
    /// provider run uses the model name.
    pub provider_label: String,
    /// Conversation metadata the caller wants carried through the save.
    ///
    /// A caller continuing a conversation it did not create -- a sub-agent run
    /// being talked to from the panel -- must not drop the fields the row already
    /// carries (`kind`, `parentSessionId`, the pin, the permission). The session
    /// record is rebuilt from the request on every save, so the values that make
    /// the row what it is have to travel with the request.
    pub metadata: Option<serde_json::Value>,
    pub mode: ToolMode,
    pub permission: PermissionMode,
    pub access: AccessSet,
    pub web_search: bool,
    /// When the conversation was first written, so a later turn does not move it.
    pub created_at: String,
    /// Lets a caller watch the run as it goes -- an approval to relay, a step to
    /// draw somewhere other than the app's own window.
    pub on_event: Option<Arc<dyn Fn(&ChatEvent) + Send + Sync>>,
}

/// What one run did.
pub struct PromptOutcome {
    pub content: String,
    pub error: Option<String>,
    /// The provider's own figures for the reply, when it reported any.
    pub usage: Option<serde_json::Value>,
    /// Whether the user's stop ended it rather than a failure.
    pub stopped: bool,
    /// How many tool calls the run made.
    ///
    /// A caller that may try another model needs this: a failure after real work
    /// cannot be replayed, while one that changed nothing can.
    pub tool_steps: usize,
}

/// Runs one prompt and saves the whole conversation.
pub async fn run_prompt(app: &AppHandle, runtime: &AiRuntime, spec: PromptRun) -> PromptOutcome {
    let database = runtime.database();
    let request = ChatRequest {
        run_id: spec.run_id.clone(),
        session_id: Some(spec.session_id.clone()),
        messages: spec.messages.clone(),
        endpoint_id: spec.endpoint_id.clone(),
        local_model: spec.local_model,
        model: spec.model.clone(),
        attachments: spec.attachments.clone(),
        additional_instructions: spec.additional_instructions.clone(),
        suppress_general_instructions: spec.suppress_general_instructions,
        delivery: spec.delivery.clone(),
        // Web search arrives with the access set: a run that may not search is not
        // handed a catalogue of searches it would be refused.
        web_search_enabled: spec.web_search,
        mode: spec.mode,
        permission: spec.permission,
        reasoning: ReasoningEffort::default(),
        skill_ids: Vec::new(),
        access: Some(spec.access),
    };

    // A channel's conversation is written before the answer, not only after it.
    // The phone's message is already the user's; leaving it out of the database
    // until the run finished meant the app's window had nothing to show until the
    // last token landed -- no new session, no ordering, no live stream to attach
    // to. Writing the turn first and announcing it is what lets the sidebar and an
    // open transcript see the message while it is being answered.
    if spec.kind != "job" {
        save_started(database.as_ref(), &spec);
        let _ = app.emit(
            "messaging-event",
            json!({ "sessionId": spec.session_id, "kind": "started" }),
        );
    }

    // The run's own event stream, collected here because an unattended run has no
    // window watching it. Filtered by run id, so a conversation running in the app
    // at the same time is not folded into this transcript.
    let collected = Arc::new(Mutex::new(Vec::<Stamped>::new()));
    let sink = Arc::clone(&collected);
    let watched = spec.run_id.clone();
    let relay = spec.on_event.clone();
    let listener = app.listen("ai-event", move |event| {
        let Ok(parsed) = serde_json::from_str::<ChatEvent>(event.payload()) else {
            return;
        };
        if parsed.run_id != watched {
            return;
        }
        if let Some(on_event) = relay.as_ref() {
            on_event(&parsed);
        }
        if let Ok(mut list) = sink.lock() {
            list.push(Stamped {
                at: Instant::now(),
                event: parsed,
            });
        }
    });

    let result = runtime.run_chat(app, request).await;
    app.unlisten(listener);

    let events = collected
        .lock()
        .map(|mut list| std::mem::take(&mut *list))
        .unwrap_or_default();
    let steps = steps_from(&events);
    // `usage` arrives on every round; the last one is the context at the answer,
    // which is the figure a `/context` request means.
    let usage = events
        .iter()
        .rev()
        .find(|stamped| stamped.event.kind == "usage")
        .and_then(|stamped| stamped.event.metrics.clone())
        .or_else(|| {
            events
                .iter()
                .rev()
                .find(|stamped| stamped.event.kind == "completed")
                .and_then(|stamped| stamped.event.metrics.clone())
        });
    let stopped = events.iter().any(|stamped| stamped.event.kind == "stopped");

    let assistant_content = match &result.error {
        Some(error) => format!("Request failed: {error}"),
        None => result.content.clone(),
    };
    save(app, &database, &spec, &assistant_content, &steps);

    PromptOutcome {
        content: result.content,
        error: result.error,
        usage,
        stopped,
        tool_steps: steps.len(),
    }
}

/// The conversation as the database holds it.
///
/// The caller's stored records win: a transcript can carry an assistant turn's
/// `activity` or a hidden notice that the narrow request view does not. A caller
/// with no history gets its messages written as turns, which is exactly what it
/// sent.
fn conversation(spec: &PromptRun) -> Vec<serde_json::Value> {
    if spec.stored_messages.is_empty() {
        spec.messages
            .iter()
            .map(|message| json!({ "role": message.role, "content": message.content }))
            .collect()
    } else {
        spec.stored_messages.clone()
    }
}

/// The session row both writes use, so the started and finished records cannot
/// disagree about anything except the messages.
fn session_record(spec: &PromptRun, messages: Vec<serde_json::Value>) -> serde_json::Value {
    let mut record = json!({
        "id": spec.session_id,
        "title": spec.title,
        "createdAt": spec.created_at,
        "updatedAt": crate::database::utc_now(),
        "kind": spec.kind,
        "mode": if spec.mode == ToolMode::Agent { "agent" } else { "chat" },
        "modelId": spec.provider_label,
        "messages": messages,
    });
    // The caller's own fields win over the generated ones. A sub-agent's row is
    // identified by `kind` and `parentSessionId`, and a follow-up save that did not
    // carry them would detach the run from the conversation that owns it.
    if let (Some(record), Some(extra)) = (
        record.as_object_mut(),
        spec.metadata.as_ref().and_then(|value| value.as_object()),
    ) {
        for (key, value) in extra {
            record.insert(key.clone(), value.clone());
        }
    }
    record
}

/// Writes the conversation as it stands when the run begins.
///
/// Nothing is invented: the turn the caller sent is the turn stored. This is a
/// real save rather than a placeholder row, so a run that never returns leaves a
/// conversation holding the question that was asked.
fn save_started(database: &Database, spec: &PromptRun) {
    let messages = conversation(spec);
    if messages.is_empty() {
        return;
    }
    if let Err(error) = database.save_chat_session(&session_record(spec, messages)) {
        tracing::warn!(session = %spec.session_id, %error, "a channel conversation could not be started");
    }
}

/// Writes the conversation back, whatever the outcome.
///
/// A failed run is exactly the case someone opens the transcript to understand, so
/// it is stored with the reason in the assistant turn rather than dropped.
fn save(
    app: &AppHandle,
    database: &Arc<Database>,
    spec: &PromptRun,
    assistant_content: &str,
    steps: &[serde_json::Value],
) {
    let mut messages = conversation(spec);
    let mut assistant = json!({
        "role": "assistant",
        "content": assistant_content,
        "modelId": spec.provider_label,
    });
    if !steps.is_empty() {
        assistant["activity"] = json!(steps);
    }
    messages.push(assistant);

    if let Err(error) = database.save_chat_session(&session_record(spec, messages)) {
        tracing::warn!(session = %spec.session_id, %error, "an unattended run could not be saved");
    }
    // A conversation written with no window watching is otherwise invisible to the
    // sidebar until something else happens to reload the list. A job already has an
    // event of its own from the scheduler, so this one is for the channels.
    if spec.kind != "job" {
        let _ = app.emit(
            "messaging-event",
            json!({ "sessionId": spec.session_id, "kind": spec.kind }),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A caller that stores history gets exactly the turns it sent back, with the
    /// answer appended -- no rewriting, no summarising, so the next turn loads what
    /// it expects.
    #[test]
    fn the_stored_conversation_is_the_one_that_was_sent() {
        let spec = PromptRun {
            run_id: "r".into(),
            session_id: "s".into(),
            title: "t".into(),
            kind: "telegram".into(),
            messages: vec![
                ChatMessage {
                    role: "user".into(),
                    content: "one".into(),
                },
                ChatMessage {
                    role: "assistant".into(),
                    content: "two".into(),
                },
            ],
            stored_messages: Vec::new(),
            attachments: Vec::new(),
            additional_instructions: None,
            suppress_general_instructions: false,
            delivery: None,
            endpoint_id: None,
            local_model: false,
            model: None,
            provider_label: "m".into(),
            metadata: None,
            mode: ToolMode::Agent,
            permission: PermissionMode::AutoWrites,
            access: AccessSet::default(),
            web_search: false,
            created_at: "2026-01-01T00:00:00Z".into(),
            on_event: None,
        };
        let turns: Vec<serde_json::Value> = spec
            .messages
            .iter()
            .map(|message| json!({ "role": message.role, "content": message.content }))
            .collect();
        assert_eq!(turns[0]["content"], "one");
        assert_eq!(turns[1]["role"], "assistant");
    }
}
