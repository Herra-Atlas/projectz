//! Letting one agent hand a task to another.
//!
//! # What a sub-agent is
//!
//! A sub-agent is not a second kind of loop. It is the same loop the parent runs
//! -- [`crate::ai::remote::client::stream_chat`] -- given a different
//! conversation to start from. That reuse is the whole design: streaming, tool
//! dispatch, the permission gate, the tool cache, cancellation and the round
//! ceiling all behave inside a sub-agent exactly as they do outside one, because
//! they are the same code.
//!
//! # Why only the final message comes back
//!
//! A sub-agent exists to keep work *out* of its parent's context. Its reading and
//! its reasoning are the expensive part and are exactly what the parent does not
//! need to pay to re-read; what the parent needs is the answer. So the transcript
//! is stored for the user to look at, and the tool result is the last message and
//! nothing else.
//!
//! # Why approvals still work inside one
//!
//! A sub-agent is handed its parent's [`ApprovalGate`](crate::ai::tools::ApprovalGate)
//! by clone, so every call it makes is checked against the same mode and the same
//! pending-prompt map. There is no path around the gate: a sub-agent doing a write
//! in `Ask` mode stops for the user just as its parent would. The prompt is
//! re-stamped with the parent's run id, because that is the only run the screen
//! knows about, and the answer comes back through the shared map.

mod prompt;
mod transcript;

use std::sync::{Arc, Mutex};
use std::time::Instant;

use serde_json::Value;

use crate::ai::remote::client::{stream_chat, subagent_started_event};
use crate::ai::remote::types::Endpoint;
use crate::ai::tools::{RunHandle, SubAgentNotice, ToolContext, ToolMode};
use crate::ai::types::{ChatEvent, ChatMessage};

use transcript::{RunRecord, Stamped};

/// Reported when a tool call somehow has no run behind it.
///
/// Reachable only from a unit test that builds a context by hand, or a run that
/// was cancelled before it started. The message names the situation rather than a
/// bare refusal, so a model that sees it can stop trying.
const NO_RUN: &str = "Sub-agents are only available during an agent run.";

/// Runs one sub-agent and returns its final message.
pub async fn spawn(arguments: Value, context: ToolContext) -> Result<String, String> {
    let run = context.run.clone().ok_or_else(|| NO_RUN.to_string())?;
    let prompt = arguments
        .get("prompt")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .ok_or_else(|| "spawn_agent requires a `prompt` describing the task".to_string())?
        .to_string();
    let label = arguments
        .get("label")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .map(str::to_string)
        .unwrap_or_default();
    let (endpoint, model) = resolve_model(&run, &arguments);

    let agent_id = uuid::Uuid::new_v4().to_string();
    let started_at = crate::database::utc_now();
    // What the user sees this run called: a label the model gave, or the model's
    // own name when it gave none.
    let display_label = if label.is_empty() {
        model.clone()
    } else {
        label.clone()
    };

    // Announced before the first token, so the panel can show the run the moment
    // it exists. A sub-agent is not written to storage until it finishes, so this
    // event -- and the same fact read back from the runtime -- is the only thing
    // that tells the panel one is at work.
    Arc::clone(&run.emit)(subagent_started_event(
        &agent_id,
        &display_label,
        run.session_id.as_deref(),
        &prompt,
        &model,
        &endpoint.id,
        &started_at,
    ));

    // Collected with arrival times so a thought can be timed. Shared into the
    // event closure by clone, and into each other through the sink.
    let events = Arc::new(Mutex::new(Vec::<Stamped>::new()));
    let collected = Arc::clone(&events);
    let on_event: Arc<dyn Fn(ChatEvent) + Send + Sync> = Arc::new({
        let emit = Arc::clone(&run.emit);
        let parent_run_id = run.run_id.clone();
        let agent_run_id = agent_id.clone();
        move |event: ChatEvent| {
            if event.kind == "tool_approval" {
                // An approval has to reach the user or the run parks forever, so
                // it is forwarded live -- under the *parent's* run id, since the
                // frontend looks a prompt up by the run it belongs to.
                emit(ChatEvent {
                    run_id: parent_run_id.clone(),
                    ..event.clone()
                });
            } else {
                // Everything else is mirrored to the panel under the agent's own
                // run id, so a run can be watched while it works. The parent's
                // listener keys by run id and knows only its own, so none of this
                // reaches the chat transcript.
                emit(ChatEvent {
                    run_id: agent_run_id.clone(),
                    ..event.clone()
                });
            }
            if let Ok(mut list) = collected.lock() {
                list.push(Stamped {
                    at: Instant::now(),
                    event,
                });
            }
        }
    });

    let searches = Arc::new(Mutex::new(Vec::<crate::websearch::WebSearchOutput>::new()));
    let collected_searches = Arc::clone(&searches);
    let on_search: Arc<dyn Fn(crate::websearch::WebSearchOutput) + Send + Sync> =
        Arc::new(move |search| {
            if let Ok(mut list) = collected_searches.lock() {
                list.push(search);
            }
        });

    let messages = vec![ChatMessage {
        role: "user".to_string(),
        content: prompt::instruction(&label, &prompt),
    }];

    let result = stream_chat(
        &endpoint,
        &model,
        &messages,
        &[],
        run.reasoning.clone(),
        run.web_search_enabled,
        // Always Agent: a sub-agent exists to do things. The spawn tool itself is
        // withheld (the `false` below), which is what keeps delegation one level
        // deep.
        ToolMode::Agent,
        run.enable_reasoning_control,
        &agent_id,
        // The parent's session scopes the cache, so a write the sub-agent makes
        // invalidates its parent's earlier cached reads rather than leaving them
        // to answer with pre-edit content.
        run.session_id.as_deref(),
        context.database.as_ref(),
        run.approval.clone(),
        Arc::clone(&context.cancelled),
        // A sub-agent's own run has no sub-agent default: it cannot spawn, so
        // there is nothing for the value to configure.
        None,
        false,
        on_event,
        on_search,
    )
    .await;

    // What gets stored. A failure is written as a reply too, so a sub-agent that
    // died is visible in the panel rather than an empty tab.
    let (content, metrics) = match &result {
        Ok((content, metrics)) => (content.clone(), metrics.clone()),
        Err(error) => (format!("The agent did not finish: {error}"), Value::Null),
    };
    let (events, searches) = (drain(&events), drain(&searches));

    if let Some(database) = context.database.as_ref() {
        let session = transcript::build_run(RunRecord {
            id: &agent_id,
            label: &label,
            prompt: &prompt,
            content: &content,
            metrics: &metrics,
            error: result.as_ref().err().map(String::as_str),
            parent_session: run.session_id.as_deref(),
            model: &model,
            provider: &endpoint.id,
            started_at: &started_at,
            events: &events,
            searches: &searches,
        });
        if let Err(save_error) = database.save_chat_session(&session) {
            tracing::warn!(agent_id, error = %save_error, "sub-agent transcript could not be saved");
        }
    }

    // Tells the panel there is a new run to read. Sent whether the run succeeded
    // or not, because a failed run is still a run the user can open.
    context.report_subagent(SubAgentNotice {
        id: agent_id,
        label: display_label,
        session_id: run.session_id.clone(),
    });

    match result {
        Ok((content, _)) if !content.trim().is_empty() => Ok(content),
        Ok(_) => Ok("The agent finished without returning an answer.".to_string()),
        Err(error) => Err(error),
    }
}

/// The endpoint and model a sub-agent should run on.
///
/// In order: an explicit `model` argument on the parent's endpoint, then the
/// model chosen in Settings, then the parent's own. The explicit argument keeps
/// the parent's endpoint because a bare model id with nowhere to send it would
/// mean guessing a provider.
fn resolve_model(run: &RunHandle, arguments: &Value) -> (Endpoint, String) {
    if let Some(model) = arguments
        .get("model")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|model| !model.is_empty())
    {
        return (run.endpoint.clone(), model.to_string());
    }
    if let Some((endpoint, model)) = &run.subagent_default {
        return (endpoint.clone(), model.clone());
    }
    (run.endpoint.clone(), run.model.clone())
}

/// Takes everything collected behind a mutex, or nothing on a poisoned lock.
fn drain<T>(slot: &Arc<Mutex<Vec<T>>>) -> Vec<T> {
    slot.lock()
        .map(|mut list| std::mem::take(&mut *list))
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai::tools::{ApprovalGate, PermissionMode};
    use crate::ai::types::ReasoningEffort;
    use serde_json::json;

    fn run(subagent_default: Option<(Endpoint, String)>) -> Arc<RunHandle> {
        Arc::new(RunHandle {
            endpoint: Endpoint {
                id: "parent-provider".into(),
                name: "Parent".into(),
                base_url: "https://example.test".into(),
                api_key: String::new(),
                models: vec!["parent-model".into()],
                disabled_models: Vec::new(),
                enabled: true,
            },
            model: "parent-model".into(),
            reasoning: ReasoningEffort("medium".into()),
            web_search_enabled: false,
            mode: ToolMode::Agent,
            enable_reasoning_control: false,
            local: false,
            run_id: "run-1".into(),
            session_id: Some("session-1".into()),
            approval: ApprovalGate::new(PermissionMode::Ask),
            emit: Arc::new(|_| {}),
            subagent_default,
        })
    }

    /// With nothing configured, a sub-agent runs on the model its parent did.
    #[test]
    fn a_sub_agent_defaults_to_the_parent_model() {
        let (endpoint, model) = resolve_model(&run(None), &json!({ "prompt": "x" }));
        assert_eq!(endpoint.id, "parent-provider");
        assert_eq!(model, "parent-model");
    }

    /// The Settings choice wins over the parent's model, but not over an explicit
    /// request from the model itself.
    #[test]
    fn the_configured_default_is_used_unless_the_call_names_one() {
        let configured = Some((
            Endpoint {
                id: "sub-provider".into(),
                name: "Sub".into(),
                base_url: "https://sub.test".into(),
                api_key: String::new(),
                models: vec!["sub-model".into()],
                disabled_models: Vec::new(),
                enabled: true,
            },
            "sub-model".to_string(),
        ));
        let (endpoint, model) = resolve_model(&run(configured.clone()), &json!({ "prompt": "x" }));
        assert_eq!(endpoint.id, "sub-provider");
        assert_eq!(model, "sub-model");

        let (endpoint, model) = resolve_model(
            &run(configured),
            &json!({ "prompt": "x", "model": "explicit-model" }),
        );
        // An explicit model rides the parent's endpoint, because a bare model id
        // names no provider.
        assert_eq!(endpoint.id, "parent-provider");
        assert_eq!(model, "explicit-model");
    }

    /// A blank model argument is the same as none, so a model that fills the field
    /// with an empty string does not silently send requests to a blank model.
    #[test]
    fn a_blank_model_argument_is_ignored() {
        let (_, model) = resolve_model(&run(None), &json!({ "prompt": "x", "model": "   " }));
        assert_eq!(model, "parent-model");
    }

    /// A context with no run reports rather than panicking, so a stray call cannot
    /// take the app down.
    #[tokio::test]
    async fn a_call_with_no_run_reports_rather_than_panicking() {
        let error = spawn(json!({ "prompt": "x" }), ToolContext::default())
            .await
            .expect_err("no run");
        assert!(error.contains("agent run"), "{error}");
    }

    /// A missing prompt is named, so the model can correct the call.
    #[tokio::test]
    async fn a_missing_prompt_is_reported() {
        let context = ToolContext {
            run: Some(run(None)),
            ..ToolContext::default()
        };
        let error = spawn(json!({}), context).await.expect_err("no prompt");
        assert!(error.contains("prompt"), "{error}");
    }
}
