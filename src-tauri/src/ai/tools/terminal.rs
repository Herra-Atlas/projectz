//! Running a shell command in the workspace.
//!
//! The most consequential tool here, and the only one whose command comes from
//! text the model wrote. The command is declared as `command_argument` so approval
//! prompts can show it to the user before the tool runs.
//!
//! # Process handling
//!
//! Output is read on a blocking thread rather than by piping to a temp file.
//! The command is not given a shell when the platform's shell adds nothing, but
//! on Windows a bare `cargo` is not an executable, so the shell is still used
//! there and is part of why the command is treated as one opaque string.

use std::path::Path;
use std::process::Stdio;

use serde_json::Value;
use tokio::process::{Child, Command};

use super::{ToolContext, ToolSpec};

/// Longest a command may run before it is killed.
///
/// Generous enough for `cargo build` on a cold cache and short enough that a
/// forgotten `sleep` is not a wait the user has to sit through.
pub(crate) const MAX_RUNTIME_SECS: u64 = 120;

/// Cap on captured output, before the shared truncation decides what the model
/// sees.
///
/// Separate from the model's cap because this one bounds *memory*: a command can
/// emit hundreds of megabytes, and stopping the read at a limit is what keeps
/// that from becoming a problem regardless of what the truncation then does.
const MAX_CAPTURED_BYTES: usize = 2 * 1024 * 1024;

/// Run a shell command in the workspace and return its output.
///
/// One tool for both a command that finishes and one that does not: with
/// `background` unset it waits, and with it set it returns an id immediately for
/// `terminal_output` and `terminal_kill` to act on. Two tools would make the model
/// choose before it knows which it needs, which it cannot -- a build that takes
/// five minutes and a dev server look identical until one of them returns.
pub const RUN_TERMINAL: ToolSpec = ToolSpec {
    name: "run_terminal",
    description: "Run a shell command in the workspace and return its combined output and exit code. \
                  On Windows the shell is Windows PowerShell 5.1; chain commands with `;`, not `&&`. \
                  Use this for builds, tests, git and other command-line work. Prefer read_file, grep \
                  and glob over shell commands for reading and finding things, because they are \
                  faster and their output is already formatted. Commands run one at a time and are \
                  killed after two minutes. Set `background` for a command that does not finish on \
                  its own -- a dev server, a watcher -- then read it with `terminal_output` and stop \
                  it with `terminal_kill`.",
    parameters: r#"{
        "type": "object",
        "properties": {
            "command": {
                "type": "string",
                "description": "The command line to run, for example `cargo test` or `git status`."
            },
            "timeout_seconds": {
                "type": "integer",
                "description": "Override the two minute limit. Values above the limit are clamped to it."
            },
            "background": {
                "type": "boolean",
                "description": "Start the command and return at once with an id, instead of waiting for it. Use for a long-running process."
            }
        },
        "required": ["command"],
        "additionalProperties": false
    }"#,
    effect: super::Effect::Write,
    // Named so the permission policy inspects this tool's own field. A terminal
    // tool with `None` here would be classified as carrying no command at all.
    command_argument: Some("command"),
    execute: |arguments, context| {
        Box::pin(async move {
            let command = required_string(&arguments, "command")?;
            let background = arguments
                .get("background")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            if background {
                let root = super::file::require_workspace()?;
                return start_background(&command, &root);
            }
            let timeout = timeout_of(&arguments)?;
            run(command, timeout, context).await
        })
    },
};

/// Read what a background command has printed and whether it is still running.
pub const TERMINAL_OUTPUT: ToolSpec = ToolSpec {
    name: "terminal_output",
    description: "Read the output a background command has printed since the last check, and \
                  whether it has finished. Call this after `run_terminal` with `background` set. \
                  Each call returns only what is new, so poll it in a loop rather than expecting \
                  the whole log each time. Pass `tail` to re-read the most recent output instead \
                  -- for example after you have already consumed it and need to look back.",
    parameters: r#"{
        "type": "object",
        "properties": {
            "id": {
                "type": "string",
                "description": "The id `run_terminal` returned when the command was started."
            },
            "tail": {
                "type": "integer",
                "description": "Return the most recent N bytes instead of only what is new. Does not advance the read position, so it can be asked for more than once."
            }
        },
        "required": ["id"],
        "additionalProperties": false
    }"#,
    // Volatile, not Read: the result is a *cursor*, not a function of the
    // arguments. With identical arguments the second poll returns the bytes
    // printed since the first, so caching it handed every later poll the same
    // first chunk -- and skipped `refresh_status`, so a finished command was
    // never reported as finished. A poll is a question about the present and
    // must reach the process every time.
    effect: super::Effect::Volatile,
    command_argument: None,
    execute: |arguments, context| {
        let _ = &context;
        let tail = arguments
            .get("tail")
            .and_then(Value::as_u64)
            .map(|bytes| bytes as usize);
        Box::pin(std::future::ready(
            required_string(&arguments, "id")
                .and_then(|id| super::background::read_output(&id, tail)),
        ))
    },
};

/// Report whether a background command is alive, and how it exited.
pub const TERMINAL_STATUS: ToolSpec = ToolSpec {
    name: "terminal_status",
    description: "Report a background command's liveness, its process id, and its exit code once \
                  it has finished, plus how much output is buffered. Use this to ask whether a \
                  server is still up instead of guessing by probing a port.",
    parameters: r#"{
        "type": "object",
        "properties": {
            "id": {
                "type": "string",
                "description": "The id `run_terminal` returned when the command was started."
            }
        },
        "required": ["id"],
        "additionalProperties": false
    }"#,
    // Volatile for the same reason as `terminal_output`: liveness is not a
    // function of the id alone, and a cached "still running" is exactly the
    // answer that stops a model from polling a command that has finished.
    effect: super::Effect::Volatile,
    command_argument: None,
    execute: |arguments, context| {
        let _ = &context;
        Box::pin(std::future::ready(
            required_string(&arguments, "id").and_then(|id| super::background::status(&id)),
        ))
    },
};

/// Stop a background command.
pub const TERMINAL_KILL: ToolSpec = ToolSpec {
    name: "terminal_kill",
    description: "Stop a command started with `run_terminal` in the background. Use this when you \
                  are done with a long-running process so it does not keep running.",
    parameters: r#"{
        "type": "object",
        "properties": {
            "id": {
                "type": "string",
                "description": "The id `run_terminal` returned when the command was started."
            }
        },
        "required": ["id"],
        "additionalProperties": false
    }"#,
    effect: super::Effect::Write,
    command_argument: None,
    execute: |arguments, context| {
        let _ = &context;
        Box::pin(std::future::ready(
            required_string(&arguments, "id").and_then(|id| super::background::kill(&id)),
        ))
    },
};

/// Starts a command in the background and reports its id.
///
/// The acknowledgement says the command is *not* watched, because a model that
/// treats this like a normal call will assume the command succeeded and never read
/// its output. Naming the two follow-up tools is what makes the next step obvious.
fn start_background(command: &str, root: &std::path::Path) -> Result<String, String> {
    let id = super::background::spawn(command, root)?;
    Ok(format!(
        "Started `{command}` in the background as `{id}`. It is not waited on. Read what it prints \
         with `terminal_output` (id `{id}`) and stop it with `terminal_kill` when you are done."
    ))
}

fn required_string(arguments: &Value, field: &str) -> Result<String, String> {
    arguments
        .get(field)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .ok_or_else(|| format!("A non-empty `{field}` is required"))
}

/// Reads the optional timeout, clamped to the hard limit.
///
/// Clamped rather than refused: a model asking for a longer timeout is asking
/// for something that will not be given, and telling it the real limit is more
/// useful than an error it cannot act on.
fn timeout_of(arguments: &Value) -> Result<u64, String> {
    let requested = arguments
        .get("timeout_seconds")
        .and_then(Value::as_u64)
        .unwrap_or(MAX_RUNTIME_SECS);
    Ok(requested.clamp(1, MAX_RUNTIME_SECS))
}

/// Builds the process for this platform's shell.
///
/// Windows PowerShell resolves PATH commands including `cargo`, `npm`, and `git`
/// shims. The wrapper returns the last native command's exit code to the caller.
/// Unix gets `sh -c`, which is what makes pipes and `&&` mean anything there.
///
/// Shared with [`super::background`] so a backgrounded command is launched by the
/// same shell the waiting path uses: a model must not find that `cargo` works
/// normally and not in the background, or the other way round.
pub(crate) fn platform_command(command: &str) -> Command {
    let mut process = if cfg!(windows) {
        let mut process = Command::new("powershell.exe");
        process
            .args(["-NoLogo", "-NoProfile", "-NonInteractive", "-Command"])
            .arg(format!(
                // **UTF-8 out, not the OEM code page.** PowerShell 5.1 writes its
                // redirected stdout in the console code page -- 437, 866, 1252, and
                // so on -- while the pipe is decoded as UTF-8. Every accented,
                // Cyrillic or CJK byte therefore arrived as U+FFFD, silently, in
                // every `git log`, `dir` and compiler diagnostic. `UTF8Encoding
                // $false` is what suppresses the BOM this would otherwise prepend.
                "[Console]::OutputEncoding = New-Object System.Text.UTF8Encoding $false; \
                 $OutputEncoding = [Console]::OutputEncoding; \
                 $global:LASTEXITCODE = $null; {command}; $commandSucceeded = $?; \
                 if ($null -ne $LASTEXITCODE) {{ exit $LASTEXITCODE }}; \
                 if ($commandSucceeded) {{ exit 0 }} else {{ exit 1 }}"
            ));
        process
    } else {
        let mut process = Command::new("sh");
        process.arg("-c").arg(command);
        process
    };
    // Keeps colour codes out of tool output, where they are only noise.
    process.env("NO_COLOR", "1");
    // No console window: a packaged app has no console to lend the shell, so
    // Windows would otherwise open one over the chat for every command.
    crate::child::hide_console_async(&mut process);
    process
}

/// Everything that makes a command a *script* rather than a program and flags.
const SHELL_SYNTAX: &[char] = &[
    ';', '&', '|', '<', '>', '"', '\'', '`', '$', '*', '?', '(', ')', '{', '}', '[', ']', '#', '\n',
    '\r',
];

/// The command as a direct spawn, when it is only a program and its arguments.
///
/// PowerShell 5.1 is a .NET host: starting it costs 150-400 ms on a cold page
/// cache, which is twenty to forty times what `git status` takes to run. Most of
/// what a model issues is a bare program with flags -- `cargo test`, `npm run
/// build`, `git status` -- and the platform resolves those from PATH itself, so
/// the shell adds nothing but that startup.
///
/// A command with quoting, a pipe, a redirect, a variable, a glob or a separator
/// is left to the shell, and so is one that fails to spawn: a cmdlet such as
/// `dir`, or a built-in such as `cd`, is not a program on PATH. Every fallback is
/// the exact command that ran before this existed, so nothing that worked can
/// stop working.
fn direct_command(command: &str) -> Option<Command> {
    if command.contains(SHELL_SYNTAX) {
        return None;
    }
    let mut parts = command.split_whitespace();
    let program = parts.next()?;
    // `FOO=bar cmd` is shell syntax even without a metacharacter in it.
    if program.contains('=') {
        return None;
    }
    let mut process = Command::new(program);
    process.args(parts);
    process.env("NO_COLOR", "1");
    crate::child::hide_console_async(&mut process);
    Some(process)
}

/// What every command needs, whichever way it is launched.
fn prepare(command_line: &mut Command, root: &Path) {
    command_line
        .current_dir(root)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    // A waiting command can still be stopped, so it needs to be reachable as a
    // tree for the same reason a background one does: on Unix `kill_tree` signals
    // the group, which only exists if the command leads one.
    #[cfg(unix)]
    command_line.process_group(0);
}

/// Starts the command directly when it can be, through the shell otherwise.
fn spawn_command(command: &str, root: &Path) -> Result<Child, String> {
    if let Some(mut direct) = direct_command(command) {
        prepare(&mut direct, root);
        if let Ok(child) = direct.spawn() {
            return Ok(child);
        }
    }
    let mut shell = platform_command(command);
    prepare(&mut shell, root);
    shell
        .spawn()
        .map_err(|error| format!("Could not start the command: {error}"))
}

/// Ends the command and everything it started.
///
/// `kill_on_drop` signals the shell the tool holds a handle to; the process the
/// user cares about is that shell's child. The pid is read before the child is
/// consumed by `wait_with_output`, so the tree-killer the background path already
/// uses can be pointed at it. Run off the async worker, because on Windows the
/// killer is `taskkill.exe` -- a process launch plus a blocking wait.
async fn stop_tree(pid: Option<u32>) {
    let Some(pid) = pid else { return };
    let _ = tokio::task::spawn_blocking(move || super::process::kill_tree(pid)).await;
}

async fn run(command: String, timeout_secs: u64, context: ToolContext) -> Result<String, String> {
    // The workspace, not whatever the process inherited. A terminal tool that
    // ran somewhere else would make every relative path in its output a lie.
    //
    // Refused outright when nothing is open. A default would be the worst case
    // for this tool specifically: a command run in a folder the user never chose
    // could write anywhere in it, so "wherever we defaulted to" is not a safe
    // answer here even though it is a merely unhelpful one for a read.
    let root = super::file::require_workspace()?;

    let child = spawn_command(&command, &root)?;
    // Taken before `wait_with_output` consumes the child: `kill_on_drop` reaches
    // only the shell, so the pid is what lets a timeout end the processes the
    // shell started. See `stop_tree`.
    let pid = child.id();

    let wait = child.wait_with_output();
    tokio::pin!(wait);

    let cancelled = std::sync::Arc::clone(&context.cancelled);
    let output = tokio::select! {
        finished = &mut wait => finished.map_err(|error| format!("Command failed: {error}"))?,
        _ = super::approval::wait_until_cancelled(&cancelled) => {
            // The tree, not just the shell. Dropping `wait` below still triggers
            // `kill_on_drop` for `powershell.exe`; this reaches the `cargo`,
            // `node` or `tsc` it started, which otherwise kept the build locked
            // and the port bound after the tool had reported a clean stop.
            stop_tree(pid).await;
            return Err("Cancelled before the command finished".into());
        }
        _ = tokio::time::sleep(std::time::Duration::from_secs(timeout_secs)) => {
            stop_tree(pid).await;
            return Err(format!(
                "The command was stopped after {timeout_secs}s without finishing. \
                 Narrow the command or run it in the background yourself."
            ));
        }
    };

    let status = output.status;
    let combined = bounded_output(&output.stdout, &output.stderr);
    Ok(format!(
        "Exit code: {}\n\n{}{}",
        status.code().unwrap_or(-1),
        combined,
        if status.success() {
            ""
        } else {
            "\n[The command reported a failure.]"
        }
    ))
}

/// Interleaves stdout and stderr, stopping at the capture limit.
///
/// stderr is included because for a build or a test it holds the part the user
/// needs; excluding it is why so many agent tools report success on a failing
/// test.
pub(crate) fn bounded_output(stdout: &[u8], stderr: &[u8]) -> String {
    let mut text = String::new();
    for (label, bytes) in [("stdout", stdout), ("stderr", stderr)] {
        if bytes.is_empty() {
            continue;
        }
        let remaining = MAX_CAPTURED_BYTES.saturating_sub(text.len());
        let slice = &bytes[..bytes.len().min(remaining)];
        let decoded = String::from_utf8_lossy(slice);
        text.push_str(&format!("[{label}]\n{decoded}"));
        if slice.len() < bytes.len() {
            text.push_str(&format!(
                "\n[output truncated at {MAX_CAPTURED_BYTES} bytes]"
            ));
            break;
        }
        text.push('\n');
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_missing_command_is_reported() {
        assert!(required_string(&serde_json::json!({}), "command")
            .expect_err("missing")
            .contains("command"));
        assert!(required_string(&serde_json::json!({ "command": "  " }), "command").is_err());
    }

    #[test]
    fn the_timeout_is_clamped_to_the_hard_limit() {
        // A model asking for longer is told the real limit rather than refused:
        // an error it cannot act on wastes a whole turn.
        assert_eq!(
            timeout_of(&serde_json::json!({ "timeout_seconds": 9_999 })).expect("timeout"),
            MAX_RUNTIME_SECS
        );
        assert_eq!(
            timeout_of(&serde_json::json!({})).expect("default"),
            MAX_RUNTIME_SECS
        );
        assert_eq!(
            timeout_of(&serde_json::json!({ "timeout_seconds": 30 })).expect("timeout"),
            30
        );
        // Zero would be an instant kill, which is never what was meant.
        assert_eq!(
            timeout_of(&serde_json::json!({ "timeout_seconds": 0 })).expect("timeout"),
            1
        );
    }

    #[test]
    fn stderr_is_kept_because_it_holds_the_failure() {
        let combined = bounded_output(b"ok", b"error: test failed");
        assert!(combined.contains("ok"), "{combined}");
        assert!(combined.contains("error: test failed"), "{combined}");
        assert!(combined.contains("[stderr]"), "{combined}");
    }

    #[test]
    fn an_absent_stream_is_omitted_rather_than_shown_empty() {
        let combined = bounded_output(b"only out", b"");
        assert!(combined.contains("only out"));
        assert!(!combined.contains("stderr"), "{combined}");
    }

    #[test]
    fn a_command_that_floods_is_cut_at_the_capture_limit() {
        // The bound that keeps a runaway command from becoming a memory problem,
        // which the later truncation would not have caught on its own.
        let flood = vec![b'x'; MAX_CAPTURED_BYTES * 2];
        let combined = bounded_output(&flood, b"");
        assert!(
            combined.len() < MAX_CAPTURED_BYTES + 200,
            "{}",
            combined.len()
        );
        assert!(combined.contains("truncated"), "should say it was cut");
    }

    #[test]
    fn the_command_field_is_the_one_the_policy_inspects() {
        // Without this the policy would classify the terminal tool as carrying
        // no command and every shell call would run unclassified.
        assert_eq!(RUN_TERMINAL.command_argument, Some("command"));
        assert_eq!(RUN_TERMINAL.effect, super::super::Effect::Write);
    }

    /// The two follow-up tools carry no command and observe only. `terminal_kill`
    /// is a write because stopping a process is the consequential half.
    ///
    /// Output and status are `Volatile`, not `Read`: both answer a question about
    /// the present, so the same arguments legitimately produce a different answer
    /// every time. Caching them returned the first poll's snapshot for every later
    /// one and skipped the liveness check entirely.
    #[test]
    fn the_follow_up_tools_are_observed_without_a_command() {
        assert_eq!(TERMINAL_OUTPUT.effect, super::super::Effect::Volatile);
        assert_eq!(TERMINAL_STATUS.effect, super::super::Effect::Volatile);
        assert_eq!(TERMINAL_KILL.effect, super::super::Effect::Write);
        for tool in [TERMINAL_OUTPUT, TERMINAL_STATUS, TERMINAL_KILL] {
            assert_eq!(tool.command_argument, None);
        }
    }

    /// Only a bare program and its flags skip the shell.
    ///
    /// The direct path exists because PowerShell's startup is 150-400 ms per call,
    /// but anything the shell interprets has to reach the shell: guessing at a
    /// pipeline or a variable would run something other than what was written.
    #[test]
    fn a_shell_command_is_not_spawned_directly() {
        assert!(direct_command("cargo test --lib").is_some());
        assert!(direct_command("git status").is_some());
        for scripted in [
            "cargo test; rm -rf src",
            "dir | more",
            "echo hi > out.txt",
            "$env:FOO = 'bar'; npm run build",
            "Get-ChildItem *.rs",
            "\"C:\\Program Files\\x.exe\" --run",
        ] {
            assert!(
                direct_command(scripted).is_none(),
                "{scripted} is shell syntax"
            );
        }
    }
}
