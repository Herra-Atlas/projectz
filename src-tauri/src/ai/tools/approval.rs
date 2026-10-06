//! Suspends a run while the user decides.
//!
//! The pure decision lives in [`super::policy`]. This file is the part that
//! cannot be tested without a UI: it emits a prompt, waits on a channel, and
//! wakes up when the user answers.
//!
//! # How the run waits
//!
//! [`Approval::wait`] parks the tool loop on a oneshot channel until the frontend
//! calls `ai_answer_approval`. It selects on the run's cancel flag at the same
//! time, so a user who stops the reply while a prompt is open is not left with a
//! dialog nobody will answer.
//!
//! Denials and timeouts are reported back to the model as the tool's result text
//! rather than failing the run. A model told "the user declined this" can carry
//! on and do something else, which is more useful than losing the conversation.

use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};

use tokio::sync::oneshot;

use super::policy::{Decision, PermissionMode};

/// What the user was asked, and what they said.
#[derive(Clone, Debug, serde::Serialize)]
pub struct ApprovalRequest {
    pub run_id: String,
    pub tool_name: String,
    /// The dangerous string for this call: a command line, or empty.
    pub command: String,
    /// Rendered as JSON for the frontend to show beside the command.
    pub arguments: serde_json::Value,
}

impl ApprovalRequest {
    /// The one-line summary shown in the prompt.
    ///
    /// Built here rather than in the frontend so a permission dialog and a log
    /// line always describe the same call the same way.
    pub fn summary(&self) -> String {
        if self.command.trim().is_empty() {
            self.tool_name.clone()
        } else {
            format!("{}: {}", self.tool_name, self.command.trim())
        }
    }
}

/// A decision the user made.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Approval {
    Allow,
    Deny,
}

/// Outstanding prompts, keyed by the tool call waiting on them.
///
/// Handles the approval side of a run. One map rather than a channel per prompt
/// because the answer arrives as a separate Tauri command that knows only the
/// call's id. Entries are removed on completion, and a prompt left unanswered by a
/// cancelled run is dropped when the run's gate is dropped.
#[derive(Clone)]
pub struct ApprovalGate {
    /// Shared rather than copied, so [`ApprovalGate::with_mode`] can re-point it.
    mode: Arc<std::sync::Mutex<PermissionMode>>,
    pending: Arc<std::sync::Mutex<std::collections::HashMap<String, oneshot::Sender<Approval>>>>,
}

impl ApprovalGate {
    pub fn new(mode: PermissionMode) -> Self {
        Self {
            mode: Arc::new(std::sync::Mutex::new(mode)),
            pending: Arc::new(std::sync::Mutex::new(std::collections::HashMap::new())),
        }
    }

    /// This gate with a different mode, **sharing the same pending map**.
    ///
    /// The sharing is the whole point. A caller that needs the current mode at
    /// send time cannot build a second gate, because the answer to a prompt
    /// arrives through a separate command holding only an id -- and a second
    /// instance would have an empty map, so the answer would find nothing and the
    /// run would park forever on a prompt the user had already answered. Re-
    /// pointing the mode in place is why this exists.
    pub fn with_mode(&self, mode: PermissionMode) -> Self {
        if let Ok(mut current) = self.mode.lock() {
            *current = mode;
        }
        self.clone()
    }

    /// The mode currently in force.
    ///
    /// Defaults to `Ask` when the lock is poisoned: a poisoned lock must not
    /// become a way to escalate to running commands without asking.
    pub fn mode(&self) -> PermissionMode {
        self.mode.lock().map(|mode| *mode).unwrap_or_default()
    }

    /// Records the user's answer and releases the waiting run.
    ///
    /// Returns `false` when nothing was waiting on this id, which happens when
    /// the run was stopped while the dialog was open. Reported rather than
    /// panicked on, because a stale answer from a closed dialog is a normal
    /// outcome and not a bug.
    pub fn answer(&self, approval_id: &str, decision: Approval) -> bool {
        let sender = self
            .pending
            .lock()
            .ok()
            .and_then(|mut pending| pending.remove(approval_id));
        match sender {
            Some(sender) => sender.send(decision).is_ok(),
            None => false,
        }
    }

    /// Settles one call according to the mode, waiting if the user must decide.
    ///
    /// `on_prompt` is called with the request when a decision is needed, and is
    /// where the run's event sink emits it to the frontend.
    pub async fn settle(
        &self,
        run_id: &str,
        request: ApprovalRequest,
        cancelled: &Arc<AtomicBool>,
        on_prompt: impl FnOnce(&ApprovalRequest, &str),
    ) -> Result<Decision, String> {
        // The run id is stamped here rather than trusted from the caller, because
        // it is what the frontend keys its dialog on and two sessions can each
        // have a call waiting at once.
        let request = ApprovalRequest {
            run_id: run_id.to_string(),
            ..request
        };
        match super::policy::decide(self.mode(), &request.tool_name) {
            Decision::Allow => Ok(Decision::Allow),
            Decision::Deny(reason) => Ok(Decision::Deny(reason)),
            Decision::Ask => {
                let approval_id = uuid::Uuid::new_v4().to_string();
                let (sender, receiver) = oneshot::channel();
                // Registered before the prompt is emitted, so an answer cannot
                // arrive for an id the map has not heard of.
                let registered =
                    self.pending
                        .lock()
                        .map_err(|error| error.to_string())
                        .map(|mut pending| {
                            pending.insert(approval_id.clone(), sender);
                        });
                if registered.is_err() {
                    return Err("The approval queue is poisoned".into());
                }

                on_prompt(&request, &approval_id);

                let answer = tokio::select! {
                    received = receiver => received.map_err(|_| {
                        // The gate was dropped without answering, which happens
                        // when the run ends. Reported as a denial so the loop
                        // moves on rather than hanging.
                        "The request was cancelled before it was answered".to_string()
                    })?,
                    _ = wait_until_cancelled(cancelled) => {
                        return Err("The user stopped this reply".to_string());
                    }
                };
                self.answer(&approval_id, answer);
                Ok(match answer {
                    Approval::Allow => Decision::Allow,
                    Approval::Deny => Decision::Deny("The user declined this call".into()),
                })
            }
        }
    }
}

/// Resolves once the run has been cancelled.
///
/// Shared rather than private to the gate: a long tool needs to abandon its own
/// work the same way, and a second copy of this loop would be a second place for
/// the two to disagree.
pub(crate) async fn wait_until_cancelled(cancelled: &Arc<AtomicBool>) {
    while !cancelled.load(Ordering::Relaxed) {
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A call that reaches the permission prompt.
    ///
    /// Deliberately not `rm`: a refused command never reaches the user, so a
    /// fixture using one would test the refusal path no matter what the test
    /// was about.
    fn request() -> ApprovalRequest {
        ApprovalRequest {
            run_id: "r1".into(),
            tool_name: "run_terminal".into(),
            command: "mkdir build".into(),
            arguments: serde_json::json!({ "command": "mkdir build" }),
        }
    }

    fn cancelled() -> Arc<AtomicBool> {
        Arc::new(AtomicBool::new(false))
    }

    fn runtime() -> tokio::runtime::Runtime {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("runtime")
    }

    /// A mode that allows needs no round trip through the user at all.
    #[test]
    fn an_allowed_call_settles_without_prompting() {
        let gate = ApprovalGate::new(PermissionMode::Full);
        let mut prompted = false;
        let decision = runtime()
            .block_on(gate.settle("r1", request(), &cancelled(), |_, _| prompted = true))
            .expect("settled");
        assert_eq!(decision, Decision::Allow);
        assert!(!prompted, "a permitted call should not prompt");
    }

    /// Full access approves every tool, including terminal calls that would
    /// otherwise require a prompt.
    #[test]
    fn full_access_does_not_prompt_for_terminal_calls() {
        let gate = ApprovalGate::new(PermissionMode::Full);
        let mut prompted = false;
        let mut terminal = request();
        terminal.command = "rm -rf x".into();
        let decision = runtime()
            .block_on(gate.settle("r1", terminal, &cancelled(), |_, _| prompted = true))
            .expect("settled");
        assert_eq!(decision, Decision::Allow);
        assert!(!prompted);
    }

    /// The whole reason the gate exists: a denial is a decision, not a failure.
    #[test]
    fn a_denied_call_settles_as_refused_and_the_run_continues() {
        let gate = ApprovalGate::new(PermissionMode::Ask);
        let runtime = runtime();
        let prompted = std::sync::Arc::new(std::sync::Mutex::new(None::<String>));

        // The answer comes from inside the prompt callback, which is the one
        // place guaranteed to run after the id is registered. Answering from a
        // separate thread would have to spin waiting for that registration and
        // would make this test depend on timing.
        let observer = prompted.clone();
        let answering = gate.clone();
        let decision =
            runtime.block_on(
                gate.settle("r1", request(), &cancelled(), move |_, approval_id| {
                    *observer.lock().expect("prompt lock") = Some(approval_id.to_string());
                    answering.answer(approval_id, Approval::Deny);
                }),
            );
        assert!(
            prompted.lock().expect("prompt lock").is_some(),
            "the prompt should have been emitted"
        );
        let decision = decision.expect("settled");
        assert!(
            matches!(decision, Decision::Deny(_)),
            "{decision:?} should be a refusal"
        );
    }

    /// Stopping the reply while a dialog is open must not leave the run parked
    /// forever. This is the deadlock the select! exists to prevent.
    #[test]
    fn cancelling_the_run_releases_a_waiting_prompt() {
        let gate = ApprovalGate::new(PermissionMode::Ask);
        let cancelled = cancelled();
        let runtime = runtime();
        let stop = cancelled.clone();
        let waiter = {
            let gate = gate.clone();
            let flag = cancelled.clone();
            std::thread::spawn(move || {
                runtime.block_on(async move {
                    gate.settle("r1", request(), &flag, |_, _| {
                        // The run is stopped the moment the dialog would appear,
                        // which is the ordering that used to hang.
                        stop.store(true, Ordering::Relaxed);
                    })
                    .await
                })
            })
        };
        let result = waiter.join().expect("thread");
        assert!(
            result.is_err(),
            "a cancelled wait must not resolve normally"
        );
    }

    /// Answering a dialog the user already closed is a normal race, not a bug.
    #[test]
    fn answering_a_stale_prompt_reports_rather_than_panics() {
        let gate = ApprovalGate::new(PermissionMode::Ask);
        assert!(!gate.answer("never-existed", Approval::Allow));
    }

    /// The freeze this whole design exists to prevent.
    ///
    /// A run holds a gate obtained from the runtime, and the user's answer
    /// arrives at the runtime's own gate. If those were two instances, the answer
    /// would find an empty map, the prompt would never be released, and the reply
    /// would hang with nothing on screen but a stop button -- which is exactly
    /// the symptom this test was written for.
    #[test]
    fn an_answer_reaching_one_gate_releases_a_prompt_waiting_on_another() {
        let runtime_gate = ApprovalGate::new(PermissionMode::Ask);
        // What a run is handed: the same pending map, mode re-pointed.
        let run_gate = runtime_gate.with_mode(PermissionMode::Ask);

        // The prompt's id is handed back over a channel, so the answering thread
        // cannot race ahead of the registration it is answering.
        let (sender, receiver) = std::sync::mpsc::channel::<String>();
        let waiter = std::thread::spawn(move || {
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("runtime")
                .block_on(
                    run_gate.settle("r1", request(), &cancelled(), move |_, id| {
                        sender.send(id.to_string()).expect("send id");
                    }),
                )
        });

        let approval_id = receiver
            .recv_timeout(std::time::Duration::from_secs(5))
            .expect("the run should have asked for approval");
        assert!(runtime_gate.answer(&approval_id, Approval::Allow));

        assert_eq!(
            waiter.join().expect("thread").expect("settled"),
            Decision::Allow
        );
    }

    /// Re-pointing the mode must not disconnect the pending map.
    #[test]
    fn re_pointing_the_mode_keeps_the_prompt_map_connected() {
        let gate = ApprovalGate::new(PermissionMode::Ask);
        let re_pointed = gate.with_mode(PermissionMode::Full);

        assert_eq!(re_pointed.mode(), PermissionMode::Full);
        // The mode is shared, not copied: changing it on one is visible on the
        // other, which is what makes a single answer path work at all.
        assert_eq!(gate.mode(), PermissionMode::Full);
    }

    /// A permission dialog names the command it is about, so a user approving
    /// `mkdir build` can tell it apart from anything else queued behind it.
    #[test]
    fn a_prompt_summarizes_the_call_it_is_about() {
        assert_eq!(request().summary(), "run_terminal: mkdir build");
        let mut terminal = request();
        terminal.command = "rm -rf x".into();
        assert_eq!(terminal.summary(), "run_terminal: rm -rf x");

        let mut without_command = request();
        without_command.command = "  ".into();
        assert_eq!(without_command.summary(), "run_terminal");
    }
}
