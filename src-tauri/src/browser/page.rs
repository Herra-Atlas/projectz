//! One attached tab, and everything the driver does to it.
//!
//! A page is attached once and then kept: enabling the domains costs three round
//! trips, and a driver that re-attached per call would pay them on every step of
//! a run.
//!
//! # The one loop that matters
//!
//! [`Page::evaluate`] runs the model's script *and* keeps listening while it
//! runs. It has to: the script calls the bridge to ask for a click, that request
//! arrives as an event on this tab's queue, and a driver that was waiting only on
//! the script's answer would never see it. So the answer and the events are
//! raced, and whichever arrives is handled -- a click dispatched, a console line
//! kept, a dialog that would otherwise block the page answered.
//!
//! # Stopping
//!
//! The reply's cancel flag is polled while the script runs, and a cancelled run
//! is ended with `Runtime.terminateExecution` rather than abandoned. Without it,
//! a stopped reply would leave a script clicking through a page in the background
//! until the call timed out -- and the user would have been told it stopped.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use base64::Engine;
use serde_json::{json, Value};
use tokio::sync::mpsc;

use super::bridge;
use super::cdp::Connection;
use super::scripts;

/// How long a page is given to finish loading after a navigation.
///
/// A page that never fires its load event is not an error -- a feed that holds a
/// connection open, a single-page app that renders first -- so this ends the wait
/// rather than failing the call.
pub const LOAD_TIMEOUT: Duration = Duration::from_secs(20);

/// How often a running script checks whether the reply was stopped.
///
/// A quarter of a second: fast enough that the stop button feels like it worked,
/// rare enough that it costs nothing next to the script it is watching.
const CANCEL_POLL: Duration = Duration::from_millis(250);

/// How long a script may run before it is stopped and reported.
///
/// A script is the model's own code, and code can wait forever -- on a promise it
/// never resolves, on a page that never settles. Without this, that is the whole
/// run hanging with nothing on screen: the answer simply never arrives, and there
/// is no other deadline between here and the user's patience. Long enough for a
/// deliberate wait inside a script, short enough that a stuck one is reported
/// while the user is still watching it.
const SCRIPT_TIMEOUT: Duration = Duration::from_secs(180);

/// How many console lines and page events are kept per call.
///
/// A page left to its own devices can log without limit, and a model reading four
/// hundred lines of a site's own debugging has been handed noise instead of the
/// page. The first few are the ones that explain what the page is doing.
const MAX_CONSOLE_LINES: usize = 40;

/// How many events are handled at the start of a call, before starting the script.
///
/// What a page did while the driver was away, bounded so that a tab nobody is
/// reading cannot build a queue the next call then has to walk.
const MAX_BACKLOG: usize = 200;

/// One attached tab.
pub struct Page {
    conn: Arc<Connection>,
    /// The browser's id for this tab.
    pub target_id: String,
    /// What this tab was showing when the browser last listed it.
    ///
    /// A free fact -- it arrives with the target listing rather than costing a
    /// round trip -- and used to name the *other* tabs in a result. The tab the
    /// model is on has its address asked for properly; a tab it might switch to
    /// only has to be recognisable.
    pub url: String,
    /// The id of our attachment to it. Every call for this tab carries it.
    session_id: String,
    /// This tab's events.
    events: mpsc::UnboundedReceiver<Value>,
    /// What happened here that the script did not ask for.
    ///
    /// A dialog answered on the model's behalf, a page that never finished
    /// loading. Kept beside its tab rather than in a global, because a run with
    /// two tabs open should not be told about the wrong one's alerts.
    notes: Vec<String>,
}

/// What a script did, and what happened while it ran.
pub struct Evaluation {
    /// The value the script returned, or why it did not finish.
    ///
    /// A failure is *not* an absent evaluation: the page is still there to be
    /// read, and a model that is shown the page after its script failed can
    /// correct itself in one more call.
    pub value: Result<Value, String>,
    /// The page's own console output, and its failed requests.
    pub console: Vec<String>,
    /// Dialogs that were answered on the model's behalf.
    pub dialogs: Vec<String>,
    /// Tabs the script opened, newest last.
    pub opened: Vec<String>,
}

impl Default for Evaluation {
    fn default() -> Self {
        Self {
            // A script that has not run yet has not failed. `Ok` rather than an
            // error, so everything that builds one of these starts from "nothing
            // has gone wrong" and only says otherwise when it has.
            value: Ok(Value::Null),
            console: Vec::new(),
            dialogs: Vec::new(),
            opened: Vec::new(),
        }
    }
}

/// Attaches to one tab and prepares it to be driven.
pub async fn attach(conn: &Arc<Connection>, target_id: &str, url: &str) -> Result<Page, String> {
    let attached = conn
        .call(
            "Target.attachToTarget",
            json!({ "targetId": target_id, "flatten": true }),
            None,
        )
        .await?;
    let session_id = attached
        .get("sessionId")
        .and_then(Value::as_str)
        .ok_or("the browser would not attach to that tab")?
        .to_string();

    // Registered before anything is enabled, so no event can arrive before there
    // is somewhere to put it. The window between the two is small and real.
    let events = conn.route(&session_id).await;
    let page = Page {
        conn: Arc::clone(conn),
        target_id: target_id.to_string(),
        url: url.to_string(),
        session_id,
        events,
        notes: Vec::new(),
    };
    page.prepare().await?;
    Ok(page)
}

impl Page {
    /// Turns on the domains this driver needs, and installs the bridge.
    async fn prepare(&self) -> Result<(), String> {
        self.call("Page.enable", json!({})).await?;
        self.call("Runtime.enable", json!({})).await?;
        // Not required: a target that cannot report network failures is still
        // drivable, and a 404 the model cannot see is better than a tab that
        // would not open.
        let _ = self.call("Log.enable", json!({})).await;
        // The bridge, for this page and every page that replaces it: a binding
        // with no execution context named is installed into the global object of
        // each context, including the ones a later navigation creates. That is
        // what makes it survive moving to another page.
        self.call("Runtime.addBinding", json!({ "name": scripts::BINDING }))
            .await?;
        Ok(())
    }

    /// Puts the page API in place, if it is not already there.
    pub async fn inject(&self) -> Result<(), String> {
        let response = self
            .call(
                "Runtime.evaluate",
                json!({ "expression": scripts::injection(), "returnByValue": false }),
            )
            .await?;
        value_of(&response).map(|_| ())
    }

    /// Runs the model's script, answering anything it asks for while it runs.
    pub async fn evaluate(&mut self, script: &str, cancelled: &AtomicBool) -> Evaluation {
        let conn = Arc::clone(&self.conn);
        let session = self.session_id.clone();

        // What the page did while nobody was looking. Handled here so the queue
        // cannot grow behind a tab that is opened once and then left.
        let backlog: Vec<Value> = std::iter::from_fn(|| self.events.try_recv().ok())
            .take(MAX_BACKLOG)
            .collect();
        let mut evaluation = Evaluation::default();
        for event in backlog {
            handle(&conn, &session, event, &mut evaluation).await;
        }

        let mut answer = match conn
            .send(
                "Runtime.evaluate",
                json!({
                    "expression": script,
                    "awaitPromise": true,
                    "returnByValue": true,
                    // Everything this driver runs in a page is in response to a
                    // request the user approved. Without the flag, a page that
                    // checks whether input came from a person treats the script's
                    // own work as untrusted.
                    "userGesture": true,
                }),
                Some(&session),
            )
            .await
        {
            Ok(answer) => answer,
            Err(error) => {
                evaluation.value = Err(error);
                return evaluation;
            }
        };

        let mut ticker = tokio::time::interval(CANCEL_POLL);
        let deadline = Instant::now() + SCRIPT_TIMEOUT;
        evaluation.value = loop {
            tokio::select! {
                answered = &mut answer => break match answered {
                    Ok(Ok(response)) => value_of(&response),
                    Ok(Err(error)) => Err(error),
                    Err(_) => Err("the browser connection closed".to_string()),
                },
                event = self.events.recv() => match event {
                    Some(event) => handle(&conn, &session, event, &mut evaluation).await,
                    None => break Err("the tab closed while the script was running".to_string()),
                },
                _ = ticker.tick() => {
                    if cancelled.load(Ordering::Relaxed) {
                        // Stopping the script where it is. Without this, a reply the
                        // user stopped would keep running in the page until the call
                        // timed out, clicking whatever it had reached.
                        let _ = conn
                            .call("Runtime.terminateExecution", json!({}), Some(&session))
                            .await;
                        break Err("Cancelled".to_string());
                    }
                    if Instant::now() >= deadline {
                        // The last resort, and the reason it exists: a script that
                        // waits on something that never happens has no other end,
                        // and the run would wait there with it.
                        let _ = conn
                            .call("Runtime.terminateExecution", json!({}), Some(&session))
                            .await;
                        break Err(format!(
                            "the script was still running after {}s and was stopped",
                            SCRIPT_TIMEOUT.as_secs()
                        ));
                    }
                }
            }
        };

        self.notes.extend(evaluation.dialogs.iter().cloned());
        evaluation
    }

    /// Evaluates one expression with no script behind it, for the driver's own use.
    ///
    /// Used for the tree and the page's address, which the driver asks for itself
    /// rather than relying on the model to ask.
    pub async fn ask(&mut self, script: &str) -> Result<Value, String> {
        let conn = Arc::clone(&self.conn);
        let session = self.session_id.clone();
        let backlog: Vec<Value> = std::iter::from_fn(|| self.events.try_recv().ok())
            .take(MAX_BACKLOG)
            .collect();
        let mut scratch = Evaluation::default();
        for event in backlog {
            handle(&conn, &session, event, &mut scratch).await;
        }

        let response = self
            .call(
                "Runtime.evaluate",
                json!({ "expression": script, "returnByValue": true, "awaitPromise": true }),
            )
            .await?;
        let value = value_of(&response)?;

        // Anything that arrived while it ran still belongs to the model's picture
        // of the page, so a dialog raised by the page's own timer is not lost
        // because the driver happened to be the one looking.
        let late: Vec<Value> = std::iter::from_fn(|| self.events.try_recv().ok())
            .take(MAX_BACKLOG)
            .collect();
        let mut evaluation = Evaluation::default();
        for event in late {
            handle(&conn, &session, event, &mut evaluation).await;
        }
        self.notes.extend(evaluation.dialogs);
        Ok(value)
    }

    /// Waits for a page to finish arriving, after a navigation ended a script.
    pub async fn wait_for_load(&mut self, limit: Duration) -> bool {
        let conn = Arc::clone(&self.conn);
        let session = self.session_id.clone();
        let deadline = Instant::now() + limit;
        let mut landed = false;
        let mut scratch = Evaluation::default();

        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                break;
            }
            match tokio::time::timeout(remaining, self.events.recv()).await {
                Ok(Some(event)) => {
                    let method = method_of(&event);
                    if method == "Page.loadEventFired" || method == "Page.frameStoppedLoading" {
                        landed = true;
                        break;
                    }
                    handle(&conn, &session, event, &mut scratch).await;
                }
                // The queue ended, which means the tab went with it.
                Ok(None) => break,
                // Out of time. A page that never fires its load event is still a
                // page, so this is a note rather than a failure.
                Err(_) => {
                    self.notes
                        .push("the page had not finished loading when the wait ended".to_string());
                    break;
                }
            }
        }
        self.notes.extend(scratch.dialogs);
        landed
    }

    /// What this tab is showing, as `(url, title)`.
    ///
    /// One call for both, because they are always wanted together and a round trip
    /// each would be two for one answer.
    pub async fn address(&mut self) -> (String, String) {
        let value = self
            .ask("({ url: location.href, title: document.title })")
            .await
            .unwrap_or(Value::Null);
        (
            value
                .get("url")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            value
                .get("title")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
        )
    }

    /// The page as numbered text, or a note if it has no tree yet.
    ///
    /// `auto` rather than the full tree: on a real page the full tree is several
    /// screens of layout wrappers, and a model reading it spends everything it has
    /// on structure. Auto shows the whole thing when it is short, and the controls
    /// with a note when it is not -- see `walker.js`.
    ///
    /// Guarded rather than assumed: a page can be showing a document the API was
    /// never installed into -- a chrome error page, a PDF viewer -- and a missing
    /// tree is worth a sentence, not a failed call.
    pub async fn tree(&mut self) -> String {
        match self
            .ask("window.__pzWalk ? window.__pzWalk('auto') : ''")
            .await
        {
            Ok(Value::String(text)) => text,
            Ok(_) => String::new(),
            Err(error) => format!("the page could not be read: {error}"),
        }
    }

    /// A picture of the viewport, as PNG bytes.
    pub async fn screenshot(&self) -> Result<Vec<u8>, String> {
        let shot = self
            .call(
                "Page.captureScreenshot",
                json!({ "format": "png", "fromSurface": true, "captureBeyondViewport": false }),
            )
            .await?;
        let encoded = shot
            .get("data")
            .and_then(Value::as_str)
            .ok_or("the browser returned no picture")?;
        base64::engine::general_purpose::STANDARD
            .decode(encoded)
            .map_err(|error| format!("the browser's picture could not be read: {error}"))
    }

    /// Takes the notes recorded against this tab, leaving it with none.
    pub fn take_notes(&mut self) -> Vec<String> {
        std::mem::take(&mut self.notes)
    }

    /// Records something about this tab that the model should be told.
    ///
    /// Used by the driver for what it decided on the model's behalf -- a picture
    /// that could not be taken, a wait that ran out -- and by nothing else: a
    /// note is not a place for a tool to narrate itself.
    pub fn note(&mut self, text: String) {
        self.notes.push(text);
    }

    /// The session this tab's calls are addressed to.
    pub fn session(&self) -> &str {
        &self.session_id
    }

    async fn call(&self, method: &str, params: Value) -> Result<Value, String> {
        self.conn.call(method, params, Some(&self.session_id)).await
    }
}

/// Handles one event from a tab: a bridge request, a log line, a dialog.
///
/// Free of `Page` on purpose. It runs inside the select loop that is also holding
/// the tab's event queue, so it cannot be a method on the tab it belongs to.
async fn handle(conn: &Arc<Connection>, session: &str, event: Value, evaluation: &mut Evaluation) {
    let params = event.get("params").cloned().unwrap_or(Value::Null);
    match method_of(&event) {
        // The page asking for something only the driver can do.
        "Runtime.bindingCalled" => {
            let payload = params
                .get("payload")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            let request = match bridge::parse(&payload) {
                Ok(request) => request,
                Err(error) => {
                    keep(
                        evaluation,
                        format!("a page request could not be read: {error}"),
                    );
                    return;
                }
            };
            let moves = bridge::navigates(&request.op);
            let id = request.id;
            let outcome = bridge::dispatch(conn, session, &request).await;

            // The page is already leaving. Its promise died with the context, and
            // resolving it would be a call about a document that no longer exists.
            if moves {
                return;
            }

            if request.op == "newTab" {
                if let Ok(created) = &outcome {
                    if let Some(target) = created.get("targetId").and_then(Value::as_str) {
                        evaluation.opened.push(target.to_string());
                    }
                }
            }

            let (message, value) = match outcome {
                Ok(value) => (Value::Null, value),
                Err(error) => (Value::String(error), Value::Null),
            };
            // Settled with one call, so the script's `await` resumes with either
            // the answer or the reason there is none.
            let settle = format!(
                "window.__pzBridge.resolve({id}, {}, {})",
                serde_json::to_string(&message).unwrap_or_else(|_| "null".into()),
                serde_json::to_string(&value).unwrap_or_else(|_| "null".into())
            );
            if let Err(error) = conn
                .call(
                    "Runtime.evaluate",
                    json!({ "expression": settle, "returnByValue": false }),
                    Some(session),
                )
                .await
            {
                tracing::debug!(error = %error, "could not answer a page request");
            }
        }

        "Runtime.consoleAPICalled" => {
            let level = params.get("type").and_then(Value::as_str).unwrap_or("log");
            let text = params
                .get("args")
                .and_then(Value::as_array)
                .map(|args| {
                    args.iter()
                        .map(describe_value)
                        .collect::<Vec<_>>()
                        .join(" ")
                })
                .unwrap_or_default();
            if !text.trim().is_empty() {
                keep(
                    evaluation,
                    format!("console.{level}: {}", shorten(&text, 300)),
                );
            }
        }

        // An error the page threw on its own -- a timer, an event handler -- which
        // the script that was running may have had nothing to do with.
        "Runtime.exceptionThrown" => {
            let details = params
                .get("exceptionDetails")
                .cloned()
                .unwrap_or(Value::Null);
            keep(
                evaluation,
                format!("uncaught: {}", describe_exception(&details)),
            );
        }

        // A failed request, which is often the whole reason a page looks broken.
        "Log.entryAdded" => {
            let entry = params.get("entry").cloned().unwrap_or(Value::Null);
            let level = entry.get("level").and_then(Value::as_str).unwrap_or("info");
            if level == "error" || level == "warning" {
                let text = entry
                    .get("text")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                let source = entry.get("url").and_then(Value::as_str).unwrap_or_default();
                keep(
                    evaluation,
                    format!("{level}: {} {source}", shorten(text, 200))
                        .trim()
                        .to_string(),
                );
            }
        }

        // A dialog stops the page until somebody answers it, and the only
        // somebody here is the driver. Answered rather than left open, because a
        // dialog nobody closes is a run that hangs with nothing to show for it --
        // and reported, because accepting a confirmation on the user's behalf is
        // not something to do silently.
        "Page.javascriptDialogOpening" => {
            let kind = params
                .get("type")
                .and_then(Value::as_str)
                .unwrap_or("dialog");
            let message = params
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let _ = conn
                .call(
                    "Page.handleJavaScriptDialog",
                    json!({ "accept": true }),
                    Some(session),
                )
                .await;
            evaluation
                .dialogs
                .push(format!("{kind}: \"{message}\" -- accepted for you"));
        }

        // The browser detaching from the tab: it was closed, or it crashed. Said
        // plainly, because the next call's failure would otherwise look like the
        // driver's fault.
        "Inspector.detached" => {
            keep(
                evaluation,
                "the tab was detached from the browser".to_string(),
            );
        }

        _ => {}
    }
}

/// Adds a line to the console, up to the cap.
fn keep(evaluation: &mut Evaluation, line: String) {
    if evaluation.console.len() < MAX_CONSOLE_LINES {
        evaluation.console.push(line);
    } else if evaluation.console.len() == MAX_CONSOLE_LINES {
        evaluation
            .console
            .push("… the page logged more than is shown here".to_string());
    }
}

/// The event's name, which is what everything here matches on.
fn method_of(event: &Value) -> &str {
    event
        .get("method")
        .and_then(Value::as_str)
        .unwrap_or_default()
}

/// The value of an `Runtime.evaluate` response, or why there is none.
///
/// A remote object carries `value` only when the result can be written as JSON. A
/// function, a DOM node or `undefined` carries none, and `description` is the
/// most useful thing left to say about it -- returning nothing at all would make
/// an expression that worked look like one that failed.
fn value_of(response: &Value) -> Result<Value, String> {
    if let Some(details) = response.get("exceptionDetails") {
        return Err(describe_exception(details));
    }
    let object = response.get("result").cloned().unwrap_or(Value::Null);
    Ok(object
        .get("value")
        .cloned()
        .or_else(|| object.get("description").cloned())
        .unwrap_or(Value::Null))
}

/// The sentence worth showing from a thrown exception.
///
/// The stack is long and the first line is the message, so the rest of it is not
/// carried into a tool result. A page's own stack frames say nothing a model can
/// act on.
fn describe_exception(details: &Value) -> String {
    if let Some(exception) = details.get("exception") {
        if let Some(description) = exception.get("description").and_then(Value::as_str) {
            return first_line(description);
        }
        if let Some(value) = exception.get("value") {
            return value.to_string();
        }
    }
    details
        .get("text")
        .and_then(Value::as_str)
        .map(str::to_string)
        .unwrap_or_else(|| "the page threw an error".to_string())
}

/// One console argument, as text.
fn describe_value(argument: &Value) -> String {
    if let Some(value) = argument.get("value") {
        return match value {
            Value::String(text) => text.clone(),
            other => other.to_string(),
        };
    }
    argument
        .get("description")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string()
}

/// Whether a failed evaluation failed because the page moved.
///
/// A navigation destroys the JavaScript context a script is running in, so the
/// protocol reports the script as having died. It did not: the page went
/// somewhere, which is an outcome the model asked for, and it has to be told
/// apart from a script that genuinely threw. The wording is the protocol's own
/// and has been stable, which is why it is matched rather than a code.
pub fn is_navigation(error: &str) -> bool {
    let lowered = error.to_ascii_lowercase();
    lowered.contains("navigated or closed")
        || lowered.contains("execution context was destroyed")
        || lowered.contains("cannot find context")
}

fn first_line(text: &str) -> String {
    text.lines().next().unwrap_or(text).trim().to_string()
}

fn shorten(text: &str, limit: usize) -> String {
    let trimmed = text.trim();
    if trimmed.chars().count() <= limit {
        return trimmed.to_string();
    }
    let mut cut: String = trimmed.chars().take(limit).collect();
    cut.push('…');
    cut
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_returned_value_is_read_out_of_the_remote_object() {
        let response = json!({ "result": { "type": "string", "value": "hello" } });
        assert_eq!(value_of(&response).expect("a value"), json!("hello"));
    }

    /// An expression that evaluates to `undefined` still worked, and reporting it
    /// as a failure would send the model looking for a problem that is not there.
    #[test]
    fn an_undefined_result_is_not_a_failure() {
        let response = json!({ "result": { "type": "undefined" } });
        assert_eq!(value_of(&response).expect("no value"), Value::Null);
    }

    #[test]
    fn a_value_that_cannot_be_json_falls_back_to_its_description() {
        let response = json!({ "result": { "type": "function", "description": "function () {}" } });
        assert_eq!(
            value_of(&response).expect("a description"),
            json!("function () {}")
        );
    }

    /// The message is the first line; the stack below it is not something a model
    /// can act on, and carrying it would spend the result on frames.
    #[test]
    fn an_exception_is_reported_by_its_message_alone() {
        let response = json!({
            "result": { "type": "object", "subtype": "error" },
            "exceptionDetails": {
                "text": "Uncaught",
                "exception": {
                    "description": "Error: element [7] is no longer on the page\n    at <anonymous>:1:1\n    at more frames"
                }
            }
        });
        let error = value_of(&response).expect_err("an exception");
        assert!(
            error.contains("element [7] is no longer on the page"),
            "{error}"
        );
        assert!(
            !error.contains("at <anonymous>"),
            "the stack was carried: {error}"
        );
    }

    #[test]
    fn an_exception_with_no_description_still_says_something() {
        let details = json!({ "text": "Uncaught" });
        assert!(!describe_exception(&details).is_empty());
    }

    #[test]
    fn console_arguments_are_joined_as_text() {
        assert_eq!(describe_value(&json!({ "value": "a" })), "a");
        assert_eq!(describe_value(&json!({ "value": 7 })), "7");
        assert_eq!(
            describe_value(&json!({ "description": "HTMLDivElement" })),
            "HTMLDivElement"
        );
    }

    /// The cap has to leave a note rather than going quiet: a page that logged a
    /// thousand lines and shows forty should say that is what happened.
    #[test]
    fn the_console_is_capped_and_says_so() {
        let mut evaluation = Evaluation::default();
        for index in 0..MAX_CONSOLE_LINES + 5 {
            keep(&mut evaluation, format!("line {index}"));
        }
        assert_eq!(evaluation.console.len(), MAX_CONSOLE_LINES + 1);
        assert!(evaluation
            .console
            .last()
            .expect("a last line")
            .contains("more than"));
    }

    #[test]
    fn a_long_console_line_is_shortened() {
        let long = "x".repeat(1000);
        let cut = shorten(&long, 100);
        assert_eq!(cut.chars().count(), 101);
        assert!(cut.ends_with('…'));
    }

    #[test]
    fn a_page_event_is_recognised_by_the_method_it_carries() {
        assert_eq!(
            method_of(&json!({ "method": "Page.loadEventFired", "params": {} })),
            "Page.loadEventFired"
        );
        assert_eq!(method_of(&json!({})), "");
    }

    /// The distinction this exists for: a click that navigated is the model
    /// getting what it asked for, and a throw is not. Reporting the first as a
    /// failure would send it looking for a problem that does not exist.
    #[test]
    fn a_navigation_is_told_apart_from_a_real_failure() {
        for moved in [
            "Inspected target navigated or closed",
            "Execution context was destroyed.",
            "Cannot find context with specified id",
        ] {
            assert!(is_navigation(moved), "{moved} should read as a navigation");
        }
        for thrown in [
            "TypeError: pz.click is not a function",
            "Error: element [7] is no longer on the page",
            "SyntaxError: Unexpected token '}'",
        ] {
            assert!(
                !is_navigation(thrown),
                "{thrown} should not read as a navigation"
            );
        }
    }
}
