//! What a page script is allowed to ask the driver to do.
//!
//! The prelude does everything it can inside the page. The things it cannot --
//! real input, and navigation -- arrive here as requests, one per call to the
//! bridge, and this is the one place that decides what they mean.
//!
//! # Why the page does not get to name a target
//!
//! Every request says *what* to do; none of them says *which browser* or *which
//! debugger endpoint*. A page script that could name those would be a page script
//! that could drive a browser someone else attached to, so the session is chosen
//! by the driver and handed in, and a request that carries one is ignored.
//!
//! # The one refusal worth stating
//!
//! `goto` accepts `http`, `https` and `about:blank`, and nothing else. A `file:`
//! URL rendered in this browser would put the contents of the user's disk into a
//! page the model then reads -- a way around the workspace boundary every other
//! tool respects, and one no approval prompt would make visible, because the
//! prompt would be about `browser_exec` and not about the file. The panel's own
//! address bar refuses the same schemes for the same reason.
//!
//! # Requests are answered, never left hanging
//!
//! Whatever happens -- an unknown action, a bad argument, a refusal -- the caller
//! gets an `Err`, and the driver hands it to the page as a rejected promise. A
//! request that produced no answer would leave the model's script waiting until
//! the run's timeout, with nothing to tell it why.

use std::sync::Arc;

use serde_json::{json, Value};

use super::cdp::Connection;
use super::input;

/// Every action the page may send.
///
/// Held against the prelude by a test: a name here with nothing behind it, or an
/// action in the page that nothing answers, is caught before it can become a tool
/// that silently does nothing.
pub const OPS: &[&str] = &[
    "back",
    "click",
    "forward",
    "goto",
    "hover",
    "insertText",
    "key",
    "newTab",
    "reload",
    "wheel",
];

/// One bridge request, as the page sent it.
#[derive(Debug)]
pub struct Request {
    /// The page's own id for this call, echoed back when it is answered.
    pub id: u64,
    /// Which action.
    pub op: String,
    /// Its arguments.
    pub args: Value,
}

/// Reads a request out of the payload the binding carried.
///
/// The payload is a string because that is what a binding carries; it is JSON the
/// page produced, so it is parsed defensively and a malformed one is reported
/// rather than assumed.
pub fn parse(payload: &str) -> Result<Request, String> {
    let value: Value = serde_json::from_str(payload)
        .map_err(|error| format!("the page sent a request that is not JSON: {error}"))?;
    let id = value
        .get("id")
        .and_then(Value::as_u64)
        .ok_or("the page sent a request with no id")?;
    let op = value
        .get("op")
        .and_then(Value::as_str)
        .ok_or("the page sent a request naming no action")?
        .to_string();
    Ok(Request {
        id,
        op,
        args: value.get("args").cloned().unwrap_or_else(|| json!({})),
    })
}

/// Whether this action ends the script by moving the page.
///
/// A navigation destroys the JavaScript context the script is running in, so the
/// script stops there and never sees an answer. The driver knows this and treats
/// it as the outcome it is -- the page landed somewhere -- rather than as a
/// failure to report.
pub fn navigates(op: &str) -> bool {
    matches!(op, "goto" | "back" | "forward" | "reload")
}

/// Performs one request against one attached tab.
pub async fn dispatch(
    conn: &Arc<Connection>,
    session: &str,
    request: &Request,
) -> Result<Value, String> {
    match request.op.as_str() {
        // Input. The coordinates were computed in the page, from the element's
        // own box, because that is the only place that knows how a page is laid
        // out right now.
        "click" => {
            let count = request
                .args
                .get("count")
                .and_then(Value::as_u64)
                .unwrap_or(1);
            input::click(
                conn,
                session,
                number(&request.args, "x")?,
                number(&request.args, "y")?,
                count as usize,
            )
            .await
        }
        "hover" => {
            input::hover(
                conn,
                session,
                number(&request.args, "x")?,
                number(&request.args, "y")?,
            )
            .await
        }
        "wheel" => {
            input::wheel(
                conn,
                session,
                number(&request.args, "x")?,
                number(&request.args, "y")?,
                request
                    .args
                    .get("dx")
                    .and_then(Value::as_f64)
                    .unwrap_or(0.0),
                request
                    .args
                    .get("dy")
                    .and_then(Value::as_f64)
                    .unwrap_or(0.0),
            )
            .await
        }
        "insertText" => input::insert_text(conn, session, &text(&request.args, "text")?).await,
        "key" => input::key(conn, session, &text(&request.args, "key")?).await,

        // Moving. The URL is checked before anything is asked of the browser, so
        // a scheme that is not allowed is refused by name rather than turning
        // into a page that loaded and should not have.
        "goto" => {
            let url = text(&request.args, "url")?;
            check_scheme(&url)?;
            conn.call("Page.navigate", json!({ "url": url }), Some(session))
                .await?;
            Ok(json!(true))
        }
        "back" => step(conn, session, -1).await,
        "forward" => step(conn, session, 1).await,
        "reload" => {
            conn.call(
                "Page.reload",
                json!({ "ignoreCache": false }),
                Some(session),
            )
            .await?;
            Ok(json!(true))
        }

        // A new tab is a target, which is a browser-level thing: it is asked of
        // the connection rather than of this tab's session. The id comes back so
        // the driver can hand the new tab to the model as the page it is now on.
        "newTab" => {
            let url = request
                .args
                .get("url")
                .and_then(Value::as_str)
                .unwrap_or("about:blank");
            check_scheme(url)?;
            let created = conn
                .call("Target.createTarget", json!({ "url": url }), None)
                .await?;
            Ok(json!({ "targetId": created.get("targetId").cloned().unwrap_or(Value::Null) }))
        }

        other => Err(format!(
            "`{other}` is not an action this driver has. It has: {}",
            OPS.join(", ")
        )),
    }
}

/// Moves one entry through the tab's own history.
///
/// Read from the browser rather than from a list kept here: the page may have
/// navigated itself, redirected, or run a script that pushed a history entry, and
/// only the browser's own history knows about all three.
async fn step(conn: &Arc<Connection>, session: &str, direction: i64) -> Result<Value, String> {
    let history = conn
        .call("Page.getNavigationHistory", json!({}), Some(session))
        .await?;
    let current = history
        .get("currentIndex")
        .and_then(Value::as_i64)
        .unwrap_or(0);
    let target = current + direction;
    let entry = history
        .get("entries")
        .and_then(Value::as_array)
        .and_then(|entries| entries.get(target.max(0) as usize))
        .ok_or_else(|| {
            if direction < 0 {
                "there is no earlier page in this tab's history".to_string()
            } else {
                "there is no later page in this tab's history".to_string()
            }
        })?;
    let entry_id = entry
        .get("id")
        .and_then(Value::as_i64)
        .ok_or("the browser did not describe its own history entry")?;
    conn.call(
        "Page.navigateToEntry",
        json!({ "entryId": entry_id }),
        Some(session),
    )
    .await?;
    Ok(json!(true))
}

/// Refuses a navigation this browser has no business making.
fn check_scheme(url: &str) -> Result<(), String> {
    let trimmed = url.trim();
    if trimmed.eq_ignore_ascii_case("about:blank") {
        return Ok(());
    }
    match trimmed.split_once("://") {
        Some(("http" | "https", _)) => Ok(()),
        Some((other, _)) => Err(format!(
            "only http and https pages can be opened, not `{other}:`"
        )),
        // A URL with no scheme at all is a typo rather than a scheme to refuse,
        // and saying which one it is turns a mystery into a fix.
        None => Err(format!("`{url}` is not a URL: it needs an https:// prefix")),
    }
}

/// A required number argument.
fn number(args: &Value, field: &str) -> Result<f64, String> {
    args.get(field)
        .and_then(Value::as_f64)
        .ok_or_else(|| format!("this action needs a numeric `{field}`"))
}

/// A required string argument.
fn text(args: &Value, field: &str) -> Result<String, String> {
    args.get(field)
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| format!("this action needs a string `{field}`"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_request_is_read_out_of_the_payload_the_page_sent() {
        let request = parse(r#"{"id":7,"op":"click","args":{"x":10,"y":20}}"#).expect("a request");
        assert_eq!(request.id, 7);
        assert_eq!(request.op, "click");
        assert_eq!(request.args["x"], 10);
    }

    #[test]
    fn a_request_with_no_arguments_reads_as_empty_rather_than_failing() {
        let request = parse(r#"{"id":1,"op":"back"}"#).expect("a request");
        assert!(request.args.as_object().expect("an object").is_empty());
    }

    #[test]
    fn a_malformed_request_is_reported_by_name() {
        assert!(parse("not json").expect_err("malformed").contains("JSON"));
        assert!(parse(r#"{"op":"back"}"#).expect_err("no id").contains("id"));
        assert!(parse(r#"{"id":1}"#)
            .expect_err("no action")
            .contains("action"));
    }

    /// A page may move itself, so a navigation is an outcome rather than a
    /// failure -- but only these four actually end the script.
    #[test]
    fn only_the_moving_actions_end_the_script() {
        for op in ["goto", "back", "forward", "reload"] {
            assert!(navigates(op), "{op} should be an ending");
        }
        for op in ["click", "key", "insertText", "wheel", "hover", "newTab"] {
            assert!(!navigates(op), "{op} should not be an ending");
        }
    }

    /// `file:` is the one that matters. Rendered in this browser it would put the
    /// user's disk in front of the model, past the workspace boundary every other
    /// tool keeps -- and past a prompt that names the tool, not the file.
    #[test]
    fn only_web_pages_may_be_navigated_to() {
        for allowed in [
            "http://example.com",
            "https://example.com/a?b=c",
            "about:blank",
        ] {
            assert!(check_scheme(allowed).is_ok(), "{allowed} should be allowed");
        }
        for refused in [
            "file:///C:/Users/secret.txt",
            "file:///etc/passwd",
            "javascript:alert(1)",
            "data:text/html,<h1>x",
            "chrome://settings",
        ] {
            let error = check_scheme(refused).expect_err("refused");
            assert!(
                error.contains("http") || error.contains("not a URL"),
                "{refused}: {error}"
            );
        }
    }

    #[test]
    fn a_url_with_no_scheme_is_told_what_it_is_missing() {
        let error = check_scheme("example.com").expect_err("no scheme");
        assert!(error.contains("https://"), "{error}");
    }

    #[test]
    fn a_missing_argument_names_the_field_it_wanted() {
        assert!(number(&json!({}), "x").expect_err("missing").contains("x"));
        assert!(text(&json!({}), "key")
            .expect_err("missing")
            .contains("key"));
    }

    /// The list is what the page is told it may ask for, so a duplicate or a
    /// mis-sorted entry is a misleading error message rather than a crash.
    #[test]
    fn the_action_list_is_sorted_and_free_of_repeats() {
        let mut sorted = OPS.to_vec();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(sorted, OPS.to_vec());
    }
}
