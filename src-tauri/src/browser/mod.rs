//! Driving a real browser, for the agent rather than for the user.
//!
//! # What this is
//!
//! One Chrome, Chromium or Edge, started on its own profile and spoken to over
//! the DevTools Protocol. One model-callable tool reaches it -- `browser_exec` --
//! and this module is everything behind that tool.
//!
//! The design follows the same rule as every other tool in this app: **the
//! general mechanism lives here and the site-specific knowledge lives in the
//! model, per request.** There is no selector for a particular site, no host it
//! branches on, and no wait tuned to one page. A driver that knows a site is a
//! driver that works on one.
//!
//! # Why a browser of its own
//!
//! [`crate::panel::browser`] is the user's: a child webview inside the app window,
//! driven from the address bar. This one is the agent's: a separate process on a
//! separate profile, which means the agent's logins are not the user's, the page
//! it is reading is not the page the user is reading, and the window can be
//! watched, minimised or on another monitor without disturbing either.
//!
//! It is also the same browser on every platform. The panel's webview is WebView2
//! on Windows and WebKit everywhere else, and WebKit has no DevTools protocol --
//! so that surface simply cannot be driven the way this one can.
//!
//! # One browser, kept warm, and closed when it is not wanted
//!
//! Starting Chrome costs a second and a half, so it is started on the first call
//! that needs it and then kept for the conversation -- that is what makes a step
//! that clicks a link cost a round trip rather than a launch.
//!
//! What it must not do is keep a window and a hundred megabytes of renderer alive
//! for the rest of the session after one task. Ten minutes past the last use it is
//! closed, and the profile it leaves behind is what makes the next start cheaper
//! than the first. That is the same bargain the local model makes: loaded when
//! asked for, released when nothing is due.
//!
//! # What this deliberately does not do
//!
//! - **Cross-origin frames.** A same-origin frame is walked into as part of the
//!   page; one from another origin has no readable document, so it is a single
//!   row and its contents are not reachable. Reaching into it needs its own
//!   execution context, per frame, which is a real feature and not a footnote.
//! - **Uploads and downloads.** A file dialog is not something a page can be
//!   driven through, and a downloaded file would land outside the workspace the
//!   rest of the tools respect.
//! - **Stealth.** This is a stock browser with a debugger attached. Sites that
//!   work to detect automation will detect this one, and pretending otherwise is
//!   not a thing a driver should quietly attempt.

mod bridge;
mod cdp;
mod input;
mod job;
mod launch;
mod page;
mod profile;
mod scripts;
mod target;

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde_json::Value;
use tokio::sync::Mutex;

use cdp::Connection;
use target::Tabs;

/// What the profile is using, for the settings page to draw.
///
/// The one thing about [`profile`] a caller outside this module needs, which is
/// why the module itself stays private: everything else it knows -- the folder's
/// name, the migration, the delete -- is the driver's business, and a command that
/// could reach the path directly is a command that could point it somewhere else.
pub use profile::Status as ProfileStatus;

/// How long the browser is kept after the last call, unless the user says
/// otherwise. See [`idle_limit`].
const IDLE_TIMEOUT: Duration = Duration::from_secs(10 * 60);

/// How often the browser is looked at to see whether anyone still wants it.
const REAP_INTERVAL: Duration = Duration::from_secs(60);

/// How long a browser that has been told to stop is waited on.
///
/// Long enough for a process tree to unwind and let go of its files, short enough
/// that a browser which refuses to die does not hold the caller -- which may be a
/// settings button, with a user watching it.
const STOP_GRACE: Duration = Duration::from_secs(10);

/// Where the general settings record lives, which holds the idle choice.
///
/// The same key the settings page writes, so the switch and the reaper cannot
/// disagree about what it says.
const GENERAL_SETTINGS: &str = "app.general";

/// The one browser this app drives.
///
/// Process-global, like [`crate::ai::tools::workspace`] and the MCP tool
/// snapshot, and for the same reason: a tool is a bare function with no app
/// handle and no database, so anything it needs has to be reachable without one.
///
/// One, not one per conversation. A second browser would be a second profile, a
/// second window, and a second copy of the logins -- for a capability that is
/// already serialised by this lock.
static BROWSER: Mutex<Option<Session>> = Mutex::const_new(None);

/// Where the browser's profile lives, given by the app at startup.
static APP_DATA: OnceLock<PathBuf> = OnceLock::new();

/// The app's database, for the one setting this module reads.
///
/// The reaper needs to know whether the user asked for the browser to be kept, and
/// a task that wakes on its own has no command behind it to be told. Reading the
/// setting here is what makes the switch in the settings page the thing that
/// decides, and it is why the timeout is re-read on every tick rather than cached.
static DATABASE: OnceLock<Arc<crate::database::Database>> = OnceLock::new();

/// The browser's process id, or zero when none is running.
///
/// Apart from the session on purpose. Shutting down at exit cannot wait on a lock
/// that a script may be holding, and killing by process id is the one operation
/// that works when nothing else may be awaited.
static PID: AtomicU32 = AtomicU32::new(0);

/// When the browser was last used, in milliseconds since the epoch.
static LAST_USED: AtomicU64 = AtomicU64::new(0);

/// Whether the idle reaper has been started, so it is started once.
static REAPER: AtomicBool = AtomicBool::new(false);

/// What one call to the browser produced.
#[derive(Debug)]
pub struct Run {
    /// The page, what the script did, and everything that happened while it ran.
    pub report: String,
    /// The viewport as PNG bytes, when a picture was asked for.
    pub screenshot: Option<Vec<u8>>,
    /// Whether the script stopped short of finishing.
    ///
    /// Carried rather than left to the caller to parse out of the report: the
    /// report contains the page either way, and the caller has to decide between
    /// a result and a failure without reading prose to work it out.
    pub failed: bool,
}

/// Tells the driver where its profile belongs.
///
/// From `setup`, beside `panel::browser::install`, so that a tool never has to
/// reach for an app handle it does not have. Two calls rather than one because the
/// two facts come from different places and are needed by different callers: the
/// path is what the browser cannot start without, while the database is only ever
/// read by the reaper, and the probe that drives a real browser has no database at
/// all.
pub fn install(app_data: PathBuf) {
    let _ = APP_DATA.set(app_data);
}

/// Tells the driver where to read whether the browser should be kept.
///
/// One setting, read on every reaper tick: the `Close when idle` choice on the
/// settings page. Absent -- which is every test, and any caller that forgets --
/// the default timeout applies, because a browser kept a little longer is a cost
/// and a browser kept forever is the thing the setting exists to prevent.
pub fn install_database(database: Arc<crate::database::Database>) {
    let _ = DATABASE.set(database);
}

/// What the browser's profile is using, for the settings page.
pub async fn profile_status() -> Result<profile::Status, String> {
    Ok(profile::status(&app_data()?).await)
}

/// Deletes the browser's profile: its cookies, its saved logins, its history.
///
/// The browser is stopped first, and not for tidiness: it holds its profile open
/// through a dozen processes, so a delete underneath it either fails outright or
/// leaves a folder that looks cleared and is not. The next call starts a new
/// browser on a new profile, which is the honest result of asking for this.
pub async fn forget_profile() -> Result<(), String> {
    if let Some(session) = BROWSER.lock().await.take() {
        session.stop().await;
    }
    profile::forget(&app_data()?).await
}

/// Runs one script in the browser, opening it first if it is not already open.
pub async fn run(
    code: &str,
    tab: Option<usize>,
    want_screenshot: bool,
    cancelled: &AtomicBool,
) -> Result<Run, String> {
    if code.trim().is_empty() {
        return Err("browser_exec needs a script to run".to_string());
    }
    let mut guard = BROWSER.lock().await;
    ensure(&mut guard).await?;
    let session = guard
        .as_mut()
        .ok_or_else(|| "the browser is not available".to_string())?;
    let outcome = session.drive(code, tab, want_screenshot, cancelled).await;
    // Touched whatever happened: a call that failed was still a call, and an idle
    // timer that only ran on success would close a browser in the middle of
    // somebody fixing a page.
    touch();
    outcome
}

/// Stops the browser, from a context that is allowed to block.
///
/// Called at exit, where nothing may be awaited. Chrome's own child processes are
/// its own concern, so the whole tree is killed by process id rather than the one
/// handle this app happens to hold.
pub fn terminate() {
    let pid = PID.swap(0, Ordering::SeqCst);
    if pid != 0 {
        kill_tree(pid);
    }
}

/// The browser, started if it is not running and replaced if it has died.
async fn ensure(slot: &mut Option<Session>) -> Result<(), String> {
    let running = match slot.as_mut() {
        Some(session) => !session.exited(),
        None => false,
    };
    if running {
        return Ok(());
    }
    // A browser that ended under us -- the user closed the window, or it crashed
    // -- is replaced rather than reported. The tabs it took with it were on the
    // user's own screen, so the loss is already visible; a call that refused to
    // work until something was restarted by hand would not be.
    if let Some(dead) = slot.take() {
        // Its own output is read before it is taken down: a browser that ended by
        // itself is the one failure that has an explanation, and this is the only
        // place that explanation exists.
        tracing::debug!("the browser exited: {}", dead.tail().await);
        dead.stop().await;
    }
    *slot = Some(Session::start().await?);
    Ok(())
}

/// A running browser and everything attached to it.
struct Session {
    child: tokio::process::Child,
    pid: u32,
    /// What the browser has printed, so a browser that dies under us can say why.
    log: launch::Log,
    conn: Arc<Connection>,
    tabs: Tabs,
}

impl Session {
    /// Starts a browser and attaches to its first tab.
    async fn start() -> Result<Self, String> {
        let binary = launch::browser_binary()?;
        // Created, and named, before the browser is asked to open it: a profile
        // the browser makes for itself is one whose name nothing set, which is
        // what left the window indistinguishable from a plain new Chrome.
        let profile = profile::ensure(&app_data()?).await?;
        let mut launched = launch::start(&binary, &profile).await?;
        PID.store(launched.pid, Ordering::SeqCst);
        // Placed in a job this process owns, so the browser dies with the app even
        // on the exits the handler never sees -- a crash, a force-quit, `Ctrl+C` in
        // dev. [`terminate`] still does the graceful path; this is the guarantee
        // behind it. See [`job`].
        job::adopt(launched.pid);

        // From here on, every failure has to take the browser down with it. A
        // window that opened and could not be reached is the worst of both: a
        // process nobody can drive, sitting in front of the user.
        let conn = match Connection::connect(&launched.websocket).await {
            Ok(conn) => conn,
            Err(error) => {
                stop_process(&mut launched);
                return Err(error);
            }
        };
        let tabs = match Tabs::attach_first(&conn).await {
            Ok(tabs) => tabs,
            Err(error) => {
                stop_process(&mut launched);
                return Err(error);
            }
        };

        start_reaper();
        Ok(Self {
            child: launched.child,
            pid: launched.pid,
            log: launched.log,
            conn,
            tabs,
        })
    }

    /// The browser's own recent output, for a report that has to explain it.
    async fn tail(&self) -> String {
        launch::tail(&self.log).await
    }

    /// Whether the browser process has ended.
    fn exited(&mut self) -> bool {
        matches!(self.child.try_wait(), Ok(Some(_)))
    }

    /// Takes the browser down.
    async fn stop(mut self) {
        // The tree first, then the handle this app holds: `start_kill` alone
        // leaves Chrome's renderer and GPU processes behind on Windows, and they
        // are most of the memory the browser was using.
        kill_tree(self.pid);
        let _ = self.child.start_kill();
        // Waited on rather than assumed, with a bound. A browser takes a moment to
        // let go of its files after being told to go, and the caller may be about
        // to delete the profile it is holding -- but a browser that will not die
        // must not hold the caller forever either.
        let _ = tokio::time::timeout(STOP_GRACE, self.child.wait()).await;
        PID.store(0, Ordering::SeqCst);
    }

    /// Runs one script against the tab in view.
    async fn drive(
        &mut self,
        code: &str,
        tab: Option<usize>,
        want_screenshot: bool,
        cancelled: &AtomicBool,
    ) -> Result<Run, String> {
        self.tabs.sync(&self.conn).await?;
        if self.tabs.is_empty() {
            return Err("the browser has no page open".to_string());
        }
        if let Some(index) = tab {
            if !self.tabs.select(index) {
                return Err(format!(
                    "there is no tab {index}. Open now: {}",
                    self.tabs.urls().join(", ")
                ));
            }
        }

        // The API, in place before the script needs it. A no-op on a page that
        // already has one, and the only thing that puts `pz` on a page that has
        // never seen this driver.
        self.page()?.inject().await?;

        let evaluation = self.page()?.evaluate(&wrap(code), cancelled).await;
        let failed = evaluation.value.is_err();

        // A script that ended by moving is not a script that failed: the page it
        // landed on is the answer. Waiting for it, and putting the API back, is
        // what makes the tree below belong to the page the script reached rather
        // than the one it left.
        if matches!(&evaluation.value, Err(error) if page::is_navigation(error)) {
            self.page()?.wait_for_load(page::LOAD_TIMEOUT).await;
            let _ = self.page()?.inject().await;
        }

        // A tab the script opened becomes the one it is working in: the page it
        // asked for is the page it wants to be on, and making it say so twice
        // would be a step spent on nothing.
        if let Some(opened) = evaluation.opened.last().cloned() {
            self.tabs.sync(&self.conn).await?;
            self.tabs.follow(&opened);
        }

        let screenshot = match want_screenshot {
            true => match self.page()?.screenshot().await {
                Ok(png) => Some(png),
                // A page being torn down cannot always be photographed, and that
                // is not a reason to lose everything else the call found out.
                Err(error) => {
                    self.page()?
                        .note(format!("a picture could not be taken: {error}"));
                    None
                }
            },
            false => None,
        };

        let (url, title) = self.page()?.address().await;
        let tree = self.page()?.tree().await;
        let notes = self.page()?.take_notes();
        let report = compose(
            &url,
            &title,
            &self.tabs.urls(),
            self.tabs.active_index(),
            &evaluation,
            &notes,
            &tree,
        );
        Ok(Run {
            report,
            screenshot,
            failed,
        })
    }

    /// The tab in view, or why there is none.
    fn page(&mut self) -> Result<&mut page::Page, String> {
        self.tabs
            .active()
            .ok_or_else(|| "the browser has no page open".to_string())
    }
}

/// The model's script, as the body of an async function.
///
/// Not a bare `eval`, which would also return the last expression: `return` is a
/// syntax error inside one, and a model that writes `return value` -- which is
/// most of them -- would get a parse failure for a script that was correct. One
/// rule, stated in the tool's own description, beats a clever one that has an
/// exception nobody can guess.
fn wrap(code: &str) -> String {
    format!("(async () => {{\n{code}\n}})()")
}

/// One result: where the page is, what the script did, and the page itself.
///
/// Built in one place, from the pieces, so a field cannot be left out of one path
/// and present in another. The order is the order a reader wants it in: where am
/// I, what happened, what does it look like.
fn compose(
    url: &str,
    title: &str,
    tabs: &[String],
    active: usize,
    evaluation: &page::Evaluation,
    notes: &[String],
    tree: &str,
) -> String {
    let mut lines = Vec::new();
    if url.is_empty() {
        lines.push("(the tab is showing nothing yet)".to_string());
    } else if title.is_empty() {
        lines.push(url.to_string());
    } else {
        lines.push(format!("{url} — {title}"));
    }

    // Only when there is a choice to make. A list of one is a line the model has
    // to read on every call to learn nothing.
    if tabs.len() > 1 {
        let listed = tabs
            .iter()
            .enumerate()
            .map(|(index, url)| {
                if index == active {
                    format!("[{index}] {url}")
                } else {
                    format!("{index} {url}")
                }
            })
            .collect::<Vec<_>>()
            .join(" | ");
        lines.push(format!("tabs: {listed}"));
    }

    match &evaluation.value {
        // A click resolves to `true` and a script that returns nothing resolves
        // to nothing. Neither is a fact about the page, and both would be a line
        // of noise on every single call.
        Ok(Value::Null) | Ok(Value::Bool(true)) => {}
        Ok(value) => lines.push(format!("returned: {value}")),
        Err(error) if page::is_navigation(error) => {
            lines.push("the page navigated".to_string());
        }
        Err(error) => lines.push(format!("script stopped: {error}")),
    }

    for line in &evaluation.console {
        lines.push(line.clone());
    }
    for note in notes {
        lines.push(note.clone());
    }

    lines.push(String::new());
    if tree.trim().is_empty() {
        lines.push("(this page has no readable content)".to_string());
    } else {
        lines.push(tree.to_string());
    }
    lines.join("\n")
}

/// The app's data directory, which the browser's profile sits under.
fn app_data() -> Result<PathBuf, String> {
    APP_DATA
        .get()
        .cloned()
        .ok_or_else(|| "the browser was not set up at startup".to_string())
}

/// Records that the browser was used just now.
fn touch() {
    LAST_USED.store(millis(), Ordering::SeqCst);
}

/// How long since the last call.
fn idle_for() -> Duration {
    match LAST_USED.load(Ordering::SeqCst) {
        0 => Duration::ZERO,
        last => Duration::from_millis(millis().saturating_sub(last)),
    }
}

fn millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|since| since.as_millis() as u64)
        .unwrap_or(0)
}

/// Closes the browser once nothing has used it for [`IDLE_TIMEOUT`].
///
/// Started once per process and never stopped, because the browser it is watching
/// is started and stopped repeatedly. The tick is a minute; the work is one lock
/// and one comparison.
fn start_reaper() {
    if REAPER.swap(true, Ordering::SeqCst) {
        return;
    }
    tauri::async_runtime::spawn(async move {
        loop {
            tokio::time::sleep(REAP_INTERVAL).await;
            // Read before the lock is taken, because the user may have asked for
            // the browser to be kept -- in which case there is nothing to do and
            // holding the lock to find that out would be rude to a call waiting on
            // it.
            let Some(limit) = idle_limit() else {
                continue;
            };
            let mut slot = BROWSER.lock().await;
            // Measured after the lock, not before: a call that started while this
            // was waiting has just touched the timer, and a browser in use is not
            // an idle one.
            if idle_for() < limit {
                continue;
            }
            if let Some(session) = slot.take() {
                tracing::debug!(
                    "closing the browser after {}s of not being used",
                    idle_for().as_secs()
                );
                session.stop().await;
            }
        }
    });
}

/// How long the browser is kept after the last call, or `None` to keep it.
///
/// Read on every tick rather than cached: the user can change this while the app
/// runs, and a reaper holding the value it started with would go on closing a
/// browser they had just asked to keep. An absent or unrecognised value is the
/// default, because the two mistakes are not the same size -- a timeout shorter
/// than asked for costs a browser start, and the other direction leaves a window
/// open, which is the thing this setting exists to prevent.
fn idle_limit() -> Option<Duration> {
    // Annotated because `Database::setting` is generic over what it deserializes,
    // and this one setting is read as the raw record it is stored as.
    let stored: Value = DATABASE.get()?.setting(GENERAL_SETTINGS)?;
    match stored.get("browserIdle").and_then(Value::as_str) {
        Some("never") => None,
        Some("30m") => Some(Duration::from_secs(30 * 60)),
        Some("1h") => Some(Duration::from_secs(60 * 60)),
        _ => Some(IDLE_TIMEOUT),
    }
}

/// Kills a browser and everything it started.
///
/// The process tree, not the process: Chrome is a dozen processes, and the one
/// this app spawned is the least of them by memory.
fn kill_tree(pid: u32) {
    #[cfg(target_os = "windows")]
    {
        let _ = std::process::Command::new("taskkill")
            .args(["/PID", &pid.to_string(), "/T", "/F"])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status();
    }
    #[cfg(not(target_os = "windows"))]
    {
        // `TERM` rather than `KILL` so the browser flushes its profile on the way
        // out, which is where the logins the agent was given are kept. Its child
        // processes exit with it.
        let _ = std::process::Command::new("kill")
            .args(["-TERM", &pid.to_string()])
            .status();
    }
}

/// Stops a browser that was started but cannot be used.
fn stop_process(launched: &mut launch::Launched) {
    kill_tree(launched.pid);
    let _ = launched.child.start_kill();
    PID.store(0, Ordering::SeqCst);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::browser::page::Evaluation;
    use serde_json::json;

    fn evaluation(value: Result<Value, String>) -> Evaluation {
        Evaluation {
            value,
            ..Evaluation::default()
        }
    }

    /// Where the page is, on the first line: everything else in the result is
    /// about that page, and a reader who does not know which one it is cannot use
    /// any of it.
    #[test]
    fn a_result_opens_with_the_page_it_is_about() {
        let report = compose(
            "https://example.com/a",
            "Example",
            &["https://example.com/a".to_string()],
            0,
            &evaluation(Ok(Value::Null)),
            &[],
            "[0]<a href=\"/b\">Next",
        );
        assert!(
            report.starts_with("https://example.com/a — Example\n"),
            "{report}"
        );
        assert!(report.contains("[0]<a href=\"/b\">Next"), "{report}");
    }

    /// The numbers of the page a reader has never seen are worth printing once.
    /// A list of one is a line that says nothing on every call.
    #[test]
    fn the_tab_list_appears_only_when_there_is_a_choice() {
        let one = compose(
            "https://a/",
            "A",
            &["https://a/".to_string()],
            0,
            &evaluation(Ok(Value::Null)),
            &[],
            "tree",
        );
        assert!(!one.contains("tabs:"), "{one}");

        let two = compose(
            "https://a/",
            "A",
            &["https://a/".to_string(), "https://b/".to_string()],
            1,
            &evaluation(Ok(Value::Null)),
            &[],
            "tree",
        );
        assert!(two.contains("tabs: 0 https://a/ | [1] https://b/"), "{two}");
    }

    /// A click resolves to `true` and a script that returns nothing resolves to
    /// nothing. Printing either would put a meaningless line on every call.
    #[test]
    fn a_value_that_says_nothing_is_not_printed() {
        for quiet in [Ok(Value::Null), Ok(Value::Bool(true))] {
            let report = compose(
                "https://a/",
                "A",
                &["https://a/".to_string()],
                0,
                &evaluation(quiet),
                &[],
                "tree",
            );
            assert!(!report.contains("returned:"), "{report}");
        }
    }

    #[test]
    fn what_the_script_returned_is_printed_when_there_is_something_to_say() {
        let report = compose(
            "https://a/",
            "A",
            &["https://a/".to_string()],
            0,
            &evaluation(Ok(json!("£12.00"))),
            &[],
            "tree",
        );
        assert!(report.contains("returned: \"£12.00\""), "{report}");
    }

    /// A navigation ended the script, which is not the script failing -- and the
    /// page that follows is the answer, so the wording must not read as an error.
    #[test]
    fn a_navigation_reads_as_an_outcome_and_a_throw_reads_as_a_failure() {
        let moved = compose(
            "https://b/",
            "B",
            &["https://b/".to_string()],
            0,
            &evaluation(Err("Inspected target navigated or closed".to_string())),
            &[],
            "tree",
        );
        assert!(moved.contains("the page navigated"), "{moved}");
        assert!(!moved.contains("script stopped"), "{moved}");

        let threw = compose(
            "https://a/",
            "A",
            &["https://a/".to_string()],
            0,
            &evaluation(Err("TypeError: pz.click is not a function".to_string())),
            &[],
            "tree",
        );
        assert!(threw.contains("script stopped: TypeError"), "{threw}");
    }

    #[test]
    fn console_lines_and_notes_sit_between_the_result_and_the_page() {
        let with_lines = Evaluation {
            value: Ok(Value::Null),
            console: vec!["console.error: 404 /missing.js".to_string()],
            ..Evaluation::default()
        };
        let report = compose(
            "https://a/",
            "A",
            &["https://a/".to_string()],
            0,
            &with_lines,
            &["alert: \"hi\" -- accepted for you".to_string()],
            "tree",
        );
        let console = report.find("404 /missing.js").expect("the console line");
        let note = report.find("accepted for you").expect("the note");
        let tree = report.find("tree").expect("the page");
        assert!(console < note && note < tree, "{report}");
    }

    /// A page with nothing to read is said to have nothing to read, rather than
    /// left as a blank tail a model would read as a truncated result.
    #[test]
    fn an_empty_page_says_so_rather_than_ending_in_nothing() {
        let report = compose(
            "about:blank",
            "",
            &["about:blank".to_string()],
            0,
            &evaluation(Ok(Value::Null)),
            &[],
            "",
        );
        assert!(report.contains("no readable content"), "{report}");
    }

    #[test]
    fn a_tab_with_no_address_is_described_rather_than_left_blank() {
        let report = compose("", "", &[], 0, &evaluation(Ok(Value::Null)), &[], "tree");
        assert!(
            report.starts_with("(the tab is showing nothing yet)"),
            "{report}"
        );
    }

    /// The body of the function, so both `await` and `return` are available. The
    /// wrapping is what the tool's description promises, so it is pinned here.
    #[test]
    fn a_script_becomes_the_body_of_an_async_function() {
        let wrapped = wrap("return await pz.text(3);");
        assert!(wrapped.starts_with("(async () => {"), "{wrapped}");
        assert!(wrapped.ends_with("})()"), "{wrapped}");
        assert!(wrapped.contains("return await pz.text(3);"), "{wrapped}");
    }

    /// A real browser, driven for real.
    ///
    /// Ignored, and skipped unless asked for, for the same reason the GGUF probe
    /// is: it starts a program, opens a window, and needs a browser installed.
    /// Everything that can be tested without one is tested above; what this covers
    /// is the part that cannot be -- that the protocol, the page tree and real input
    /// line up on an actual page.
    ///
    /// ```text
    /// $env:PROJECTZ_PROBE_BROWSER="1"; cargo test --lib browser -- --ignored --nocapture
    /// ```
    #[test]
    #[ignore = "starts a real browser window"]
    fn a_real_browser_can_be_driven() {
        if std::env::var("PROJECTZ_PROBE_BROWSER").is_err() {
            return;
        }
        install(std::env::temp_dir().join("projectz-browser-probe"));
        let profile_root = std::env::temp_dir().join("projectz-browser-probe");
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .expect("a test runtime");
        let cancelled = AtomicBool::new(false);

        runtime.block_on(async {
            // A page built from nothing, so this needs no network and no site.
            let built = run(
                "document.body.innerHTML = \
                   '<button id=\"b\">Press me</button><input id=\"i\" placeholder=\"Name\">'; \
                 window.__clicks = 0; \
                 document.getElementById('b').addEventListener('click', function () { \
                   window.__clicks += 1; }); \
                 return 'built';",
                None,
                false,
                &cancelled,
            )
            .await
            .expect("a browser should start and run this");
            assert!(!built.failed, "{}", built.report);
            assert!(
                built.report.contains("Press me"),
                "the tree is missing the button:\n{}",
                built.report
            );
            // Printed, because this is the test somebody runs by hand when the
            // driver looks wrong, and the tree is the thing they came to see.
            println!("\n--- what the driver saw ---\n{}\n", built.report);

            // The numbers are the token everything else acts on. A tree without
            // them reads perfectly well and cannot be used at all, which is the
            // kind of failure that survives a reading of the code.
            assert!(
                built.report.contains("[0]<button"),
                "the tree is not numbered:\n{}",
                built.report
            );

            // The profile, made before the browser was asked for it and named in
            // the browser's own state -- which is what the window's menu reads, and
            // what was missing when the profile could not be told apart from a
            // plain new Chrome.
            let profile_dir = profile::dir(&profile_root);
            let state = std::fs::read_to_string(profile_dir.join("Local State"))
                .expect("the browser's state file should be there");
            let state: serde_json::Value = serde_json::from_str(&state).expect("valid JSON");
            assert_eq!(
                state["profile"]["info_cache"]["Default"]["name"],
                profile::NAME,
                "the profile is not named in the browser's own state"
            );

            // A real click, through the browser's own input pipeline. The page's
            // own listener is what says it arrived, which is the whole point of
            // dispatching input rather than calling the element's `click`.
            let clicked = run(
                "const found = await pz.find('Press me'); \
                 if (!found.length) return 'the button was not found'; \
                 await pz.click(found[0].index); \
                 return window.__clicks;",
                None,
                false,
                &cancelled,
            )
            .await
            .expect("a click");
            assert!(!clicked.failed, "{}", clicked.report);
            assert!(
                clicked.report.contains("returned: 1"),
                "the click did not reach the page:\n{}",
                clicked.report
            );

            // Typing, which is a focus and an insertion rather than a run of key
            // events. The field's own value is what says it arrived.
            let typed = run(
                "const field = (await pz.find('Name'))[0]; \
                 await pz.type(field.index, 'hello'); \
                 return document.getElementById('i').value;",
                None,
                false,
                &cancelled,
            )
            .await
            .expect("typing");
            assert!(!typed.failed, "{}", typed.report);
            assert!(
                typed.report.contains("returned: \"hello\""),
                "the text did not land in the field:\n{}",
                typed.report
            );

            // The three ways a long page is read without reading all of it, on a
            // page built to have each shape: prose to search, a part to scope to,
            // and a shadow root that `querySelector` cannot see into at all.
            let shaped = run(
                "document.body.innerHTML = \
                   '<div id=\"pane\"><h3>Inbox</h3><p>Your delivery code is AB-1234.</p></div>' \
                   + '<div id=\"panel\"></div>'; \
                 const root = document.getElementById('panel').attachShadow({ mode: 'open' }); \
                 root.innerHTML = '<button id=\"inner\">Inside the shadow</button>'; \
                 return 'shaped';",
                None,
                false,
                &cancelled,
            )
            .await
            .expect("a page with each shape");
            assert!(!shaped.failed, "{}", shaped.report);

            // Searching the prose, which is the half of a page a tree is bad at.
            let found = run(
                "return await pz.findText('AB-1234');",
                None,
                false,
                &cancelled,
            )
            .await
            .expect("a search");
            assert!(
                found.report.contains("> Your delivery code is AB-1234."),
                "the search did not find the line:\n{}",
                found.report
            );

            // Scoping the tree to one part of the page. The shadow content is
            // elsewhere, and a scope that leaked would be a scope that did nothing.
            let scoped = run(
                "const tree = await pz.snapshot('#pane'); \
                 return { shows: tree.indexOf('Inbox') !== -1, leaked: tree.indexOf('Inside the shadow') !== -1 };",
                None,
                false,
                &cancelled,
            )
            .await
            .expect("a scoped tree");
            // Asked as a question the page answers, because the result also carries
            // the driver's own tree of the *whole* page -- which contains the shadow
            // button either way and would make an assertion on the text prove
            // nothing.
            assert!(
                scoped.report.contains("\"shows\":true"),
                "the scope showed nothing:\n{}",
                scoped.report
            );
            assert!(
                scoped.report.contains("\"leaked\":false"),
                "the scope leaked:\n{}",
                scoped.report
            );

            // And the shadow root is part of the page, because a component-built
            // app whose content the driver cannot see reads as an empty page --
            // which is worse for a model than a long one.
            let whole = run("return await pz.snapshot();", None, false, &cancelled)
                .await
                .expect("the whole tree");
            assert!(
                whole.report.contains("Inside the shadow"),
                "the shadow root is missing from the tree:\n{}",
                whole.report
            );

            // And the reading helpers see it too. `innerText` stops at a shadow
            // boundary, which is why a message body could be on the page and absent
            // from both `text()` and `findText()` -- the two things a model reaches
            // for before it thinks of walking a shadow root by hand.
            let searched = run(
                "const hits = await pz.findText('Inside the shadow'); \
                 return { hit: hits.indexOf('> Inside the shadow') !== -1 };",
                None,
                false,
                &cancelled,
            )
            .await
            .expect("a search into the shadow root");
            // A boolean computed in the page, not a substring of the report: the
            // report also carries the driver's own tree, and the tree contains the
            // same words with the same punctuation -- which is how this assertion
            // passed once while the collector was returning nothing.
            assert!(
                searched.report.contains("\"hit\":true"),
                "the search did not see into the shadow root:\n{}",
                searched.report
            );

            // One part of the page as text, which is the cheapest way to read a
            // value when the value is all that is wanted.
            let read = run("return await pz.text('#pane');", None, false, &cancelled)
                .await
                .expect("scoped text");
            assert!(read.report.contains("AB-1234"), "{}", read.report);

            // A frame *inside* a shadow root: the shape that made a message body
            // unreadable. `querySelectorAll('iframe')` finds nothing there, so the
            // text has to be collected through both boundaries.
            run(
                "document.body.innerHTML = '<div id=\"host\"></div>'; \
                 const host = document.getElementById('host').attachShadow({ mode: 'open' }); \
                 host.innerHTML = '<iframe id=\"f\"></iframe>'; \
                 host.querySelector('iframe').srcdoc = '<p>body text IFRAME-CODE-77</p>'; \
                 return 'framed';",
                None,
                false,
                &cancelled,
            )
            .await
            .expect("a framed page");
            let framed = run(
                "await pz.wait(600); \
                 const hits = await pz.findText('IFRAME-CODE-77'); \
                 return { hit: hits.indexOf('> body text IFRAME-CODE-77') !== -1 };",
                None,
                false,
                &cancelled,
            )
            .await
            .expect("a search into a frame");
            assert!(
                framed.report.contains("\"hit\":true"),
                "the search did not reach a frame inside a shadow root:\n{}",
                framed.report
            );

            // A long page keeps its prose. The result used to fall back to the
            // controls above 160 lines, which dropped exactly the text a reader
            // came for -- and a value the model never received is one it cannot
            // search for or reason about.
            run(
                "document.body.innerHTML = '<div>' + Array.from({length: 220}, function (_, i) { \
                   return '<p>filler line ' + i + '</p>'; }).join('') + '</div><p>reference SWORDFISH-42</p>'; \
                 return 'long';",
                None,
                false,
                &cancelled,
            )
            .await
            .expect("a long page");
            let long = run("return await pz.snapshot();", None, false, &cancelled)
                .await
                .expect("a long tree");
            assert!(
                long.report.contains("SWORDFISH-42"),
                "the prose was filtered out of the result:\n{}",
                long.report
            );

            // And a search names the actionable element that contains the text,
            // not only the text: a row is labelled by what is inside it, so this
            // is "find the row, then click it" in one call.
            run(
                "document.body.innerHTML = '<button id=\"row\"><span>Vahvista posti</span></button>'; \
                 return 'row';",
                None,
                false,
                &cancelled,
            )
            .await
            .expect("a row");
            let row = run("return await pz.find('Vahvista');", None, false, &cancelled)
                .await
                .expect("a search for the row");
            assert!(
                row.report.contains("\"label\":\"Vahvista posti\""),
                "the row was not named by its own text:\n{}",
                row.report
            );

            // The promise the whole profile exists for: what the browser stores in
            // one run is there in the next. Cookies do not exist on `about:blank`,
            // so this needs a real origin -- loopback, so still no network -- and
            // the browser is left the way quitting leaves it, because a profile
            // flushed by a graceful close is the only case a user ever sees.
            let origin = serve_probe_page().await;
            run(
                &format!("await pz.goto('{origin}'); return 'gone';"),
                None,
                false,
                &cancelled,
            )
            .await
            .expect("a navigation to the probe's own page");

            let stored = run(
                // With an expiry, because a session cookie is *supposed* to be gone
                // after the browser closes: what a profile keeps is a credential
                // with a lifetime, and that is the thing worth proving.
                "document.cookie = 'pz_probe=remembered; max-age=86400'; return document.cookie;",
                None,
                false,
                &cancelled,
            )
            .await
            .expect("setting a cookie");
            assert!(
                stored.report.contains("pz_probe=remembered"),
                "the cookie was not set:\n{}",
                stored.report
            );

            close_gracefully().await;
            run(
                &format!("await pz.goto('{origin}'); return 'gone';"),
                None,
                false,
                &cancelled,
            )
            .await
            .expect("a navigation after the restart");

            let remembered = run("return document.cookie;", None, false, &cancelled)
                .await
                .expect("reading the cookie back");
            assert!(
                remembered.report.contains("pz_probe=remembered"),
                "the profile did not survive the restart:\n{}",
                remembered.report
            );

            // An element that is gone is refused by name, rather than clicked at
            // whatever took its place.
            let stale = run("return await pz.click(9999);", None, false, &cancelled)
                .await
                .expect("a refusal");
            assert!(stale.failed, "clicking a missing element should fail");
            assert!(
                stale.report.contains("no longer on the page"),
                "the refusal does not say why:\n{}",
                stale.report
            );

            // A navigation, which ends the script that asked for it and is reported
            // as the outcome it is.
            let moved = run(
                "await pz.goto('about:blank'); return 'unreachable';",
                None,
                false,
                &cancelled,
            )
            .await
            .expect("a navigation");
            assert!(
                moved.report.contains("the page navigated"),
                "a navigation was not recognised:\n{}",
                moved.report
            );

            // A tab the script opened becomes the one it is on, and the result says
            // there is now a choice.
            let opened = run(
                "await pz.newTab('about:blank'); return 'opened';",
                None,
                false,
                &cancelled,
            )
            .await
            .expect("a new tab");
            assert!(
                opened.report.contains("tabs:"),
                "a second tab was not reported:\n{}",
                opened.report
            );

            // And forgetting it, which is what the settings button does: the
            // browser is stopped first, then the folder goes. A delete underneath a
            // running browser either fails or leaves a profile that looks cleared.
            forget_profile().await.expect("forgetting the profile");
            assert!(
                !profile::dir(&profile_root).exists(),
                "the profile survived being forgotten"
            );
        });

        terminate();
    }

    /// A one-page server on loopback, returning the origin to visit.
    ///
    /// The probe needs a real origin because a cookie cannot be set on
    /// `about:blank`, and a cookie is the only honest way to ask whether the
    /// profile remembered anything at all. Loopback rather than a site, so the
    /// probe still needs no network, and every request is answered the same way
    /// because the page it serves is not what is under test.
    async fn serve_probe_page() -> String {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("a loopback port");
        let port = listener.local_addr().expect("the bound address").port();
        tokio::spawn(async move {
            while let Ok((mut socket, _)) = listener.accept().await {
                let mut request = [0u8; 1024];
                let _ = socket.read(&mut request).await;
                let body = "<!doctype html><title>Probe</title><p>Probe page</p>";
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = socket.write_all(response.as_bytes()).await;
                let _ = socket.shutdown().await;
            }
        });
        format!("http://127.0.0.1:{port}/")
    }

    /// Closes the browser the way quitting does, so its profile is written out.
    ///
    /// Deliberately not [`terminate`], which is a hard kill. A profile is flushed
    /// as the browser leaves, so a test that proved persistence across a killed
    /// process would be testing something no user experiences -- and would pass or
    /// fail on when Chrome happened to commit its stores.
    async fn close_gracefully() {
        {
            let guard = BROWSER.lock().await;
            let Some(session) = guard.as_ref() else {
                return;
            };
            // The answer never arrives -- the browser is closing -- so the call is
            // made for its effect and its error is the expected one.
            let _ = session
                .conn
                .call("Browser.close", serde_json::json!({}), None)
                .await;
        }
        // Waited on rather than assumed: a browser that has been asked to close
        // still has to write its profile out, and the next call must find it gone
        // rather than racing the shutdown.
        for _ in 0..100 {
            tokio::time::sleep(Duration::from_millis(50)).await;
            let mut guard = BROWSER.lock().await;
            if !guard.as_mut().is_some_and(|session| !session.exited()) {
                return;
            }
        }
    }
}
