//! Asking the user a question and waiting for the answer.
//!
//! A model that needs a decision only the user can make -- an ambiguous
//! requirement, a choice between approaches, a value it would otherwise have to
//! guess -- has, until now, had no way to ask. It could write the question into
//! its answer and end the turn, but then the answer is prose in a transcript
//! rather than a prompt, and the user has to translate it back into a reply.
//!
//! # Why it reuses the approval machinery
//!
//! A question is a prompt: the run parks on it, the frontend answers through a
//! Tauri command carrying only an id, and a stop has to release it. That is
//! exactly what [`super::ApprovalGate`] already does for permissions, so the
//! question rides the same gate over its own map rather than inventing a second
//! suspend-and-wake mechanism that would have to be kept in step with the first.
//!
//! # Why it is `Volatile` rather than a `Write`
//!
//! `Effect` answers two questions -- may this result be cached, and does it make
//! what was cached before go stale -- and this tool's answers are "no" and "no".
//! A cached answer to a question asked twice would be the first answer returned
//! as the second's, so it is never cached. But asking changed no file, so nothing
//! the model read beforehand went stale, and invalidating it would only make the
//! conversation pay to read the same files twice.

use std::sync::Arc;

use serde_json::Value;

use crate::ai::types::ChatEvent;

use super::{QuestionRequest, ToolContext, ToolSpec};

/// Asks the user a question and waits for the reply.
pub const ASK_USER: ToolSpec = ToolSpec {
    name: "ask_user",
    description: "Ask the user a question and wait for their answer. Use this when you genuinely \
                  cannot proceed without a decision only they can make — an ambiguous requirement, \
                  a choice between approaches, or a value you would otherwise have to guess. Offer \
                  `options` for a question with natural choices; the user can still type their own. \
                  Do not use it to confirm routine steps or to ask permission — just do the work. \
                  The reply pauses until they answer.",
    parameters: r#"{
        "type": "object",
        "properties": {
            "question": {
                "type": "string",
                "description": "The question to ask. Make it specific and self-contained, so it can be answered without re-reading the conversation."
            },
            "options": {
                "type": "array",
                "items": { "type": "string" },
                "description": "Suggested answers offered as buttons, for example [\"Postgres\", \"SQLite\"]. Optional; the user can always answer in their own words."
            }
        },
        "required": ["question"],
        "additionalProperties": false
    }"#,
    // Not a write: its result must never be cached -- an answer asked twice has
    // to be asked afresh -- but a question changes nothing, so marking it `Write`
    // only cost the conversation every read it had cached before asking.
    effect: super::Effect::Volatile,
    command_argument: None,
    execute: |arguments, context| Box::pin(ask(arguments, context)),
};

/// Reports when the call has no run behind it, which only a hand-built context does.
const NO_RUN: &str = "ask_user is only available during an agent run.";

async fn ask(arguments: Value, context: ToolContext) -> Result<String, String> {
    let run = context.run.clone().ok_or_else(|| NO_RUN.to_string())?;
    let question = arguments
        .get("question")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .ok_or("ask_user requires a non-empty `question`")?
        .to_string();
    // Blank options are dropped rather than shown as empty buttons, and the list
    // is otherwise passed through as given.
    let options: Vec<String> = arguments
        .get("options")
        .and_then(Value::as_array)
        .map(|values| {
            values
                .iter()
                .filter_map(Value::as_str)
                .map(str::trim)
                .filter(|option| !option.is_empty())
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default();

    let request = QuestionRequest {
        run_id: run.run_id.clone(),
        question,
        options,
    };
    let emit_run_id = run.run_id.clone();
    let emit = Arc::clone(&run.emit);
    let answer = run
        .approval
        .ask(
            &run.run_id,
            request,
            &context.cancelled,
            move |request, question_id| {
                // Emitted under the run's own id so it reaches the panel watching
                // this conversation. The id is what the answer command keys on.
                emit(ChatEvent {
                    run_id: emit_run_id.clone(),
                    session_id: None,
                    sequence: 0,
                    kind: "user_question".into(),
                    text: Some(request.question.clone()),
                    error: None,
                    metrics: Some(serde_json::json!({
                        "question_id": question_id,
                        "question": request.question,
                        "options": request.options,
                    })),
                });
            },
        )
        .await?;
    Ok(format!("The user answered: {answer}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// With no run there is no gate to park on, so the call reports rather than
    /// panicking. Reachable only from a hand-built context.
    #[tokio::test]
    async fn a_call_with_no_run_reports_rather_than_panicking() {
        let error = ask(
            serde_json::json!({ "question": "x" }),
            ToolContext::default(),
        )
        .await
        .expect_err("no run");
        assert!(error.contains("agent run"), "{error}");
    }

    /// Never cached -- a stale answer returned as a fresh one is worse than no
    /// answer -- but it invalidates nothing, because it changed nothing.
    #[test]
    fn asking_is_volatile_and_runs_no_command() {
        assert_eq!(ASK_USER.effect, super::super::Effect::Volatile);
        assert_eq!(ASK_USER.command_argument, None);
    }
}
