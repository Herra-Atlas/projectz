//! One learning run: read what is new, ask, merge, record.
//!
//! A run is a sequence of independent batches rather than one request. That is
//! what makes a failure cheap: every batch that succeeded has already written its
//! items and advanced its watermarks, so the next attempt starts at the batch that
//! failed instead of at the beginning of the history.
//!
//! # Why the retry is here and not in the client
//!
//! `complete_once_raw` already resends a request once for a connection error or a
//! non-2xx. This retry is the other half: a reply that arrived but could not be
//! read. The second attempt suppresses thinking, because a model that reasons for
//! longer than the output budget produces a truncated object every time, and
//! asking it the same way again cannot help.

use std::collections::HashSet;

use serde_json::{json, Value};

use crate::ai::remote::client::{complete_once_raw, ReportedUsage, Thinking};
use crate::ai::remote::types::Endpoint;
use crate::ai::runtime::AiRuntime;
use crate::database::taste::NewTasteRun;
use crate::database::{utc_now, Database};

use super::{corpus, merge, ops, prompt, settings::TasteSettings};

/// Output ceiling for one batch.
///
/// Sized for a reply carrying a dozen operations plus a report, with room for a
/// model that thinks before it writes. The second attempt suppresses thinking, so
/// this does not have to carry both a long deliberation and a long answer.
const MAX_TOKENS: u32 = 2_000;

/// Longest report kept on a run. It is prose for a person debugging the feature,
/// not a second profile.
const MAX_REPORT_CHARS: usize = 4_000;

/// How one run ended.
#[derive(Clone, Debug)]
pub struct RunOutcome {
    /// `completed`, `skipped` or `failed`.
    pub status: &'static str,
    /// Whether retrying soon could reasonably help. A missing model is not
    /// transient; a provider that refused a connection is.
    pub transient: bool,
}

/// Why a model could not be used.
struct Resolved {
    endpoint: Endpoint,
    model: String,
    /// Whether the endpoint is the bundled local server. It changes how thinking
    /// is suppressed, and whether a model may be touched at all.
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

    let mut items = merge::normalize(database.taste_items()?);

    if corpus.batches.is_empty() {
        let document = merge::render(&items);
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

    let resolved = match resolve_model(runtime, settings) {
        Ok(resolved) => resolved,
        Err(reason) => {
            return record_failure(
                database,
                &started_at,
                None,
                None,
                reason,
                false,
                Progress::default(),
                ReportedUsage::default(),
            )
        }
    };

    let mut progress = Progress::default();
    let mut sessions = HashSet::new();
    let mut usage = ReportedUsage::default();
    let mut reports: Vec<String> = Vec::new();

    for (index, batch) in corpus.batches.iter().enumerate() {
        let messages = [
            json!({ "role": "system", "content": prompt::SYSTEM }),
            json!({
                "role": "user",
                "content": prompt::user_prompt(&items, batch, &now),
            }),
        ];

        match ask(&resolved, &messages).await {
            Ok((reply, batch_usage)) => {
                let outcome = merge::merge(&mut items, &reply, &now);
                // The profile is written before the watermarks: if the second write
                // fails, the next run re-reads the batch and merges it again rather
                // than losing it. Re-applying an operation is idempotent because an
                // `add` that restates an existing fact merges into it.
                database.replace_taste_items(&items)?;
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
                let report = reply.report.trim();
                if !report.is_empty() {
                    reports.push(report.to_string());
                }
            }
            Err(reason) => {
                // Keep the batches already learned and say exactly which one failed.
                // A retry next day starts from the unadvanced watermark, so nothing
                // is asked twice and nothing is skipped.
                progress.sessions = sessions.len() as i64;
                return record_failure(
                    database,
                    &started_at,
                    Some(&resolved),
                    Some(merge::render(&items)),
                    format!("Batch {} of {}: {reason}", index + 1, corpus.batches.len()),
                    true,
                    progress,
                    usage,
                );
            }
        }
    }

    progress.sessions = sessions.len() as i64;
    let document = merge::render(&items);
    let report = if reports.is_empty() {
        None
    } else {
        Some(clip_report(reports.join("\n\n")))
    };
    let reason = corpus.truncated.then(|| {
        "Some conversations were left for the next run because this one reached its limit."
            .to_string()
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

/// One batch's answer, with the shared retry ladder.
///
/// The second and later attempts suppress thinking. The first failure is often a
/// reply that spent its whole budget thinking and stopped mid-object; asking the
/// same question the same way would produce the same truncation, and a wait is
/// added between attempts for the same reason the chat page has one -- a provider
/// having a bad minute is worth a few seconds of patience.
async fn ask(
    resolved: &Resolved,
    messages: &[Value],
) -> Result<(ops::RawReply, ReportedUsage), String> {
    let mut last = "The model did not answer.".to_string();
    // Zero delay for the first attempt, then the shared ladder.
    let delays = std::iter::once(0u64).chain(crate::ai::RETRY_DELAYS_SECONDS);
    for (attempt, delay) in delays.enumerate() {
        if delay > 0 {
            tokio::time::sleep(std::time::Duration::from_secs(delay)).await;
        }
        let thinking = if resolved.local || attempt > 0 {
            Thinking::Suppress
        } else {
            Thinking::Allow
        };
        match complete_once_raw(
            &resolved.endpoint,
            &resolved.model,
            messages,
            Some(MAX_TOKENS),
            thinking,
        )
        .await
        {
            Ok((raw, usage, _payload)) => match ops::parse_reply(&raw) {
                Ok(reply) => return Ok((reply, usage)),
                Err(error) => last = error,
            },
            Err(error) => last = error,
        }
    }
    Err(last)
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
        let outcome = run(&database, &runtime, &TasteSettings::default())
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
        let outcome = run(&database, &runtime, &TasteSettings::default())
            .await
            .expect("run");
        assert_eq!(outcome.status, "skipped");
        let run = database.latest_taste_run().expect("latest").expect("row");
        assert_eq!(run.status, "skipped");
        assert!(run.model_id.is_none());
        assert_eq!(run.sessions_read, 0);
    }
}
