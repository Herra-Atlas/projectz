//! Ending a whole process tree, not just the shell we hold a handle to.
//!
//! A background command is launched through a shell (`powershell.exe` on Windows,
//! `sh` elsewhere), so the `Child` the registry holds is the *shell*. The process
//! the user actually cares about -- a dev server, a Python script -- is a child of
//! that shell, and `Child::start_kill` signals only the shell. Killing the handle
//! leaves the grandchild running with nothing watching it: the tool reports
//! success while the port is still bound, which is the failure this module exists
//! to fix.
//!
//! The kill asks the platform to walk the tree rather than signalling the handle:
//!
//! - **Windows** uses `taskkill /T /F`, which enumerates descendants by parent id
//!   at the moment it runs. It catches a child of the shell, which is the common
//!   case (a `python -m http.server` started by `run_terminal`). A process that
//!   has already been *reparented* -- detached on purpose -- is not reachable this
//!   way; a Job Object with `KILL_ON_JOB_CLOSE` would catch that too, at the cost
//!   of a new dependency, so it is left as the next step rather than taken now.
//! - **Unix** puts the child in its own process group at spawn (see
//!   [`super::background::spawn`]) and signals the whole group, so the shell's
//!   children go with it.
//!
//! Whether the tree *actually died* is confirmed by the caller against the shell
//! `Child` handle -- see [`super::background::kill`] -- because that is the one
//! piece of evidence we hold directly.

/// Ends `pid` and its descendants.
///
/// `Err` means the killer could not be run, or it reported a failure that is not
/// the benign "already gone". Whether the process is now dead is confirmed by the
/// caller against the `Child` handle.
pub(crate) fn kill_tree(pid: u32) -> Result<(), String> {
    #[cfg(windows)]
    {
        kill_tree_windows(pid)
    }
    #[cfg(not(windows))]
    {
        kill_tree_unix(pid)
    }
}

/// `taskkill /T /F` walks the tree from `pid` and force-terminates it.
#[cfg(windows)]
fn kill_tree_windows(pid: u32) -> Result<(), String> {
    // The one process helper, so the console flag and the refreshed PATH are
    // applied here as they are everywhere else.
    let mut command = std::process::Command::new("taskkill");
    command.args(["/PID", &pid.to_string(), "/T", "/F"]);
    let output = crate::child::hide_console(&mut command)
        .output()
        .map_err(|error| format!("Could not run taskkill: {error}"))?;
    if output.status.success() {
        return Ok(());
    }
    // Exit code 128 is taskkill's "no process with that id", which happens when it
    // already exited -- not our failure, and the code is stable where the message
    // is localised. Any other code is a real error worth surfacing.
    if output.status.code() == Some(128) {
        return Ok(());
    }
    let message = String::from_utf8_lossy(&output.stderr).trim().to_string();
    Err(if message.is_empty() {
        format!(
            "taskkill exited with {}",
            output.status.code().unwrap_or(-1)
        )
    } else {
        message
    })
}

/// Signals the child's process group, falling back to the single process.
#[cfg(not(windows))]
fn kill_tree_unix(pid: u32) -> Result<(), String> {
    // A negative pid addresses the whole process group, which `spawn` made equal
    // to the child's id. `-KILL` because a background command is being torn down,
    // not asked to stop.
    let group = std::process::Command::new("kill")
        .args(["-KILL", &format!("-{pid}")])
        .output()
        .map_err(|error| format!("Could not run kill: {error}"))?;
    if group.status.success() {
        return Ok(());
    }
    // No such group (the child was not a group leader): fall back to the single
    // process so an ordinary child still stops.
    let single = std::process::Command::new("kill")
        .args(["-KILL", &pid.to_string()])
        .output()
        .map_err(|error| format!("Could not run kill: {error}"))?;
    if single.status.success() {
        return Ok(());
    }
    let message = String::from_utf8_lossy(&single.stderr).trim().to_string();
    // "No such process" means it is already gone, which is not our failure.
    if message.contains("No such process") {
        return Ok(());
    }
    Err(if message.is_empty() {
        "could not stop the process".to_string()
    } else {
        message
    })
}
