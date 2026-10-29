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
//! # Why the tool returns at once
//!
//! A sub-agent exists to keep work *out* of its parent's context and to let the
//! parent keep working. So `sub_agent` starts the run in the background and returns
//! only an acknowledgement; the run's report is delivered later as its own message
//! (`subagent_result`), which the chat sends to the model as a new turn. The
//! agent's reading and reasoning never enter the parent's context, and the parent
//! is never made to wait on the clock the agent runs for.
//!
//! # Why approvals still work inside one
//!
//! A sub-agent is handed its parent's [`ApprovalGate`](crate::ai::tools::ApprovalGate)
//! by clone, so every call it makes is checked against the same mode and the same
//! pending-prompt map. There is no path around the gate: a sub-agent doing a write
//! in `Ask` mode stops for the user just as its parent would. The prompt is
//! re-stamped with the parent's run id, because that is the only run the screen
//! knows about, and the answer comes back through the shared map.
//!
//! # Why a parent can also ask
//!
//! Emitting the report at the end of the run is still the default: a parent that
//! does not need the answer should never be made to wait for it. But a parent whose
//! *next step* depends on that answer had no door at all -- it had to finish its
//! reply and receive the report as a new turn, which is a whole second request for
//! one dependency. [`wait`] is that door: the model names the agent id from the
//! spawn acknowledgement and parks one call on it, so dependent work happens in
//! one turn. The report is then handed back through that call instead of as a
//! hidden turn -- see [`registry::AgentRecord::finish`] for why the choice is made
//! once, and not by a race at emit time.

pub mod commands;
mod prompt;
mod registry;
mod stats;
pub(crate) mod transcript;

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde_json::Value;

use crate::ai::remote::client::{stream_chat, subagent_event, subagent_started_event};
use crate::ai::remote::types::Endpoint;
use crate::ai::tools::{AccessSet, RunHandle, SubAgentNotice, ToolContext, ToolMode};
use crate::ai::types::{ChatEvent, ChatMessage};

use transcript::{RunRecord, Stamped};

/// Reported when a tool call somehow has no run behind it.
///
/// Reachable only from a unit test that builds a context by hand, or a run that
/// was cancelled before it started. The message names the situation rather than a
/// bare refusal, so a model that sees it can stop trying.
const NO_RUN: &str = "Sub-agents are only available during an agent run.";

/// Stops one sub-agent by its id.
///
/// The stop flags live in [`registry`] beside each run's progress and its report,
/// so stopping an agent, watching it and reading its answer are questions about one
/// record rather than three that could disagree. This is what the Stop button
/// reaches.
///
/// Returns whether a run was found. A run that has already finished is not an
/// error the user needs to see: the stop button may be pressed in the moment the
/// report lands.
pub fn cancel(agent_id: &str) -> bool {
    registry::cancel(agent_id)
}

/// Starts one sub-agent and returns at once, before it has done any work.
///
/// The run is handed to a background task, so the parent is not made to wait: it
/// can keep working, and the agent's report is delivered as its own message when
/// the run finishes (see [`run_agent`]). What comes back here is only an
/// acknowledgement -- that is the tool result the model reads, and it is
/// deliberately short so the parent's next turn is cheap.
pub async fn spawn(arguments: Value, context: ToolContext) -> Result<String, String> {
    let run = context.run.clone().ok_or_else(|| NO_RUN.to_string())?;
    let prompt = arguments
        .get("instructions")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .ok_or_else(|| {
            "sub_agent requires an `instructions` field describing the task".to_string()
        })?
        .to_string();
    // The parallel cap, stated plainly rather than queued behind. Sub-agents
    // return at once, so "wait for a slot" would be a hedge the model cannot
    // see through: it would read an acknowledgement for an agent that is never
    // going to run. A refusal names the head-room as zero and tells the model
    // to carry on without spawning.
    let ceiling = run.limits.subagent_parallel;
    if ceiling > 0 {
        let running = run
            .subagent_running
            .load(std::sync::atomic::Ordering::SeqCst);
        if running >= ceiling {
            return Ok(format!(
                "Application already has {running}/{ceiling} sub agents running, \
                 unable to spawn more currently. Finish your own task with the \
                 work you have already read, or ask again after a report arrives."
            ));
        }
    }
    let label = arguments
        .get("label")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .map(str::to_string)
        .unwrap_or_default();
    let (endpoint, model) = resolve_model(&run, &arguments);
    // The kind narrows what the agent may do, not only what it is told. A
    // read-only agent is handed a gate whose access set has every writing
    // capability switched off, so even a cached or invented call is refused by
    // `decide_with_host_access` -- the prompt and the gate agree, and the gate is the
    // one that cannot be argued with.
    let kind = prompt::Kind::parse(arguments.get("type").and_then(Value::as_str));
    let approval = if kind.may_change() {
        run.approval.clone()
    } else {
        run.approval.with_access(AccessSet {
            write: false,
            terminal: false,
            subagents: false,
            jobs: false,
            // MCP tools are writes by classification, so a read-only agent cannot
            // reach them even when the server itself only reads.
            mcp: false,
            // Reading, including the web, is exactly what an explore or plan agent
            // is for.
            read: true,
            web: true,
        })
    };

    let agent_id = uuid::Uuid::new_v4().to_string();
    // What the user sees this run called: a label the model gave, or the model's
    // own name when it gave none.
    let display_label = if label.is_empty() {
        model.clone()
    } else {
        label.clone()
    };
    // The record is created before the first token, and deliberately: everything a
    // parent can ask about this agent -- is it alive, where has it got to, what did
    // it say -- has to have an answer from the moment the id exists, because the id
    // is handed to the model in the acknowledgement below.
    let record = registry::register(&agent_id, &display_label);
    // A flag of this agent's own, so Stop can end one run without ending the
    // parent or a sibling. A watcher in `run_agent` also sets it when the parent
    // is stopped, so the parent's own stop still reaches its agents.
    let cancelled = record.cancelled_flag();
    let started_at = crate::database::utc_now();

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

    let task = AgentTask {
        // Cloned rather than moved: the spawn still owns the Arc, and the slot
        // increment below has to write through it after the task has left.
        run: run.clone(),
        context,
        endpoint,
        model,
        prompt,
        label,
        display_label: display_label.clone(),
        agent_id: agent_id.clone(),
        started_at,
        kind,
        approval,
        cancelled,
        record,
    };
    tokio::spawn(run_agent(task));
    // Counted only once the task is launched: a refusal above never consumes a
    // slot, and an increment on the cancelling path in `run_agent` would race
    // the refusal that reads it.
    run.subagent_running
        .fetch_add(1, std::sync::atomic::Ordering::SeqCst);

    Ok(format!(
        "Agent `{display_label}` is working in the background (id `{agent_id}`). It cannot see this \
conversation and cannot ask you questions. Its report will arrive as its own message when it \
finishes -- and a run that fails reports the failure, with its reason, instead. If your next step \
needs the answer now, call `subagent_wait` with that id instead of finishing your turn. Otherwise \
do not wait: carry on with anything else, or finish your answer."
    ))
}

/// How long a wait parks when the call does not say.
///
/// Thirty seconds is long enough that a short research task usually lands inside
/// one call and short enough that a reply is never silently held for minutes. The
/// ceiling exists because the model is choosing the number: a wait it cannot be
/// woken from until its own limit is a stuck reply, and the user's Stop is the
/// only thing that would end it.
const WAIT_SECONDS: u64 = 30;
const WAIT_SECONDS_MAX: u64 = 120;

/// Parks one call on a sub-agent, and answers with its report or its progress.
///
/// This is the parent's *pull* half of delegation. `sub_agent` still returns at
/// once and the report still arrives on its own, so nothing here changes the
/// default; a model reaches for this only when the next step genuinely needs the
/// answer. Waiting costs no tokens while it waits, which is why it is one call
/// with a limit rather than a poll the loop has to keep round-tripping.
pub async fn wait(arguments: Value, context: ToolContext) -> Result<String, String> {
    let agent_id = arguments
        .get("agent_id")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|id| !id.is_empty())
        .ok_or_else(|| {
            "subagent_wait requires an `agent_id` -- the id printed when the agent was started"
                .to_string()
        })?;
    // Named rather than silently empty: an id from another conversation, or from a
    // previous launch, is something the model can correct once it knows that is the
    // problem. Records are process-wide and pruned, so an old id is genuinely gone.
    let Some(record) = registry::find(agent_id) else {
        return Err(format!(
            "No sub-agent with id `{agent_id}`. It may have been started by another conversation, \
             or the app may have been restarted since. Results are delivered when a run finishes, \
             so carry on with what you have."
        ));
    };

    if arguments
        .get("peek")
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        return Ok(match record.outcome() {
            Some(outcome) => report_text(&record, outcome),
            None => progress_text(record.label(), &record.progress(), "is still working"),
        });
    }

    let seconds = arguments
        .get("seconds")
        .and_then(Value::as_u64)
        .unwrap_or(WAIT_SECONDS)
        .clamp(1, WAIT_SECONDS_MAX);
    match record
        .wait_for(Duration::from_secs(seconds), &context.cancelled)
        .await
    {
        registry::Finish::Ready(outcome) => Ok(report_text(&record, outcome)),
        registry::Finish::Running(progress) => Ok(format!(
            "{} No answer within {seconds}s. Call again, or carry on without it -- its report still \
             arrives as its own message when it finishes.",
            progress_text(record.label(), &progress, "is still working")
        )),
        // A stop is the user ending the run, not a failed tool call: the reply is
        // over either way, so this is stated plainly rather than drawn as an error.
        registry::Finish::Stopped => Ok(format!(
            "Stopped waiting for `{}` because the run was stopped.",
            record.label()
        )),
    }
}

/// The finished run's report, worded so a parent can tell agents apart.
fn report_text(record: &registry::AgentRecord, outcome: registry::Outcome) -> String {
    if outcome.failed {
        // The stored failure already names the agent and carries the provider's
        // reason -- see `report_for`.
        return outcome.report;
    }
    if outcome.report.trim().is_empty() {
        return format!(
            "Agent `{}` finished without a written report.",
            record.label()
        );
    }
    format!(
        "Agent `{}` finished. Its report:\n\n{}",
        record.label(),
        outcome.report
    )
}

/// One line about where a running agent has got to.
///
/// The step count is what a reader can act on: an agent still on its first read is
/// worth waiting for, one twenty calls deep is not, and the last step says what it
/// is doing rather than that it is busy.
fn progress_text(label: &str, progress: &registry::Progress, verb: &str) -> String {
    let steps = match progress.steps {
        0 => "no steps yet".to_string(),
        1 => "1 step so far".to_string(),
        count => format!("{count} steps so far"),
    };
    let last = if progress.last_step.is_empty() {
        String::new()
    } else {
        format!(", last: {}", progress.last_step)
    };
    format!(
        "Agent `{label}` {verb} ({}s, {steps}{last}).",
        progress.elapsed_seconds
    )
}

/// Everything one background sub-agent needs, moved off the call stack.
/// A struct rather than a dozen captured locals so the task body reads as one
/// thing: the run it belongs to, the conversation it started from, and the fields
/// the panel and the transcript are built from.
struct AgentTask {
    run: Arc<RunHandle>,
    context: ToolContext,
    endpoint: Endpoint,
    model: String,
    prompt: String,
    label: String,
    display_label: String,
    agent_id: String,
    started_at: String,
    /// What kind of work this is, which frames the prompt and narrowed the gate.
    kind: prompt::Kind,
    /// The approval gate to run under. Its own clone of the parent's when the kind
    /// may change things, or a read-only narrowed one when it may not.
    approval: crate::ai::tools::ApprovalGate,
    /// This agent's own stop flag, registered so the panel can end it alone.
    cancelled: Arc<AtomicBool>,
    /// What the parent can ask about this agent while it works, and read when it
    /// has finished. Shared with the registry, so it is the same value the tool
    /// looks up by id.
    record: Arc<registry::AgentRecord>,
}

/// Runs the sub-agent to completion and delivers its result.
///
/// The same loop the blocking version ran -- same streaming, same tools, same
/// permission gate, same transcript -- except that it ends by emitting the report
/// as its own message rather than returning it as a tool result. By the time it
/// finishes the parent's turn has usually ended, so a return value would have
/// nobody to receive it.
async fn run_agent(task: AgentTask) {
    let AgentTask {
        run,
        context,
        endpoint,
        model,
        prompt,
        label,
        display_label,
        agent_id,
        started_at,
        kind,
        approval,
        cancelled,
        record,
    } = task;

    // The parent's stop must still reach its agents, but this agent's own stop
    // must not reach the parent. A watcher folds the parent's flag into this
    // one, and ends as soon as either is set -- including on a normal finish,
    // because `watch.abort()` below stops it.
    let watch = tokio::spawn({
        let parent = Arc::clone(&context.cancelled);
        let child = Arc::clone(&cancelled);
        async move {
            while !parent.load(Ordering::Relaxed) && !child.load(Ordering::Relaxed) {
                tokio::time::sleep(std::time::Duration::from_millis(100)).await;
            }
            child.store(true, Ordering::Relaxed);
        }
    });

    // Collected with arrival times so a thought can be timed. Shared into the
    // event closure by clone, and into each other through the sink.
    let events = Arc::new(Mutex::new(Vec::<Stamped>::new()));
    let collected = Arc::clone(&events);
    // What an attempt produced, so a failed one may be retried only when it
    // changed nothing. A run that already streamed an answer or called a tool is
    // final: replaying it could duplicate output or repeat a write. Reset before
    // each attempt, exactly as the parent's own ladder does.
    let produced = Arc::new(AtomicUsize::new(0));
    let tool_calls = Arc::new(AtomicUsize::new(0));
    let produced_counter = Arc::clone(&produced);
    let tool_counter = Arc::clone(&tool_calls);
    // What a `subagent_wait` reads while it is parked: how many calls the run has
    // made and the last one. Updated here rather than derived later because the
    // events are this run's only account of its own progress, and this closure is
    // the one place they all pass through.
    let progress = Arc::clone(&record);
    let on_event: Arc<dyn Fn(ChatEvent) + Send + Sync> = Arc::new({
        let emit = Arc::clone(&run.emit);
        let parent_run_id = run.run_id.clone();
        let agent_run_id = agent_id.clone();
        move |event: ChatEvent| {
            match event.kind.as_str() {
                "delta" => {
                    produced_counter.fetch_add(1, Ordering::Relaxed);
                }
                "tool_call" => {
                    tool_counter.fetch_add(1, Ordering::Relaxed);
                    // The label is what a reader would call the step; the tool
                    // name is the fallback for an event that carries no summary.
                    let label = event
                        .metrics
                        .as_ref()
                        .and_then(|metrics| metrics.get("label"))
                        .and_then(Value::as_str)
                        .or(event.text.as_deref())
                        .unwrap_or_default();
                    progress.note_step(label);
                }
                _ => {}
            }
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
        content: prompt::instruction(kind, &label, &prompt),
    }];

    // Written before the run rather than after it. The transcript used to be saved
    // only when the loop returned, so an agent whose process ended -- the app quit,
    // a restart -- left no row at all: the panel had nothing to show, and even the
    // task it had been given was gone. The finished run saves the same id, and the
    // turn is upserted with the reply appended.
    if let Some(database) = context.database.as_ref() {
        let started = transcript::build_started(
            &agent_id,
            &label,
            &prompt,
            run.session_id.as_deref(),
            &model,
            &endpoint.id,
            &started_at,
        );
        if let Err(error) = database.save_chat_session(&started) {
            tracing::warn!(agent_id, %error, "a sub-agent's start could not be saved");
        }
    }

    let context_window = if endpoint.id == crate::ai::local::LOCAL_ENDPOINT_ID {
        run.local_context_window
    } else {
        context
            .database
            .as_ref()
            .and_then(|database| database.model_capabilities(&endpoint.id, &model).ok())
            .and_then(|capabilities| capabilities.context_length)
            .and_then(|context| u64::try_from(context).ok())
    };

    // The same ladder the parent run applies, for the same reason: a delegated
    // run that failed without producing a token or calling a tool changed nothing,
    // so it can be asked again; one that already answered or acted is final. It
    // used to fail on the first transient error -- "The agent did not finish:
    // Empty response from remote" with no attempt to recover -- because the ladder
    // lived only in the runtime, and a sub-agent calls `stream_chat` directly.
    let mut retries = 0usize;
    let result = loop {
        produced.store(0, Ordering::Relaxed);
        tool_calls.store(0, Ordering::Relaxed);
        let attempt = stream_chat(
            &endpoint,
            &model,
            &messages,
            &[],
            // A delegated run answers its parent, not the channel. Channel wording and
            // delivery belong to the turn the user is actually reading. The general
            // instructions, however, follow the same door as the parent: replacing
            // them for Telegram must not leak them back through a delegated task.
            None,
            run.suppress_general_instructions,
            None,
            // The configured effort wins; the parent's is the fallback. Resolved
            // here rather than by the runtime because the whole override is one
            // string the parent already carries.
            run.limits
                .subagent_effort
                .clone()
                .unwrap_or_else(|| run.reasoning.clone()),
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
            // The narrowed gate for a read-only kind, or the parent's own for a
            // general one. Either way the pending-prompt map is shared, so an
            // approval still reaches the same answer command.
            approval.clone(),
            Arc::clone(&cancelled),
            // A sub-agent's own run has no sub-agent default: it cannot spawn, so
            // there is nothing for the value to configure.
            None,
            // Nor a vision model: a sub-agent has no reason to open an image, and
            // withholding it keeps the preference resolved in one place.
            None,
            // The parent's compression knobs, so a long delegated run compresses the
            // way the conversation that started it does.
            run.compression,
            // The sub-agent's own ceiling is the *sub-agent* one, not the parent
            // reply's; the parallel cap has no meaning inside an agent that cannot
            // spawn.
            crate::ai::types::RunLimits {
                rounds: run.limits.subagent_rounds,
                subagent_rounds: run.limits.subagent_rounds,
                subagent_parallel: run.limits.subagent_parallel,
                subagent_effort: run.limits.subagent_effort.clone(),
            },
            context_window,
            run.local_context_window,
            None,
            false,
            Arc::clone(&on_event),
            Arc::clone(&on_search),
        )
        .await;
        match attempt {
            Err(reason) => {
                let Some((next, delay)) = crate::ai::runtime::retry_step(
                    &reason,
                    retries,
                    produced.load(Ordering::Relaxed),
                    tool_calls.load(Ordering::Relaxed),
                ) else {
                    break Err(reason);
                };
                retries = next;
                tracing::info!(
                    agent_id = %agent_id,
                    attempt = retries,
                    delay,
                    %reason,
                    "retrying a sub-agent that changed nothing"
                );
                // Under the agent's own run id: the panel is the surface watching
                // this run, and the parent's listener keys on its own id.
                on_event(ChatEvent {
                    run_id: agent_id.clone(),
                    session_id: run.session_id.clone(),
                    sequence: 0,
                    kind: "retrying".into(),
                    text: Some(reason.clone()),
                    error: None,
                    metrics: Some(serde_json::json!({
                        "attempt": retries,
                        "attempts": crate::ai::RETRY_DELAYS_SECONDS.len(),
                        "delay_seconds": delay,
                        "error": reason,
                    })),
                });
                if crate::ai::runtime::wait_before_retry(delay, &cancelled).await {
                    // A stop during the wait is the user ending the run, not a
                    // failure: the same shape a cancelled `stream_chat` returns, so
                    // no "did not finish" report is sent for work nobody cancelled
                    // but the user.
                    break Ok((String::new(), Value::Null));
                }
            }
            other => break other,
        }
    };

    // What gets stored. A failure is written as a reply too, so a sub-agent that
    // died is visible in the panel rather than an empty tab.
    let (content, metrics) = match &result {
        Ok((content, metrics)) => (content.clone(), metrics.clone()),
        Err(error) => (format!("The agent did not finish: {error}"), Value::Null),
    };
    let (events, searches) = (drain(&events), drain(&searches));
    // What the run cost and how much it did, for the parent's `sub_agent` row. Read
    // from the same events the transcript is built from, so the two can never
    // disagree about how many calls or rounds there were -- see [`stats`].
    let run_stats = stats::summarize(&metrics, &events);

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

    // Released here rather than in the caller, so the slot is held for as long
    // as the agent is actually working -- including a run that failed -- and a
    // cap of one cannot be eaten by a task that is already done.
    release_slot(&run);
    // Tells the panel there is a new run to read, and clears it from the running
    // registry. Emitted directly rather than through the run's sink: the parent's
    // loop may already be over, and nothing would drain it.
    Arc::clone(&run.emit)(subagent_event(
        &run.run_id,
        &SubAgentNotice {
            id: agent_id.clone(),
            label: display_label.clone(),
            session_id: run.session_id.clone(),
        },
    ));

    // The report, as its own message in the parent conversation.
    //
    // **A failure is sent too, and that is the point of this branch.** The parent
    // was told the report would arrive as its own message and was told explicitly
    // not to wait for it, so a run that dies without one leaves the conversation
    // believing an answer is still coming that never will. Naming the failure lets
    // the model retry, route around it, or tell the user -- which is the whole
    // reason the report exists. [`report_for`] holds the rule; this only delivers
    // what it returns.
    // The run is over, whoever ended it, so nothing can stop it any more and the
    // watcher has nothing left to watch.
    watch.abort();

    // Recorded before the event, because the event's `delivered` says whether a
    // parked `subagent_wait` already took it. An empty success is still recorded:
    // a wait parked on it must come back with an answer, even if that answer is
    // "finished and said nothing".
    let report = report_for(&result, &display_label);
    let awaited = record.finish(registry::Outcome {
        report: report
            .as_ref()
            .map(|(text, _)| text.clone())
            .unwrap_or_default(),
        failed: report.as_ref().map(|(_, failed)| *failed).unwrap_or(false),
    });

    let Some((report, failed)) = report else {
        return;
    };
    let session_id = run.session_id.clone();
    Arc::clone(&run.emit)(ChatEvent {
        run_id: agent_id.clone(),
        session_id: session_id.clone(),
        sequence: 0,
        kind: "subagent_result".into(),
        text: Some(report),
        error: None,
        metrics: Some(serde_json::json!({
            "agent_id": agent_id,
            "label": display_label,
            "session_id": session_id,
            // Lets the chat draw the `sub_agent` row as failed -- and say so to the
            // user -- rather than showing the report as if the work had succeeded.
            "failed": failed,
            // Whether the report becomes a hidden turn. A wait that was parked when
            // the run ended has already handed it to the model as that call's
            // result, so sending it again would spend the conversation a second
            // reply to say what it has just read.
            "delivered": !awaited,
            // What the run cost and how much it did, drawn under the report on the
            // `sub_agent` row -- the delegated run's own totals, so a delegate can
            // be judged from the reply that spawned it.
            "stats": run_stats,
        })),
    });
}

/// Releases one parallel slot.
///
/// The increment and the release are the only writers, and each spawn has
/// exactly one of each, so the counter cannot drift; there is no saturating
/// dance to defend against a release nobody earned.
fn release_slot(run: &RunHandle) {
    run.subagent_running
        .fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
}

/// The message a finished sub-agent hands to its parent, and whether it failed.
///
/// `None` means there is nothing worth sending: a run that succeeded but wrote
/// only whitespace, which the panel already shows as an empty reply.
///
/// A failure is always `Some`, because the parent is owed a word either way --
/// silence is indistinguishable from an agent still at work, and the parent was
/// told not to wait. Extracted from the emit above so the rule can be tested on
/// its own, without a live provider.
fn report_for(result: &Result<(String, Value), String>, label: &str) -> Option<(String, bool)> {
    match result {
        Ok((content, _)) if !content.trim().is_empty() => Some((content.clone(), false)),
        Ok(_) => None,
        Err(error) => Some((
            format!("The sub-agent `{label}` did not finish: {error}"),
            true,
        )),
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
        run_with(subagent_default, 0)
    }

    /// The same handle, with the parallel sub-agent cap set. `0` -- the other
    /// fixture's value -- removes the ceiling entirely, which is why the tests
    /// above it never need to know the cap exists.
    fn run_with(subagent_default: Option<(Endpoint, String)>, parallel: usize) -> Arc<RunHandle> {
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
            enable_reasoning_control: false,
            local: false,
            run_id: "run-1".into(),
            session_id: Some("session-1".into()),
            approval: ApprovalGate::new(PermissionMode::Ask),
            emit: Arc::new(|_| {}),
            subagent_default,
            vision_model: None,
            compression: crate::ai::compact::Compression::default(),
            limits: crate::ai::types::RunLimits {
                subagent_parallel: parallel,
                ..crate::ai::types::RunLimits::default()
            },
            subagent_running: std::sync::atomic::AtomicUsize::new(0),
            local_context_window: None,
            jobs_created: std::sync::atomic::AtomicUsize::new(0),
            delivery_target: None,
            suppress_general_instructions: false,
        })
    }

    /// The parallel cap is refused by name rather than queued behind: a
    /// sub-agent returns at once, so an acknowledgement for a slot that will
    /// never free up is the one lie a tool result must not tell.
    #[tokio::test]
    async fn a_spawn_past_the_parallel_cap_is_refused_by_name() {
        let handle = run_with(None, 2);
        // Two already in flight, which is the whole ceiling.
        handle
            .subagent_running
            .store(2, std::sync::atomic::Ordering::SeqCst);
        let context = ToolContext {
            run: Some(handle),
            ..ToolContext::default()
        };
        let refusal = spawn(json!({ "instructions": "x" }), context)
            .await
            .expect("a refusal is a tool result, not an error");
        assert!(refusal.contains("2/2"), "{refusal}");
        assert!(refusal.contains("unable to spawn more"), "{refusal}");
    }

    /// A failed sub-agent reports to its parent rather than going silent. This is
    /// the bug the branch exists to fix: the parent was told a report would arrive
    /// and not to wait, so a run that died without one left the conversation
    /// waiting for a message that never came.
    #[test]
    fn a_failed_sub_agent_reports_its_failure_to_its_parent() {
        let (text, failed) = report_for(
            &Err("HTTP 400: The stealth model could not complete the request.".to_string()),
            "car research",
        )
        .expect("a failure is always reported");
        assert!(failed);
        assert!(text.contains("car research"), "{text}");
        assert!(text.contains("HTTP 400"), "{text}");
    }

    /// A successful answer is delivered as the report, and is not marked failed.
    #[test]
    fn an_answered_sub_agent_reports_its_answer() {
        let (text, failed) = report_for(
            &Ok(("Found three cars.".to_string(), Value::Null)),
            "car research",
        )
        .expect("an answer is reported");
        assert!(!failed);
        assert_eq!(text, "Found three cars.");
    }

    /// An empty success sends nothing: there is no failure to name and no answer to
    /// deliver, so the panel is left to show the run.
    #[test]
    fn an_empty_success_sends_nothing() {
        assert!(report_for(&Ok(("   ".to_string(), Value::Null)), "x").is_none());
    }

    /// With nothing configured, a sub-agent runs on the model its parent did.
    #[test]
    fn a_sub_agent_defaults_to_the_parent_model() {
        let (endpoint, model) = resolve_model(&run(None), &json!({ "instructions": "x" }));
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
        let (endpoint, model) =
            resolve_model(&run(configured.clone()), &json!({ "instructions": "x" }));
        assert_eq!(endpoint.id, "sub-provider");
        assert_eq!(model, "sub-model");

        let (endpoint, model) = resolve_model(
            &run(configured),
            &json!({ "instructions": "x", "model": "explicit-model" }),
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
        let (_, model) = resolve_model(&run(None), &json!({ "instructions": "x", "model": "   " }));
        assert_eq!(model, "parent-model");
    }

    /// A context with no run reports rather than panicking, so a stray call cannot
    /// take the app down.
    #[tokio::test]
    async fn a_call_with_no_run_reports_rather_than_panicking() {
        let error = spawn(json!({ "instructions": "x" }), ToolContext::default())
            .await
            .expect_err("no run");
        assert!(error.contains("agent run"), "{error}");
    }

    /// A missing `instructions` field is named, so the model can correct the call.
    #[tokio::test]
    async fn a_missing_instructions_is_reported() {
        let context = ToolContext {
            run: Some(run(None)),
            ..ToolContext::default()
        };
        let error = spawn(json!({}), context)
            .await
            .expect_err("no instructions");
        assert!(error.contains("instructions"), "{error}");
    }
}
