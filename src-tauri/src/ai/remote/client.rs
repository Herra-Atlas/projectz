use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};

use base64::Engine;
use futures_util::StreamExt;
use serde_json::{json, Value};
use tracing::{info, warn};

use crate::ai::remote::{
    stream_event::parse_chunk,
    tool_calls::{StreamedToolCalls, ToolCall},
    types::Endpoint,
};
use crate::ai::tools::{ApprovalRequest, Decision, Effect, ToolCache};
use crate::ai::types::{ChatAttachment, ChatEvent, ChatMessage, ReasoningEffort};

#[cfg(test)]
mod tests {
    use crate::ai::types::ReasoningEffort;

    /// A level the provider listed goes out byte for byte. Providers spell these
    /// differently — `none` on one, `off` on another — so any mapping here is a
    /// guess that fails with HTTP 400 on the next provider.
    #[test]
    fn a_reasoning_level_is_sent_exactly_as_the_provider_spelled_it() {
        for value in ["none", "off", "minimal", "low", "xhigh", "max", "MEDIUM"] {
            let effort = ReasoningEffort(value.to_string());
            assert_eq!(effort.wire_value(), Some(value));
        }
    }

    /// A model that reports no reasoning support gets no field at all, rather
    /// than a field with a guessed value in it.
    #[test]
    fn an_empty_level_sends_nothing() {
        assert_eq!(ReasoningEffort(String::new()).wire_value(), None);
        assert_eq!(ReasoningEffort("   ".to_string()).wire_value(), None);
    }

    /// A started sub-agent is announced under its **own** run id, which is what
    /// lets the panel stream it while the parent's listener -- keyed on the
    /// parent's run -- ignores every forwarded event.
    #[test]
    fn a_started_sub_agent_is_announced_under_its_own_run_id() {
        let event = super::subagent_started_event(
            "agent-1",
            "car research",
            Some("chat-1"),
            "Find every car",
            "gpt",
            "provider-1",
            "2026-01-01T00:00:00Z",
        );
        assert_eq!(event.run_id, "agent-1");
        assert_eq!(event.kind, "subagent");
        assert_eq!(event.text.as_deref(), Some("car research"));
        let metrics = event.metrics.expect("metrics are always present");
        assert_eq!(metrics["agent_id"], "agent-1");
        assert_eq!(metrics["state"], "running");
        assert_eq!(metrics["session_id"], "chat-1");
        assert_eq!(metrics["prompt"], "Find every car");
    }

    /// The finished notice carries an explicit `done`, so the panel's two
    /// lifecycle events are never told apart by the absence of a field.
    #[test]
    fn a_finished_sub_agent_is_announced_as_done() {
        let notice = crate::ai::tools::SubAgentNotice {
            id: "agent-1".into(),
            label: "car research".into(),
            session_id: Some("chat-1".into()),
        };
        let event = super::subagent_event("run-1", &notice);
        let metrics = event.metrics.expect("metrics are always present");
        assert_eq!(metrics["state"], "done");
        assert_eq!(metrics["agent_id"], "agent-1");
    }
}

/// Stream a completion, running any tools the model asks for.
///
/// `database` scopes the tool cache. Passing `None` disables caching for the
/// run, which is correct for a call with no session: an entry keyed without one
/// could answer a later conversation with this one's reads.
pub async fn stream_chat(
    endpoint: &Endpoint,
    model: &str,
    messages: &[ChatMessage],
    attachments: &[ChatAttachment],
    reasoning: ReasoningEffort,
    web_search_enabled: bool,
    mode: crate::ai::tools::ToolMode,
    enable_reasoning_control: bool,
    run_id: &str,
    session_id: Option<&str>,
    database: Option<&Arc<crate::database::Database>>,
    // Prompts through the same event callback as everything else, rather than a
    // second event path the frontend would have to know about.
    approval: crate::ai::tools::ApprovalGate,
    cancelled: Arc<AtomicBool>,
    // The model a sub-agent should use when nothing else names one. Resolved by
    // the runtime from the stored preference, so a local selection has already
    // become a live endpoint by the time it arrives here and a tool never has to
    // reach for the model registry itself.
    subagent_default: Option<(Endpoint, String)>,
    // The model `read_file` shows an image to, resolved by the runtime from the
    // stored vision preference. `None` when the user has chosen none.
    vision_default: Option<(Endpoint, String)>,
    // How aggressively to elide old context before sending. Read from the stored
    // preference by the runtime and carried on the request, like `reasoning`.
    compaction: crate::ai::compact::Compaction,
    // Whether a spawned sub-agent may itself spawn. `false` for a sub-agent's own
    // run, which is what keeps delegation one level deep.
    allow_subagents: bool,
    // Shareable rather than `FnMut`: several sub-agents can run at once, and an
    // approval prompt any of them raises has to reach the screen from whichever
    // one asked. An `Arc` of a plain `Fn` is the smallest thing that allows that.
    on_event: Arc<dyn Fn(ChatEvent) + Send + Sync>,
    on_search: Arc<dyn Fn(crate::websearch::WebSearchOutput) + Send + Sync>,
) -> Result<(String, Value), String> {
    let client = reqwest::Client::builder()
        .connect_timeout(std::time::Duration::from_secs(15))
        .build()
        .map_err(|error| error.to_string())?;
    let payload = prepare_messages(messages, attachments)?;
    // Filtered by the run's access set before anything else sees it, so a
    // capability this run does not hold is never advertised and cannot be asked
    // for. A call that arrives anyway is refused by `policy` in the loop; hiding
    // is the first half, refusing is the backstop.
    let mut registry =
        crate::ai::tools::registry_for_with(mode, web_search_enabled, allow_subagents);
    registry.retain(|name| approval.access().is_allowed(name));

    let session = session_id.unwrap_or_default();
    let cache = session_id
        .and_then(|_| database)
        .map(|database| ToolCache::new(Arc::clone(database)));
    // The cache issues its own strictly increasing stamps rather than sharing one
    // timestamp for the whole run. A single shared `now` gave every read and every
    // write in a run an identical stamp, which made a write's invalidation
    // indistinguishable from the reads taken before it -- so a re-read after an
    // edit could be answered from the cache with the pre-edit file.
    // See `ToolCache::next_stamp`.

    let mut conversation = prepend_environment(payload, mode, database);
    // Elide old context before the first send, so a conversation reopened from a
    // long history does not fail on its own length. The transcript in the database
    // is untouched; only this request's copy of it is trimmed.
    crate::ai::compact::apply(&mut conversation, compaction);
    let mut usage = Value::Null;
    // Tools are advertised on every round rather than only the first. A model
    // that finishes a turn after using a tool has still seen the schema, so
    // withholding it on round two buys nothing and makes the two rounds'
    // requests differ for no gain.
    let tool_specs = registry.specs();
    // Shared across every call in the run: one collector, drained by the loop.
    let sink = crate::ai::tools::Sink::default();
    // What a tool is told about this run, so one of them -- `sub_agent` -- can
    // start work of its own. Built here, where every part already exists, and
    // shared so several delegated runs can be launched from the same call site.
    let run = Arc::new(crate::ai::tools::RunHandle {
        endpoint: endpoint.clone(),
        model: model.to_string(),
        reasoning: reasoning.clone(),
        web_search_enabled,
        enable_reasoning_control,
        // The same flag the reasoning control rides on is exactly whether the
        // model is local: a request carries `reasoning_control` only for the
        // bundled server. It is what decides that concurrent sub-agents fall back
        // to sequential, because that server serves one request at a time.
        local: enable_reasoning_control,
        run_id: run_id.to_string(),
        session_id: session_id.map(str::to_string),
        approval: approval.clone(),
        emit: Arc::clone(&on_event),
        subagent_default,
        vision_model: vision_default,
        compaction,
        // Counted per run, so the ceiling is on what this reply may arrange.
        jobs_created: std::sync::atomic::AtomicUsize::new(0),
    });
    for _ in 0..MAX_TOOL_ROUNDS {
        if cancelled.load(Ordering::Relaxed) {
            return Ok((String::new(), Value::Null));
        }
        emit_status(run_id, "thinking", &on_event);
        let (tool_calls, content, round_usage, reasoning_text) = stream_completion(
            &client,
            endpoint,
            model,
            &conversation,
            // Borrowed because the tool-call loop sends the same level on every round,
            // and a `&str` is all the request body ever needs from it.
            &reasoning,
            tool_specs.as_ref(),
            run_id,
            Arc::clone(&cancelled),
            enable_reasoning_control,
            &on_event,
        )
        .await?;
        if usage.is_null() {
            usage = round_usage.clone();
        } else if let (Some(target), Some(values)) =
            (usage.as_object_mut(), round_usage.as_object())
        {
            // Later rounds carry the final answer's throughput, so they win.
            for (key, value) in values {
                target.insert(key.clone(), value.clone());
            }
        }

        if tool_calls.is_empty() {
            // Close the reasoning that preceded this round, if the model spent
            // any. A reply with tools emits its thinking before them and its
            // answer after, and each pass is a separate stretch of time — the
            // same model thinking again after reading a result is not one
            // unbroken thought, and merging the two would report a duration the
            // user never waited through continuously.
            if reasoning_text.chars().count() >= MIN_REPORTED_REASONING_CHARS {
                on_event(reasoning_segment(run_id, reasoning_text));
            }
            if content.is_empty() {
                return Err("Empty response from remote".to_string());
            }
            return Ok((content, usage));
        }

        // The tools arrived, so this round's reasoning is over. Emitted before
        // the first tool row so the panel reads in the order it happened: think,
        // act, think again.
        if reasoning_text.chars().count() >= MIN_REPORTED_REASONING_CHARS {
            on_event(reasoning_segment(run_id, reasoning_text));
        }

        // The assistant turn that requested the tools is echoed back before their
        // results. A `tool` message has to answer a `tool_calls` message that
        // came before it, so the order here is required, not cosmetic.
        conversation.push(json!({
            "role": "assistant",
            "content": if content.is_empty() { Value::Null } else { Value::String(content) },
            "tool_calls": tool_calls.iter().map(|call| json!({
                "id": call.id,
                "type": "function",
                "function": {"name": call.name, "arguments": call.arguments}
            })).collect::<Vec<_>>()
        }));

        // Every call in the round is executed and answered, not just the first.
        // A model that asks to read three files expects three results; stopping
        // at one makes it re-issue the same batch on the next round.
        //
        // Sequential by default, because tool results are appended in the order
        // the provider asked for them and a read taken before a write must stay
        // ordered before it. The one exception is delegation: consecutive
        // `sub_agent` calls are independent by construction -- each has its own
        // conversation -- so they are the only thing worth overlapping, and they
        // are exactly what "several agents at once" means. A local model cannot be
        // asked for more than one request at a time, so it always runs them in
        // turn.
        let runner = CallRunner {
            registry: &registry,
            cache: cache.as_ref(),
            session,
            database,
            run: &run,
            approval: &approval,
            cancelled: &cancelled,
            sink: &sink,
            run_id,
            emit: &on_event,
        };

        let mut index = 0;
        while index < tool_calls.len() {
            let spawn = crate::ai::tools::agent::SUB_AGENT.name;
            if !run.local && tool_calls[index].name == spawn {
                // The run of consecutive delegations, launched together.
                let start = index;
                let mut end = index;
                while end < tool_calls.len() && tool_calls[end].name == spawn {
                    end += 1;
                }
                // Pushed back in the order asked for, whatever order they finish
                // in, because a `tool` message has to answer the `tool_calls`
                // entry that came before it.
                let outcomes = futures_util::future::join_all(
                    tool_calls[start..end]
                        .iter()
                        .enumerate()
                        .map(|(offset, call)| runner.settle(start + offset, call)),
                )
                .await;
                for outcome in outcomes {
                    conversation.push(outcome?);
                }
                index = end;
            } else {
                conversation.push(runner.settle(index, &tool_calls[index]).await?);
                index += 1;
            }

            // Everything the tools reported while they ran. Drained after the call
            // (or batch) rather than inside it, so one tool's search is never
            // attributed to another's row.
            for search in sink.drain_searches() {
                on_search(search);
            }
        }
    }
    Err(format!(
        "Stopped after {MAX_TOOL_ROUNDS} tool rounds without a final answer"
    ))
}

/// How many times a model may call tools before it must answer.
///
/// A ceiling rather than a budget on tokens, because this is what stops a model
/// that loops on a failing tool from spending the user's money indefinitely.
/// Raised from twelve to a hundred on the reasoning that a *cached* prefix makes
/// the repeated turns of a long agentic run nearly free: the system prompt, the
/// tool schemas and the whole finished transcript sit behind the provider's
/// breakpoint, so round ninety costs a fraction of round one. The ceiling that
/// actually protects the bill is `MAX_TOOL_OUTPUT` per result and the cache
/// below it, not this number.
const MAX_TOOL_ROUNDS: usize = 100;
const STREAM_IDLE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(60);

/// The `role: "tool"` message answering one call.
fn tool_message(call_id: &str, content: &str) -> Value {
    json!({
        "role": "tool",
        "tool_call_id": call_id,
        "content": content
    })
}

/// The run's shared state, borrowed for the length of one round.
///
/// A struct rather than a dozen parameters because every field is the same for
/// each call in a round: only the call itself changes. It is what lets the round
/// run one call at a time or several at once without the per-call body being
/// written twice — [`CallRunner::settle`] is the single place a tool call is
/// gated, run, cached and reported, whether it is awaited on its own or joined
/// with others.
struct CallRunner<'a> {
    registry: &'a crate::ai::tools::ToolRegistry,
    cache: Option<&'a ToolCache>,
    session: &'a str,
    database: Option<&'a Arc<crate::database::Database>>,
    run: &'a Arc<crate::ai::tools::RunHandle>,
    approval: &'a crate::ai::tools::ApprovalGate,
    cancelled: &'a Arc<AtomicBool>,
    sink: &'a crate::ai::tools::Sink,
    run_id: &'a str,
    emit: &'a Arc<dyn Fn(ChatEvent) + Send + Sync>,
}

impl<'a> CallRunner<'a> {
    /// Gates, runs and reports one call, returning the message that answers it.
    ///
    /// `Err` means the *run* should end — today only a stop arriving while a
    /// permission prompt was open. Every other failure, including the call's own,
    /// comes back as `Ok` carrying text for the model, because a model that is
    /// told why a call failed can correct it and one whose run ended cannot.
    async fn settle(&self, index: usize, call: &ToolCall) -> Result<Value, String> {
        let arguments = match self.registry.parse_arguments(&call.name, &call.arguments) {
            Ok(arguments) => arguments,
            // A malformed call is reported back as that call's result rather than
            // ending the run, so the model can correct it itself.
            Err(error) => {
                // Reported even though it never ran. A call the panel omits is a
                // call the user cannot see the model attempt, and a silently
                // dropped malformed call looks exactly like a model that changed
                // its mind. Marked cached because no execution means no measured
                // time, and a missing duration must not read as "instant".
                (self.emit)(tool_result_event(
                    self.run_id,
                    index,
                    call,
                    &crate::ai::tools::ui::summarize(&call.name, &Value::Null, &error, false),
                    None,
                    true,
                    &error,
                ));
                return Ok(tool_message(&call.id, &error));
            }
        };

        let effect = self
            .registry
            .get(&call.name)
            .map(|tool| tool.effect)
            .unwrap_or(Effect::Read);

        // A repeat of a call already made in this conversation is answered from
        // the cache when nothing has been written since. Emitted rather than
        // skipped: the model did ask for it, and a panel that silently omitted a
        // repeat would leave the user waiting for a read that is never announced.
        if let Some(cached) = self
            .cache
            .and_then(|cache| cache.get(self.session, &call.name, effect, &arguments))
        {
            (self.emit)(tool_result_event(
                self.run_id,
                index,
                call,
                &crate::ai::tools::ui::summarize(&call.name, &arguments, &cached, true),
                None,
                true,
                &cached,
            ));
            return Ok(tool_message(&call.id, &cached));
        }

        // Announced before the gate rather than after it, so a row exists for a
        // call the user is being asked to approve. The duration starts here, which
        // counts time spent on a prompt as the call's time: that is time the user
        // waited, and hiding it would report a tool as instant while a dialog sat
        // open.
        emit_status(self.run_id, tool_status(&call.name), self.emit);
        (self.emit)(tool_call_event(self.run_id, index, call, &arguments));
        let call_started = std::time::Instant::now();
        // The gate runs here, on the loop's behalf rather than the tool's. A tool
        // that forgot to check would still be checked, which is the only
        // arrangement that makes the gate worth having.
        let decision = self
            .approval
            .settle(
                self.run_id,
                ApprovalRequest {
                    run_id: String::new(),
                    tool_name: call.name.clone(),
                    // The tool's own declared command field. Empty for every tool
                    // that does not run a shell, which is how the policy knows it
                    // is not looking at a command line.
                    command: self.registry.dangerous_string(&call.name, &arguments),
                    arguments: arguments.clone(),
                },
                self.cancelled,
                |request, approval_id| {
                    (self.emit)(ChatEvent {
                        run_id: request.run_id.clone(),
                        session_id: None,
                        sequence: 0,
                        kind: "tool_approval".into(),
                        text: Some(request.summary()),
                        error: None,
                        metrics: Some(serde_json::json!({
                            "approval_id": approval_id,
                            "tool": request.tool_name,
                            "command": request.command,
                            "arguments": request.arguments,
                        })),
                    });
                },
            )
            .await;
        let decision = match decision {
            Ok(decision) => decision,
            // The user stopped the reply while the prompt was open. A stop has to
            // be distinguishable from a reply that produced nothing, so the run
            // ends rather than returning an empty answer.
            Err(error) => {
                tracing::debug!(error = %error, "approval ended before it was answered");
                return Err(crate::ai::STOPPED.to_string());
            }
        };
        if let Decision::Deny(reason) = decision {
            // Refused and declined both become the tool's result text, so the model
            // can carry on rather than losing the conversation. Shown as a refused
            // row rather than nothing: the user declined it, and reporting it as
            // attempted-and-cancelled would misreport their own decision.
            (self.emit)(tool_result_event(
                self.run_id,
                index,
                call,
                &crate::ai::tools::ui::summarize(&call.name, &arguments, &reason, false),
                Some(call_started.elapsed().as_secs_f64()),
                true,
                &reason,
            ));
            return Ok(tool_message(&call.id, &reason));
        }

        // One context per call, sharing the collector. A tool's own failure
        // becomes its result text: the model is told what went wrong and gets
        // another turn to fix it.
        let context = crate::ai::tools::ToolContext {
            cancelled: Arc::clone(self.cancelled),
            sink: self.sink.clone(),
            // The same handle the run's tool cache is scoped to, so a tool reading
            // stored state and the cache keyed on it cannot disagree about which
            // database this conversation is using.
            database: self.database.cloned(),
            // What lets `sub_agent` start a run of its own.
            run: Some(Arc::clone(self.run)),
        };
        // `succeeded` is carried out because the text alone cannot tell a failure
        // from a tool that legitimately returned those words.
        let (result, succeeded) = match self
            .registry
            .run(&call.name, arguments.clone(), context)
            .await
        {
            Ok(text) => (text, true),
            Err(error) => (error, false),
        };
        // The diff is drained here, per call, because this is the only place that
        // knows which write produced it. Draining per round would hand two writes'
        // changes to one row and show the wrong file's diff against the wrong path.
        let mut summary =
            crate::ai::tools::ui::summarize(&call.name, &arguments, &result, succeeded);
        if let Some(diff) = self.sink.drain_diffs().into_iter().next() {
            summary.attach_diff(diff);
        }
        // Announced whether or not it worked. A tool that failed still ran, and
        // the row is what shows the user why before the model decides what to do.
        (self.emit)(tool_result_event(
            self.run_id,
            index,
            call,
            &summary,
            Some(call_started.elapsed().as_secs_f64()),
            false,
            &result,
        ));
        // A failure is never cached. The model is expected to retry with corrected
        // arguments, and a cached error would answer that retry with the same
        // failure every time.
        if succeeded {
            if let Some(cache) = self.cache {
                cache.put(self.session, &call.name, effect, &arguments, &result);
            }
        }
        Ok(tool_message(&call.id, &result))
    }
}

/// The event announcing a finished sub-agent, so the panel knows to re-read.
///
/// Carries only enough to identify the run, because the transcript itself is in
/// the database — sending it through the event would put a copy on the wire for a
/// tab that may not be open.
pub(crate) fn subagent_event(run_id: &str, notice: &crate::ai::tools::SubAgentNotice) -> ChatEvent {
    ChatEvent {
        run_id: run_id.to_string(),
        session_id: None,
        sequence: 0,
        kind: "subagent".into(),
        text: Some(notice.label.clone()),
        error: None,
        metrics: Some(json!({
            "agent_id": notice.id,
            "label": notice.label,
            "session_id": notice.session_id,
            "state": "done",
        })),
    }
}

/// The event announcing a sub-agent that has just begun, so the panel can show it
/// while it works.
///
/// Carries the task and the model, because the run is not written to the database
/// until it finishes — a panel opened mid-run has nothing else to draw the brief
/// from. Emitted directly rather than through the drain (see `Sink`): the drain is
/// read after the spawning call settles, which is when the run is already over,
/// so a start reported that way would arrive no earlier than the finish.
pub(crate) fn subagent_started_event(
    agent_id: &str,
    label: &str,
    parent_session: Option<&str>,
    prompt: &str,
    model: &str,
    provider: &str,
    started_at: &str,
) -> ChatEvent {
    ChatEvent {
        // The agent's own id, so the stream it forwards nets out under the same
        // run id and the parent's listener — which knows only the parent's run —
        // ignores every one of them.
        run_id: agent_id.to_string(),
        session_id: None,
        sequence: 0,
        kind: "subagent".into(),
        text: Some(label.to_string()),
        error: None,
        metrics: Some(json!({
            "agent_id": agent_id,
            "label": label,
            "session_id": parent_session,
            "state": "running",
            "prompt": prompt,
            "model": model,
            "provider": provider,
            "started_at": started_at,
        })),
    }
}

/// A provider's refusal, explained when the cause is one we recognise.
///
/// A local engine reports a malformed tool call as a bare HTTP 500 carrying its own
/// parser's JSON -- "Failed to parse tool call arguments as JSON" -- which reads as a
/// bug in the app rather than what it is: the model wrote a call the server could not
/// read, nearly always because its answer ran past the context window and was cut off
/// mid-argument. Naming that is the difference between the user raising their context
/// size or giving the job a smaller task, and a bug report about a 500.
fn explain_failure(status: reqwest::StatusCode, body: &str) -> String {
    if body.contains("Failed to parse tool call arguments") {
        return format!(
            "The model wrote a tool call that could not be read ({status}). Its answer was most \
             likely cut off before the call finished, which happens when a conversation runs past \
             the context window. Raise the context size, or give the job a smaller task."
        );
    }
    if body.is_empty() {
        return format!("HTTP {status}");
    }
    format!("HTTP {status}: {body}")
}

/// Shortest stretch of thinking worth a row of its own.
///
/// Providers emit fragments — a stray newline, an opening token of the answer
/// misread as thinking. Turning each of those into a "Thought for 0s" row would
/// fill the panel with steps that say nothing, so a segment has to be at least
/// this long before it is shown at all. A real thought is far longer, and one
/// this short has no content worth reading.
const MIN_REPORTED_REASONING_CHARS: usize = 24;

/// One finished stretch of thinking, as a panel step.
///
/// A separate `kind` rather than an inferred end-of-segment on the last
/// `reasoning_delta`: the frontend would otherwise have to guess when a thought
/// stopped, and the only moment that is knowable is here, where the tool calls
/// or the answer proved it ended.
fn reasoning_segment(run_id: &str, reasoning: String) -> ChatEvent {
    ChatEvent {
        run_id: run_id.to_string(),
        session_id: None,
        sequence: 0,
        kind: "reasoning_segment".into(),
        text: Some(reasoning),
        error: None,
        metrics: None,
    }
}

/// A tool call as it begins, before it runs.
///
/// Summarized with an empty result because the call has not happened yet. Only
/// the label and the detail come out of that summary, and both are read from the
/// arguments alone, so nothing here describes an outcome that has not occurred.
///
/// Carries no `seconds`: the frontend starts this row's clock when the event
/// arrives and stops it on the matching result, which is what keeps a running
/// row ticking rather than jumping to a figure.
fn tool_call_event(run_id: &str, index: usize, call: &ToolCall, arguments: &Value) -> ChatEvent {
    let mut fields =
        crate::ai::tools::ui::summarize(&call.name, arguments, "", true).event_fields();
    let object = fields
        .as_object_mut()
        .expect("summary fields are an object");
    object.insert("index".into(), Value::from(index));
    object.insert("running".into(), Value::Bool(true));
    step_event(run_id, "tool_call", call, fields)
}

/// A tool call as it ends, whatever the outcome.
///
/// `result` is the text the model was actually given, so the panel can show a
/// user the same thing rather than a summary of it. Capped on the way out for a
/// second time and for a different reason: [`output::MAX_TOOL_OUTPUT`] bounds
/// what a model has to read, but the panel's copy is stored in the transcript,
/// where an uncapped `read_file` or a build log would bloat every future load of
/// that conversation. A smaller cap is deliberate — this is a transcript, not a
/// debug log.
///
/// `cached` distinguishes a call answered from the tool cache — or one that
/// never ran because its arguments were malformed — from a call that genuinely
/// executed. Both are shown; a repeat read that took real time once and no time
/// at all the second time should not look identical in the transcript.
fn tool_result_event(
    run_id: &str,
    index: usize,
    call: &ToolCall,
    summary: &crate::ai::tools::ToolSummary,
    seconds: Option<f64>,
    cached: bool,
    result: &str,
) -> ChatEvent {
    let mut fields = summary.event_fields();
    let object = fields
        .as_object_mut()
        .expect("summary fields are an object");
    object.insert("index".into(), Value::from(index));
    object.insert("running".into(), Value::Bool(false));
    // Absent rather than zero for a call that never ran. Zero would read as a
    // genuinely instant result, which is a different claim.
    if let Some(seconds) = seconds {
        object.insert("seconds".into(), Value::from(seconds));
    }
    if cached {
        object.insert("cached".into(), Value::Bool(true));
    }
    // Only for a call that produced something worth reading. A denial or a
    // malformed call is already reported by `outcome`, and repeating it here
    // would print the same sentence twice on the same row.
    let snippet = crate::ai::tools::ui::panel_snippet(result, summary.outcome.is_err());
    if let Some(snippet) = snippet {
        object.insert("output".into(), Value::from(snippet));
    }
    step_event(run_id, "tool_result", call, fields)
}

/// Wraps step fields in the event the chat loop already speaks.
///
/// One constructor for both step kinds so the frontend reads one shape: a tool
/// name and a bag of fields. The name is what the frontend keys the row by, and
/// sending it on both ends of the call is what lets a running row be matched to
/// the result that closes it.
fn step_event(run_id: &str, kind: &str, call: &ToolCall, metrics: Value) -> ChatEvent {
    ChatEvent {
        run_id: run_id.to_string(),
        session_id: None,
        sequence: 0,
        kind: kind.into(),
        // The tool name, so the frontend can key a row without parsing the
        // label back out of prose.
        text: Some(call.name.clone()),
        error: None,
        metrics: Some(metrics),
    }
}

/// Status shown while a named tool runs.
fn tool_status(name: &str) -> &'static str {
    match name {
        "search_web" => "searching",
        _ => "using_tools",
    }
}

async fn stream_completion(
    client: &reqwest::Client,
    endpoint: &Endpoint,
    model: &str,
    messages: &[Value],
    reasoning: &ReasoningEffort,
    tools: Option<&Value>,
    run_id: &str,
    cancelled: Arc<AtomicBool>,
    enable_reasoning_control: bool,
    on_event: &Arc<dyn Fn(ChatEvent) + Send + Sync>,
) -> Result<(Vec<ToolCall>, String, Value, String), String> {
    let mut body = json!({"model": model, "messages": messages, "stream": true, "stream_options": {"include_usage": true}});
    // Passed through exactly as the provider spelled it. The field is only added
    // when there is a level to send, because a model that takes no reasoning
    // control rejects the field as firmly as it rejects a wrong value.
    if let Some(value) = reasoning.wire_value() {
        body["reasoning_effort"] = Value::String(value.to_string());
    }
    if enable_reasoning_control {
        body["reasoning_control"] = Value::Bool(true);
    }
    if let Some(tools) = tools {
        body["tools"] = tools.clone();
        body["tool_choice"] = Value::String("auto".into());
    }
    let url = crate::ai::remote::types::chat_url(&endpoint.base_url);
    info!(run_id, endpoint = %endpoint.name, model, messages = messages.len(), "remote stream start");
    let mut request = client.post(&url).json(&body);
    if !endpoint.api_key.trim().is_empty() {
        request = request.bearer_auth(&endpoint.api_key);
    }
    let response = request
        .send()
        .await
        .map_err(|error| format!("Request failed: {error}"))?;
    if !response.status().is_success() {
        let status = response.status();
        let response_body = response.text().await.unwrap_or_default();
        warn!(run_id, %status, "remote rejected");
        return Err(explain_failure(status, &response_body));
    }

    let mut bytes = response.bytes_stream();
    let mut byte_buffer = Vec::new();
    let mut event_data = Vec::new();
    let mut content = String::new();
    // Collected here as well as streamed. The deltas go out as they arrive so the
    // panel can show the thought forming, and this keeps the finished segment so
    // the loop can close it with a duration and emit it as one step.
    let mut reasoning = String::new();
    let mut metrics = json!({});
    let mut tool_calls = StreamedToolCalls::default();
    let mut sequence = 0u64;
    let mut is_finished = false;
    let mut completion_id: Option<String> = None;
    let mut is_reasoning = false;
    let mut last_chunk_at = tokio::time::Instant::now();
    'stream: loop {
        let idle_for = tokio::time::Instant::now().duration_since(last_chunk_at);
        let idle_remaining = STREAM_IDLE_TIMEOUT.saturating_sub(idle_for);
        let chunk = tokio::select! {
            _ = tokio::time::sleep(std::time::Duration::from_millis(100)) => {
                if cancelled.load(Ordering::Relaxed) { return Ok((Vec::new(), content, metrics, reasoning)); }
                if idle_remaining.is_zero() {
                    return Err(format!("Stream stalled: no response data for {} seconds", STREAM_IDLE_TIMEOUT.as_secs()));
                }
                continue 'stream;
            },
            _ = tokio::time::sleep(idle_remaining) => {
                return Err(format!("Stream stalled: no response data for {} seconds", STREAM_IDLE_TIMEOUT.as_secs()));
            },
            chunk = bytes.next() => match chunk {
                None => break 'stream,
                Some(Err(error)) => return Err(format!("Stream error: {error}")),
                Some(Ok(chunk)) => chunk,
            },
        };
        last_chunk_at = tokio::time::Instant::now();
        byte_buffer.extend_from_slice(&chunk);
        while let Some(newline) = byte_buffer.iter().position(|byte| *byte == b'\n') {
            let mut line_bytes: Vec<u8> = byte_buffer.drain(..=newline).collect();
            line_bytes.pop();
            if line_bytes.last() == Some(&b'\r') {
                line_bytes.pop();
            }
            let line = String::from_utf8(line_bytes)
                .map_err(|error| format!("Invalid UTF-8 in event stream: {error}"))?;
            if line.is_empty() {
                if event_data.is_empty() {
                    continue;
                }
                let data = event_data.join("\n");
                event_data.clear();
                if data == "[DONE]" {
                    break 'stream;
                }
                let Ok(value) = serde_json::from_str::<Value>(&data) else {
                    continue;
                };
                let Some(chunk) = parse_chunk(&value, &mut tool_calls) else {
                    continue;
                };
                if completion_id.is_none() {
                    if let Some(id) = chunk.completion_id.clone() {
                        completion_id = Some(id.clone());
                        on_event(ChatEvent {
                            run_id: run_id.to_string(),
                            session_id: None,
                            sequence: 0,
                            kind: "completion_id".into(),
                            text: Some(id),
                            error: None,
                            metrics: None,
                        });
                    }
                }
                if let Some(prompt_tokens) = chunk.prompt_tokens {
                    metrics["prompt_tokens"] = json!(prompt_tokens);
                }
                if let Some(completion_tokens) = chunk.completion_tokens {
                    metrics["completion_tokens"] = json!(completion_tokens);
                }
                // Only recorded when the provider actually reported it, so a
                // missing figure stays missing rather than reading as "cached
                // nothing" in the statistics.
                if let Some(cached_tokens) = chunk.cached_tokens {
                    metrics["cached_tokens"] = json!(cached_tokens);
                }
                if let Some(prompt_speed) = chunk.prompt_eval_tokens_per_second {
                    metrics["prompt_eval_tokens_per_second"] = json!(prompt_speed.round());
                }
                if let Some(generation_speed) = chunk.generation_tokens_per_second {
                    metrics["generation_tokens_per_second"] = json!(generation_speed.round());
                }
                let finished = chunk.finish_reason.is_some();
                if !is_finished {
                    if let Some(text) = chunk.content {
                        if is_reasoning {
                            is_reasoning = false;
                            on_event(ChatEvent {
                                run_id: run_id.to_string(),
                                session_id: None,
                                sequence: 0,
                                kind: "reasoning_phase".into(),
                                text: Some("answer".into()),
                                error: None,
                                metrics: None,
                            });
                        }
                        content.push_str(&text);
                        sequence += 1;
                        on_event(ChatEvent {
                            run_id: run_id.to_string(),
                            session_id: None,
                            sequence,
                            kind: "delta".into(),
                            text: Some(text),
                            error: None,
                            metrics: None,
                        });
                    }
                    if let Some(text) = chunk.reasoning {
                        if !is_reasoning {
                            is_reasoning = true;
                            on_event(ChatEvent {
                                run_id: run_id.to_string(),
                                session_id: None,
                                sequence: 0,
                                kind: "reasoning_phase".into(),
                                text: Some("reasoning".into()),
                                error: None,
                                metrics: None,
                            });
                        }
                        reasoning.push_str(&text);
                        sequence += 1;
                        on_event(ChatEvent {
                            run_id: run_id.to_string(),
                            session_id: None,
                            sequence,
                            kind: "reasoning_delta".into(),
                            text: Some(text),
                            error: None,
                            metrics: None,
                        });
                    }
                }
                is_finished |= finished;
            } else if let Some(data) = line.strip_prefix("data:") {
                event_data.push(data.strip_prefix(' ').unwrap_or(data).to_string());
            }
        }
    }
    Ok((tool_calls.finish(), content, metrics, reasoning))
}

/// Puts the machine's own facts ahead of the conversation.
///
/// **This is the only thing added to the prefix that varies per run, so it is
/// kept to the facts a model cannot infer.** The shell and the workspace root are
/// both invisible from a tool schema: every path in and out is relative, so
/// nothing a tool returns would ever reveal where it was operating, and a model
/// left to infer will report a Linux container's `/app` as the root. See
/// [`crate::ai::prompts::chat`] for the reasoning.
///
/// Ordering matters in two ways. The note goes **before** any system message the
/// user wrote, so a project instruction cannot contradict the machine it is being
/// sent to. And a Chat-mode run gets nothing at all: it carries only the web
/// tools, which touch no filesystem, so there is no machine to describe and no
/// skill to read -- keeping its prefix as short and as stable as it can be.
///
/// **One leading message, not two.** Everything the model needs before the
/// conversation has to arrive as a single `system` turn: llama.cpp's templates
/// reject a `system` message that is not first ("System message must be at the
/// beginning"), so a second one partway down is not portable. The environment note
/// and the skill catalogue are therefore one block joined by a blank line.
///
/// **The skill catalogue is Agent-only and does not move when a skill is read.**
/// Reading a skill is a tool call, which grows the conversation rather than this
/// prefix; only editing or disabling a skill rewrites the catalogue, which is the
/// honest cost of it being here at all.
///
/// **The root in the note is a deliberate exception to cache stability.** Naming
/// the folder means the prefix changes when the user switches workspace, so the
/// first reply in a new folder pays full price for its prefix. That is the right
/// trade: the alternative is a model confidently working in a directory the user
/// did not choose, and a wrong answer there costs far more than one uncached
/// turn. The root does not change between turns in a conversation, which is the
/// case caching actually protects.
fn prepend_environment(
    mut messages: Vec<Value>,
    mode: crate::ai::tools::ToolMode,
    database: Option<&Arc<crate::database::Database>>,
) -> Vec<Value> {
    // Only an Agent run touches the machine: its filesystem and terminal tools are
    // the ones that need to be told the shell and the workspace root. A Chat run
    // has only the web tools, which take URLs and never a path, so it gets no note.
    let machine_tools = mode == crate::ai::tools::ToolMode::Agent;
    let root = crate::ai::tools::workspace::root_string();
    let mut sections = Vec::new();
    if let Some(note) = crate::ai::prompts::chat::environment_note(
        machine_tools,
        (!root.is_empty()).then_some(root.as_str()),
    ) {
        sections.push(note);
    }
    // The catalogue is Agent-only: `skill_read` is what makes it actionable, and a
    // Chat run has no tool to read one with. It rides in the same leading block
    // rather than a message of its own, because llama.cpp rejects a `system` turn
    // that is not first.
    if machine_tools {
        if let Some(catalogue) =
            crate::ai::tools::skills::index(database.map(|database| database.as_ref()))
        {
            sections.push(catalogue);
        }
    }
    if sections.is_empty() {
        return messages;
    }
    messages.insert(
        0,
        json!({ "role": "system", "content": sections.join("\n\n") }),
    );
    messages
}

/// Run a single non-streaming completion and return the message text.
///
/// Used for short internal tasks such as generating a conversation title,
/// where there is no UI to stream into and no need for events or tools.
///
/// `max_tokens` is passed through because an uncapped request lets a reasoning
/// model deliberate at length before answering, and every token beyond the few
/// words the task needs is latency the caller waits for. It is an `Option` so a
/// caller with no natural limit can still omit it.
pub async fn complete_once(
    endpoint: &Endpoint,
    model: &str,
    messages: &[Value],
    max_tokens: Option<u32>,
) -> Result<String, String> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(60))
        .build()
        .map_err(|error| error.to_string())?;
    // No `reasoning_effort` here on purpose. The field is not portable: one
    // provider expects `none` under a different key, another accepts `off` only
    // for models that can disable thinking, and a third rejects it outright.
    // There is no single value that works everywhere, and sending a wrong one
    // fails the whole request with HTTP 400. The title task is small and
    // `max_tokens` is what keeps it short, so the field is simply not sent.
    let body = json!({
        "model": model,
        "messages": messages,
        "stream": false,
        "max_tokens": max_tokens,
    });
    let url = crate::ai::remote::types::chat_url(&endpoint.base_url);
    let mut request = client.post(&url).json(&body);
    if !endpoint.api_key.trim().is_empty() {
        request = request.bearer_auth(&endpoint.api_key);
    }
    let response = request
        .send()
        .await
        .map_err(|error| format!("Request failed: {error}"))?;
    if !response.status().is_success() {
        let status = response.status();
        let response_body = response.text().await.unwrap_or_default();
        warn!(endpoint = %endpoint.name, model, %status, "completion rejected");
        return Err(explain_failure(status, &response_body));
    }
    let payload: Value = response
        .json()
        .await
        .map_err(|error| format!("Invalid response: {error}"))?;
    // A reasoning model may answer with no content and put its thinking in
    // `reasoning_content` instead. That is a usable answer, not a failure, so it
    // is read before giving up.
    let content = payload["choices"][0]["message"]["content"]
        .as_str()
        .or_else(|| payload["choices"][0]["message"]["reasoning_content"].as_str())
        .unwrap_or_default()
        .to_string();
    Ok(content)
}

fn prepare_messages(
    messages: &[ChatMessage],
    attachments: &[ChatAttachment],
) -> Result<Vec<Value>, String> {
    let last_user_index = messages.iter().rposition(|message| message.role == "user");
    messages.iter().enumerate().map(|(index, message)| {
        if Some(index) != last_user_index || attachments.is_empty() {
            return Ok(json!({"role": message.role, "content": message.content}));
        }
        let mut content = vec![json!({"type":"text", "text":message.content})];
        for attachment in attachments {
            let decoded = base64::engine::general_purpose::STANDARD.decode(&attachment.data_base64).map_err(|_| format!("Invalid file data for {}", attachment.name))?;
            if decoded.len() > 15 * 1024 * 1024 { return Err(format!("{} is larger than 15 MB", attachment.name)); }
            if matches!(attachment.mime_type.as_str(), "image/png" | "image/jpeg" | "image/webp" | "image/gif") {
                content.push(json!({"type":"image_url", "image_url":{"url":format!("data:{};base64,{}", attachment.mime_type, attachment.data_base64)}}));
            } else {
                let text = String::from_utf8(decoded).map_err(|_| format!("{} is not valid UTF-8", attachment.name))?;
                content.push(json!({"type":"text", "text":format!("\n\nAttached file: {}\n```\n{}\n```", attachment.name, text)}));
            }
        }
        Ok(json!({"role":message.role, "content":content}))
    }).collect()
}

fn emit_status(run_id: &str, status: &str, emit: &Arc<dyn Fn(ChatEvent) + Send + Sync>) {
    emit(ChatEvent {
        run_id: run_id.to_string(),
        session_id: None,
        sequence: 0,
        kind: "status".into(),
        text: Some(status.into()),
        error: None,
        metrics: None,
    });
}
