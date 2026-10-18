//! Finding and starting the browser this app drives.
//!
//! # A real browser, not the panel's webview
//!
//! The right panel already has a child webview ([`crate::panel::browser`]), and
//! that one is the *user's*. Driving it would work on Windows only -- it is
//! WebView2, and WebKit has no DevTools protocol -- and it would put the agent's
//! browsing in the same surface the user is reading. A separate Chrome is the
//! same browser on every platform, has its own profile, and can be watched,
//! minimised or ignored without disturbing the panel.
//!
//! # The port is chosen by Chrome, not by us
//!
//! `--remote-debugging-port=0` makes the browser pick a free port and print it:
//!
//! ```text
//! DevTools listening on ws://127.0.0.1:52341/devtools/browser/6f0c...
//! ```
//!
//! That line on stderr *is* the readiness signal, and it is also the whole
//! handshake -- there is no fixed port to collide with a second copy of the app,
//! and no probing loop to decide that something is up. Reading it is what
//! [`start`] does.
//!
//! # The profile is ours
//!
//! A dedicated `--user-data-dir` is required rather than optional: a recent Chrome
//! refuses remote debugging against the default profile, precisely so a tool
//! cannot attach to the browser holding a user's logins. Ours is a folder of this
//! app's own, which means a login the agent performs survives to the next run --
//! and the user's own browsing is untouched. It also means the panel's browser and
//! the agent's browser have separate cookie jars, which is the honest way round:
//! neither can reach the other's sessions.
//!
//! Where that folder is, what it is called, and how it is deleted is
//! [`super::profile`]'s business. This file only passes the path to `--user-data-dir`.

use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;

use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::{Child, Command};
use tokio::sync::{oneshot, Mutex};

/// How long the browser is given to print its WebSocket address.
///
/// A cold Chrome on a slow disk can take a few seconds; past this it is not
/// starting, and reporting that beats waiting on a window that never opens.
const START_TIMEOUT: Duration = Duration::from_secs(30);

/// How many of the browser's own stderr lines are kept for a failure report.
///
/// The reason a launch failed is in the *last* lines, not the first, so this is
/// a ring rather than a head. Small on purpose: it is quoted into a tool result.
const LOG_TAIL: usize = 40;

/// The browser's recent stderr, shared with whoever has to report on it.
///
/// A ring rather than a log: the reason a browser failed is in the last few lines
/// it printed, and a browser can print a great deal before that.
pub type Log = Arc<Mutex<VecDeque<String>>>;

/// A browser that is running, and how to talk to it.
pub struct Launched {
    /// The process. Held so the browser dies with the app rather than outliving
    /// it as an orphan window nobody asked for.
    pub child: Child,
    /// Its process id, kept separately because killing the tree by id is the one
    /// operation that has to work when nothing else may be awaited.
    pub pid: u32,
    /// The DevTools address its stderr announced.
    pub websocket: String,
    /// The last of its stderr, for a failure report.
    ///
    /// Kept for the whole life of the browser, not just its startup: the failure
    /// worth explaining is usually the later one, and this is the only place the
    /// browser says why it went.
    pub log: Log,
}

/// Starts the browser on `profile`, and waits until it says where to reach it.
pub async fn start(binary: &Path, profile: &Path) -> Result<Launched, String> {
    tokio::fs::create_dir_all(profile)
        .await
        .map_err(|error| format!("Could not create {}: {error}", profile.display()))?;

    let mut child = Command::new(binary)
        .args(arguments(profile))
        .stdin(Stdio::null())
        // Discarded: Chrome writes nothing here that is not on stderr, and an
        // unread pipe fills at 64 KB and wedges the browser behind it.
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| format!("Could not start {}: {error}", binary.display()))?;

    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| "The browser started without an error stream".to_string())?;
    let log: Log = Arc::new(Mutex::new(VecDeque::new()));
    let (announce, announced) = oneshot::channel();
    tokio::spawn(pump(stderr, Arc::clone(&log), announce));

    match tokio::time::timeout(START_TIMEOUT, announced).await {
        Ok(Ok(websocket)) => Ok(Launched {
            pid: child.id().unwrap_or_default(),
            child,
            websocket,
            log,
        }),
        // The reader finished without finding an address, which means the process
        // ended first -- a profile already locked by another copy, or a build of
        // the browser that will not take these arguments.
        Ok(Err(_)) => Err(format!(
            "The browser closed before it was ready. {}",
            tail(&log).await
        )),
        Err(_) => {
            let _ = child.start_kill();
            Err(format!(
                "The browser did not become ready within {}s. {}",
                START_TIMEOUT.as_secs(),
                tail(&log).await
            ))
        }
    }
}

/// Reads the browser's stderr for the DevTools address, keeping a tail of it.
///
/// One task for both jobs rather than two, because the stream can only be read
/// once: whoever reads it is also the only one who will see the announcement.
async fn pump(stderr: tokio::process::ChildStderr, log: Log, announce: oneshot::Sender<String>) {
    let mut lines = BufReader::new(stderr).lines();
    let mut announce = Some(announce);
    while let Ok(Some(line)) = lines.next_line().await {
        // Kept even after the address is found: a browser that dies mid-run has
        // usually said why just before it went, and that is the only place it says.
        //
        // Awaiting the lock rather than trying it: a browser that fails fast --
        // a profile another process still holds, a build that will not take these
        // arguments -- prints its reason and exits in the time this task takes to
        // start, and a line dropped here is the whole explanation gone. The
        // failure report it feeds then says "it printed nothing", which is true
        // and useless.
        {
            let mut log = log.lock().await;
            if log.len() == LOG_TAIL {
                log.pop_front();
            }
            log.push_back(line.clone());
        }
        // The address, once. Taken out of the option rather than cloned -- a
        // `oneshot::Sender` is not clonable, and the first `ws://` in the output is
        // the only one worth sending. The option is left alone when a line carries
        // none: the browser prints its banner before this one, and clearing it on
        // those lines would mean the address was never sent at all.
        if let Some(websocket) = websocket_in(&line) {
            if let Some(announce) = announce.take() {
                let _ = announce.send(websocket);
            }
        }
    }
}

/// The last few lines the browser printed, as one line, for a report.
pub async fn tail(log: &Log) -> String {
    let log = log.lock().await;
    let recent: Vec<&str> = log.iter().rev().take(6).rev().map(String::as_str).collect();
    if recent.is_empty() {
        return "It printed nothing.".to_string();
    }
    format!("It said: {}", recent.join(" | "))
}

/// The DevTools address in one line of the browser's output, if it is there.
///
/// Matched on `ws://` rather than on the whole announcement because the wording
/// has moved between versions ("DevTools listening on …"), while the address
/// never has.
fn websocket_in(line: &str) -> Option<String> {
    let start = line.find("ws://")?;
    let rest = &line[start..];
    let end = rest.find(char::is_whitespace).unwrap_or(rest.len());
    Some(rest[..end].to_string())
}

/// The arguments the browser is started with.
///
/// Deliberately short. Every flag here is one a browser needs to be *driven*
/// rather than used, and anything else -- window size, language, extensions --
/// is left to the browser's own defaults so this stays a stock browser that
/// happens to have a debugger attached.
fn arguments(profile: &Path) -> Vec<String> {
    vec![
        // A free port, announced on stderr. Comparing this with a fixed port: a
        // fixed one is a number two copies of the app, or a stale process, can
        // disagree about, and the failure is a browser nobody can reach.
        "--remote-debugging-port=0".to_string(),
        // Required, not cosmetic: without a directory of its own the browser
        // refuses to be debugged, and with one of its own the logins the agent
        // makes are still there next time.
        format!("--user-data-dir={}", profile.display()),
        // The three dialogs a fresh profile would otherwise stop at, none of
        // which a user is present to answer.
        "--no-first-run".to_string(),
        "--no-default-browser-check".to_string(),
        "--disable-sync".to_string(),
        // A page that calls `window.open` would otherwise be refused, and a new
        // tab is a thing the model is allowed to ask for.
        "--disable-popup-blocking".to_string(),
        // The features that talk to Google on their own. They make a run slower
        // and noisier for something the user did not ask for.
        "--disable-background-networking".to_string(),
        "--disable-component-update".to_string(),
        "--disable-features=Translate,OptimizationHints,MediaRouter".to_string(),
        // A window big enough to lay out a real site, and placed where it is
        // visible rather than behind whatever is open: the point of a headed
        // browser is that the user can watch, and a guessed 800x600 puts the
        // site's own breakpoint in charge of what they see.
        "--window-size=1280,900".to_string(),
        "--window-position=60,60".to_string(),
        // The page it opens on. Chrome refuses to start with no window at all in
        // some builds; an explicit blank page is also the first tab the driver
        // attaches to, so there is always exactly one.
        "about:blank".to_string(),
    ]
}

/// The browser to drive, or why none could be found.
///
/// Chrome first, then a Chromium build, then Edge -- in that order because the
/// protocol is Chrome's and the further down the list the more likely something
/// differs. All of them speak it, so this is a preference rather than a support
/// matrix.
pub fn browser_binary() -> Result<PathBuf, String> {
    for candidate in known_locations() {
        if candidate.is_file() {
            return Ok(candidate);
        }
    }
    for name in path_names() {
        if let Some(found) = in_path(name) {
            return Ok(found);
        }
    }
    Err(format!(
        "No browser to drive was found. Install Google Chrome, Chromium, or Microsoft Edge, \
         and try again. (Looked for {})",
        path_names().join(", ")
    ))
}

/// Where a browser is installed, by the conventions of each platform.
///
/// Checked before `PATH` because an installed browser is usually not on it -- on
/// Windows and macOS the launcher is a shell shortcut and the executable lives in
/// a versioned folder, which is exactly why `PATH` alone finds nothing.
fn known_locations() -> Vec<PathBuf> {
    let mut found = Vec::new();
    #[cfg(target_os = "windows")]
    {
        let roots = ["PROGRAMFILES", "PROGRAMFILES(X86)", "LOCALAPPDATA"];
        let relative = [
            ["Google", "Chrome", "Application", "chrome.exe"].as_slice(),
            ["Chromium", "Application", "chromium.exe"].as_slice(),
            ["Microsoft", "Edge", "Application", "msedge.exe"].as_slice(),
            ["BraveSoftware", "Brave-Browser", "Application", "brave.exe"].as_slice(),
        ];
        for root in roots {
            let Some(root) = std::env::var_os(root) else {
                continue;
            };
            for parts in relative {
                let mut path = PathBuf::from(&root);
                for part in parts {
                    path.push(part);
                }
                found.push(path);
            }
        }
    }
    #[cfg(target_os = "macos")]
    {
        for (directory, executable) in [
            ("Google Chrome.app", "Google Chrome"),
            ("Chromium.app", "Chromium"),
            ("Microsoft Edge.app", "Microsoft Edge"),
            ("Brave Browser.app", "Brave Browser"),
        ] {
            found.push(
                Path::new("/Applications")
                    .join(directory)
                    .join("Contents/MacOS")
                    .join(executable),
            );
        }
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        // The prefixes a distribution installs into, including the ones a
        // packaged Chromium lands in rather than a plain `/usr/bin`.
        let prefixes = [
            "/usr/bin",
            "/usr/local/bin",
            "/snap/bin",
            "/opt/google/chrome",
        ];
        for executable in [
            "google-chrome",
            "google-chrome-stable",
            "chromium",
            "chromium-browser",
            "microsoft-edge",
            "brave-browser",
        ] {
            for prefix in prefixes {
                found.push(Path::new(prefix).join(executable));
            }
        }
    }
    found
}

/// The executable names to look for on `PATH`.
fn path_names() -> Vec<&'static str> {
    #[cfg(target_os = "windows")]
    {
        vec!["chrome.exe", "chromium.exe", "msedge.exe", "brave.exe"]
    }
    #[cfg(not(target_os = "windows"))]
    {
        vec![
            "google-chrome",
            "google-chrome-stable",
            "chromium",
            "chromium-browser",
            "microsoft-edge",
            "brave-browser",
        ]
    }
}

/// The first executable of this name on `PATH`.
///
/// Hand-rolled rather than pulled from a crate: it is a split and a `join`, and
/// a dependency for a loop this short is a dependency to keep updated.
fn in_path(name: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|directory| directory.join(name))
        .find(|candidate| candidate.is_file())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The line Chrome prints, verbatim, including the leading banner text.
    #[test]
    fn the_devtools_address_is_read_out_of_the_announcement() {
        let line = "DevTools listening on ws://127.0.0.1:52341/devtools/browser/6f0c-1e2d";
        assert_eq!(
            websocket_in(line).as_deref(),
            Some("ws://127.0.0.1:52341/devtools/browser/6f0c-1e2d")
        );
    }

    #[test]
    fn an_ordinary_log_line_carries_no_address() {
        assert_eq!(websocket_in("[12345:67890:INFO:CONSOLE(1)] hello"), None);
        assert_eq!(websocket_in(""), None);
    }

    /// The address is not the last thing printed, so it is taken up to the
    /// whitespace rather than to the end of the line.
    #[test]
    fn an_address_is_ended_by_whitespace() {
        let line = "DevTools listening on ws://127.0.0.1:9/devtools/browser/a done";
        assert_eq!(
            websocket_in(line).as_deref(),
            Some("ws://127.0.0.1:9/devtools/browser/a")
        );
    }

    /// The three flags the driver cannot work without: a port to be reached on,
    /// a profile of its own (a recent Chrome refuses otherwise), and a page to
    /// attach to.
    #[test]
    fn the_arguments_are_the_ones_a_driven_browser_needs() {
        let args = arguments(Path::new("C:\\profiles\\browser"));
        assert!(args.iter().any(|arg| arg == "--remote-debugging-port=0"));
        assert!(args.iter().any(|arg| arg.starts_with("--user-data-dir=")));
        assert!(args.iter().any(|arg| arg == "about:blank"));
    }
}
