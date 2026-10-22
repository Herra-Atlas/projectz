//! One connection to one MCP server.
//!
//! # Two transports, one protocol
//!
//! A server is either a program the app starts -- `npx -y …`, a compiled binary
//! -- or a URL it calls. The first speaks newline-delimited JSON-RPC on the
//! child's stdin and stdout; the second speaks streamable HTTP, where a POST
//! carries one message and the answer comes back as JSON or as a short SSE
//! stream. Which one a server is, is decided by its command: `http://` or
//! `https://` is a URL, anything else is a program. There is no third field to
//! keep in step with the two.
//!
//! # The three exchanges
//!
//! `initialize` (once, then a `notifications/initialized`), `tools/list` and
//! `tools/call`. Everything else in the specification -- resources, prompts,
//! sampling -- is left alone, because a surface the frontend does not draw is a
//! surface the user cannot use.
//!
//! # Why it is hand-written
//!
//! The SDK would be a dependency, a second async runtime assumption and a version
//! to track, for a protocol whose client half is the three messages above. The
//! parts that are genuinely fiddly -- a notification arriving while another
//! request is in flight, a server that prints a banner before it speaks, a server
//! that has died since the last call, an answer that arrives as a stream rather
//! than as a body -- are handled once, here.

use std::process::Stdio as ProcessStdio;
use std::time::Duration;

use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, Lines};
use tokio::process::{Child, ChildStdin, ChildStdout};

use super::McpServer;

/// The protocol revision this client speaks.
///
/// Sent, not negotiated: a server that does not know it answers with the version
/// it does know, and the three methods used here have not changed between the
/// revisions that exist.
const PROTOCOL_VERSION: &str = "2024-11-05";

/// How long one exchange may take.
///
/// A server that has not answered a `tools/list` in this long is not slow, it is
/// stuck, and a run waiting on it holds a conversation open for nothing.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

/// One tool as the server described it.
#[derive(Clone, Debug, serde::Deserialize)]
pub struct RemoteTool {
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    /// The argument schema, passed through to the model unchanged. A server that
    /// omits it gets a permissive object, because refusing to offer a tool that
    /// forgot its schema would hide a working tool over a cosmetic fault.
    #[serde(default, rename = "inputSchema")]
    pub input_schema: Option<Value>,
}

/// A live connection, over whichever transport the server's command named.
pub struct Connection {
    transport: Transport,
}

enum Transport {
    /// A program the app started.
    Process(ProcessTransport),
    /// A URL the app calls.
    Http(HttpTransport),
}

impl Connection {
    /// Connects and completes the handshake.
    pub async fn start(server: &McpServer) -> Result<Self, String> {
        let mut transport = if is_url(&server.command) {
            Transport::Http(HttpTransport::new(server))
        } else {
            Transport::Process(ProcessTransport::start(server).await?)
        };
        transport
            .request(
                "initialize",
                json!({
                    "protocolVersion": PROTOCOL_VERSION,
                    "capabilities": {},
                    "clientInfo": { "name": "projectz", "version": env!("CARGO_PKG_VERSION") },
                }),
            )
            .await?;
        // A notification: no id, no answer. Sending it is what tells a server the
        // handshake is over and requests may follow.
        transport
            .notify("notifications/initialized", json!({}))
            .await?;
        Ok(Self { transport })
    }

    /// The tools the server offers.
    pub async fn list_tools(&mut self) -> Result<Vec<RemoteTool>, String> {
        let result = self.transport.request("tools/list", json!({})).await?;
        let tools = result.get("tools").cloned().unwrap_or_else(|| json!([]));
        serde_json::from_value(tools)
            .map_err(|error| format!("the tool list did not read back: {error}"))
    }

    /// Calls one tool and returns the text of its answer.
    pub async fn call_tool(&mut self, name: &str, arguments: Value) -> Result<String, String> {
        let result = self
            .transport
            .request(
                "tools/call",
                json!({ "name": name, "arguments": arguments }),
            )
            .await?;
        Ok(text_of(&result))
    }
}

impl Transport {
    async fn request(&mut self, method: &str, params: Value) -> Result<Value, String> {
        match self {
            Self::Process(transport) => transport.request(method, params).await,
            Self::Http(transport) => transport.request(method, params).await,
        }
    }

    async fn notify(&mut self, method: &str, params: Value) -> Result<(), String> {
        match self {
            Self::Process(transport) => transport.notify(method, params).await,
            Self::Http(transport) => transport.notify(method, params).await,
        }
    }
}

/// A child process and its pipes.
struct ProcessTransport {
    /// Dropped with the connection, and `kill_on_drop` is set on the command, so
    /// a server cannot outlive the app or a refresh that replaced it.
    _child: Child,
    stdin: ChildStdin,
    stdout: Lines<BufReader<ChildStdout>>,
    /// Increments per request. Ids only have to be unique within one connection.
    next_id: u64,
}

impl ProcessTransport {
    async fn start(server: &McpServer) -> Result<Self, String> {
        let mut child = spawn(server)?;
        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| "the server has no input pipe".to_string())?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| "the server has no output pipe".to_string())?;
        Ok(Self {
            _child: child,
            stdin,
            stdout: BufReader::new(stdout).lines(),
            next_id: 1,
        })
    }

    async fn request(&mut self, method: &str, params: Value) -> Result<Value, String> {
        let id = self.next_id;
        self.next_id += 1;
        self.send(&json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params,
        }))
        .await?;

        let answer = tokio::time::timeout(REQUEST_TIMEOUT, self.await_id(id))
            .await
            .map_err(|_| {
                format!(
                    "`{method}` did not answer within {}s",
                    REQUEST_TIMEOUT.as_secs()
                )
            })??;
        answer_of(method, answer)
    }

    async fn notify(&mut self, method: &str, params: Value) -> Result<(), String> {
        self.send(&json!({ "jsonrpc": "2.0", "method": method, "params": params }))
            .await
    }

    /// Writes one message, as a single line.
    async fn send(&mut self, message: &Value) -> Result<(), String> {
        let mut line = serde_json::to_string(message).map_err(|error| error.to_string())?;
        line.push('\n');
        self.stdin
            .write_all(line.as_bytes())
            .await
            .map_err(|error| format!("could not write to the server: {error}"))?;
        self.stdin
            .flush()
            .await
            .map_err(|error| format!("could not flush to the server: {error}"))
    }

    /// Reads until the answer with this id arrives.
    ///
    /// Everything else is skipped rather than treated as an error: a server may
    /// log to a notification, answer a request this client no longer waits for,
    /// or -- before it speaks the protocol -- print a banner. None of those say
    /// the call failed, and the one that matters is still coming.
    async fn await_id(&mut self, id: u64) -> Result<Value, String> {
        loop {
            let Some(line) = self
                .stdout
                .next_line()
                .await
                .map_err(|error| format!("could not read from the server: {error}"))?
            else {
                return Err("the server closed its output".to_string());
            };
            let Ok(message) = serde_json::from_str::<Value>(&line) else {
                continue;
            };
            if message.get("id").and_then(Value::as_u64) == Some(id) {
                return Ok(message);
            }
        }
    }
}

/// A URL, a credential and the session a streamable-HTTP server hands out.
struct HttpTransport {
    client: reqwest::Client,
    url: String,
    /// The bearer token, or empty.
    key: String,
    /// Returned by `initialize` and sent on every request after it, when the
    /// server issues one. Absent is normal for a stateless server.
    session: Option<String>,
    next_id: u64,
}

impl HttpTransport {
    fn new(server: &McpServer) -> Self {
        Self {
            client: reqwest::Client::new(),
            url: server.command.trim().to_string(),
            key: server.key.trim().to_string(),
            session: None,
            next_id: 1,
        }
    }

    async fn request(&mut self, method: &str, params: Value) -> Result<Value, String> {
        let id = self.next_id;
        self.next_id += 1;
        let body = self
            .post(&json!({
                "jsonrpc": "2.0",
                "id": id,
                "method": method,
                "params": params,
            }))
            .await?;

        let answer = messages_in(&body)
            .into_iter()
            .find(|message| message.get("id").and_then(Value::as_u64) == Some(id));
        let Some(answer) = answer else {
            return Err(format!("`{method}` was not answered"));
        };
        answer_of(method, answer)
    }

    async fn notify(&mut self, method: &str, params: Value) -> Result<(), String> {
        // The body is ignored. A notification is answered with 202 and nothing,
        // and a server that says more than that is saying it to nobody.
        self.post(&json!({ "jsonrpc": "2.0", "method": method, "params": params }))
            .await
            .map(|_| ())
    }

    /// Posts one message and returns the body.
    async fn post(&mut self, message: &Value) -> Result<String, String> {
        let mut request = self
            .client
            .post(&self.url)
            // Both are offered, because a server that can stream does and one
            // that cannot answers with a plain body. Asking for only one of them
            // is how a working server looks broken.
            .header("accept", "application/json, text/event-stream")
            .json(message);
        if !self.key.is_empty() {
            request = request.bearer_auth(&self.key);
        }
        if let Some(session) = &self.session {
            request = request.header("mcp-session-id", session.clone());
        }

        // One timeout around the whole exchange rather than a client-level one:
        // what has to be bounded is the answer, not either half of it.
        let (session, status, body) = tokio::time::timeout(REQUEST_TIMEOUT, async {
            let response = request
                .send()
                .await
                .map_err(|error| format!("could not reach the server: {error}"))?;
            let session = response
                .headers()
                .get("mcp-session-id")
                .and_then(|value| value.to_str().ok())
                .map(str::to_string);
            let status = response.status();
            let body = response
                .text()
                .await
                .map_err(|error| format!("could not read the answer: {error}"))?;
            Ok::<_, String>((session, status, body))
        })
        .await
        .map_err(|_| {
            format!(
                "the server did not answer within {}s",
                REQUEST_TIMEOUT.as_secs()
            )
        })??;

        if let Some(session) = session {
            self.session = Some(session);
        }
        if !status.is_success() {
            let detail = body.trim();
            return Err(if detail.is_empty() {
                format!("the server answered {status}")
            } else {
                format!("the server answered {status}: {detail}")
            });
        }
        Ok(body)
    }
}

/// Whether a server's command names a URL rather than a program.
fn is_url(command: &str) -> bool {
    let command = command.trim().to_ascii_lowercase();
    command.starts_with("http://") || command.starts_with("https://")
}

/// The credential as an environment variable, if there is one.
///
/// A bare value becomes `MCP_API_KEY` -- the name a server that needs a key
/// almost always reads -- and `NAME=value` sets that name instead, for one that
/// documents a different variable.
fn key_env(key: &str) -> Option<(String, String)> {
    let key = key.trim();
    if key.is_empty() {
        return None;
    }
    match key.split_once('=') {
        Some((name, value)) => Some((name.trim().to_string(), value.trim().to_string())),
        None => Some(("MCP_API_KEY".to_string(), key.to_string())),
    }
}

/// The JSON-RPC messages in a response body.
///
/// A streamable-HTTP server answers with either a plain JSON message or an SSE
/// stream whose `data:` lines carry them, and which one arrives is the server's
/// choice per request. Both are handled here rather than at the call site,
/// because a caller looking for its reply should not have to know which shape it
/// came in.
fn messages_in(body: &str) -> Vec<Value> {
    let body = body.trim();
    if body.is_empty() {
        return Vec::new();
    }
    if let Ok(value) = serde_json::from_str::<Value>(body) {
        return match value {
            Value::Array(messages) => messages,
            single => vec![single],
        };
    }
    body.lines()
        .filter_map(|line| line.strip_prefix("data:"))
        .filter_map(|payload| serde_json::from_str::<Value>(payload.trim()).ok())
        .collect()
}

/// The result of an answer, or the error it carried.
fn answer_of(method: &str, answer: Value) -> Result<Value, String> {
    if let Some(error) = answer.get("error") {
        return Err(error_text(error));
    }
    answer
        .get("result")
        .cloned()
        .ok_or_else(|| format!("`{method}` answered without a result"))
}

/// Starts the child process.
fn spawn(server: &McpServer) -> Result<Child, String> {
    let build = |program: String| {
        let mut command = tokio::process::Command::new(program);
        command
            .args(&server.args)
            .stdin(ProcessStdio::piped())
            .stdout(ProcessStdio::piped())
            // Inherited rather than piped: nothing reads a pipe we own, and a
            // server that logs one line too many would block on a full one
            // instead of answering. Inherited, its complaints land in the app's
            // own log, which is where someone diagnosing it would look.
            .stderr(ProcessStdio::inherit())
            // A server must not outlive the app. Without this a quit, a crash or
            // a reload leaves the process behind with no window to close it from.
            .kill_on_drop(true);
        // The app's own working directory is the source tree in a dev build, and a
        // server that writes a file to a relative path would put it there --
        // Playwright MCP's screenshots and traces are the case that surfaced. Give
        // the process a folder of its own, and point the system temp variables at
        // it so a temporary file the server makes lands in the same place. A user's
        // own `TMP` in the server's environment still wins, because it is applied
        // below.
        let scratch = super::scratch_dir(server);
        if std::fs::create_dir_all(&scratch).is_ok() {
            command
                .current_dir(&scratch)
                .env("TMPDIR", &scratch)
                .env("TMP", &scratch)
                .env("TEMP", &scratch);
        }
        for (name, value) in &server.env {
            command.env(name, value);
        }
        if let Some((name, value)) = key_env(&server.key) {
            command.env(name, value);
        }
        // A server is usually a console program; without this, starting one opens
        // a window over the app and closing it kills the server.
        crate::child::hide_console_async(&mut command);
        command
    };

    match build(server.command.clone()).spawn() {
        Ok(child) => Ok(child),
        // On Windows a bare `npx` is `npx.cmd`, and `Command` does not search
        // PATHEXT. Trying the shim is what makes `npx -y …`, the commonest MCP
        // server of all, start without the user knowing the difference.
        Err(error) if cfg!(windows) && error.kind() == std::io::ErrorKind::NotFound => {
            build(format!("{}.cmd", server.command))
                .spawn()
                .map_err(|retry| {
                    format!(
                        "could not start `{}`: {error} (and `{}.cmd`: {retry})",
                        server.command, server.command
                    )
                })
        }
        Err(error) => Err(format!("could not start `{}`: {error}", server.command)),
    }
}

/// The text of a `tools/call` result.
///
/// MCP answers with a list of content blocks rather than a string. Text blocks
/// are joined; anything else is described by its kind, so an image or a resource
/// link reaches the model as "the server returned an image" rather than as
/// nothing at all.
fn text_of(result: &Value) -> String {
    let mut parts: Vec<String> = Vec::new();
    if let Some(blocks) = result.get("content").and_then(Value::as_array) {
        for block in blocks {
            match block.get("type").and_then(Value::as_str) {
                Some("text") => {
                    if let Some(text) = block.get("text").and_then(Value::as_str) {
                        parts.push(text.to_string());
                    }
                }
                Some(kind) => parts.push(format!("[the server returned {kind}]")),
                None => {}
            }
        }
    }
    // A server may answer with structured content and no blocks.
    if parts.is_empty() {
        if let Some(structured) = result.get("structuredContent") {
            parts.push(structured.to_string());
        }
    }
    if result
        .get("isError")
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        parts.insert(0, "the server reported an error:".to_string());
    }
    if parts.is_empty() {
        return "the server returned nothing".to_string();
    }
    parts.join("\n")
}

/// A JSON-RPC error object as one line.
fn error_text(error: &Value) -> String {
    let message = error
        .get("message")
        .and_then(Value::as_str)
        .unwrap_or("the server refused the call");
    match error.get("code").and_then(Value::as_i64) {
        Some(code) => format!("{message} ({code})"),
        None => message.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_blocks_are_joined() {
        let result = json!({ "content": [
            { "type": "text", "text": "first" },
            { "type": "text", "text": "second" },
        ]});
        assert_eq!(text_of(&result), "first\nsecond");
    }

    /// An image has no text, and reporting nothing would read as a tool that ran
    /// and found nothing -- which is a different fact from "it sent a picture".
    #[test]
    fn a_non_text_block_is_described_rather_than_dropped() {
        let result = json!({ "content": [{ "type": "image", "data": "…" }] });
        assert_eq!(text_of(&result), "[the server returned image]");
    }

    #[test]
    fn structured_content_is_used_when_there_are_no_blocks() {
        let result = json!({ "structuredContent": { "rows": 3 } });
        assert_eq!(text_of(&result), "{\"rows\":3}");
    }

    #[test]
    fn a_flagged_error_says_so() {
        let result = json!({
            "isError": true,
            "content": [{ "type": "text", "text": "no such file" }],
        });
        assert!(text_of(&result).starts_with("the server reported an error"));
    }

    #[test]
    fn an_error_object_becomes_one_line_with_its_code() {
        let error = json!({ "code": -32601, "message": "Method not found" });
        assert_eq!(error_text(&error), "Method not found (-32601)");
    }

    /// What decides which transport a server gets, since it is one field rather
    /// than a setting the user has to keep in step with it.
    #[test]
    fn a_url_command_picks_the_http_transport() {
        assert!(is_url("http://localhost:8931/mcp"));
        assert!(is_url("  HTTPS://example.com/mcp  "));
        assert!(!is_url("npx"));
        assert!(!is_url("C:\\tools\\server.exe"));
    }

    /// A bare key becomes the variable a server almost always reads, and the
    /// `NAME=value` form is how a user points at one it does not.
    #[test]
    fn a_key_becomes_an_environment_variable() {
        assert_eq!(
            key_env("sk-abc"),
            Some(("MCP_API_KEY".to_string(), "sk-abc".to_string()))
        );
        assert_eq!(
            key_env("GITHUB_TOKEN=ghp_x"),
            Some(("GITHUB_TOKEN".to_string(), "ghp_x".to_string()))
        );
        assert_eq!(key_env("   "), None);
    }

    /// The answer to a POST arrives as JSON or as a short SSE stream, and the
    /// caller should not have to know which.
    #[test]
    fn a_body_is_read_as_json_or_as_a_stream() {
        let plain = messages_in(r#"{"jsonrpc":"2.0","id":1,"result":{}}"#);
        assert_eq!(plain.len(), 1);
        assert_eq!(plain[0]["id"], 1);

        // A batch is a list of messages, not one.
        assert_eq!(messages_in(r#"[{"id":1},{"id":2}]"#).len(), 2);

        let streamed =
            messages_in("event: message\ndata: {\"jsonrpc\":\"2.0\",\"id\":7,\"result\":{}}\n\n");
        assert_eq!(streamed.len(), 1);
        assert_eq!(streamed[0]["id"], 7);

        // A stream may carry more than the reply, so everything parseable is kept
        // and the caller picks its own id out of that.
        let noisy =
            messages_in("data: {\"method\":\"notifications/progress\"}\ndata: {\"id\":9}\n");
        assert_eq!(noisy.len(), 2);
        assert_eq!(noisy[1]["id"], 9);

        assert!(messages_in("").is_empty());
        // Neither shape is no messages, not an error: the caller reports the
        // missing answer, which says more than "this was not JSON".
        assert!(messages_in("<html>not mcp</html>").is_empty());
    }

    /// A result is unwrapped and an error is reported, whichever transport the
    /// answer came from.
    #[test]
    fn an_answer_is_the_result_or_the_error() {
        assert_eq!(
            answer_of("tools/list", json!({ "result": { "tools": [] } })).expect("result"),
            json!({ "tools": [] })
        );
        let error =
            answer_of("tools/call", json!({ "error": { "message": "no" } })).expect_err("error");
        assert_eq!(error, "no");
        assert!(answer_of("tools/call", json!({ "id": 1 })).is_err());
    }
}
