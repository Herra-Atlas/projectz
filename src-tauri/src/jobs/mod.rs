//! Running saved jobs on their own.
//!
//! # The shape of the loop
//!
//! One task wakes on a fixed tick and asks the same three questions of every job
//! that is due: *may nobody else be doing this* (its lane), *is what it depends on
//! done*, and *is the machine free*. Only when all three answer yes does a run
//! start, and even then not immediately -- see the confirm step below.
//!
//! # Why a job is confirmed before it starts
//!
//! A single "the machine looks free" reading is a coincidence: RAM dips between a
//! browser tab closing and the next one opening, and a job that took that moment as
//! permission would start into the spike that followed. So a passing sample is only
//! a *candidate*: the job's next run is pushed out by [`CONFIRM`] and, if the
//! machine still looks free when that arrives, it starts. One calm reading proves
//! nothing; two, a couple of minutes apart, are worth acting on. A blocked reading
//! resets the candidate, so a near-miss does not leave the job primed.
//!
//! # Why stopping is a flag and not a kill
//!
//! A run that has exceeded a limit is cancelled the same way the Stop button
//! cancels one: the flag is set, the current step finishes, and the loop stops
//! before the next. There is no separate "abort" path for jobs, which is what keeps
//! a stopped job's transcript and tool history intact. The limit must be exceeded
//! for [`SUSTAINED`] -- two ticks in a row -- so one spike does not end a run.
//!
//! # Why lanes rather than a worker pool
//!
//! The user's question was which jobs may overlap. A lane is the answer: jobs that
//! name the same lane run one at a time, jobs in different lanes do not wait for
//! each other. Everything defaults to one lane, so the out-of-the-box behaviour is
//! "one at a time", and overlap is something a job asks for.
//!
//! A lane is also where the local-model rule lives without a special case: a local
//! job defaults to the shared lane, and the runner would serialise behind a single
//! `llama-server` anyway (`parallel` is 1).

mod conditions;
mod runner;
mod schedule;

/// Re-exported for the command layer, which reports total memory so the editor can
/// show a percentage as the bytes it means.
pub use conditions::ram_total_bytes;
/// Re-exported so the command layer decides a job's first run with the same rules
/// the scheduler uses for its later ones.
pub use schedule::{first_run, stamp};

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use chrono::{DateTime, Utc};
use tauri::{AppHandle, Emitter};

use crate::ai::runtime::AiRuntime;
use crate::database::jobs::Job;

/// How often the world is looked at. Cheap: two syscalls and one indexed query.
const TICK: Duration = Duration::from_secs(15);
/// How long a job waits between a first passing sample and the confirming one.
const CONFIRM: Duration = Duration::from_secs(150);
/// How long a blocked job waits before it is looked at again.
const RETRY: Duration = Duration::from_secs(180);
/// How long a limit must stay exceeded before a running job yields.
const SUSTAINED: Duration = Duration::from_secs(20);
/// How long a run is immune to the resource check at all.
///
/// Loading a local model and starting work raises RAM and VRAM by its own doing,
/// so a job judged against a limit in its first seconds would be stopped by the
/// very memory it needs -- "it just ends right away". The grace covers the ramp;
/// after it, the limit means what it says, and the run has still had to hold over
/// the limit for `SUSTAINED` on top of that.
const STOP_GRACE: Duration = Duration::from_secs(120);
/// How old a passing sample may be and still count as the second opinion.
///
/// Longer than [`CONFIRM`] on purpose: the job deferred by the confirm delay has
/// to find its own earlier reading still inside this window, or it would wait
/// forever. It also means a job that comes due while the machine has been calm for
/// a few minutes starts at once, because the evidence is already on record.
const RECENT: Duration = Duration::from_secs(240);

/// Everything the scheduler remembers between ticks.
///
/// In memory rather than in the database because none of it survives a restart
/// usefully: a confirmation is about the last two minutes, and a pause is about
/// this session. Losing them costs one extra confirm cycle at most.
#[derive(Default)]
struct State {
    /// Lane name -> the job currently occupying it.
    lanes: Mutex<HashMap<String, String>>,
    /// Job id -> the chat run id, so a run can be cancelled.
    running: Mutex<HashMap<String, String>>,
    /// Jobs that have passed one sample and await the confirming one.
    last_pass: Mutex<HashMap<String, DateTime<Utc>>>,
    /// Job id -> when its running job first exceeded a limit.
    over_since: Mutex<HashMap<String, DateTime<Utc>>>,
    /// Job id -> when its run started, so the resource grace can be measured.
    started: Mutex<HashMap<String, DateTime<Utc>>>,
    /// Whether the whole scheduler is switched off.
    paused: AtomicBool,
    /// Wakes the loop early when something has changed under it.
    wake: tokio::sync::Notify,
}

/// The scheduler, as the app holds it.
///
/// Cloned into the tick task and registered as state so a command can pause it.
#[derive(Clone)]
pub struct Scheduler {
    state: Arc<State>,
}

impl Scheduler {
    /// Starts the tick loop and returns the handle the app manages.
    pub fn start(app: AppHandle, runtime: AiRuntime) -> Self {
        let state = Arc::new(State::default());
        let scheduler = Self {
            state: Arc::clone(&state),
        };
        let shared = Arc::clone(&state);
        tauri::async_runtime::spawn(async move {
            // The first look is one tick in, not at launch: the app is still
            // opening its database and reading settings, and a job that fired
            // into that would be racing its own setup.
            loop {
                // The next tick, or something finishing -- whichever is first. A run
                // that ends is the moment another job may become startable, and
                // leaving the queued one to wait out the rest of the interval is
                // exactly the "it should have started straight away" gap.
                tokio::select! {
                    _ = tokio::time::sleep(TICK) => {}
                    _ = shared.wake.notified() => {}
                }
                if shared.paused.load(Ordering::Relaxed) {
                    continue;
                }
                tick(&app, &runtime, &shared);
            }
        });
        scheduler
    }

    /// Turns the whole scheduler off or back on.
    pub fn set_paused(&self, paused: bool) {
        self.state.paused.store(paused, Ordering::Relaxed);
    }

    pub fn is_paused(&self) -> bool {
        self.state.paused.load(Ordering::Relaxed)
    }

    /// The jobs running right now, for the header chip.
    pub fn running_jobs(&self) -> Vec<String> {
        self.state
            .running
            .lock()
            .map(|running| running.keys().cloned().collect())
            .unwrap_or_default()
    }

    /// Starts a job now, for the "Run now" button.
    ///
    /// Skips the clock *and* the conditions -- the user is asking for it at this
    /// moment, and the two-sample confirm exists to stop the app starting work
    /// nobody asked for, which is not what this is. The lane is still respected:
    /// starting here must not put two runs in one lane at once.
    pub fn run_now(&self, app: &AppHandle, runtime: &AiRuntime, job_id: &str) -> Result<(), String> {
        let job = runtime
            .database()
            .job(job_id)?
            .ok_or_else(|| format!("No job with id {job_id}"))?;
        if self.running_jobs().iter().any(|id| id == job_id) {
            return Err("That job is already running".to_string());
        }
        let lane_busy = self
            .state
            .lanes
            .lock()
            .map(|lanes| lanes.contains_key(&job.lane))
            .unwrap_or(true);
        if lane_busy {
            return Err(format!(
                "Another job is already running in the `{}` lane",
                job.lane
            ));
        }
        start(app, runtime, &self.state, job);
        Ok(())
    }
}

/// One pass: stop what must stop, then start what may start.
fn tick(app: &AppHandle, runtime: &AiRuntime, state: &Arc<State>) {
    let now = Utc::now();
    let machine = conditions::sample();

    stop_busy_runs(app, runtime, state, &machine, now);

    let due = match runtime.database().due_jobs(&schedule::stamp(now)) {
        Ok(jobs) => jobs,
        Err(error) => {
            tracing::warn!(%error, "the scheduler could not read its jobs");
            return;
        }
    };

    for job in due {
        if state
            .running
            .lock()
            .map(|running| running.contains_key(&job.id))
            .unwrap_or(true)
        {
            continue;
        }
        // The lane is busy with another job. Left due, so the next tick tries
        // again -- no bookkeeping needed for "queued".
        if state
            .lanes
            .lock()
            .map(|lanes| lanes.contains_key(&job.lane))
            .unwrap_or(true)
        {
            continue;
        }
        if let Some(reason) = unmet_dependency(runtime, &job) {
            // Not deferred. A job put back by three minutes for a dependency that
            // clears a second later sits idle with nothing to explain it -- and the
            // wait costs nothing to re-check, since the row is only read once a tick.
            emit(app, &job, "waiting", Some(&reason));
            continue;
        }
        if let Some(reason) = conditions::start_blocked_by(&machine, &job.conditions) {
            // A blocked reading is not evidence of calm, so the record is cleared:
            // the confirming sample has to be two unbroken calm readings, not two
            // out of three.
            if let Ok(mut pass) = state.last_pass.lock() {
                pass.remove(&job.id);
            }
            defer(runtime, &job, now);
            emit(app, &job, "waiting", Some(&reason));
            continue;
        }

        // A provider model starts instantly and costs this machine nothing, so the
        // confirming pass is skipped for it: that delay exists to let a local model's
        // own memory use settle, and a cloud job never touches it.
        let local = matches!(job.model, crate::database::jobs::JobModel::Local { .. });
        let confirmed = !local
            || state
                .last_pass
                .lock()
                .map(|pass| {
                    pass.get(&job.id)
                        .is_some_and(|at| (now - *at).num_seconds() <= RECENT.as_secs() as i64)
                })
                .unwrap_or(false);
        if !confirmed {
            defer_by(runtime, &job, now, CONFIRM);
            emit(
                app,
                &job,
                "confirming",
                Some("the machine looks free; checking again shortly"),
            );
            continue;
        }

        start(app, runtime, state, job);
    }

    // Which jobs the machine is clear for, refreshed once per tick. Recorded for
    // every enabled job rather than only the ones that are due, so a job whose time
    // arrives while the machine is already calm has that evidence waiting.
    for job in runtime.database().list_jobs().unwrap_or_default() {
        if !job.enabled {
            continue;
        }
        let clear = conditions::start_blocked_by(&machine, &job.conditions).is_none();
        if let Ok(mut pass) = state.last_pass.lock() {
            if clear {
                pass.insert(job.id.clone(), now);
            } else {
                pass.remove(&job.id);
            }
        }
    }
}

/// Ends a run whose machine has been over a limit for long enough.
fn stop_busy_runs(
    app: &AppHandle,
    runtime: &AiRuntime,
    state: &Arc<State>,
    machine: &conditions::MachineState,
    now: DateTime<Utc>,
) {
    let running: Vec<(String, String)> = state
        .running
        .lock()
        .map(|running| {
            running
                .iter()
                .map(|(job, run)| (job.clone(), run.clone()))
                .collect()
        })
        .unwrap_or_default();

    for (job_id, chat_run_id) in running {
        let job = match runtime.database().job(&job_id) {
            Ok(Some(job)) => job,
            // Deleted mid-run: nothing to consult about thresholds.
            _ => continue,
        };
        if !job.conditions.stop_when_busy {
            if let Ok(mut over) = state.over_since.lock() {
                over.remove(&job_id);
            }
            continue;
        }
        // Inside the grace the run is not judged at all: the memory it is using is
        // the memory it needs to be running.
        let in_grace = state
            .started
            .lock()
            .ok()
            .and_then(|started| started.get(&job_id).copied())
            .is_some_and(|started| (now - started).num_seconds() < STOP_GRACE.as_secs() as i64);
        if in_grace {
            continue;
        }
        let Some(reason) = conditions::over_limit(machine, &job.conditions) else {
            if let Ok(mut over) = state.over_since.lock() {
                over.remove(&job_id);
            }
            continue;
        };
        let sustained = {
            let Ok(mut over) = state.over_since.lock() else {
                continue;
            };
            let since = *over.entry(job_id.clone()).or_insert(now);
            (now - since).num_seconds() >= SUSTAINED.as_secs() as i64
        };
        if !sustained {
            continue;
        }
        tracing::info!(job = %job_id, %reason, "stopping a job: the machine is busy");
        runtime.cancel_chat(&chat_run_id);
        if let Ok(mut over) = state.over_since.lock() {
            over.remove(&job_id);
        }
        emit(app, &job, "stopping", Some(&reason));
    }
}

/// Starts one job and hands the work to its own task.
fn start(app: &AppHandle, runtime: &AiRuntime, state: &Arc<State>, job: Job) {
    let chat_run_id = format!("job:{}:{}", job.id, uuid::Uuid::new_v4());
    let run_row = runtime.database().start_job_run(&job.id, None).ok();
    // Claimed before the task is spawned, so the next tick cannot start a second
    // run in the same lane while this one is still coming up.
    if let Ok(mut lanes) = state.lanes.lock() {
        lanes.insert(job.lane.clone(), job.id.clone());
    }
    if let Ok(mut running) = state.running.lock() {
        running.insert(job.id.clone(), chat_run_id.clone());
    }
    if let Ok(mut started) = state.started.lock() {
        started.insert(job.id.clone(), Utc::now());
    }
    emit(app, &job, "running", None);
    tracing::info!(job = %job.id, %chat_run_id, "starting a scheduled job");

    let app = app.clone();
    let runtime = runtime.clone();
    let state = Arc::clone(state);
    tauri::async_runtime::spawn(async move {
        let outcome = runner::run(&app, &runtime, &job, &chat_run_id).await;
        if let Ok(mut lanes) = state.lanes.lock() {
            lanes.remove(&job.lane);
        }
        if let Ok(mut running) = state.running.lock() {
            running.remove(&job.id);
        }
        if let Ok(mut over) = state.over_since.lock() {
            over.remove(&job.id);
        }
        if let Ok(mut started) = state.started.lock() {
            started.remove(&job.id);
        }
        // The next time comes from the schedule, so the interval is measured from
        // the end of the last attempt rather than from a time it may have missed.
        let next = schedule::next_run(&job.schedule, Utc::now());
        if let Err(error) =
            runtime
                .database()
                .touch_job(&job.id, outcome.status, next.as_deref())
        {
            tracing::warn!(job = %job.id, %error, "could not record a job's next run");
        }
        if let Some(run_row) = run_row {
            let _ = runtime.database().finish_job_run(
                &run_row,
                outcome.status,
                outcome.reason.as_deref(),
                Some(&outcome.session_id),
            );
        }
        emit(&app, &job, outcome.status, outcome.reason.as_deref());
        // Woken last, after the outcome is on record: a job behind this one checks
        // whether it succeeded, and a look taken before the write would find nothing
        // and cost it another interval.
        state.wake.notify_one();
    });
}

/// The first dependency that has never completed, as a reason sentence.
fn unmet_dependency(runtime: &AiRuntime, job: &Job) -> Option<String> {
    for dependency in &job.depends_on {
        match runtime.database().last_success_at(dependency) {
            Ok(Some(_)) => {}
            Ok(None) => {
                let name = runtime
                    .database()
                    .job(dependency)
                    .ok()
                    .flatten()
                    .map(|job| job.name)
                    .unwrap_or_else(|| dependency.clone());
                return Some(format!("waiting for `{name}` to finish once"));
            }
            Err(error) => return Some(format!("could not check whether it may run: {error}")),
        }
    }
    None
}

/// Pushes a job's next run out by the retry delay, without claiming it ran.
fn defer(runtime: &AiRuntime, job: &Job, now: DateTime<Utc>) {
    defer_by(runtime, job, now, RETRY);
}

/// Pushes a job's next run out by a given delay.
fn defer_by(runtime: &AiRuntime, job: &Job, now: DateTime<Utc>, after: Duration) {
    let next = schedule::stamp(now + chrono::Duration::seconds(after.as_secs() as i64));
    if let Err(error) = runtime.database().set_job_next_run(&job.id, Some(&next)) {
        tracing::warn!(job = %job.id, %error, "could not defer a job");
    }
}

/// Tells the window what a job is doing.
fn emit(app: &AppHandle, job: &Job, status: &str, reason: Option<&str>) {
    let _ = app.emit(
        "job-event",
        serde_json::json!({
            "jobId": job.id,
            "name": job.name,
            "status": status,
            "reason": reason,
        }),
    );
}
