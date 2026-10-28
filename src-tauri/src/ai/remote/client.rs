use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};

use base64::Engine;
use futures_util::{FutureExt, StreamExt};
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
    use super::{
        accumulate_usage, budget_notice, merge_round_usage, BUDGET_FINAL_REMAINING,
        REQUEST_ATTEMPTS,
    };
    use crate::ai::types::ReasoningEffort;
    use serde_json::{json, Value};

    /// The retry is exactly one, so a real failure is reported rather than retried
    /// indefinitely. The requirement is "try again once, then say it failed", and
    /// this number is the whole of that promise.
    #[test]
    fn a_request_is_retried_exactly_once() {
        assert_eq!(REQUEST_ATTEMPTS, 2);
    }

    /// The budget is stated, and the last one is an instruction to answer.
    ///
    /// This is the only thing that turns the round ceiling from a wall into a
    /// budget: a model told how much is left will spend it on the task, and a
    /// model told its next answer is the last will write a report instead of
    /// starting something it cannot finish.
    #[test]
    fn a_budget_notice_states_what_is_left() {
        let warned = budget_notice(12);
        let text = warned["content"].as_str().expect("content");
        assert!(text.contains("12 tool rounds left"), "{text}");
        assert!(!text.contains("Stop calling tools"), "{text}");
        // A user turn, because llama.cpp rejects a system message that is not
        // first and this arrives mid-conversation.
        assert_eq!(warned["role"], "user");

        let final_call = budget_notice(0);
        let last = final_call["content"].as_str().expect("content");
        assert!(last.contains("Stop calling tools"), "{last}");
        assert!(last.contains("still open"), "{last}");
        // The last notice is handed over with no tools at all, so it must read as
        // the floor of the budget rather than as one more warning.
        assert!(BUDGET_FINAL_REMAINING < 12);
    }

    /// The merge the answer and the wrap-up call both use.
    ///
    /// Last-wins on the context figure, accumulate on the spend -- two different
    /// questions about the same round that must not collapse into one another.
    #[test]
    fn a_round_merges_as_context_last_wins_and_spend_accumulated() {
        let mut usage = Value::Null;
        merge_round_usage(
            &mut usage,
            &json!({"prompt_tokens": 100, "completion_tokens": 5}),
        );
        merge_round_usage(
            &mut usage,
            &json!({"prompt_tokens": 400, "completion_tokens": 7}),
        );
        assert_eq!(
            usage["prompt_tokens"], 400,
            "the context is the latest round's"
        );
        assert_eq!(
            usage["used_prompt_tokens"], 500,
            "the spend is every round's"
        );
        assert_eq!(usage["used_completion_tokens"], 12);
    }

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

    /// A reply that ran a tool is billed for both rounds. The context at the
    /// answer (7,000) is what the charts plot; the spend (12,000) is what the
    /// `used_*` totals carry, and the two must not collapse into one another.
    /// `context_tokens` keeps the conversation *before* the tool round, which is
    /// what the next request will actually contain.
    #[test]
    fn used_tokens_add_up_across_a_reply_while_prompt_tokens_keep_the_last() {
        let mut usage = Value::Null;
        merge_round_usage(
            &mut usage,
            &json!({"prompt_tokens": 5000, "completion_tokens": 40}),
        );
        // The second round after the tool result, merged last-wins as the loop does.
        merge_round_usage(
            &mut usage,
            &json!({"prompt_tokens": 7000, "completion_tokens": 60}),
        );

        assert_eq!(usage["prompt_tokens"], 7000);
        assert_eq!(usage["context_tokens"], 5000);
        assert_eq!(usage["used_prompt_tokens"], 12000);
        assert_eq!(usage["used_completion_tokens"], 100);
    }

    /// A provider that reports no usage must leave the totals absent, not zero:
    /// the frontend reads absence as "unknown" and falls back to the last-wins
    /// figure, whereas a zero would report a billed-but-unmeasured reply as free.
    #[test]
    fn an_unreported_round_leaves_the_totals_absent() {
        let mut usage = json!({});
        accumulate_usage(&mut usage, &json!({}));
        assert!(usage.get("used_prompt_tokens").is_none());
        assert!(usage.get("used_completion_tokens").is_none());
    }

    /// A cancelled run returns a null usage object, and nothing may be grafted
    /// onto it -- an object check is what stops a panic building totals there.
    #[test]
    fn a_null_usage_is_left_untouched() {
        let mut usage = Value::Null;
        accumulate_usage(&mut usage, &json!({"prompt_tokens": 10}));
        assert!(usage.is_null());
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
    additional_instructions: Option<&str>,
    suppress_general_instructions: bool,
    // Where this run came from, when a tool it calls -- `schedule_job` -- has to
    // remember how to reach the person later.
    delivery: Option<crate::ai::types::DeliveryTarget>,
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
    compression: crate::ai::compact::Compression,
    // The run's own ceilings: rounds for this loop, and what a spawned
    // sub-agent gets for its own rounds, concurrency and effort. Resolved by
    // the runtime from the stored preferences, like the rest of the knobs.
    limits: crate::ai::types::RunLimits,
    context_window: Option<u64>,
    local_context_window: Option<u64>,
    // How full the context was when the previous reply's first request was sent.
    // The reply's first round, not its last: the last round of a tool-using reply
    // carries results that are never stored, so it describes a request this turn
    // will not make. Deciding from it compacted conversations that had not grown,
    // and a compaction is a cache miss for every token it rewrites.
    previous_context_tokens: Option<u64>,
    // Whether a spawned sub-agent may itself spawn. `false` for a sub-agent's own
    // run, which is what keeps delegation one level deep.
    allow_subagents: bool,
    // Shareable rather than `FnMut`: several sub-agents can run at once, and an
    // approval prompt any of them raises has to reach the screen from whichever
    // one asked. An `Arc` of a plain `Fn` is the smallest thing that allows that.
    on_event: Arc<dyn Fn(ChatEvent) + Send + Sync>,
    on_search: Arc<dyn Fn(crate::websearch::WebSearchOutput) + Send + Sync>,
) -> Result<(String, Value), String> {
    // Shared across every run and every internal completion: see `http_client`.
    let client = http_client();
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

    let mut conversation = prepend_environment(
        payload,
        mode,
        database,
        additional_instructions,
        suppress_general_instructions,
    );
    // **Compression is a boundary, never a step.** It is decided once, from the
    // previous reply's measured prompt size, before this run sends anything. The
    // loop below deliberately does not re-check it: a run that crosses the trigger
    // part-way through is still using the tool output it just received, and
    // eliding that mid-work would remove the evidence the model is reasoning over.
    // A long run is bounded by its step budget and by each tool result's own cap.
    let target_tokens = context_window.map(|window| {
        // The configured target, as a token number. At least one: a target of
        // zero would demand a request of nothing, and `apply` would strip the
        // whole protected tail chasing it -- which no setting should mean.
        (((window as f64) * compression.target) as usize).max(1)
    });
    let passes =
        crate::ai::compact::should_compress(compression, previous_context_tokens, context_window);
    if passes {
        let outcome = crate::ai::compact::apply(&mut conversation, compression, target_tokens);
        // A pass that changed nothing is not news. A chat transcript carries no
        // `tool` messages to stub, so a pass can legitimately reach this point
        // with the request untouched -- and the notice used to be appended anyway,
        // adding a hidden turn to every later request that said nothing happened.
        let changed = outcome.removed > 0 || !outcome.dropped.is_empty();
        if changed {
            // Tell the transcript. The user asked to see this the way a finished
            // sub-agent is shown, rather than discovering later that context moved.
            on_event(ChatEvent {
                run_id: run_id.to_string(),
                session_id: None,
                sequence: 0,
                kind: "compaction".into(),
                text: Some(crate::ai::compact::notice().to_string()),
                error: None,
                metrics: None,
            });
        }
        // The summary is the only depth that costs a model call, and it runs only
        // when elision and the drop both left the request above the target. The
        // estimate losing to the *summariser's own output* is fine: whatever comes
        // back is one bounded message, and a long summary is still far shorter than
        // the turns it replaced.
        let still_over = target_tokens
            .map(|target| crate::ai::compact::estimate_tokens(&conversation) > target)
            .unwrap_or(false);
        if still_over && !outcome.dropped.is_empty() {
            if let Some(summary) = summarise_dropped(
                &endpoint,
                model,
                &outcome.dropped,
                compression.model_timeout_secs,
            )
            .await
            {
                // After the system block -- a `user` summary mid-conversation is
                // a turn the template accepts, a second `system` one may not be.
                let insert_at = conversation
                    .iter()
                    .take_while(|message| {
                        message.get("role").and_then(Value::as_str) == Some("system")
                    })
                    .count();
                conversation.insert(
                    insert_at,
                    json!({ "role": "user", "content": format!("{} {}", crate::ai::compact::SUMMARY_PREFIX, summary) }),
                );
            }
        }
    }
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
        compression,
        limits: limits.clone(),
        // Counted across the handle so sub-agents spawned by this run still
        // raise it while they work, and the spawn gate can refuse from anywhere.
        subagent_running: std::sync::atomic::AtomicUsize::new(0),
        local_context_window,
        // Counted per run, so the ceiling is on what this reply may arrange.
        jobs_created: std::sync::atomic::AtomicUsize::new(0),
        delivery_target: delivery,
        suppress_general_instructions,
    });
    for round in 0..limits.rounds {
        if cancelled.load(Ordering::Relaxed) {
            return Ok((String::new(), Value::Null));
        }
        // The budget, stated before the request that will spend it. See
        // `budget_notice` for why this is the only thing that makes the ceiling
        // anything other than a wall the model walks into.
        let remaining = limits.rounds - round;
        if remaining == BUDGET_WARN_REMAINING || remaining == BUDGET_FINAL_REMAINING {
            conversation.push(budget_notice(remaining));
        }
        emit_status(run_id, "thinking", &on_event);
        let (tool_calls, content, round_usage, reasoning_text) = stream_completion(
            client,
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
        merge_round_usage(&mut usage, &round_usage);
        // The context the provider just counted, as it arrives. Emitted mid-run
        // because the composer's ring is the only place a user can see a reply
        // approaching the window, and the answer's own figure does not exist until
        // the reply is over -- by which point it is too late to be useful.
        if let Some(prompt_tokens) = round_usage.get("prompt_tokens").and_then(Value::as_u64) {
            on_event(ChatEvent {
                run_id: run_id.to_string(),
                session_id: None,
                sequence: 0,
                kind: "usage".into(),
                text: None,
                error: None,
                metrics: Some(json!({ "prompt_tokens": prompt_tokens })),
            });
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
    // The ceiling is a budget, not a guillotine.
    //
    // This used to end the run with an error, which threw away everything the
    // reply had done and told the user "Stopped after 100 tool rounds without a
    // final answer" -- the least useful sentence available at that moment. Instead
    // the model is asked once more with **tools withheld**: it cannot open another
    // line of enquiry, so the only thing it can do is answer in prose -- what it
    // did, what is verified, what is still open, and anything it needs. The
    // notices pushed on the way here are what keep that from being a surprise.
    emit_status(run_id, "thinking", &on_event);
    conversation.push(budget_notice(0));
    let (_, content, round_usage, reasoning_text) = stream_completion(
        client,
        endpoint,
        model,
        &conversation,
        &reasoning,
        // No tools. That is the whole mechanism: a round that could call a tool
        // would be another round, and there is no budget left for one.
        None,
        run_id,
        Arc::clone(&cancelled),
        enable_reasoning_control,
        &on_event,
    )
    .await?;
    merge_round_usage(&mut usage, &round_usage);
    if reasoning_text.chars().count() >= MIN_REPORTED_REASONING_CHARS {
        on_event(reasoning_segment(run_id, reasoning_text));
    }
    if content.trim().is_empty() {
        return Err(format!(
            "Stopped after {} tool rounds without a final answer",
            limits.rounds
        ));
    }
    Ok((content, usage))
}

/// How many times a model may call tools before it must answer.
///
/// The ceiling is [`crate::ai::types::RunLimits::rounds`], read from the stored
/// preferences and carried on the request, so the setting lives in the Model
/// section rather than here. What it protects against is unchanged: a model that
/// loops on a failing tool spends the user's money indefinitely. Its reasoning
/// survives too -- repeated turns are made cheap by the queued prefix, so the
/// ceiling that actually protects the bill is each tool result's own cap and the
/// cache, not this number.
const STREAM_IDLE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(60);

/// Rounds left when the model is told how much budget it has.
///
/// Nothing else in the loop tells a model to stop. It answers when it decides it
/// is finished, and a model that keeps finding one more thing to check will keep
/// going until the ceiling ends the run for it -- which is how a small task
/// reached a hundred rounds. Two notices turn the ceiling into a budget the model
/// can plan against: one early enough to change what it does next, and one at the
/// end so the last answer is a report rather than a truncated thought.
const BUDGET_WARN_REMAINING: usize = 12;

/// Rounds left when the model is told to stop looking and answer.
///
/// The final call is made with no tools at all, so this is a warning rather than
/// the mechanism -- it exists so the model spends its last rounds writing rather
/// than starting something it cannot finish.
const BUDGET_FINAL_REMAINING: usize = 3;

/// The budget as the model reads it, appended to the request mid-conversation.
///
/// A `user` turn rather than a `system` one, even though it is the app talking
/// and not the user: llama.cpp rejects a `system` message that is not first, and
/// this has to arrive where the model already is. Nothing here reaches the
/// transcript -- it is pushed onto the run's request copy, and the frontend
/// builds the stored conversation from its own messages -- so it is not something
/// the user has to read past.
///
/// `remaining == 0` is the closing instruction, handed over with the tools
/// withheld, which is why it asks for a report rather than for economy.
fn budget_notice(remaining: usize) -> Value {
    let content = if remaining <= BUDGET_FINAL_REMAINING {
        format!(
            "[Budget] {remaining} tool rounds left, and then this reply is stopped mid-task. \
             Stop calling tools and answer now: what you have done, what you verified, what is \
             still open, and anything you need from the user. A partial answer that says where it \
             stopped is worth far more than a run that ends without one."
        )
    } else {
        format!(
            "[Budget] You have {remaining} tool rounds left. Spend them on the task rather than \
             on re-reading or re-checking what you have already established, and move towards an \
             answer from here -- batch the calls you still need, and finish with a report rather \
             than opening a new line of enquiry."
        )
    };
    json!({ "role": "user", "content": content })
}

/// How much of the dropped text one summary may read.
const SUMMARY_MAX_CHARS: usize = 24_000;
/// How much of any single turn the summary keeps whole.
const SUMMARY_TURN_CHARS: usize = 4_000;
/// What a summary may be, in tokens -- a context brief, not an essay.
const SUMMARY_MAX_TOKENS: u64 = 1_000;

/// Summarises the turns a compression pass dropped, hard-bounded in time.
///
/// This is the pass's third depth, reached only when elision and the turn drop
/// left the request above the configured target, and it is the only part of the
/// pass that can be *slow* -- which is why it exists at all as a setting. A local
/// model replies at a handful of tokens per second on a cold build, and a summary
/// that waits unbounded would park the reply that triggered the pass for the
/// duration of an engine warm-up. The timeout is the wall: the request is
/// cancelled there and the caller keeps the elided result.
///
/// A plain post, not the streaming loop: the reply is never shown while it is
/// being written, the endpoint is already known, and a one-shot call with a
/// hard timeout is smaller than the streaming machinery by an order of
/// magnitude.
async fn summarise_dropped(
    endpoint: &Endpoint,
    model: &str,
    dropped: &[(String, String)],
    timeout_secs: u64,
) -> Option<String> {
    if dropped.is_empty() {
        return None;
    }
    // The oldest turns come first (the pass reports them oldest first), and the
    // budget is spent on whatever fit -- a middle turn that will not fit whole
    // is clipped, because a cut-off tool log is still worth more in the summary
    // than its absence.
    let mut transcript = String::new();
    let mut budget = SUMMARY_MAX_CHARS;
    for (role, content) in dropped {
        if budget == 0 {
            break;
        }
        let mut piece = content.chars().take(SUMMARY_TURN_CHARS).collect::<String>();
        if content.chars().count() > piece.chars().count() {
            piece.push_str(" …[cut]… ");
        }
        let entry = format!("{role}: {piece}\n\n");
        if entry.len() >= budget {
            break;
        }
        budget -= entry.len();
        transcript.push_str(&entry);
    }
    if transcript.is_empty() {
        return None;
    }

    let client = http_client();
    // A bearer header when the endpoint has a key, nothing when it does not --
    // the same shape the streaming call uses for the same endpoint.
    let mut request = client
        .post(format!(
            "{}/chat/completions",
            endpoint.base_url.trim_end_matches('/')
        ))
        // Per request, since the pool is shared: a summary has its own budget.
        .timeout(std::time::Duration::from_secs(timeout_secs.max(1)))
        .json(&serde_json::json!({
            "model": model,
            "messages": [
                { "role": "system", "content": "You compress older parts of an ongoing conversation into a short factual summary for an assistant who never saw them: what was asked, what was found or done, what is decided or still open. Bullet points in the same language as the conversation. Never editorialise, never omit a user request or a decision." },
                { "role": "user", "content": transcript }
            ],
            "max_tokens": SUMMARY_MAX_TOKENS
        }));
    if !endpoint.api_key.is_empty() {
        request = request.bearer_auth(&endpoint.api_key);
    }
    let response = request.send().await.ok()?;
    if !response.status().is_success() {
        return None;
    }
    let value: Value = response.json().await.ok()?;
    value["choices"][0]["message"]["content"]
        .as_str()
        .map(str::trim)
        .filter(|content| !content.is_empty())
        .map(str::to_string)
}

/// Folds one round's reported usage into the reply's running figures.
///
/// The last-wins half is the context at the answer, which is what the context
/// ring and the per-reply chart plot, so a later round overwrites an earlier one.
/// The `used_*` totals are the exception: what a *reply spent* is the sum of every
/// round it made, because a tool round re-sends the whole conversation and that
/// second input is real spend the last-wins figure discards. Kept as one function
/// because the final wrap-up call needs exactly the same merge.
///
/// `context_tokens` is the exception to the exception: it is the **first** round's
/// input, recorded once and never overwritten. That is the size of the conversation
/// the reply started from -- which is the only figure compaction may be decided
/// from, because this reply's own tool rounds are never stored and so are not in
/// the next request. Deciding from the last round made a tool-heavy reply look like
/// a nearly-full window and trimmed a conversation that had not grown.
fn merge_round_usage(usage: &mut Value, round: &Value) {
    if usage.is_null() {
        *usage = round.clone();
        if let (Some(target), Some(start)) = (usage.as_object_mut(), round.get("prompt_tokens")) {
            target.insert("context_tokens".to_string(), start.clone());
        }
    } else if let (Some(target), Some(values)) = (usage.as_object_mut(), round.as_object()) {
        // Later rounds carry the final answer's throughput, so they win.
        for (key, value) in values {
            target.insert(key.clone(), value.clone());
        }
    }
    accumulate_usage(usage, round);
}

/// Folds one round's reported usage into the reply's running totals.
///
/// `usage` carries the last-wins figures a later round overwrites: `prompt_tokens`
/// is the context at the answer, which is what the context ring and the
/// per-reply chart want. `used_prompt_tokens` and `used_completion_tokens` are the
/// exception, because what a *reply spent* is the sum of every round it made -- a
/// tool round re-sends the whole conversation, and that second input is real spend
/// the last-wins figure discards. Kept under their own keys rather than folded into
/// `prompt_tokens`, so a value that means "how full the context was" is never
/// mistaken for one that means "how much was billed".
///
/// A round that reported nothing leaves the totals untouched, so a provider that
/// never sends usage yields no `used_*` keys at all rather than a misleading zero.
/// The frontend reads their absence as "unknown" and falls back to the
/// last-wins figure, which is exact for a reply that made a single request.
///
/// `requests` is counted here rather than read from the provider, because nobody
/// reports it: one call to this function *is* one request to the model, whatever
/// the body said. A reply that ran three tools made four requests, and that is
/// the number the statistics page shows.
fn accumulate_usage(usage: &mut Value, round: &Value) {
    let Some(target) = usage.as_object_mut() else {
        return;
    };
    for (reported, total) in [
        ("prompt_tokens", "used_prompt_tokens"),
        ("completion_tokens", "used_completion_tokens"),
    ] {
        if let Some(value) = round.get(reported).and_then(Value::as_u64) {
            let running = target.get(total).and_then(Value::as_u64).unwrap_or(0);
            target.insert(total.to_string(), json!(running + value));
        }
    }
    let requests = target.get("requests").and_then(Value::as_u64).unwrap_or(0);
    target.insert("requests".to_string(), json!(requests + 1));
}

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

        // Both kinds of tool, because this decides caching and invalidation. The
        // fallback is only reached for a name that was not in the registry, which
        // the parse step above has already refused.
        let effect = self.registry.effect_of(&call.name).unwrap_or(Effect::Read);

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
        // **A tool that panics is a failed call, not a dead app.** A tool runs
        // inside the reply's own task, so an unwinding panic here used to take the
        // whole window down mid-reply -- the user's "the app just restarted", and
        // the loss of everything the run had done. Catching it turns the bad call
        // into text the model can read and route around, and the loop carries on.
        //
        // `AssertUnwindSafe` is the honest wrapper: a tool's own state is not
        // promised to be intact after it panics, and the only thing read from it
        // afterwards is the message this produces. The panic itself is logged by
        // the app's hook (`lib.rs`).
        //
        // `succeeded` is carried out because the text alone cannot tell a failure
        // from a tool that legitimately returned those words.
        let outcome =
            std::panic::AssertUnwindSafe(self.registry.run(&call.name, arguments.clone(), context))
                .catch_unwind()
                .await;
        let (result, succeeded) = match outcome {
            Ok(Ok(text)) => (text, true),
            Ok(Err(error)) => (error, false),
            Err(_) => (
                format!(
                    "{} crashed while running -- a bug in the app, not in your call, and it has \
                     been logged. Try a different approach.",
                    call.name
                ),
                false,
            ),
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

/// How many times one request to the provider is attempted before its failure is
/// final.
///
/// One retry, and no more. The second attempt covers a transient fault -- a
/// gateway that dropped the connection, a provider that answered 502 to a request
/// it would have taken a moment later -- and past that a repeated failure is a
/// real one, so whoever is waiting is told rather than being made to sit through a
/// third and fourth try. The retry is applied to the *request*, never to the run:
/// a request that failed produced no tokens and changed nothing, so re-sending it
/// cannot duplicate output or replay a tool.
const REQUEST_ATTEMPTS: u32 = 2;

/// How long to wait before the retry, so a moment's fault has a chance to clear.
const RETRY_DELAY: std::time::Duration = std::time::Duration::from_millis(500);

/// One connection pool for every provider request in the app.
///
/// A `reqwest::Client` owns its connection pool, TLS state and DNS cache. Building
/// one per run -- and one more per internal completion -- meant opening a fresh
/// connection (and, for a remote provider, paying a full TLS handshake) before
/// every reply. The only per-call thing about the old clients was a timeout, and
/// that now rides the request. `connect_timeout` stays here because it is about the
/// pool, not about one call.
fn http_client() -> &'static reqwest::Client {
    static CLIENT: std::sync::OnceLock<reqwest::Client> = std::sync::OnceLock::new();
    CLIENT.get_or_init(|| {
        reqwest::Client::builder()
            .connect_timeout(std::time::Duration::from_secs(15))
            .build()
            .expect("a client with only a connect timeout cannot fail to build")
    })
}

/// Sends one chat request, retrying once before a failure is final.
///
/// **Only the request is retried, never a stream that has already begun.** This
/// returns the moment a response is accepted, so a failure here means no token was
/// emitted and nothing on the provider's side was changed beyond the attempt
/// itself -- re-sending is therefore safe. A stream that dies mid-flight is a
/// different situation: half an answer has already reached the conversation, so
/// [`stream_completion`], which owns that case, does not replay it.
///
/// The error is the same one [`explain_failure`] would have produced for the final
/// attempt, so what the user finally reads is unchanged by the retry being here.
/// Both the streaming path and the plain completion go through this, which is what
/// makes one retry apply to a reply, a scheduled job and a sub-agent alike.
async fn send_request(
    client: &reqwest::Client,
    endpoint: &Endpoint,
    body: &impl serde::Serialize,
    timeout: Option<std::time::Duration>,
) -> Result<reqwest::Response, String> {
    let url = crate::ai::remote::types::chat_url(&endpoint.base_url);
    let mut last_error = String::new();
    for attempt in 1..=REQUEST_ATTEMPTS {
        let mut request = client.post(&url);
        // Per request rather than on the client: the pool is shared, and a title
        // completion and a reply have no business sharing one budget.
        if let Some(timeout) = timeout {
            request = request.timeout(timeout);
        }
        let mut request = request.json(body);
        if !endpoint.api_key.trim().is_empty() {
            request = request.bearer_auth(&endpoint.api_key);
        }
        match request.send().await {
            Ok(response) if response.status().is_success() => return Ok(response),
            Ok(response) => {
                let status = response.status();
                let response_body = response.text().await.unwrap_or_default();
                last_error = explain_failure(status, &response_body);
                warn!(endpoint = %endpoint.name, attempt, %status, "provider rejected the request");
            }
            Err(error) => {
                last_error = format!("Request failed: {error}");
                warn!(endpoint = %endpoint.name, attempt, %error, "chat request failed");
            }
        }
        if attempt < REQUEST_ATTEMPTS {
            tokio::time::sleep(RETRY_DELAY).await;
        }
    }
    Err(last_error)
}

/// The streaming request, borrowed rather than copied.
///
/// The `json!` macro this replaced built an owned `Value` from `messages` and
/// `tools`, which meant a deep copy of the whole conversation and of the ~40 KB
/// tool array on **every round** of every run, then serialized the copy. A
/// borrowed struct is serialized straight to the wire, so a request costs one
/// serialize and no allocation proportional to the context.
#[derive(serde::Serialize)]
struct StreamBody<'a> {
    model: &'a str,
    messages: &'a [Value],
    stream: bool,
    stream_options: StreamOptions,
    #[serde(skip_serializing_if = "Option::is_none")]
    reasoning_effort: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    reasoning_control: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tools: Option<&'a Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_choice: Option<&'static str>,
}

#[derive(serde::Serialize)]
struct StreamOptions {
    include_usage: bool,
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
    // Passed through exactly as the provider spelled it. The field is only added
    // when there is a level to send, because a model that takes no reasoning
    // control rejects the field as firmly as it rejects a wrong value.
    let body = StreamBody {
        model,
        messages,
        stream: true,
        stream_options: StreamOptions { include_usage: true },
        reasoning_effort: reasoning.wire_value(),
        reasoning_control: enable_reasoning_control.then_some(true),
        tools,
        tool_choice: tools.map(|_| "auto"),
    };
    info!(run_id, endpoint = %endpoint.name, model, messages = messages.len(), "remote stream start");
    // The one retry lives in here, so a transient provider fault on any round of
    // any run -- a reply, a job, a sub-agent -- is absorbed before it becomes a
    // failure someone has to hear about.
    let response = send_request(client, endpoint, &body, None).await?;

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
/// prefix; only editing, adding or removing a skill folder rewrites the
/// catalogue, which is the honest cost of it being here at all.
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
    additional_instructions: Option<&str>,
    suppress_general_instructions: bool,
) -> Vec<Value> {
    // Only an Agent run touches the machine: its filesystem and terminal tools are
    // the ones that need to be told the shell and the workspace root. A Chat run
    // has only the web tools, which take URLs and never a path, so it gets no note.
    let machine_tools = mode == crate::ai::tools::ToolMode::Agent;
    let root = crate::ai::tools::workspace::root_string();
    let mut sections = Vec::new();
    // The user's own instructions, first and for **both** modes. They used to be
    // prepended by the chat view, which meant a run started anywhere else -- a job, a
    // message from a phone -- silently dropped them. The prompt is the app's, so it is
    // assembled here where every caller gets it.
    if !suppress_general_instructions {
        if let Some(instructions) = database.and_then(|database| {
            database
                .setting::<serde_json::Value>("app.general")
                .and_then(|general| {
                    general
                        .get("instructions")
                        .and_then(serde_json::Value::as_str)
                        .map(str::trim)
                        .filter(|text| !text.is_empty())
                        .map(str::to_string)
                })
        }) {
            sections.push(instructions);
        }
    }
    // The door's own rules -- a channel's plain-text note, its shared
    // instructions and the sender's profile. More specific than the general
    // instructions, and before the environment so a messaging run still knows
    // which machine and folder it is on.
    if let Some(additional) = additional_instructions
        .map(str::trim)
        .filter(|text| !text.is_empty())
    {
        sections.push(additional.to_string());
    }
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
        if let Some(catalogue) = crate::ai::tools::skills::index() {
            sections.push(catalogue);
        }
    }
    // What ProjectZ has learned about the user. Both modes get it: it is context
    // about the person, not a fact about the machine, and a Chat run is exactly
    // where a preference about tone or a known interest is useful.
    //
    // The block is a cached string, refreshed only when the profile changes (a run,
    // a switch, a reset). It must stay byte-identical between requests or the
    // provider's cached prefix is thrown away for no new information.
    if let Some(taste) = crate::taste::cached_prompt() {
        sections.push(taste);
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

/// The token figures a non-streaming reply reported.
///
/// Read from the response to one short internal request -- a title, a taste batch -- where
/// there is no stream to count events from. The plain [`complete_once`] drops them; taste
/// keeps them, because a learning run's spend is a figure the user can see.
///
/// Every field is an `Option` because "the provider did not say" and "the provider spent
/// nothing" are different facts, and the statistics page has to be able to tell them apart:
/// a default of zero would turn an unknown into a measurement.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct ReportedUsage {
    pub prompt_tokens: Option<u64>,
    pub completion_tokens: Option<u64>,
    pub cached_tokens: Option<u64>,
}

/// Whether the model may think before it answers.
///
/// **Not a preference.** Every caller wants thinking allowed -- a model that reasons is a better
/// model, and nothing here should take that away. The single exception is the second attempt of a
/// short completion whose first reply was cut off at the token ceiling *while the model was still
/// thinking*: repeating that request unchanged is repeating something that has already been shown
/// not to fit, so the retry is made without the thinking instead.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Thinking {
    Allow,
    /// The salvage attempt: thinking off, by both of the switches a server might have.
    Suppress,
}

/// Runs one non-streaming completion and returns what it said *and* what it cost.
///
/// The plain [`complete_once`] below is this with the figures dropped, so there is one
/// implementation of the request and one opinion about where a provider puts its usage.
pub async fn complete_once_with_usage(
    endpoint: &Endpoint,
    model: &str,
    messages: &[Value],
    max_tokens: Option<u32>,
) -> Result<(String, ReportedUsage), String> {
    let (content, usage, _) =
        complete_once_raw(endpoint, model, messages, max_tokens, Thinking::Allow).await?;
    Ok((content, usage))
}

/// The same request, with the provider's own response kept.
///
/// [`complete_once_with_usage`] keeps only the text and the figures, which is everything a
/// caller normally wants. This keeps the payload that text was read out of, for a caller
/// that has to explain a reply it could not use: `reasoning_content` is a *fallback* there,
/// so a provider that sent both loses its thinking. The request itself is identical, and
/// there is one implementation of it.
pub async fn complete_once_raw(
    endpoint: &Endpoint,
    model: &str,
    messages: &[Value],
    max_tokens: Option<u32>,
    thinking: Thinking,
) -> Result<(String, ReportedUsage, Value), String> {
    let client = http_client();
    // No `reasoning_effort` here on purpose. The field is not portable: one
    // provider expects `none` under a different key, another accepts `off` only
    // for models that can disable thinking, and a third rejects it outright.
    // There is no single value that works everywhere, and sending a wrong one
    // fails the whole request with HTTP 400. The title task is small and
    // `max_tokens` is what keeps it short, so the field is simply not sent.
    let mut body = json!({
        "model": model,
        "messages": messages,
        "stream": false,
        "max_tokens": max_tokens,
    });
    if thinking == Thinking::Suppress {
        // llama.cpp's own switch, for a template that has one (Qwen3's, MiniCPM's and others), and
        // sent **only** on a salvage attempt. The portability argument above does not apply here:
        // that request has already failed once, so a field this server does not know is either
        // ignored -- which is what an unknown field does everywhere -- or it fails a request that
        // was going to fail the same way again. Nothing is sent on the first attempt.
        body["chat_template_kwargs"] = json!({ "enable_thinking": false });
    }
    // Same single retry as the streaming path: a short internal completion can
    // meet a transient provider fault just as a reply can.
    let response = send_request(
        client,
        endpoint,
        &body,
        // The old per-call client carried this; the shared pool does not, so it
        // rides the request instead.
        Some(std::time::Duration::from_secs(60)),
    )
    .await?;
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
    // Read here rather than by each caller, so the one place that knows a provider's shape
    // is the one place that reads its figures -- including the cache token, which several
    // providers spell differently and which `stream_event` already knows how to find.
    let usage = ReportedUsage {
        prompt_tokens: payload
            .pointer("/usage/prompt_tokens")
            .and_then(Value::as_u64),
        completion_tokens: payload
            .pointer("/usage/completion_tokens")
            .and_then(Value::as_u64),
        cached_tokens: crate::ai::remote::stream_event::cached_tokens(&payload),
    };
    Ok((content, usage, payload))
}

/// Runs one non-streaming completion and returns only its text.
///
/// Used for short internal tasks such as generating a conversation title, where there is no
/// UI to stream into and no need for events or tools.
///
/// `max_tokens` is passed through because an uncapped request lets a reasoning model
/// deliberate at length before answering, and every token beyond the few words the task
/// needs is latency the caller waits for. It is an `Option` so a caller with no natural
/// limit can still omit it.
pub async fn complete_once(
    endpoint: &Endpoint,
    model: &str,
    messages: &[Value],
    max_tokens: Option<u32>,
) -> Result<String, String> {
    complete_once_with_usage(endpoint, model, messages, max_tokens)
        .await
        .map(|(content, _usage)| content)
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
