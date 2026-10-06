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

use std::process::Stdio;

use serde_json::Value;
use tokio::process::Command;

use super::{ToolContext, ToolSpec};

/// Longest a command may run before it is killed.
///
/// Generous enough for `cargo build` on a cold cache and short enough that a
/// forgotten `sleep` is not a wait the user has to sit through.
const MAX_RUNTIME_SECS: u64 = 120;

/// Cap on captured output, before the shared truncation decides what the model
/// sees.
///
/// Separate from the model's cap because this one bounds *memory*: a command can
/// emit hundreds of megabytes, and stopping the read at a limit is what keeps
/// that from becoming a problem regardless of what the truncation then does.
const MAX_CAPTURED_BYTES: usize = 2 * 1024 * 1024;

/// Run a shell command in the workspace and return its output.
pub const RUN_TERMINAL: ToolSpec = ToolSpec {
    name: "run_terminal",
    description: "Run a shell command in the workspace and return its combined output and exit code. \
                  On Windows the shell is Windows PowerShell 5.1; chain commands with `;`, not `&&`. \
                  Use this for builds, tests, git and other command-line work. Prefer read_file and \
                  search_files over shell commands for reading and finding things, because they are \
                  faster and their output is already formatted. Commands run one at a time and are \
                  killed after two minutes.",
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
            let timeout = timeout_of(&arguments)?;
            run(command, timeout, context).await
        })
    },
};

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
fn platform_command(command: &str) -> Command {
    let mut process = if cfg!(windows) {
        let mut process = Command::new("powershell.exe");
        process
            .args(["-NoLogo", "-NoProfile", "-NonInteractive", "-Command"])
            .arg(format!(
                "$global:LASTEXITCODE = $null; {command}; $commandSucceeded = $?; if ($null -ne $LASTEXITCODE) {{ exit $LASTEXITCODE }}; if ($commandSucceeded) {{ exit 0 }} else {{ exit 1 }}"
            ));
        process
    } else {
        let mut process = Command::new("sh");
        process.arg("-c").arg(command);
        process
    };
    // Keeps the environment override on Windows from launching a second console
    // window behind the app for every command.
    process.env("NO_COLOR", "1");
    process
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

    let mut command_line = platform_command(&command);
    command_line
        .current_dir(&root)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    let child = command_line
        .spawn()
        .map_err(|error| format!("Could not start the command: {error}"))?;

    let wait = child.wait_with_output();
    tokio::pin!(wait);

    let cancelled = std::sync::Arc::clone(&context.cancelled);
    let output = tokio::select! {
        finished = &mut wait => finished.map_err(|error| format!("Command failed: {error}"))?,
        _ = super::approval::wait_until_cancelled(&cancelled) => {
            // The child is dropped with `kill_on_drop` set, so the process goes
            // with the future. Leaving it running would let a cancelled reply
            // keep burning the user's CPU with nothing watching it.
            return Err("Cancelled before the command finished".into());
        }
        _ = tokio::time::sleep(std::time::Duration::from_secs(timeout_secs)) => {
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
fn bounded_output(stdout: &[u8], stderr: &[u8]) -> String {
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
}
