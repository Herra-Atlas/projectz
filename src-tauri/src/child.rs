//! Starting console programs without their console.
//!
//! Windows gives every console program a window of its own unless it is told
//! otherwise. A GUI app that shells out -- the agent's terminal, Piper, the local
//! model server, an MCP server -- would otherwise flash black boxes over the
//! window it is drawing, which looks like the app is broken.
//!
//! `CREATE_NO_WINDOW` creates the process with no console at all. It is not the
//! same as hiding a window: there is never one to hide, so nothing appears even
//! for a moment. Pipes still work, which is the whole point: every caller here
//! reads the output.
//!
//! Kept in one place because the flag has to be set before `spawn`, which makes
//! it easy to add a new child process and forget.
//!
//! The child's `PATH` is prepared here too. A process inherits its environment
//! once, so a tool installed after the app started is invisible to every shell it
//! opens until the app restarts; reading the user and machine `PATH` from the
//! registry at spawn time is what closes that gap. See `path.rs`.

#[cfg(windows)]
mod path;

#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// Hides the console of a `std::process::Command` and refreshes its `PATH`.
pub fn hide_console(command: &mut std::process::Command) -> &mut std::process::Command {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        path::apply_std(command);
        command.creation_flags(CREATE_NO_WINDOW);
    }
    command
}

/// Hides the console of a `tokio::process::Command` and refreshes its `PATH`.
pub fn hide_console_async(command: &mut tokio::process::Command) -> &mut tokio::process::Command {
    #[cfg(windows)]
    {
        path::apply_async(command);
        command.creation_flags(CREATE_NO_WINDOW);
    }
    command
}
