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
/// has no natural end. Past this the oldest bytes are dropped rather than the
/// newest: a dev server's most recent lines are what a reader (and the model) is
/// asking about, and a buffer that froze at the cap answered "No new output" for
/// the rest of the process's life. See [`Buffer::push`].
const MAX_BUFFER_BYTES: usize = 256 * 1024;

/// Most finished commands kept in the table before the oldest are dropped.
///
/// The registry is process-global and lives for the whole app run, so without a
/// bound every command a session ever ran would be held -- and with it up to
/// [`MAX_BUFFER_BYTES`] of output each. Both bounds together are what keep it
/// finite. See [`reap_finished`].
const MAX_TRACKED: usize = 64;

/// Every background command this app has started and not yet reaped.
fn registry() -> &'static Mutex<HashMap<String, Process>> {
    static REGISTRY: OnceLock<Mutex<HashMap<String, Process>>> = OnceLock::new();
    REGISTRY.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Accumulated output, shared with the reader tasks.
///
/// A rolling window, not a prefix: once the cap is reached the oldest bytes are
/// discarded and `dropped` records how many, so a reader's position stays an
/// absolute offset into the stream rather than an index into a buffer that moves.
#[derive(Default)]
struct Buffer {
    bytes: Vec<u8>,
    /// Bytes discarded from the front to stay under the cap.
    dropped: u64,
    /// Something was discarded, so a read can say the beginning is gone.
    truncated: bool,
}

impl Buffer {
    /// Appends, then trims the front back to the cap.
    fn push(&mut self, chunk: &[u8]) {
        if chunk.is_empty() {
            return;
        }
        self.bytes.extend_from_slice(chunk);
        if self.bytes.len() > MAX_BUFFER_BYTES {
            let excess = self.bytes.len() - MAX_BUFFER_BYTES;
            self.bytes.drain(..excess);
            self.dropped += excess as u64;
            self.truncated = true;
        }
    }

    /// Where a read starting at absolute offset `cursor` begins in `bytes`.
    ///
    /// A cursor older than the window is clamped to the first byte still held:
    /// the bytes it pointed at are gone, and the honest answer is the oldest
    /// output there is rather than a gap.
    fn start_for(&self, cursor: usize) -> usize {
        cursor
            .saturating_sub(self.dropped as usize)
            .min(self.bytes.len())
    }

    /// The absolute offset just past the last byte held.
    fn end(&self) -> usize {
        self.dropped as usize + self.bytes.len()
    }
}

/// One running or finished background command.
struct Process {
    command: String,
    buffer: Arc<Mutex<Buffer>>,
    /// Absolute offset of the first byte not yet handed to the model.
    ///
    /// Absolute rather than an index into `buffer`, because the front of that
    /// buffer moves once the cap is reached. See [`Buffer`].
    cursor: usize,
    /// Set once `try_wait` reports the child has exited.
    status: Option<String>,
    child: Child,
}

/// Starts a command in the background and returns its id.
pub fn spawn(command: &str, root: &Path) -> Result<String, String> {
    let mut command_line = super::terminal::platform_command(command);
    command_line
        .current_dir(root)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        // Reaped if the app exits with the process still tracked, so a dev server
        // started here is not left orphaned by a crash.
        .kill_on_drop(true);
    // On Unix, give the child its own process group so `kill_tree` can signal the
    // whole tree with a single negative pid. Windows reaches the tree through
    // `taskkill /T` instead and needs no setup here.
    #[cfg(unix)]
    command_line.process_group(0);
    let mut child = command_line
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
    // Finished, fully-read commands are dropped here rather than never, so a
    // session that runs many commands does not hold every buffer for the life of
    // the app. See [`reap_finished`].
    reap_finished(&mut processes);
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
///
/// With `tail` set this instead returns the most recent `tail` bytes without
/// moving the read cursor, so output already consumed can be looked at again --
/// the same buffer, read two ways. Without it the read is incremental and the
/// cursor advances, which is what keeps a poll cheap for a chatty server.
pub fn read_output(id: &str, tail: Option<usize>) -> Result<String, String> {
    let mut processes = registry().lock().map_err(|error| error.to_string())?;
    let process = processes.get_mut(id).ok_or_else(|| unknown(id))?;
    refresh_status(process);

    let (chunk, truncated, next_cursor) = {
        let buffer = process.buffer.lock().map_err(|error| error.to_string())?;
        match tail {
            Some(count) => {
                let start = buffer.bytes.len().saturating_sub(count);
                (buffer.bytes[start..].to_vec(), buffer.truncated, None)
            }
            None => {
                let start = buffer.start_for(process.cursor);
                (
                    buffer.bytes[start..].to_vec(),
                    buffer.truncated,
                    Some(buffer.end()),
                )
            }
        }
    };
    // The next read resumes at the absolute end of the stream, not at "cursor plus
    // what was copied": a window that discarded its front would otherwise leave the
    // cursor short by exactly what was dropped, and the same bytes would arrive
    // again.
    if let Some(next) = next_cursor {
        process.cursor = next;
    }

    let header = match (&process.status, tail) {
        (Some(status), _) => format!("`{id}` ({}) has finished: {status}.", process.command),
        (None, Some(_)) => format!(
            "`{id}` ({}) is still running; showing the most recent output.",
            process.command
        ),
        (None, None) => format!("`{id}` ({}) is still running.", process.command),
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
        out.push_str(
            "\n\n[older output was dropped to keep this command's buffer at 256 KB; what is shown \
             is the most recent output]",
        );
    }
    Ok(out)
}

/// Whether a background command is alive, its id, and its exit code once finished.
///
/// The deliberate alternative to inferring liveness by probing a port. Reads the
/// same `status` a poll fills in; it changes nothing.
pub fn status(id: &str) -> Result<String, String> {
    let mut processes = registry().lock().map_err(|error| error.to_string())?;
    let process = processes.get_mut(id).ok_or_else(|| unknown(id))?;
    refresh_status(process);

    // Absolute figures: the buffer's front moves once the cap is reached, so its
    // length alone would undercount everything the command has printed.
    let (buffered, unread) = process
        .buffer
        .lock()
        .map(|buffer| (buffer.end(), buffer.end().saturating_sub(process.cursor)))
        .unwrap_or((0, 0));
    let pid = process
        .child
        .id()
        .map(|pid| format!(" pid {pid}."))
        .unwrap_or_default();
    let state = match &process.status {
        Some(status) => format!("has finished: {status}"),
        None => "is running".to_string(),
    };
    Ok(format!(
        "`{id}` ({}) {state}.{pid}\n{buffered} bytes of output so far, {unread} unread.",
        process.command
    ))
}

/// Stops a running command and its descendants.
pub fn kill(id: &str) -> Result<String, String> {
    let mut processes = registry().lock().map_err(|error| error.to_string())?;
    let process = processes.get_mut(id).ok_or_else(|| unknown(id))?;

    refresh_status(process);
    if let Some(status) = &process.status {
        return Ok(format!("`{id}` had already finished: {status}."));
    }

    let pid = process
        .child
        .id()
        .ok_or_else(|| format!("`{id}` has no process id to stop."))?;
    // Signals the shell *and* its children. Killing the handle alone is what left
    // a `python.exe` running while this tool reported success.
    super::process::kill_tree(pid)?;

    // `taskkill /T /F` and `kill -KILL` return once the signal is delivered, so
    // give the OS a moment to reap the shell before believing it is gone. A kill
    // that reports success while the process still lives is worse than one that
    // admits it could not confirm.
    match confirm_exit(&mut process.child) {
        Some(status) => {
            process.status = Some(status.clone());
            Ok(format!("Stopped `{id}` ({status})."))
        }
        None => Err(format!(
            "`{id}` is still running after the kill (pid {pid}). It may be detached from the \
             shell this tool started; stop it yourself if it should be gone."
        )),
    }
}

/// Polls a command's status into its record, without blocking.
///
/// `try_wait` also reaps the child, so an exited command is not a zombie, and the
/// handle stays usable by [`kill`] at the same time.
fn refresh_status(process: &mut Process) {
    if process.status.is_none() {
        if let Ok(Some(status)) = process.child.try_wait() {
            process.status = Some(describe(status));
        }
    }
}

/// Waits briefly for a killed child to be reaped, returning how it exited.
///
/// Bounded so a kill never hangs: a shell that is genuinely gone reports within
/// milliseconds, and one that is not is reported rather than waited on forever.
fn confirm_exit(child: &mut Child) -> Option<String> {
    let deadline = std::time::Instant::now() + std::time::Duration::from_millis(500);
    loop {
        if let Ok(Some(status)) = child.try_wait() {
            return Some(describe(status));
        }
        if std::time::Instant::now() >= deadline {
            return None;
        }
        std::thread::sleep(std::time::Duration::from_millis(15));
    }
}

/// Drops finished commands whose output has been fully read.
///
/// Nothing will ask for those again: the bytes were handed over and the process is
/// gone. A finished command with *unread* output is kept, so a `terminal_output`
/// after the fact still finds what it missed. A hard cap stops the table growing
/// without bound when a command is finished but never read back.
fn reap_finished(processes: &mut HashMap<String, Process>) {
    processes.retain(|_, process| {
        if process.status.is_none() {
            return true;
        }
        let fully_read = process
            .buffer
            .lock()
            .map(|buffer| process.cursor >= buffer.end())
            .unwrap_or(false);
        !fully_read
    });
    if processes.len() > MAX_TRACKED {
        processes.retain(|_, process| process.status.is_none());
    }
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
                    buffer.push(&chunk[..read]);
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
            let output = read_output(&id, None).expect("read");
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
            let after = read_output(&id, None).expect("read");
            assert!(after.contains("finished"), "{after}");
        });
    }

    /// A `tail` read re-serves bytes the incremental read has already consumed, so
    /// output that was polled past can still be looked at again.
    #[test]
    fn a_tail_read_re_serves_output_without_consuming_it() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("runtime");
        runtime.block_on(async {
            let id = spawn("ping 127.0.0.1 -n 30", &root()).expect("spawn");
            tokio::time::sleep(std::time::Duration::from_millis(300)).await;
            // Drain what is there, then confirm the tail still finds it.
            let _ = read_output(&id, None).expect("read");
            let tail = read_output(&id, Some(64)).expect("tail");
            assert!(tail.contains("most recent output"), "{tail}");
            // And an incremental read is still empty, so the tail moved no cursor.
            let incremental = read_output(&id, None).expect("read");
            assert!(incremental.contains("No new output"), "{incremental}");
            let _ = kill(&id).expect("kill");
        });
    }

    /// A status read says whether the command is alive, which is the fact the
    /// model used to have to infer by probing a port.
    #[test]
    fn status_reports_a_running_command() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("runtime");
        runtime.block_on(async {
            let id = spawn("ping 127.0.0.1 -n 30", &root()).expect("spawn");
            tokio::time::sleep(std::time::Duration::from_millis(300)).await;
            let report = status(&id).expect("status");
            assert!(report.contains("is running"), "{report}");
            assert!(report.contains("pid"), "{report}");
            let _ = kill(&id).expect("kill");
        });
    }

    /// An unknown id is reported, not panicked on: the common cause is a restart.
    #[test]
    fn an_unknown_id_is_reported() {
        assert!(read_output("term-nope", None)
            .expect_err("unknown")
            .contains("restart"));
        assert!(kill("term-nope").expect_err("unknown").contains("restart"));
        assert!(status("term-nope")
            .expect_err("unknown")
            .contains("restart"));
    }

    /// The window keeps the newest bytes, and a cursor still means one place.
    ///
    /// Past the cap the buffer used to freeze, so every later read reported "No
    /// new output" while the process was still printing exactly what the model was
    /// waiting for. Dropping from the front is what keeps the tool useful for a
    /// long-running server; the absolute cursor is what stops the kept bytes being
    /// handed over twice.
    #[test]
    fn the_buffer_rolls_and_keeps_the_cursor_absolute() {
        let mut buffer = Buffer::default();
        buffer.push(b"first");
        assert_eq!(buffer.end(), 5);
        assert_eq!(buffer.start_for(0), 0);

        buffer.push(&vec![b'x'; MAX_BUFFER_BYTES]);
        let dropped = buffer.dropped as usize;
        assert!(buffer.truncated, "the beginning went, and it is reported");
        assert_eq!(dropped, 5, "exactly what was older than the window");
        assert_eq!(buffer.bytes.len(), MAX_BUFFER_BYTES);
        assert_eq!(buffer.end(), 5 + MAX_BUFFER_BYTES);

        // A cursor that pointed at bytes now gone reads from the oldest held byte
        // rather than panicking or claiming a gap.
        assert_eq!(buffer.start_for(0), 0);
        // A cursor at the end has nothing new, which is what `terminal_status`
        // and the reaper both ask.
        assert_eq!(buffer.start_for(buffer.end()), buffer.bytes.len());
    }
}
