//! The sub-agents at work, and what each one has to say.
//!
//! # Why this is one record and not a flag
//!
//! A sub-agent used to be a cancel flag in a map and nothing else: the report
//! existed only as an event emitted at the end of the run, which the parent could
//! not read until its own turn was over. `subagent_wait` needs a place to *ask*,
//! so the same entry now carries the stop flag, the run's progress and, once it
//! ends, its outcome. One map rather than two, because every question here
//! ("is it still running", "can I stop it", "what did it say") is about one agent.
//!
//! # What the handshake is for
//!
//! The report must reach the model exactly once. It has two possible doors: a
//! hidden turn in the parent conversation, or the result of a `subagent_wait`
//! that was blocked when the run ended. [`AgentRecord::finish`] resolves that in
//! one lock: a wait already in progress ([`Waiting`]) claims the report, so
//! `run_agent` emits the event without the hidden turn. Deciding it at emit time
//! instead would race the waiter's wakeup, and the loser would cost the
//! conversation a whole extra reply.
//!
//! A poll made *after* the run finished cannot claim it -- the turn was already
//! queued -- so it gets the report as a tool result too. That is the rare case and
//! the cheap one: a few hundred tokens read twice, rather than no answer at all.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use tokio::sync::Notify;

/// How long a finished agent's record is kept.
///
/// A parent can reach for a result after the run ended -- the tool is what the
/// model uses *instead of* waiting on a later reply -- so the record must outlive
/// the task that made it. Ten minutes of that is plenty, and pruning on the next
/// spawn keeps a long session from accumulating every agent it ever started.
const KEEP_FINISHED: Duration = Duration::from_secs(10 * 60);

/// How often a wait re-checks a cancelled run.
///
/// A wait is parked on a notification, and the parent's own Stop sets a flag
/// rather than notifying: this bound is what keeps it from outliving a stop for
/// longer than one tick. One timer per waiting call, at 100 ms, is nothing beside
/// the model request it is waiting on.
const CANCEL_TICK: Duration = Duration::from_millis(100);

/// What a finished run leaves behind for its parent.
#[derive(Clone)]
pub struct Outcome {
    /// The text the parent receives: the answer, or the failure named.
    pub report: String,
    pub failed: bool,
}

/// How far along a running agent is, for a `peek` or a timed-out wait.
pub struct Progress {
    pub steps: usize,
    pub last_step: String,
    pub elapsed_seconds: u64,
}

/// What [`AgentRecord::wait_for`] came back with.
pub enum Finish {
    /// The run ended; this is its result.
    Ready(Outcome),
    /// Still working when the wait's own limit ran out.
    Running(Progress),
    /// The parent was stopped first. Nothing is accumulated for a reply that will
    /// never be sent, so this carries no progress.
    Stopped,
}

/// Everything known about one sub-agent.
pub struct AgentRecord {
    label: String,
    started: Instant,
    /// The run's stop flag. Shared with the task so `cancel` reaches it and the
    /// parent's own stop can be folded into it without a second map.
    cancelled: Arc<AtomicBool>,
    state: Mutex<State>,
    /// Wakes a waiting `subagent_wait` the moment the run ends.
    finished: Notify,
}

#[derive(Default)]
struct State {
    steps: usize,
    last_step: String,
    outcome: Option<Outcome>,
    finished_at: Option<Instant>,
    /// A wait is parked on this record, so the report is that call's to return.
    awaiting: bool,
    /// `run_agent` emitted the report as the conversation's own turn.
    delivered: bool,
}

impl AgentRecord {
    /// The name the user sees and the report is introduced with.
    pub fn label(&self) -> &str {
        &self.label
    }

    /// The stop flag, shared by clone with the run and its parent watcher.
    pub fn cancelled_flag(&self) -> Arc<AtomicBool> {
        Arc::clone(&self.cancelled)
    }

    /// Stops this agent alone.
    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::Relaxed);
    }

    /// Counts one finished call the agent made, and names it.
    ///
    /// Called from the run's own event sink, which is already on the hot path of
    /// every step, so it does no more than two writes behind one lock.
    pub fn note_step(&self, label: &str) {
        if let Ok(mut state) = self.state.lock() {
            state.steps += 1;
            if !label.is_empty() {
                state.last_step = label.to_string();
            }
        }
    }

    /// Records the end of the run and says whether a wait is holding it.
    ///
    /// `true` means a `subagent_wait` was parked and has taken ownership of the
    /// report, so `run_agent` must not also deliver it as a hidden turn. Decided
    /// under the same lock that sets the outcome, so the two cannot both claim it
    /// or both drop it.
    pub fn finish(&self, outcome: Outcome) -> bool {
        let awaited = {
            let Ok(mut state) = self.state.lock() else {
                // A poisoned lock means another holder panicked. Treat the report
                // as delivered: a lost hidden turn is worse than a duplicate.
                return false;
            };
            state.outcome = Some(outcome);
            state.finished_at = Some(Instant::now());
            state.delivered = !state.awaiting;
            state.awaiting
        };
        self.finished.notify_waiters();
        awaited
    }

    /// The result, if the run has ended.
    pub fn outcome(&self) -> Option<Outcome> {
        self.state
            .lock()
            .ok()
            .and_then(|state| state.outcome.clone())
    }

    /// Whether the report was handed to the conversation rather than to a waiter.
    ///
    /// Test-only: the decision is made once, in [`AgentRecord::finish`], and
    /// nothing in the app asks again.
    #[cfg(test)]
    pub fn delivered(&self) -> bool {
        self.state
            .lock()
            .map(|state| state.delivered)
            .unwrap_or(false)
    }

    /// Where the run is now, for a status answer.
    pub fn progress(&self) -> Progress {
        let (steps, last_step) = self
            .state
            .lock()
            .map(|state| (state.steps, state.last_step.clone()))
            .unwrap_or_default();
        Progress {
            steps,
            last_step,
            elapsed_seconds: self.started.elapsed().as_secs(),
        }
    }

    /// Parks until the run ends, the limit passes, or the parent is stopped.
    ///
    /// The [`Waiting`] guard is taken **before** the first outcome check and held
    /// across every loop turn, so `finish` always sees a waiter that was genuinely
    /// parked. The loop's own notification is an optimisation: a wakeup that lands
    /// between the state check and `notified()` is caught by the tick below, so a
    /// missed notification costs at most [`CANCEL_TICK`], never the whole wait.
    pub async fn wait_for(&self, limit: Duration, cancel: &AtomicBool) -> Finish {
        let _waiting = Waiting::new(self);
        let deadline = Instant::now() + limit;
        loop {
            if let Some(outcome) = self.outcome() {
                return Finish::Ready(outcome);
            }
            if cancel.load(Ordering::Relaxed) {
                return Finish::Stopped;
            }
            if Instant::now() >= deadline {
                return Finish::Running(self.progress());
            }
            tokio::select! {
                _ = self.finished.notified() => {}
                _ = tokio::time::sleep(CANCEL_TICK) => {}
            }
        }
    }
}

/// Claims the record's outcome for a parked wait, for as long as it is alive.
///
/// A guard rather than two calls because every return path in a wait -- finished,
/// timed out, stopped -- must clear the claim; one that forgot would make the next
/// `finish` suppress a hidden turn nobody was there to take.
struct Waiting<'a>(&'a AgentRecord);

impl<'a> Waiting<'a> {
    fn new(record: &'a AgentRecord) -> Self {
        if let Ok(mut state) = record.state.lock() {
            state.awaiting = true;
        }
        Self(record)
    }
}

impl Drop for Waiting<'_> {
    fn drop(&mut self) {
        if let Ok(mut state) = self.0.state.lock() {
            state.awaiting = false;
        }
    }
}

/// The records, keyed by agent id.
///
/// Process-wide for the same reason the flags it replaced were: an agent id is a
/// UUID minted at spawn, so no second value could name one, and the tool that asks
/// has no run map to read it from.
fn records() -> &'static Mutex<HashMap<String, Arc<AgentRecord>>> {
    static RECORDS: OnceLock<Mutex<HashMap<String, Arc<AgentRecord>>>> = OnceLock::new();
    RECORDS.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Starts the record for one agent, before a single token has been asked for.
pub fn register(id: &str, label: &str) -> Arc<AgentRecord> {
    prune();
    let record = Arc::new(AgentRecord {
        label: label.to_string(),
        started: Instant::now(),
        cancelled: Arc::new(AtomicBool::new(false)),
        state: Mutex::new(State::default()),
        finished: Notify::new(),
    });
    if let Ok(mut records) = records().lock() {
        records.insert(id.to_string(), Arc::clone(&record));
    }
    record
}

/// The record for one agent, if this process started it.
pub fn find(id: &str) -> Option<Arc<AgentRecord>> {
    records()
        .lock()
        .ok()
        .and_then(|records| records.get(id).cloned())
}

/// Stops one agent by its id. Returns whether a record was found.
///
/// A run that has already finished is not an error: the stop button may be pressed
/// in the moment the report lands.
pub fn cancel(id: &str) -> bool {
    match find(id) {
        Some(record) => {
            record.cancel();
            true
        }
        None => false,
    }
}

/// Drops records whose run ended long enough ago that nobody can still want them.
///
/// Only finished ones are considered, so a slow agent is never forgotten while it
/// works, and the map holds at most the agents of one session's recent history.
fn prune() {
    let Ok(mut records) = records().lock() else {
        return;
    };
    records.retain(|_, record| {
        let Ok(state) = record.state.lock() else {
            return false;
        };
        match state.finished_at {
            Some(at) => at.elapsed() < KEEP_FINISHED,
            None => true,
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn outcome(report: &str) -> Outcome {
        Outcome {
            report: report.to_string(),
            failed: false,
        }
    }

    const AGENT_ID: &str = "agent-test";

    fn record() -> Arc<AgentRecord> {
        register(AGENT_ID, "a test agent")
    }

    /// An agent that has not finished has nothing to hand over, and says where it is.
    #[test]
    fn a_running_agent_reports_progress_and_no_outcome() {
        let record = record();
        record.note_step("read_file");
        record.note_step("grep");
        assert!(record.outcome().is_none());
        let progress = record.progress();
        assert_eq!(progress.steps, 2);
        assert_eq!(progress.last_step, "grep");
    }

    /// A finished agent's report is available to anyone who asks afterwards.
    #[test]
    fn a_finished_agent_keeps_its_report() {
        let record = record();
        assert!(!record.finish(outcome("three cars")));
        let outcome = record.outcome().expect("kept");
        assert_eq!(outcome.report, "three cars");
        assert!(record.delivered());
    }

    /// The handshake: a wait parked when the run ends takes the report, so the run
    /// must not also emit a hidden turn for it. This is the rule that stops a
    /// delegated task costing the conversation a second whole reply.
    #[tokio::test]
    async fn a_parked_wait_takes_the_report_from_the_hidden_turn() {
        let record = record();
        let waiter = tokio::spawn({
            let record = Arc::clone(&record);
            async move {
                record
                    .wait_for(Duration::from_secs(5), &AtomicBool::new(false))
                    .await
            }
        });
        // Give the task a turn to park on the record before finishing.
        tokio::time::sleep(Duration::from_millis(20)).await;

        let awaited = record.finish(outcome("the answer"));
        assert!(awaited, "a parked wait must be recognised");
        assert!(!record.delivered(), "the hidden turn is suppressed");

        match waiter.await.expect("waiter") {
            Finish::Ready(outcome) => assert_eq!(outcome.report, "the answer"),
            _ => panic!("the wait should have finished"),
        }
    }

    /// A wait that outlives its own limit returns progress rather than blocking the
    /// reply indefinitely -- the model can then carry on or ask again.
    #[tokio::test]
    async fn a_wait_past_its_limit_returns_progress() {
        let record = record();
        match record
            .wait_for(Duration::from_millis(10), &AtomicBool::new(false))
            .await
        {
            Finish::Running(progress) => assert_eq!(progress.steps, 0),
            _ => panic!("the wait should have timed out"),
        }
    }

    /// A stop ends the wait at once; the run's own end is not invented for it.
    #[tokio::test]
    async fn a_stopped_parent_ends_the_wait() {
        let record = record();
        let cancel = AtomicBool::new(true);
        assert!(matches!(
            record.wait_for(Duration::from_secs(5), &cancel).await,
            Finish::Stopped
        ));
        assert!(record.outcome().is_none());
    }

    /// Stopping by id reaches the flag the run itself polls.
    #[test]
    fn cancelling_by_id_sets_the_runs_own_flag() {
        let record = record();
        let flag = record.cancelled_flag();
        assert!(cancel(AGENT_ID));
        assert!(flag.load(Ordering::Relaxed));
        assert!(!cancel("no-such-agent"));
    }
}
