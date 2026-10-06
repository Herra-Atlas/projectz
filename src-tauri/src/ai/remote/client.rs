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
    mut on_event: impl FnMut(ChatEvent),
    mut on_search: impl FnMut(crate::websearch::WebSearchOutput),
) -> Result<(String, Value), String> {
    let client = reqwest::Client::builder()
        .connect_timeout(std::time::Duration::from_secs(15))
        .build()
        .map_err(|error| error.to_string())?;
    let payload = prepare_messages(messages, attachments)?;
    let registry = crate::ai::tools::registry_for(mode, web_search_enabled);

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

    let mut conversation = prepend_environment(payload, mode, web_search_enabled, database);
    let mut usage = Value::Null;
    // Tools are advertised on every round rather than only the first. A model
    // that finishes a turn after using a tool has still seen the schema, so
    // withholding it on round two buys nothing and makes the two rounds'
    // requests differ for no gain.
    let tool_specs = registry.specs();
    // Shared across every call in the run: one collector, drained by the loop.
    let sink = crate::ai::tools::Sink::default();
    for _ in 0..MAX_TOOL_ROUNDS {
        if cancelled.load(Ordering::Relaxed) {
            return Ok((String::new(), Value::Null));
        }
        emit_status(run_id, "thinking", &mut on_event);
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
            &mut on_event,
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
        // Sequential rather than concurrent. Tool results are appended in the
        // order the provider asked for them, which is what it expects to find;
        // and a model that requests three reads gains little from running them
        // at once, since it waits for all three either way.
        for (index, call) in tool_calls.iter().enumerate() {
            let arguments = match registry.parse_arguments(&call.name, &call.arguments) {
                Ok(arguments) => arguments,
                // A malformed call is reported back as that call's result rather
                // than ending the run, so the model can correct it itself.
                Err(error) => {
                    conversation.push(tool_message(&call.id, &error));
                    // Reported even though it never ran. A call the panel omits
                    // is a call the user cannot see the model attempt, and a
                    // silently-dropped malformed call looks exactly like a model
                    // that changed its mind.
                    on_event(tool_result_event(
                        run_id,
                        index,
                        call,
                        &crate::ai::tools::ui::summarize(&call.name, &Value::Null, &error, false),
                        // No execution means no measured time, so the row is
                        // reported as cached: the frontend treats a missing
                        // duration as zero and this must not read as "instant".
                        None,
                        true,
                        &error,
                    ));
                    continue;
                }
            };

            let effect = registry
                .get(&call.name)
                .map(|tool| tool.effect)
                .unwrap_or(Effect::Read);

            // A repeat of a call already made in this conversation is answered
            // from the cache when nothing has been written since. Saves both the
            // round trip and the tokens for a result the model already has.
            //
            // Emitted rather than skipped: the model did ask for it, and a panel
            // that silently omits a repeat would leave the user waiting for a
            // read that is never announced and then wonder why the model moved
            // on. Marked cached so it can be drawn differently from a call that
            // actually touched the disk.
            if let Some(cached) = cache
                .as_ref()
                .and_then(|cache| cache.get(session, &call.name, effect, &arguments))
            {
                conversation.push(tool_message(&call.id, &cached));
                on_event(tool_result_event(
                    run_id,
                    index,
                    call,
                    &crate::ai::tools::ui::summarize(&call.name, &arguments, &cached, true),
                    None,
                    true,
                    &cached,
                ));
                continue;
            }

            // Announced before the gate rather than after it, so a row exists
            // for a call the user is being asked to approve. The duration starts
            // here, which means waiting on a permission prompt is counted as the
            // call's time: that is time the user spent waiting, and hiding it
            // would report a tool as near-instant while a dialog sat open.
            emit_status(run_id, tool_status(&call.name), &mut on_event);
            on_event(tool_call_event(run_id, index, call, &arguments));
            let call_started = std::time::Instant::now();
            // The gate runs here, on the loop's behalf rather than the tool's. A
            // tool that forgot to check would still be checked, which is the only
            // arrangement that makes the gate worth having.
            let decision = approval
                .settle(
                    run_id,
                    ApprovalRequest {
                        run_id: String::new(),
                        tool_name: call.name.clone(),
                        // The tool's own declared command field. Empty for every
                        // tool that does not run a shell, which is how the policy
                        // knows it is not looking at a command line.
                        command: registry.dangerous_string(&call.name, &arguments),
                        arguments: arguments.clone(),
                    },
                    &cancelled,
                    |request, approval_id| {
                        on_event(ChatEvent {
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
                // The user stopped the reply while the prompt was open. Ending
                // quietly is what stop means -- but returning `Ok` with empty
                // content would report a *completed* zero-length answer, which
                // the frontend stores as an assistant message. A stop has to be
                // distinguishable from a reply that produced nothing.
                Err(error) => {
                    tracing::debug!(error = %error, "approval ended before it was answered");
                    return Err(crate::ai::STOPPED.to_string());
                }
            };
            if let Decision::Deny(reason) = decision {
                // Refused and declined both become the tool's result text, so
                // the model can carry on rather than losing the conversation.
                conversation.push(tool_message(&call.id, &reason));
                // A refusal is reported as a refused row rather than nothing.
                // The user declined it, so showing it as attempted-and-cancelled
                // would misreport their own decision as the model's.
                on_event(tool_result_event(
                    run_id,
                    index,
                    call,
                    &crate::ai::tools::ui::summarize(&call.name, &arguments, &reason, false),
                    Some(call_started.elapsed().as_secs_f64()),
                    true,
                    &reason,
                ));
                continue;
            }

            // One context per call, sharing the collector. A tool's own failure
            // becomes its result text: the model is told what went wrong and
            // gets another turn to fix it.
            let context = crate::ai::tools::ToolContext {
                cancelled: Arc::clone(&cancelled),
                sink: sink.clone(),
                // The same handle the run's tool cache is scoped to, so a tool
                // reading stored state and the cache keyed on it cannot disagree
                // about which database this conversation is using.
                database: database.cloned(),
            };
            // `succeeded` is carried out because the text alone cannot tell a
            // failure from a tool that legitimately returned those words.
            let (result, succeeded) =
                match registry.run(&call.name, arguments.clone(), context).await {
                    Ok(text) => (text, true),
                    Err(error) => (error, false),
                };
            // The diff is drained here, per call, because this is the only place
            // that knows which write produced it. Draining per round would hand
            // two writes' changes to one row and show the wrong file's diff
            // against the wrong path.
            let mut summary =
                crate::ai::tools::ui::summarize(&call.name, &arguments, &result, succeeded);
            if let Some(diff) = sink.drain_diffs().into_iter().next() {
                summary.attach_diff(diff);
            }
            // Announced whether or not it worked. A tool that failed still ran,
            // and the row is what shows the user why before the model decides
            // what to do about it.
            on_event(tool_result_event(
                run_id,
                index,
                call,
                &summary,
                Some(call_started.elapsed().as_secs_f64()),
                false,
                &result,
            ));
            // A failure is never cached. The model is expected to retry with
            // corrected arguments, and a cached error would answer that retry
            // with the same failure every time.
            if succeeded {
                if let Some(cache) = &cache {
                    cache.put(session, &call.name, effect, &arguments, &result);
                }
            }
            conversation.push(tool_message(&call.id, &result));

            // Anything the tool reported is drained here, where the run's own
            // callback is still borrowed and in scope.
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
    on_event: &mut impl FnMut(ChatEvent),
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
        return Err(if response_body.is_empty() {
            format!("HTTP {status}")
        } else {
            format!("HTTP {status}: {response_body}")
        });
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
/// sent to. And nothing is prepended at all when the run has no tools, so a
/// Chat-mode request keeps byte-identical prefix across every turn -- which is
/// exactly the invariant the prompt-caching notes protect, and paying for it with
/// a note about a filesystem the run cannot touch would be a poor trade.
///
/// **One leading message, not two.** Everything the model needs before the
/// conversation has to arrive as a single `system` turn: llama.cpp's templates
/// reject a `system` message that is not first ("System message must be at the
/// beginning"), so a second one partway down is not portable. The environment note
/// and the skill catalogue are therefore one block joined by a blank line.
///
/// **The skill catalogue does not change when a skill is applied.** A skill the
/// user checks in the composer travels inline with that message instead, so
/// picking one moves no part of this prefix. Only editing or disabling a skill
/// changes it, which is the honest cost of the catalogue being here at all.
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
    web_search_enabled: bool,
    database: Option<&Arc<crate::database::Database>>,
) -> Vec<Value> {
    let has_tools = !crate::ai::tools::registry_for(mode, web_search_enabled)
        .names()
        .is_empty();
    let root = crate::ai::tools::workspace::root_string();
    let mut sections = Vec::new();
    if let Some(note) = crate::ai::prompts::chat::environment_note(
        has_tools,
        (!root.is_empty()).then_some(root.as_str()),
    ) {
        sections.push(note);
    }
    // The skill catalogue rides in the same leading block rather than getting a
    // message of its own: llama.cpp rejects a `system` turn that is not first, so
    // everything the model needs before the conversation has to be one message.
    // Only sent when tools exist, because `skill_read` is what makes it actionable
    // -- in Chat mode there is nothing to read it with.
    if has_tools {
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
        return Err(if response_body.is_empty() {
            format!("HTTP {status}")
        } else {
            format!("HTTP {status}: {response_body}")
        });
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

fn emit_status(run_id: &str, status: &str, on_event: &mut impl FnMut(ChatEvent)) {
    on_event(ChatEvent {
        run_id: run_id.to_string(),
        session_id: None,
        sequence: 0,
        kind: "status".into(),
        text: Some(status.into()),
        error: None,
        metrics: None,
    });
}
