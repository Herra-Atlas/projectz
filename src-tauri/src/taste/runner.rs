//! One learning run: read what is new, ask, merge, record.
//!
//! A run is a sequence of independent batches rather than one request. That is
//! what makes a failure cheap: every batch that succeeded has already written its
//! items and advanced its watermarks, so the next attempt starts at the batch that
//! failed instead of at the beginning of the history.
//!
//! # Why the retry is short
//!
//! `complete_once_raw` already resends a request once for a connection error or a
//! non-2xx. This retry is the other half: a reply that arrived but could not be
//! read in the one-line contract. A second attempt is worth it, with a reminder of
//! the shape appended. A long ladder is not: a bad reply from a small model is
//! usually a format slip, not a provider having a bad minute, and waiting fifteen
//! seconds between four attempts is what made a run feel slow.

use std::collections::HashSet;

use serde_json::{json, Value};

use crate::ai::remote::client::{complete_once_raw, ReportedUsage, Thinking};
use crate::ai::remote::types::Endpoint;
use crate::ai::runtime::AiRuntime;
use crate::database::taste::NewTasteRun;
use crate::database::{utc_now, Database};

use super::{corpus, merge, prompt, reply, settings::TasteSettings};

/// Output ceiling for one batch.
///
/// Eight facts are about 150 tokens. **A ceiling is headroom, not spend**: a
/// model that answers in four lines still stops at the end of its answer. It costs
/// something only when the provider spends the budget before writing, which is
/// exactly the free/reasoning case that failed at 300 and again at 900.
const MAX_TOKENS: u32 = 1_500;

/// Output ceiling for the retry.
///
/// A provider that ignores the thinking switch has spent the first budget before
/// it reached a fact; the retry gets enough room to finish thinking and still
/// write four lines. Nothing is paid unless the retry happens.
const RETRY_TOKENS: u32 = 2_500;

/// Attempts at one batch, including the first.
const ATTEMPTS: usize = 2;

/// Seconds before the second attempt.
///
/// The shared chat ladder waits 1, 3, 5 and 7 seconds between attempts. A format
/// retry does not need that: the request is small, the second attempt carries a
/// reminder, and a long wait is the difference between a run finishing now and
/// finishing after the user has looked away.
const RETRY_WAIT_SECONDS: u64 = 2;

/// Longest report kept on a run. The learner writes a one-line summary now, so
/// this only bounds a provider's failure message.
const MAX_REPORT_CHARS: usize = 1_000;

/// How one run ended.
#[derive(Clone, Debug)]
pub struct RunOutcome {
    /// `completed`, `skipped` or `failed`.
    pub status: &'static str,
    /// Whether retrying soon could reasonably help. A missing model is not
    /// transient; a provider that refused a connection is.
    pub transient: bool,
}

/// The model a run will use.
struct Resolved {
    endpoint: Endpoint,
    model: String,
    /// Whether the endpoint is the bundled local server.
    ///
    /// It decides how thinking is suppressed: the local switch is portable and
    /// sent on every attempt, while a remote provider only gets it on the salvage
    /// attempt (see `complete_once_raw`).
    local: bool,
}

/// Runs one learning pass.
///
/// Every expected failure is recorded as a failed run rather than returned as an
/// error: the user opens the Taste page to see why it last did nothing, and an
/// error that only reached a log would be invisible there. `Err` is reserved for a
/// database failure, which means the record itself could not be written.
pub async fn run(
    database: &Database,
    runtime: &AiRuntime,
    settings: &TasteSettings,
    manual: bool,
    on_event: &(dyn Fn(&str) + Sync),
) -> Result<RunOutcome, String> {
    let started_at = utc_now();
    let now = started_at.clone();

    // **Read the history before resolving a model.** A run with nothing unread is
    // a successful no-op, not a failure: it must not need a configured model, and
    // it must not spend a request finding out there is nothing to say. This is
    // also what makes toggling the feature on after everything has been read a
    // quiet "nothing new" rather than an error about a missing model.
    let corpus = corpus::build(database)?;

    // Empty turns cost nothing to be done with, and not advancing them would scan
    // the same rows on every run.
    for (session, ordinal) in &corpus.empty_sessions {
        database.set_taste_watermark(session, *ordinal)?;
    }

    if corpus.batches.is_empty() {
        on_event("Nothing new to read.");
        let document = merge::render(&merge::normalize(database.taste_items()?));
        database.record_taste_run(&NewTasteRun {
            started_at,
            status: "skipped".to_string(),
            reason: Some("No new conversations to learn from.".to_string()),
            model_id: None,
            provider_id: None,
            sessions_read: 0,
            messages_read: 0,
            prompt_tokens: None,
            completion_tokens: None,
            added: 0,
            updated: 0,
            removed: 0,
            report: None,
            document: Some(document),
        })?;
        return Ok(RunOutcome {
            status: "skipped",
            transient: false,
        });
    }

    on_event(&format!(
        "Read {} conversations · {} turns · {} batch{}.",
        corpus.sessions,
        corpus.messages,
        corpus.batches.len(),
        plural(corpus.batches.len() as i64)
    ));

    let resolved = match resolve_model(runtime, settings) {
        Ok(resolved) => resolved,
        Err(reason) => {
            on_event(&format!("Failed: {reason}"));
            return record_failure(
                database,
                &started_at,
                None,
                None,
                reason,
                false,
                Progress::default(),
                ReportedUsage::default(),
            );
        }
    };
    on_event(&format!("Using {}.", resolved.model));

    let mut progress = Progress::default();
    let mut sessions = HashSet::new();
    let mut usage = ReportedUsage::default();
    let mut stopped_reason: Option<String> = None;

    for (index, batch) in corpus.batches.iter().enumerate() {
        // A run is a few requests, not one; the user may switch the feature off
        // between them. A manual run ignores the switch -- it is a preview the
        // settings page asked for -- but an automatic one stops paying for work
        // the user just disabled.
        if index > 0 && !manual && !TasteSettings::load(database).enabled {
            stopped_reason = Some("Taste was switched off during the run.".to_string());
            break;
        }
        // **Read the profile again per batch.** A run waits on a model, and the
        // settings page lets the user edit or delete a fact while it waits. A
        // snapshot taken before the first request would write those changes back
        // as if they had never happened.
        let items = merge::normalize(database.taste_items()?);
        let messages = [
            json!({ "role": "system", "content": prompt::SYSTEM }),
            json!({
                "role": "user",
                "content": prompt::user_prompt(&items, batch),
            }),
        ];

        on_event(&format!(
            "Batch {} of {} · reading {} turns.",
            index + 1,
            corpus.batches.len(),
            batch.messages
        ));
        match ask(&resolved, &messages, on_event).await {
            Ok((answer, batch_usage, partial)) => {
                // Read once more immediately before the write: this is the same
                // reason, narrowed to the window between the request and the merge.
                let mut latest = merge::normalize(database.taste_items()?);
                let outcome = merge::merge(&mut latest, &answer, &now);
                // The profile is written before the watermarks: if the second write
                // fails, the next run re-reads the batch and merges it again rather
                // than losing it. Re-applying a fact is idempotent because a line
                // that restates an existing fact merges into it.
                database.replace_taste_items(&latest)?;
                for (session, ordinal) in batch.watermarks() {
                    database.set_taste_watermark(&session, ordinal)?;
                }
                for session in &batch.sessions {
                    sessions.insert(session.id.clone());
                }
                progress.messages += batch.messages as i64;
                progress.added += outcome.added as i64;
                progress.updated += outcome.updated as i64;
                progress.removed += outcome.removed as i64;
                usage = add_usage(usage, batch_usage);
                on_event(&format!(
                    "Batch {} · {} added, {} reinforced{}.",
                    index + 1,
                    outcome.added,
                    outcome.updated,
                    if partial {
                        " (reply was cut off; kept the complete facts)"
                    } else {
                        ""
                    }
                ));
            }
            Err(reason) => {
                // Keep the batches already learned and say exactly which one failed.
                // A retry next day starts from the unadvanced watermark, so nothing
                // is asked twice and nothing is skipped.
                on_event(&format!("Batch {} failed: {reason}", index + 1));
                progress.sessions = sessions.len() as i64;
                return record_failure(
                    database,
                    &started_at,
                    Some(&resolved),
                    Some(merge::render(&merge::normalize(database.taste_items()?))),
                    format!("Batch {} of {}: {reason}", index + 1, corpus.batches.len()),
                    true,
                    progress,
                    usage,
                );
            }
        }
    }

    progress.sessions = sessions.len() as i64;
    let document = merge::render(&merge::normalize(database.taste_items()?));
    // Written from the counts rather than asked of the model: a report line is
    // output tokens and one more thing a small model can get wrong, and "3 added,
    // 1 reinforced" is the whole of what a person needs from it.
    let report = Some(describe(&progress));
    on_event(&match &stopped_reason {
        Some(name) => format!("Stopped: {name}"),
        None => format!(
            "Done · {} added, {} reinforced.",
            progress.added, progress.updated
        ),
    });
    let reason = stopped_reason.or_else(|| {
        corpus.truncated.then(|| {
            "Some conversations were left for the next run because this one reached its limit."
                .to_string()
        })
    });
    database.record_taste_run(&NewTasteRun {
        started_at,
        status: "completed".to_string(),
        reason: reason.clone(),
        model_id: Some(resolved.model),
        provider_id: Some(resolved.endpoint.id),
        sessions_read: progress.sessions,
        messages_read: progress.messages,
        prompt_tokens: usage.prompt_tokens.map(|tokens| tokens as i64),
        completion_tokens: usage.completion_tokens.map(|tokens| tokens as i64),
        added: progress.added,
        updated: progress.updated,
        removed: progress.removed,
        report,
        document: Some(document),
    })?;

    Ok(RunOutcome {
        status: "completed",
        transient: false,
    })
}

/// What a run changed so far. Copied into a failed row so partial work is visible.
#[derive(Default, Clone, Copy)]
struct Progress {
    sessions: i64,
    messages: i64,
    added: i64,
    updated: i64,
    removed: i64,
}

/// Writes a failed run and returns it as the outcome.
#[allow(clippy::too_many_arguments)]
fn record_failure(
    database: &Database,
    started_at: &str,
    resolved: Option<&Resolved>,
    document: Option<String>,
    reason: String,
    transient: bool,
    progress: Progress,
    usage: ReportedUsage,
) -> Result<RunOutcome, String> {
    let roll_up = |value: Option<u64>| value.map(|tokens| tokens as i64);
    database.record_taste_run(&NewTasteRun {
        started_at: started_at.to_string(),
        status: "failed".to_string(),
        reason: Some(clip_report(reason.clone())),
        model_id: resolved.map(|resolved| resolved.model.clone()),
        provider_id: resolved.map(|resolved| resolved.endpoint.id.clone()),
        sessions_read: progress.sessions,
        messages_read: progress.messages,
        prompt_tokens: roll_up(usage.prompt_tokens),
        completion_tokens: roll_up(usage.completion_tokens),
        added: progress.added,
        updated: progress.updated,
        removed: progress.removed,
        report: None,
        document,
    })?;
    Ok(RunOutcome {
        status: "failed",
        transient,
    })
}

/// Picks the first configured model that can actually answer.
///
/// A local selection is held to the same rule as every background task: it must
/// be the model that is already loaded. Loading one would evict the model the user
/// is chatting with, which is a far worse outcome than a learning run that waits.
fn resolve_model(runtime: &AiRuntime, settings: &TasteSettings) -> Result<Resolved, String> {
    if !settings.has_model() {
        return Err("No taste model is selected.".to_string());
    }

    let mut last = "No taste model is available.".to_string();
    for selection in &settings.models {
        let local_id = selection
            .get("localModelId")
            .and_then(Value::as_str)
            .filter(|id| !id.trim().is_empty());
        if let Some(wanted) = local_id {
            match runtime.local_models().status().loaded_model_id.as_deref() {
                Some(loaded) if loaded == wanted => {
                    if let Some((endpoint, model)) = runtime.local_models().loaded_endpoint() {
                        return Ok(Resolved {
                            endpoint,
                            model,
                            local: true,
                        });
                    }
                    last = "The local taste model is still starting.".to_string();
                }
                Some(_) => {
                    last = "A different local model is loaded. Taste only uses a model that is \
                            already running, because loading one would unload the model your \
                            chat is using."
                        .to_string();
                }
                None => {
                    last = "The local taste model is not loaded. Load it in Settings > Local \
                            first; taste never loads or unloads a model itself."
                        .to_string();
                }
            }
            continue;
        }

        match runtime.resolve_selection(selection) {
            Ok((endpoint, model)) => {
                return Ok(Resolved {
                    endpoint,
                    model,
                    local: false,
                })
            }
            Err(error) => last = error,
        }
    }
    Err(last)
}

/// One batch's answer, with one format retry.
///
/// **Thinking is suppressed on the local model and on every retry.** Extraction is
/// a short classification task; a model that deliberates over it spends seconds and
/// output tokens and returns the same lines. A remote first attempt is left alone
/// because the suppression field is not portable (see `complete_once_raw`).
///
/// The second attempt appends the shape as a reminder and asks for fewer lines:
/// if the model answered in prose -- or spent the budget thinking -- asking it
/// again identically is asking it to fail identically.
///
/// **A cut-off reply is not thrown away.** A free provider that stops at the token
/// ceiling usually did write facts before it stopped; the complete `FACT |
/// CONFIDENCE` lines are kept, the batch is marked partial, and the run continues
/// to the next batch instead of failing outright.
async fn ask(
    resolved: &Resolved,
    messages: &[Value],
    on_event: &(dyn Fn(&str) + Sync),
) -> Result<(reply::RawReply, ReportedUsage, bool), String> {
    let mut last = "The model did not answer.".to_string();
    for attempt in 0..ATTEMPTS {
        if attempt > 0 {
            tokio::time::sleep(std::time::Duration::from_secs(RETRY_WAIT_SECONDS)).await;
        }
        let attempt_messages = reminded(messages, attempt > 0);
        // Suppression is portable on the bundled local server and only a salvage
        // switch for a remote one -- `complete_once_raw` explains why sending it
        // on a first remote request is not safe.
        let thinking = if resolved.local || attempt > 0 {
            Thinking::Suppress
        } else {
            Thinking::Allow
        };
        match complete_once_raw(
            &resolved.endpoint,
            &resolved.model,
            &attempt_messages,
            Some(if attempt == 0 {
                MAX_TOKENS
            } else {
                RETRY_TOKENS
            }),
            thinking,
        )
        .await
        {
            Ok((raw, usage, payload)) => {
                let cut_off = truncated(&payload);
                match reply::parse_reply(&raw) {
                    // Complete lines from a cut-off reply are still facts the model
                    // did write. Keeping them is what stops a free provider from
                    // failing every batch; the flag makes the run say so.
                    Ok(parsed) if cut_off && !parsed.facts.is_empty() => {
                        return Ok((parsed, usage, true))
                    }
                    Ok(parsed) if !cut_off => return Ok((parsed, usage, false)),
                    Ok(_) => {
                        last = "The reply was cut off at the output limit.".to_string();
                        if attempt + 1 < ATTEMPTS {
                            on_event(
                                "Reply was cut off before any fact; asking again with a larger budget.",
                            );
                        }
                    }
                    Err(error) => {
                        last = error;
                        if cut_off {
                            last = "The reply was cut off at the output limit.".to_string();
                            if attempt + 1 < ATTEMPTS {
                                on_event("Reply was cut off; asking again with fewer lines.");
                            }
                        } else if attempt + 1 < ATTEMPTS {
                            on_event(
                                "Reply did not use the FACT | CONFIDENCE shape; asking again.",
                            );
                        }
                    }
                }
            }
            Err(error) => {
                last = error;
                if attempt + 1 < ATTEMPTS {
                    on_event("The model did not answer; asking again.");
                }
            }
        }
    }
    Err(last)
}

/// Whether the provider stopped the reply at the token ceiling.
fn truncated(payload: &Value) -> bool {
    let finish = payload
        .get("choices")
        .and_then(Value::as_array)
        .and_then(|choices| choices.first())
        .and_then(|choice| choice.get("finish_reason"))
        .and_then(Value::as_str);
    matches!(finish, Some("length") | Some("max_tokens"))
}

/// The request, with the answer shape appended on a retry.
fn reminded(messages: &[Value], retry: bool) -> Vec<Value> {
    let mut copy = messages.to_vec();
    if !retry {
        return copy;
    }
    if let Some(last) = copy.last_mut() {
        let text = last
            .get("content")
            .and_then(Value::as_str)
            .unwrap_or_default();
        // Fewer lines as well as the right shape: a truncated reply is the other
        // way a first attempt fails, and asking for the same eight would cut it
        // off in the same place.
        last["content"] = json!(format!(
            "{text}\n\nDo not think or explain. Return only FACT | CONFIDENCE lines, \
             or NONE. At most 4 lines, each under 15 words."
        ));
    }
    copy
}

/// What a finished run did, in one line.
fn describe(progress: &Progress) -> String {
    match (progress.added, progress.updated) {
        (0, 0) => "Nothing new was learned.".to_string(),
        (added, 0) => format!("{added} fact{} added.", plural(added)),
        (0, updated) => format!("{updated} existing fact{} reinforced.", plural(updated)),
        (added, updated) => format!("{added} fact{} added, {updated} reinforced.", plural(added)),
    }
}

fn plural(count: i64) -> &'static str {
    if count == 1 {
        ""
    } else {
        "s"
    }
}

/// Adds two attempts' figures without turning "unknown" into zero.
fn add_usage(left: ReportedUsage, right: ReportedUsage) -> ReportedUsage {
    let sum = |a: Option<u64>, b: Option<u64>| match (a, b) {
        (Some(a), Some(b)) => Some(a.saturating_add(b)),
        (Some(value), None) | (None, Some(value)) => Some(value),
        (None, None) => None,
    };
    ReportedUsage {
        prompt_tokens: sum(left.prompt_tokens, right.prompt_tokens),
        completion_tokens: sum(left.completion_tokens, right.completion_tokens),
        cached_tokens: sum(left.cached_tokens, right.cached_tokens),
    }
}

fn clip_report(report: String) -> String {
    if report.chars().count() <= MAX_REPORT_CHARS {
        return report;
    }
    let mut clipped: String = report.chars().take(MAX_REPORT_CHARS - 1).collect();
    clipped.push('…');
    clipped
}

#[cfg(test)]
mod tests {
    use super::*;

    fn runtime(label: &str) -> crate::ai::runtime::AiRuntime {
        crate::ai::runtime::AiRuntime::new_with_database(
            std::env::temp_dir(),
            std::sync::Arc::new(
                Database::open(std::env::temp_dir().join(format!(
                    "projectz-taste-test-{label}-{}.sqlite3",
                    std::process::id()
                )))
                .expect("runtime database"),
            ),
        )
    }

    fn seed_session(database: &Database) {
        database
            .save_chat_session(&serde_json::json!({
                "id": "s1",
                "title": "A conversation",
                "createdAt": "2026-01-02T00:00:00.000Z",
                "updatedAt": "2026-01-02T00:00:00.000Z",
                "messages": [
                    { "role": "user", "content": "I use Rust (programming language) for backend services." },
                ],
            }))
            .expect("seed");
    }

    /// A run with unread history and no configured model is a failure, and it is
    /// not transient: waiting a day cannot select a model the user never chose.
    #[tokio::test]
    async fn a_run_with_history_and_no_model_is_a_non_transient_failure() {
        let database = Database::in_memory().expect("database");
        let runtime = runtime("no-model");
        seed_session(&database);
        let outcome = run(
            &database,
            &runtime,
            &TasteSettings::default(),
            false,
            &|_line| {},
        )
        .await
        .expect("run");
        assert_eq!(outcome.status, "failed");
        assert!(!outcome.transient);
        let run = database.latest_taste_run().expect("latest").expect("row");
        assert!(run
            .reason
            .as_deref()
            .is_some_and(|reason| reason.contains("No taste model")));
        assert_eq!(run.status, "failed");
    }

    /// Nothing unread is a successful no-op even with no model selected. The
    /// history is checked before the model, so a quiet week does not produce an
    /// error about a setting that has nothing to do with it.
    #[tokio::test]
    async fn a_run_with_nothing_new_is_skipped_without_a_model() {
        let database = Database::in_memory().expect("database");
        let runtime = runtime("nothing-new");
        let outcome = run(
            &database,
            &runtime,
            &TasteSettings::default(),
            false,
            &|_line| {},
        )
        .await
        .expect("run");
        assert_eq!(outcome.status, "skipped");
        let run = database.latest_taste_run().expect("latest").expect("row");
        assert_eq!(run.status, "skipped");
        assert!(run.model_id.is_none());
        assert_eq!(run.sessions_read, 0);
    }
}
