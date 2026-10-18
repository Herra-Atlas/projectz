//! The Chrome DevTools Protocol, over one WebSocket.
//!
//! # What this is
//!
//! Chrome exposes everything it can do -- navigating, running JavaScript, real
//! mouse and keyboard input, screenshots -- as JSON messages on a WebSocket:
//! `{id, method, params, sessionId}` out, `{id, result}` or an event back.
//!
//! There is no client library here for the same reason there is none in
//! [`crate::mcp::client`]: the part of the protocol this app uses is a handful of
//! messages, and the awkward parts are the same three every time -- an event
//! arriving between a call and its answer, a call aimed at one tab among several,
//! and a socket that closes with calls still in flight. They are handled once,
//! here, so no other file has to think about them.
//!
//! # Calls, sessions, events
//!
//! Every call carries an increasing `id` and its answer is matched back to the
//! waiter through [`Connection::pending`]. A *session* is an attached tab; its
//! events carry `sessionId` and are queued for that tab alone, so one page's
//! `Page.loadEventFired` can never be read as another's.
//!
//! # Why the reader is its own task
//!
//! Calls and events share the socket, so something must be reading it at all
//! times: an event nobody consumes stays in the buffer and eventually stalls
//! every call behind it. One task owns the receiving half and routes by id, which
//! is also what keeps a reply from being read twice.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use futures_util::stream::{SplitSink, SplitStream};
use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Value};
use tokio::net::TcpStream;
use tokio::sync::{mpsc, oneshot, Mutex};
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};

/// How long a call may take before it is reported as unanswered.
///
/// Generous, because `Runtime.evaluate` legitimately waits on the page: a script
/// that scrolls, waits, then reads can take seconds. It exists so that a wedged
/// renderer fails the call instead of holding the run open for good.
const CALL_TIMEOUT: Duration = Duration::from_secs(120);

type Socket = WebSocketStream<MaybeTlsStream<TcpStream>>;
/// The sending half, shared one call at a time.
type Sink = SplitSink<Socket, Message>;
/// What a waiter is handed: the call's `result`, or the protocol's error text.
type Waiter = oneshot::Sender<Result<Value, String>>;

/// One WebSocket to a browser, with in-flight calls and per-session event queues.
pub struct Connection {
    /// Sending half. Behind a lock because a frame must go out whole; the lock is
    /// held for the write and not for the answer, so two calls in flight do not
    /// serialise on each other's round trip.
    out: Mutex<Sink>,
    /// Calls waiting for an answer, by id.
    pending: Arc<Mutex<HashMap<u64, Waiter>>>,
    /// Where each attached session's events are queued.
    routes: Arc<Mutex<HashMap<String, mpsc::UnboundedSender<Value>>>>,
    next_id: AtomicU64,
}

impl Connection {
    /// Opens the socket and starts the reader.
    pub async fn connect(url: &str) -> Result<Arc<Self>, String> {
        let (socket, _response) = tokio_tungstenite::connect_async(url)
            .await
            .map_err(|error| format!("Could not reach the browser at {url}: {error}"))?;
        let (out, source) = socket.split();
        let pending: Arc<Mutex<HashMap<u64, Waiter>>> = Arc::new(Mutex::new(HashMap::new()));
        let routes = Arc::new(Mutex::new(HashMap::new()));
        tokio::spawn(read(source, Arc::clone(&pending), Arc::clone(&routes)));
        Ok(Arc::new(Self {
            out: Mutex::new(out),
            pending,
            routes,
            // Zero is never used, so the first call is 1 and an id is never seen
            // twice -- a reused id would resolve somebody else's waiter.
            next_id: AtomicU64::new(1),
        }))
    }

    /// Sends one call and waits for its answer.
    pub async fn call(
        &self,
        method: &str,
        params: Value,
        session: Option<&str>,
    ) -> Result<Value, String> {
        let answer = self.send(method, params, session).await?;
        tokio::time::timeout(CALL_TIMEOUT, answer)
            .await
            .map_err(|_| format!("{method} did not answer within {}s", CALL_TIMEOUT.as_secs()))?
            .map_err(|_| "The browser connection closed".to_string())?
    }

    /// Sends one call and returns the waiter, without waiting on it.
    ///
    /// The half of [`Connection::call`] that a page needs: a script may ask the
    /// driver to do something *while it is running* (a real click, a typed key),
    /// and that request arrives as an event on the same socket. Waiting on the
    /// answer here would mean nobody was listening for it.
    pub async fn send(
        &self,
        method: &str,
        params: Value,
        session: Option<&str>,
    ) -> Result<oneshot::Receiver<Result<Value, String>>, String> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let (sender, answer) = oneshot::channel();
        self.pending.lock().await.insert(id, sender);

        let mut message = json!({ "id": id, "method": method, "params": params });
        if let Some(session) = session {
            message["sessionId"] = Value::String(session.to_string());
        }
        if let Err(error) = self
            .out
            .lock()
            .await
            .send(Message::Text(message.to_string().into()))
            .await
        {
            // The waiter is removed rather than left to time out: nothing is
            // coming, and a dangling entry is a leak per failed send.
            self.pending.lock().await.remove(&id);
            return Err(format!("The browser connection closed: {error}"));
        }
        Ok(answer)
    }

    /// Registers a queue for one attached session's events.
    ///
    /// Called before the session is enabled, so no event can arrive before there
    /// is somewhere to put it.
    pub async fn route(&self, session: &str) -> mpsc::UnboundedReceiver<Value> {
        let (sender, receiver) = mpsc::unbounded_channel();
        self.routes.lock().await.insert(session.to_string(), sender);
        receiver
    }

    /// Forgets a session's queue, once its target is gone.
    pub async fn unroute(&self, session: &str) {
        self.routes.lock().await.remove(session);
    }
}

/// Reads the socket until it closes: answers go to their waiter, events to their
/// session's queue, and anything else is dropped.
async fn read(
    mut source: SplitStream<Socket>,
    pending: Arc<Mutex<HashMap<u64, Waiter>>>,
    routes: Arc<Mutex<HashMap<String, mpsc::UnboundedSender<Value>>>>,
) {
    while let Some(message) = source.next().await {
        let text = match message {
            Ok(Message::Text(text)) => text.to_string(),
            // Chrome frames in text, but a binary frame carrying UTF-8 JSON is
            // read the same way rather than dropped on a technicality.
            Ok(Message::Binary(bytes)) => match String::from_utf8(bytes.to_vec()) {
                Ok(text) => text,
                Err(_) => continue,
            },
            // Pings are answered by the library and pongs carry nothing.
            Ok(_) => continue,
            Err(error) => {
                tracing::debug!(error = %error, "the browser socket ended with an error");
                break;
            }
        };

        let Ok(value) = serde_json::from_str::<Value>(&text) else {
            continue;
        };

        if let Some(id) = value.get("id").and_then(Value::as_u64) {
            let waiter = pending.lock().await.remove(&id);
            if let Some(waiter) = waiter {
                let outcome = match value.get("error") {
                    // The protocol's own wording is the most useful description of
                    // what went wrong, so it is passed through rather than
                    // replaced with a second sentence about the same failure.
                    Some(error) => Err(protocol_error(error)),
                    None => Ok(value.get("result").cloned().unwrap_or(Value::Null)),
                };
                let _ = waiter.send(outcome);
            }
            continue;
        }

        // An event. Routed when it names a session this connection knows;
        // browser-level events (`Target.targetCreated`) have no consumer here and
        // are dropped rather than queued for nobody.
        if let Some(session) = value.get("sessionId").and_then(Value::as_str) {
            let route = routes.lock().await.get(session).cloned();
            if let Some(route) = route {
                let _ = route.send(value);
            }
        }
    }

    // The socket is over. Dropping every waiter wakes each caller with the same
    // report instead of leaving them to run out their own long timeouts.
    pending.lock().await.clear();
}

/// The text of a protocol error object.
///
/// `message` is the sentence worth showing to a model; `data` often repeats it,
/// so it is only appended when it says something new.
fn protocol_error(error: &Value) -> String {
    let message = error
        .get("message")
        .and_then(Value::as_str)
        .unwrap_or("the browser reported an error");
    match error.get("data").and_then(Value::as_str) {
        Some(data) if !data.is_empty() && !message.contains(data) => {
            format!("{message} ({data})")
        }
        _ => message.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_protocol_error_keeps_the_message_and_adds_what_is_new() {
        let error = json!({ "code": -32000, "message": "Target closed", "data": "no such tab" });
        assert_eq!(protocol_error(&error), "Target closed (no such tab)");
    }

    /// Chrome's evaluate failures often carry the same sentence twice, and a
    /// doubled sentence reads as two separate problems.
    #[test]
    fn data_already_contained_in_the_message_is_not_repeated() {
        let error = json!({
            "code": -32000,
            "message": "Inspected target navigated or closed",
            "data": "Inspected target navigated or closed"
        });
        let text = protocol_error(&error);
        assert_eq!(text.matches("navigated").count(), 1, "{text}");
    }

    #[test]
    fn an_error_without_a_message_still_produces_one() {
        assert!(!protocol_error(&json!({ "code": -32000 })).is_empty());
    }

    /// The message shape Chrome reads: `sessionId` is present only for a call
    /// aimed at an attached tab, and absent for browser-level ones.
    #[test]
    fn a_message_is_addressed_to_a_session_only_when_given_one() {
        let mut message = json!({ "id": 1, "method": "Page.enable", "params": {} });
        assert!(message.get("sessionId").is_none());
        message["sessionId"] = Value::String("S1".into());
        assert_eq!(message["sessionId"], "S1");
    }
}
