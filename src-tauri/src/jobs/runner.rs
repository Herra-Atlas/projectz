//! Running one job, once.
//!
//! # A job run is an ordinary run
//!
//! The scheduler does not have its own model loop. It builds a [`ChatRequest`] and
//! hands it to `AiRuntime::run_chat`, so a job's run goes through the same tool
//! registry, the same permission gate, the same prompt prefix and the same event
//! stream as a reply the user typed. The two things that differ are the two the
//! design calls for: the request carries the job's `access` set, and nobody is
//! watching, so the permission level is whatever the job stored rather than a
//! level read from the composer.
//!
//! # Why the local model is started here
//!
//! A job on a local model cannot assume the engine is up -- the app may have just
//! launched, or the user unloaded it hours ago. So the run starts it and waits,
//! bounded, for the server to answer. A load that never completes is a failure with
//! a reason, not a run that hangs until the app is closed.
//!
//! # What the run writes
//!
//! The transcript is saved as a normal session with `kind: "job"`, so it appears in
//! history and opens in the chat view like any other conversation. The completion
//! and its reason are recorded on the job's `job_runs` row by the caller; this
//! returns them.

use std::time::Duration;

use chrono::Utc;
use tauri::AppHandle;

use super::schedule;
use crate::ai::runtime::AiRuntime;
use crate::ai::types::ChatMessage;
use crate::database::jobs::{Job, JobModel};

/// How long to wait for a local engine to come up before giving up.
const LOCAL_LOAD_LIMIT: Duration = Duration::from_secs(300);
/// How often that wait re-checks.
const LOCAL_LOAD_POLL: Duration = Duration::from_secs(2);
/// How soon another job must be due for the model to be kept loaded.
///
/// Covers the scheduler's confirm delay with room to spare, so a job whose turn
/// comes next still finds the model resident.
const KEEP_LOADED: Duration = Duration::from_secs(240);

/// How a run ended.
pub struct Outcome {
    /// `completed`, `failed` or `stopped`.
    pub status: &'static str,
    /// Why, when it is not simply "completed".
    pub reason: Option<String>,
    /// The session the run was saved as.
    pub session_id: String,
    /// The agent's answer, so a channel that asked for the job can be sent it
    /// rather than only a status line.
    pub content: String,
}

/// Runs one job to completion.
///
/// `run_id` is the chat run id, provided by the caller so it can cancel this run
/// when the machine gets busy.
pub async fn run(app: &AppHandle, runtime: &AiRuntime, job: &Job, run_id: &str) -> Outcome {
    let session_id = uuid::Uuid::new_v4().to_string();
    let started_at = crate::database::utc_now();

    // A job may name the folder it works in, and the user asked for that. The tools
    // resolve against a process-global root, so the job's folder is put in place for
    // the run and the previous one restored when it ends -- including on every early
    // return, which is why the run itself is a separate function.
    //
    // **The caveat is real.** There is one root and not one per run, so a reply the
    // user sends *while a job is running* resolves against the job's folder too.
    // Jobs are normally scheduled for when the machine is idle, which is why this
    // is acceptable for now; making the root per-run means threading it through
    // every tool.
    let previous_root = crate::ai::tools::workspace::root();
    if let Some(path) = job
        .workspace
        .as_deref()
        .map(str::trim)
        .filter(|path| !path.is_empty())
    {
        if let Err(error) = crate::ai::tools::workspace::set_root(std::path::Path::new(path)) {
            return Outcome {
                status: "failed",
                reason: Some(format!("workspace: {error}")),
                session_id,
                content: String::new(),
            };
        }
    }

    let outcome = execute(app, runtime, job, run_id, &session_id, &started_at).await;
    match previous_root {
        Some(root) => {
            let _ = crate::ai::tools::workspace::set_root(&root);
        }
        None => crate::ai::tools::workspace::clear_root(),
    }
    outcome
}

/// The run itself, with the workspace already in place.
async fn execute(
    app: &AppHandle,
    runtime: &AiRuntime,
    job: &Job,
    run_id: &str,
    session_id: &str,
    started_at: &str,
) -> Outcome {
    let (endpoint_id, local_model, model, provider_label) = match &job.model {
        JobModel::Local { id } => {
            let loaded_here = match ensure_local_loaded(app, runtime, id).await {
                Ok(loaded) => loaded,
                Err(reason) => {
                    return Outcome {
                        status: "failed",
                        reason: Some(reason),
                        session_id: session_id.to_string(),
                        content: String::new(),
                    };
                }
            };
            if loaded_here {
                // Attributed to the job even though the user may later keep using
                // it: what decides the release is whether the *interface* asked for
                // this model, and so far it has not.
                runtime.local_models().mark_loaded_for_job();
            }
            // The local path ignores the model name: the runtime uses the loaded
            // server's own alias, which is what keep the two from disagreeing.
            (None, true, None, id.clone())
        }
        JobModel::Remote { endpoint_id, model } => (
            Some(endpoint_id.clone()),
            false,
            Some(model.clone()),
            model.clone(),
        ),
    };

    // The request, the event collection and the transcript write live in
    // `ai::run`, shared with the messaging channels: a scheduled job and a message
    // from a phone are the same run to the model, and a second copy here is a
    // second place for the two to drift.
    let result = crate::ai::run::run_prompt(
        app,
        runtime,
        crate::ai::run::PromptRun {
            run_id: run_id.to_string(),
            session_id: session_id.to_string(),
            title: job.name.clone(),
            // `job` rather than `chat`: the sidebar still lists it -- only sub-agents
            // are hidden -- but the distinction is there for a filter to use later.
            kind: "job".to_string(),
            messages: vec![ChatMessage {
                role: "user".to_string(),
                content: job.prompt.clone(),
            }],
            // A job starts a conversation rather than continuing one, so there is
            // no stored record to preserve.
            stored_messages: Vec::new(),
            // A job's own run carries no channel files or prompt overlay. Its
            // completion is delivered through the target remembered when it was
            // scheduled, not through this run.
            attachments: Vec::new(),
            additional_instructions: None,
            suppress_general_instructions: false,
            delivery: None,
            endpoint_id,
            local_model,
            model,
            provider_label,
            mode: job.mode,
            permission: job.permission,
            // The job's own access set, with scheduling taken off it. Enforced here rather
            // than trusting how the job was created: a job made from the Jobs screen can
            // have every capability switched on, and it still must not schedule more work.
            // One generation, whatever the row says.
            access: crate::ai::tools::AccessSet {
                jobs: false,
                ..job.access
            },
            // Web search is on exactly when the job may use it, so a run that cannot
            // search is not handed a catalogue of searches it will be refused.
            web_search: job.access.web,
            created_at: started_at.to_string(),
            on_event: None,
        },
    )
    .await;

    let outcome = match result.error {
        Some(reason) => Outcome {
            status: "failed",
            reason: Some(reason),
            session_id: session_id.to_string(),
            content: String::new(),
        },
        None => Outcome {
            status: "completed",
            reason: None,
            session_id: session_id.to_string(),
            content: result.content,
        },
    };
    // Asked after every run rather than only local ones: a job on a provider model
    // can be the last thing to finish while a local model sits loaded from an
    // earlier job, and that is exactly the case where it should be let go.
    release_local_model(runtime);
    outcome
}

/// Makes sure the named local model is the one loaded.
///
/// Returns whether **this call** started it. That is the difference between "the
/// job needed a model and got one" and "a model the user already had loaded",
/// which is what decides whether the run may unload it afterwards.
async fn ensure_local_loaded(
    app: &AppHandle,
    runtime: &AiRuntime,
    id: &str,
) -> Result<bool, String> {
    if runtime.local_models().status().loaded_model_id.as_deref() == Some(id) {
        return Ok(false);
    }
    let manager = runtime.local_models();
    manager.load(app.clone(), id.to_string())?;
    let deadline = tokio::time::Instant::now() + LOCAL_LOAD_LIMIT;
    loop {
        if runtime.local_models().status().loaded_model_id.as_deref() == Some(id) {
            return Ok(true);
        }
        if tokio::time::Instant::now() >= deadline {
            return Err(format!("local model {id} did not finish loading in time"));
        }
        // A load that has already failed leaves neither figure set, which is the
        // difference between "still coming up" and "not going to".
        let status = runtime.local_models().status();
        if status.loading_model_id.is_none() && status.loaded_model_id.is_none() {
            return Err(format!("local model {id} could not be loaded"));
        }
        tokio::time::sleep(LOCAL_LOAD_POLL).await;
    }
}

/// Frees the GPU after a run, unless another job is about to want the same model.
///
/// A scheduled run is usually the only reason a model is loaded, and leaving it
/// resident holds memory the user asked to have back. A job that is due in a minute
/// or two is the exception: it should not pay a full load again, so the model stays
/// while another enabled job on that model is close behind.
///
/// Two things are read from the *manager* rather than from this run. Which model is
/// loaded, because the job that ends is often not the job that loaded it -- two jobs
/// on one model is exactly the case that used to leak it -- and whether a job loaded
/// it at all, because a model the user asked for from the interface is theirs to
/// keep.
fn release_local_model(runtime: &AiRuntime) {
    let manager = runtime.local_models();
    if !manager.loaded_for_job() {
        return;
    }
    let Some(loaded) = manager.status().loaded_model_id else {
        return;
    };
    let soon =
        schedule::stamp(Utc::now() + chrono::Duration::seconds(KEEP_LOADED.as_secs() as i64));
    let followed = runtime
        .database()
        .list_jobs()
        .unwrap_or_default()
        .into_iter()
        .any(|job| {
            job.enabled
                && matches!(&job.model, JobModel::Local { id } if *id == loaded)
                && job
                    .next_run_at
                    .as_deref()
                    .is_some_and(|at| at <= soon.as_str())
        });
    if followed {
        tracing::info!(model = %loaded, "keeping the local model: another job is next");
        return;
    }
    tracing::info!(model = %loaded, "unloading the local model: no job is waiting for it");
    manager.unload();
}
