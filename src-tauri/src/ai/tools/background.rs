//! Commands that outlive the call that started them.
//!
//! `run_terminal` waits for its command and kills it after two minutes, which is
//! right for a build or a test and wrong for a dev server, a watcher, or anything
//! a person would leave running. Those need three verbs rather than one: start it,
//! read what it has printed since last time, and stop it.
//!
//! # Why a global registry
//!
//! The process has to survive the tool call returning, so it cannot live on that
//! call's stack or in the run's context -- the next call that reads its output is
//! a different call, possibly from a different round. One process-wide table keyed
//! by a short id is what lets `terminal_output` find what `run_terminal` left
//! running.
//!
//! # Why the child is polled, not awaited
//!
//! A task that awaited the child would own it, and then nothing could stop it.
//! Keeping the [`Child`] in the table and calling `try_wait` when the model asks
//! for output leaves the handle available to `terminal_kill` at the same time.
//! Output is still pushed by background reader tasks, so it accumulates whether or
//! not anyone is asking.

use std::collections::HashMap;
use std::path::Path;
use std::process::Stdio;
use std::sync::{Arc, Mutex, OnceLock};

use tokio::process::Child;

/// Most output kept per process.
///
/// A cap because a runaway command can print without limit, and the reader task
/// has no natural end. Past this the buffer stops growing and says so, rather than
/// holding gigabytes for a command the user has forgotten about.
const MAX_BUFFER_BYTES: usize = 256 * 1024;

/// Every background command this app has started and not yet reaped.
fn registry() -> &'static Mutex<HashMap<String, Process>> {
    static REGISTRY: OnceLock<Mutex<HashMap<String, Process>>> = OnceLock::new();
    REGISTRY.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Accumulated output, shared with the reader tasks.
#[derive(Default)]
struct Buffer {
    bytes: Vec<u8>,
    /// The cap was reached and later output was dropped.
    truncated: bool,
}

/// One running or finished background command.
struct Process {
    command: String,
    buffer: Arc<Mutex<Buffer>>,
    /// How much of `buffer` has already been handed to the model.
    cursor: usize,
    /// Set once `try_wait` reports the child has exited.
    status: Option<String>,
    child: Child,
}

/// Starts a command in the background and returns its id.
pub fn spawn(command: &str, root: &Path) -> Result<String, String> {
    let mut child = super::terminal::platform_command(command)
        .current_dir(root)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        // Reaped if the app exits with the process still tracked, so a dev server
        // started here is not left orphaned by a crash.
        .kill_on_drop(true)
        .spawn()
        .map_err(|error| format!("Could not start the command: {error}"))?;

    let buffer = Arc::new(Mutex::new(Buffer::default()));
    if let Some(stdout) = child.stdout.take() {
        pump(stdout, Arc::clone(&buffer));
    }
    if let Some(stderr) = child.stderr.take() {
        pump(stderr, Arc::clone(&buffer));
    }

    // Short and readable rather than a full uuid: a model has to quote this id
    // back, and eight hex characters were always unique enough for the handful of
    // commands one session leaves running.
    let id = format!(
        "term-{}",
        &uuid::Uuid::new_v4().to_string().replace('-', "")[..8]
    );
    let mut processes = registry().lock().map_err(|error| error.to_string())?;
    processes.insert(
        id.clone(),
        Process {
            command: command.to_string(),
            buffer,
            cursor: 0,
            status: None,
            child,
        },
    );
    Ok(id)
}

/// Output printed since the last read, and whether the command has finished.
pub fn read_output(id: &str) -> Result<String, String> {
    let mut processes = registry().lock().map_err(|error| error.to_string())?;
    let process = processes.get_mut(id).ok_or_else(|| unknown(id))?;

    // Polled rather than awaited, so the handle stays usable by `kill` at the same
    // time. `try_wait` also reaps the child, so an exited command is not a zombie.
    if process.status.is_none() {
        if let Ok(Some(status)) = process.child.try_wait() {
            process.status = Some(describe(status));
        }
    }

    let (chunk, truncated) = {
        let buffer = process.buffer.lock().map_err(|error| error.to_string())?;
        let cursor = process.cursor.min(buffer.bytes.len());
        (buffer.bytes[cursor..].to_vec(), buffer.truncated)
    };
    process.cursor += chunk.len();

    let header = match &process.status {
        Some(status) => format!("`{id}` ({}) has finished: {status}.", process.command),
        None => format!("`{id}` ({}) is still running.", process.command),
    };
    let body = String::from_utf8_lossy(&chunk);
    let mut out = header;
    if body.is_empty() {
        out.push_str("\nNo new output.");
    } else {
        out.push_str("\n\n");
        out.push_str(&body);
    }
    if truncated {
        out.push_str(&format!(
            "\n\n[output was cut at {MAX_BUFFER_BYTES} bytes; this command prints too much to keep]"
        ));
    }
    Ok(out)
}

/// Stops a running command.
pub fn kill(id: &str) -> Result<String, String> {
    let mut processes = registry().lock().map_err(|error| error.to_string())?;
    let process = processes.get_mut(id).ok_or_else(|| unknown(id))?;

    if let Ok(Some(status)) = process.child.try_wait() {
        process.status = Some(describe(status));
    }
    if let Some(status) = &process.status {
        return Ok(format!("`{id}` had already finished: {status}."));
    }
    process
        .child
        .start_kill()
        .map_err(|error| format!("Could not stop `{id}`: {error}"))?;
    Ok(format!("Stopped `{id}`."))
}

/// The message for an id nobody knows, naming the likely cause.
///
/// Ids are forgotten when the app restarts, which is the common way to see this:
/// a model that was told an id in a previous session will quote it back after the
/// registry is empty.
fn unknown(id: &str) -> String {
    format!(
        "No background command `{id}` is known. It may have been started in an earlier session; \
         background commands do not survive a restart. Start it again if you still need it."
    )
}

/// Human wording for a process that has exited.
fn describe(status: std::process::ExitStatus) -> String {
    match status.code() {
        Some(0) => "exited with code 0".to_string(),
        Some(code) => format!("exited with code {code}"),
        None => "was terminated".to_string(),
    }
}

/// Reads a child's output into `buffer` until it closes.
fn pump<R>(mut reader: R, buffer: Arc<Mutex<Buffer>>)
where
    R: tokio::io::AsyncRead + Unpin + Send + 'static,
{
    tokio::spawn(async move {
        use tokio::io::AsyncReadExt;
        let mut chunk = [0u8; 8192];
        loop {
            match reader.read(&mut chunk).await {
                // Zero bytes is end of stream; an error means the pipe closed with
                // the process, which is the same thing to us.
                Ok(0) | Err(_) => break,
                Ok(read) => {
                    let Ok(mut buffer) = buffer.lock() else {
                        break;
                    };
                    if buffer.bytes.len() < MAX_BUFFER_BYTES {
                        buffer.bytes.extend_from_slice(&chunk[..read]);
                    } else {
                        buffer.truncated = true;
                    }
                }
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn root() -> std::path::PathBuf {
        std::env::temp_dir()
    }

    /// An id can be used to read output and then to stop the command.
    #[test]
    fn a_started_command_can_be_read_and_stopped() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("runtime");
        runtime.block_on(async {
            // `ping` runs indefinitely on Windows and Unix alike, so it is a
            // command that will still be alive when we stop it.
            let id = spawn("ping 127.0.0.1 -n 30", &root()).expect("spawn");
            tokio::time::sleep(std::time::Duration::from_millis(300)).await;
            let output = read_output(&id).expect("read");
            assert!(
                output.contains("still running") || output.contains("finished"),
                "{output}"
            );
            let stopped = kill(&id).expect("kill");
            assert!(
                stopped.contains("Stopped") || stopped.contains("finished"),
                "{stopped}"
            );
            // A second read now reports an exit rather than inventing output.
            tokio::time::sleep(std::time::Duration::from_millis(200)).await;
            let after = read_output(&id).expect("read");
            assert!(after.contains("finished"), "{after}");
        });
    }

    /// An unknown id is reported, not panicked on: the common cause is a restart.
    #[test]
    fn an_unknown_id_is_reported() {
        assert!(read_output("term-nope")
            .expect_err("unknown")
            .contains("restart"));
        assert!(kill("term-nope").expect_err("unknown").contains("restart"));
    }
}
