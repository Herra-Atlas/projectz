//! Starting a local model from a message.
//!
//! The loader keeps one model resident, so starting another stops the current one.
//! That is why the chat asks before doing it, and why the switch that permits it is
//! off by default: a message can arrive when nobody is at the machine.

use std::time::Duration;

use crate::messaging::Messaging;

/// How long to wait for an engine to come up before calling it a failure.
const LOAD_LIMIT: Duration = Duration::from_secs(300);
/// How often that wait re-checks.
const LOAD_POLL: Duration = Duration::from_secs(2);
/// A load returns before the spawned engine has marked itself as starting. Until
/// this much time has passed, "neither figure set" means "not yet", not "failed".
const START_GRACE: Duration = Duration::from_secs(10);

/// Whether the named local model is the one currently loaded.
pub fn is_loaded(hub: &Messaging, id: &str) -> bool {
    hub.runtime()
        .local_models()
        .status()
        .loaded_model_id
        .as_deref()
        == Some(id)
}

/// Starts a local model and waits until it is answering, bounded.
///
/// Returns a reason rather than hanging: an engine that never comes up is a failure
/// with something to say, not a chat that stops replying.
pub async fn ensure_loaded(hub: &Messaging, id: &str) -> Result<(), String> {
    if is_loaded(hub, id) {
        return Ok(());
    }
    hub.runtime()
        .local_models()
        .load(hub.app().clone(), id.to_string())?;
    let started = tokio::time::Instant::now();
    let deadline = started + LOAD_LIMIT;
    loop {
        let status = hub.runtime().local_models().status();
        if status.loaded_model_id.as_deref() == Some(id) {
            return Ok(());
        }
        // A load that has already failed leaves neither figure set, which is the
        // difference between "still coming up" and "not going to".
        if started.elapsed() >= START_GRACE
            && status.loading_model_id.is_none()
            && status.loaded_model_id.is_none()
        {
            return Err("the engine stopped before it reported a model".to_string());
        }
        if tokio::time::Instant::now() >= deadline {
            return Err("it did not finish loading in time".to_string());
        }
        tokio::time::sleep(LOAD_POLL).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The wait is bounded, and it is long enough for a large GGUF on a cold disk.
    #[test]
    fn the_load_wait_is_bounded_and_generous() {
        assert_eq!(LOAD_LIMIT, Duration::from_secs(300));
        assert_eq!(LOAD_POLL, Duration::from_secs(2));
        assert!(START_GRACE < LOAD_LIMIT);
    }
}
