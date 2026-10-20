//! The socket the extension connects to, and the page it is set up from.
//!
//! # One port, two jobs, told apart by peeking
//!
//! The first thing on a new connection is decided without reading it: `peek` looks at
//! the request line and the headers while leaving them in the socket, so a WebSocket
//! upgrade is handed to the protocol implementation with its handshake intact, and
//! anything else is answered as HTTP by [`landing`]. Reading the head first and pushing
//! it back is not possible, and holding two ports would mean two URLs to explain -- one
//! for the extension and one for the person.
//!
//! # One connection, one browser profile
//!
//! Each profile that has the extension installed opens its own connection, and the app
//! treats them alike: an answer goes back down the connection that asked for it, so a
//! card is drawn by the browser that has the page. Nothing is broadcast except the switch
//! itself, which is one fact about the whole machine.
//!
//! # Why the handshake is the first *frame* and the origin check is the first *lock*
//!
//! They guard different things. The origin check runs during the HTTP upgrade and refuses
//! a connection a **page** opened -- the actual threat, since any site can reach
//! `127.0.0.1` and a watcher that answers any caller is a reader of the user's screen that
//! the web can drive. The token then runs as an ordinary frame, because it is the thing
//! the page hands over and can be replaced, and because a token check inside the upgrade
//! would mean an unparseable handshake on a wrong token rather than a sentence saying what
//! is wrong.
//!
//! # Why a page is answered in the loop that received it
//!
//! One browser's looks are answered in order, one at a time. Several profiles connect as
//! several tasks, so they do not wait on each other, and a single profile asking about two
//! pages at once is a user changing tabs -- serialised for them by being answered in the
//! order they arrived. The cost is that a slow model makes that one browser's next look
//! wait; the alternative, answering out of order, puts cards on a page in an order nobody
//! asked for.

use futures_util::{SinkExt, StreamExt};
use tauri::AppHandle;
use tokio::io::AsyncWriteExt;
use tokio::net::{TcpListener, TcpStream};
use tokio_tungstenite::tungstenite::handshake::server::{ErrorResponse, Request, Response};
use tokio_tungstenite::tungstenite::Message;

use super::{landing, CardFrame, ClientFrame, ServerFrame, ID, PORT, PROTOCOL};
use crate::assist::{Assist, AssistAnswer, AssistRequest};

/// How long to wait after a failed `accept` before listening again.
///
/// An accept failure on a listening socket is transient in practice -- a connection reset
/// between the SYN and the accept. Sleeping rather than breaking is what keeps one such
/// failure from ending the bridge for the rest of the session, which would look to the
/// user like the extension silently going deaf.
const ACCEPT_BACKOFF: std::time::Duration = std::time::Duration::from_millis(250);

/// How much of a request head is looked at while deciding what it is.
///
/// A browser's request line and headers for both a `GET` and an upgrade fit with room to
/// spare, and the bound is what keeps a connection that sends nothing from holding a
/// buffer open indefinitely.
const HEAD_LIMIT: usize = 8 * 1024;
/// How many times a connection is peeked at while its head arrives.
///
/// A head that arrives split across two packets is the case this covers; at 5ms a try, the
/// whole budget is a fifth of a second, after which the request is answered as HTTP so a
/// half-arrived request is never left hanging.
const HEAD_TRIES: usize = 40;
const HEAD_PAUSE: std::time::Duration = std::time::Duration::from_millis(5);

/// Listens for the extension and serves the setup page, for the life of the app.
pub async fn serve(assist: Assist, runtime: crate::ai::runtime::AiRuntime, app: AppHandle) {
    let listener = match TcpListener::bind(("127.0.0.1", PORT)).await {
        Ok(listener) => listener,
        // The preferred port is taken, which is normal before the app has settled (or when
        // something else on the machine holds it). Any free port will do: the one that was
        // used is reported on the status page and in Settings.
        Err(error) => {
            tracing::info!(%error, port = PORT, "the assist port is busy; taking any free one");
            match TcpListener::bind(("127.0.0.1", 0)).await {
                Ok(listener) => listener,
                Err(error) => {
                    assist.bridge_failed(&error.to_string());
                    return;
                }
            }
        }
    };
    let port = listener
        .local_addr()
        .map(|address| address.port())
        .unwrap_or(PORT);
    assist.bridge_ready(port);
    tracing::info!(port, "the assist bridge is listening");

    loop {
        match listener.accept().await {
            Ok((stream, _)) => {
                // A cheap clone: the handle is an `Arc` inside, so every connection shares
                // one memory and one switch.
                let assist = assist.clone();
                let runtime = runtime.clone();
                let app = app.clone();
                tauri::async_runtime::spawn(async move {
                    if let Err(error) = handle(stream, assist, runtime, app).await {
                        tracing::debug!(%error, "an assist connection ended");
                    }
                });
            }
            Err(error) => {
                tracing::warn!(%error, "the assist bridge could not accept a connection");
                tokio::time::sleep(ACCEPT_BACKOFF).await;
            }
        }
    }
}

/// One connection: a page, or an extension.
async fn handle(
    stream: TcpStream,
    assist: Assist,
    runtime: crate::ai::runtime::AiRuntime,
    app: AppHandle,
) -> Result<(), String> {
    let Some(head) = peek_head(&stream).await else {
        return Ok(());
    };
    if is_upgrade(&head) {
        return websocket(stream, assist, runtime, app).await;
    }
    answer_page(stream, &assist, &head).await
}

/// Answers a plain request with the page the app serves.
///
/// Split from [`handle`] so the whole HTTP half -- peek, decide, route, respond -- can be
/// exercised over a real socket without a browser or a window in the picture. That is the
/// half a person meets first, and the half where a mistake means a setup page that never
/// loads and a feature nobody can turn on.
async fn answer_page(mut stream: TcpStream, assist: &Assist, head: &str) -> Result<(), String> {
    let path = request_path(head).unwrap_or_else(|| "/".to_string());
    let response = landing::route(&path, assist);
    stream
        .write_all(&response.into_bytes())
        .await
        .map_err(|error| error.to_string())?;
    let _ = stream.shutdown().await;
    Ok(())
}

/// Reads the request head without consuming it.
///
/// `peek` is the whole trick: the bytes stay in the socket, so whatever the request turns
/// out to be is still there for the thing that answers it.
async fn peek_head(stream: &TcpStream) -> Option<String> {
    let mut buffer = vec![0u8; HEAD_LIMIT];
    let mut head = String::new();
    for _ in 0..HEAD_TRIES {
        match stream.peek(&mut buffer).await {
            // Closed before sending anything: a port scan, or a browser probing.
            Ok(0) => break,
            Ok(count) => {
                head = String::from_utf8_lossy(&buffer[..count]).into_owned();
                if head.contains("\r\n\r\n") {
                    return Some(head);
                }
            }
            Err(_) => break,
        }
        tokio::time::sleep(HEAD_PAUSE).await;
    }
    if head.is_empty() {
        None
    } else {
        Some(head)
    }
}

/// Whether a head is a WebSocket upgrade rather than a plain request.
///
/// Matched on the one header only the upgrade has, case-insensitively, because a browser
/// is free to spell it `Upgrade: WebSocket`.
fn is_upgrade(head: &str) -> bool {
    head.to_ascii_lowercase().contains("upgrade: websocket")
}

/// The path out of a request line, without the query string.
fn request_path(head: &str) -> Option<String> {
    let line = head.lines().next()?;
    let mut parts = line.split_whitespace();
    let _method = parts.next()?;
    let target = parts.next()?;
    Some(target.split(['?', '#']).next().unwrap_or("/").to_string())
}

/// Counts a paired connection while it lasts, so the settings page can say whether a
/// browser is attached.
///
/// A guard rather than paired calls because every exit path from the loop below -- a close
/// frame, a dead socket, a returned error -- has to decrement the same counter, and a path
/// that forgot to would leave the count permanently high.
struct Attached<'a>(&'a Assist);

impl Drop for Attached<'_> {
    fn drop(&mut self) {
        self.0.client_left();
    }
}

/// The extension's half: upgrade, pair, then answer what it asks.
async fn websocket(
    stream: TcpStream,
    assist: Assist,
    runtime: crate::ai::runtime::AiRuntime,
    app: AppHandle,
) -> Result<(), String> {
    let socket =
        tokio_tungstenite::accept_hdr_async(stream, |request: &Request, response: Response| {
            // Absent is allowed: a plain test client has no origin, and it still has to
            // produce the token. What is refused is a *page*: `http`/`https` is the shape a
            // website's own scripts arrive with.
            let from_a_page = request
                .headers()
                .get("origin")
                .and_then(|value| value.to_str().ok())
                .is_some_and(|origin| {
                    origin.starts_with("http://") || origin.starts_with("https://")
                });
            if from_a_page {
                tracing::warn!("a page tried to connect to the assist bridge");
                return Err(ErrorResponse::new(Some(
                    "a web page may not connect to this socket".to_string(),
                )));
            }
            Ok(response)
        })
        .await
        .map_err(|error| error.to_string())?;

    let (mut write, mut read) = socket.split();
    let mut updates = assist.subscribe();
    let mut attached: Option<Attached> = None;

    loop {
        tokio::select! {
            incoming = read.next() => {
                let Some(message) = incoming else { break };
                let text = match message {
                    Ok(Message::Text(text)) => text.to_string(),
                    Ok(Message::Close(_)) | Err(_) => break,
                    // Binary, ping, pong and control frames: this protocol carries JSON in
                    // text frames and nothing else, so anything else is stepped over
                    // rather than guessed at.
                    Ok(_) => continue,
                };
                let frame: ClientFrame = match serde_json::from_str(&text) {
                    Ok(frame) => frame,
                    Err(error) => {
                        send(&mut write, &ServerFrame::Error { message: format!("Unreadable frame: {error}") }).await?;
                        continue;
                    }
                };
                let reply = match frame {
                    ClientFrame::Hello { token, client } => {
                        if !assist.token_is_valid(&token) {
                            tracing::warn!("an assist connection offered the wrong token");
                            send(&mut write, &ServerFrame::Error {
                                message: "The pairing token does not match this app's. Open the app's status page and pair again.".to_string(),
                            }).await?;
                            // Closed rather than left open to try again: a wrong token is a
                            // wrong token, and letting a socket retry it forever is a
                            // guessing game.
                            break;
                        }
                        let profile = client.as_ref().and_then(|info| info.profile.clone());
                        let version = client.as_ref().and_then(|info| info.version.clone());
                        tracing::info!(?profile, ?version, "an assist connection paired");
                        assist.client_joined(profile);
                        attached = Some(Attached(&assist));
                        let settings = assist.settings();
                        ServerFrame::Welcome {
                            enabled: settings.enabled,
                            display: !settings.dry_run,
                            protocol: PROTOCOL,
                            // The limits and the spending ride the welcome because this is the
                            // first thing the extension ever hears: a tab that paired a second
                            // ago must not pre-filter with numbers of its own until the next
                            // frame arrives.
                            limits: assist.limits(),
                            usage: assist.usage(),
                        }
                    }
                    ClientFrame::Page { id, frame } => {
                        let blocks = frame.blocks.len();
                        let answer = assist.ask(&app, &runtime, AssistRequest {
                            frame,
                            source: ID.to_string(),
                        }).await;
                        // One read of the record for the two facts this reply needs, and the
                        // spending is read after the look rather than before it, because this
                        // frame reports what the look cost.
                        let settings = assist.settings();
                        let usage = assist.usage();
                        // In dry-run nothing is drawn, but the verdict still goes back:
                        // the point of the mode is to see what would have happened, and a
                        // silent socket would show the user nothing at all.
                        let display = !settings.dry_run;
                        match &answer {
                            AssistAnswer::Answered { cards, .. } => ServerFrame::Cards {
                                id,
                                stage: answer.stage().as_str(),
                                reason: String::new(),
                                screened: blocks_considered(&answer, blocks),
                                display,
                                usage,
                                cards: cards.iter().map(|card| CardFrame {
                                    index: card.index,
                                    key: card.key.clone(),
                                    answer: card.text.clone(),
                                    subject: card.subject.clone(),
                                    confidence: card.confidence,
                                    kind: card.kind.as_str(),
                                    shape: card.shape.as_str(),
                                }).collect(),
                            },
                            AssistAnswer::Skipped { reason, .. } => ServerFrame::Cards {
                                id,
                                stage: answer.stage().as_str(),
                                reason: reason.clone(),
                                screened: blocks_considered(&answer, blocks),
                                display: false,
                                usage,
                                cards: Vec::new(),
                            },
                            AssistAnswer::Failed { reason } => ServerFrame::Cards {
                                id,
                                stage: answer.stage().as_str(),
                                reason: reason.clone(),
                                screened: blocks,
                                display: false,
                                usage,
                                cards: Vec::new(),
                            },
                        }
                    }
                    ClientFrame::SetEnabled { enabled } => {
                        // The switch is answered on the broadcast below, to every connection
                        // at once: the tray, Settings and this popup are three doors into one
                        // fact, and each of them learns it the same way.
                        if let Err(error) = assist.set_enabled(&app, enabled) {
                            send(&mut write, &ServerFrame::Error { message: error }).await?;
                        }
                        continue;
                    }
                    // The frame is built by the handle rather than here, so a popup that has
                    // just opened and the tray item that has just been clicked read the state
                    // out of one description of it.
                    ClientFrame::Ping => assist.state_frame(),
                };
                send(&mut write, &reply).await?;
            }
            update = updates.recv() => {
                match update {
                    Ok(text) => {
                        if write.send(Message::Text(text.into())).await.is_err() { break }
                    }
                    // Lagged means an old state change was dropped, which the next one
                    // supersedes; Closed means the app is going away.
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                }
            }
        }
    }
    drop(attached);
    Ok(())
}

/// How many blocks were looked at, from whichever answer came back.
fn blocks_considered(answer: &AssistAnswer, fallback: usize) -> usize {
    match answer {
        AssistAnswer::Answered { screened, .. } => *screened,
        AssistAnswer::Skipped { screened, .. } => *screened,
        AssistAnswer::Failed { .. } => fallback,
    }
}

/// Sends one frame, reporting a dead socket as an error the caller can act on.
async fn send(
    write: &mut futures_util::stream::SplitSink<
        tokio_tungstenite::WebSocketStream<TcpStream>,
        Message,
    >,
    frame: &ServerFrame,
) -> Result<(), String> {
    let text = serde_json::to_string(frame).map_err(|error| error.to_string())?;
    write
        .send(Message::Text(text.into()))
        .await
        .map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    /// The two things that arrive on this port, told apart by the header only one of them
    /// has. Getting this wrong sends an extension's handshake to the HTML router, which
    /// answers 200 with a web page and leaves a socket that never pairs.
    #[test]
    fn a_request_is_told_apart_from_an_upgrade() {
        let get = "GET /status HTTP/1.1\r\nHost: 127.0.0.1:43128\r\n\r\n";
        assert!(!is_upgrade(get));
        assert_eq!(request_path(get).as_deref(), Some("/status"));

        let upgrade = "GET / HTTP/1.1\r\nHost: 127.0.0.1:43128\r\nUpgrade: websocket\r\nConnection: Upgrade\r\n\r\n";
        assert!(is_upgrade(upgrade));
        // A browser may spell the value however it likes.
        assert!(is_upgrade("GET / HTTP/1.1\r\nupgrade: WebSocket\r\n\r\n"));
    }

    /// The path is taken without its query string or fragment, because the page is opened
    /// with both when the extension hands the token over.
    #[test]
    fn the_path_drops_the_query_string() {
        let head = "GET /extension.zip?v=2 HTTP/1.1\r\nHost: x\r\n\r\n";
        assert_eq!(request_path(head).as_deref(), Some("/extension.zip"));
        let head = "GET /#pair HTTP/1.1\r\n\r\n";
        assert_eq!(request_path(head).as_deref(), Some("/"));
        // A request line that is not a request line must not panic a listener.
        assert_eq!(request_path("nonsense"), None);
    }

    /// Builds an assist handle backed by a throwaway database, and a server that answers
    /// exactly one request on a port the operating system picks.
    async fn one_request_server() -> (u16, tokio::task::JoinHandle<()>) {
        let database = Arc::new(crate::database::Database::open_in_memory());
        let assist = Assist::new(database, None);
        let listener = TcpListener::bind(("127.0.0.1", 0))
            .await
            .expect("an ephemeral port");
        let port = listener.local_addr().expect("an address").port();
        let task = tokio::spawn(async move {
            if let Ok((stream, _)) = listener.accept().await {
                // The real path, minus the branch that only a WebSocket takes: peek the
                // head without consuming it, then answer what it turns out to be.
                if let Some(head) = peek_head(&stream).await {
                    let _ = answer_page(stream, &assist, &head).await;
                }
            }
        });
        (port, task)
    }

    /// The peek, the decision and the response, run as one -- over a real socket, because
    /// the three only work together and each is wrong in a way the others cannot catch.
    #[tokio::test]
    async fn a_plain_request_gets_the_setup_page_over_a_real_socket() {
        let (port, server) = one_request_server().await;
        let mut client = TcpStream::connect(("127.0.0.1", port))
            .await
            .expect("connect");
        client
            .write_all(b"GET / HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n")
            .await
            .expect("write");
        let mut response = Vec::new();
        client.read_to_end(&mut response).await.expect("read");
        server.abort();
        let response = String::from_utf8(response).expect("utf-8");
        assert!(response.starts_with("HTTP/1.1 200 OK"), "{response}");
        assert!(response.contains("text/html"), "the page should be HTML");
        assert!(
            response.contains("Load it in Chrome"),
            "the page should carry its own instructions"
        );
    }

    /// The same socket, asked for the status, answers with the token the extension pairs
    /// with -- which is the whole point of the page existing.
    #[tokio::test]
    async fn the_status_answer_carries_the_pairing_token() {
        let (port, server) = one_request_server().await;
        let mut client = TcpStream::connect(("127.0.0.1", port))
            .await
            .expect("connect");
        client
            .write_all(b"GET /status HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n")
            .await
            .expect("write");
        let mut response = Vec::new();
        client.read_to_end(&mut response).await.expect("read");
        server.abort();
        let response = String::from_utf8(response).expect("utf-8");
        let body = response.split("\r\n\r\n").nth(1).expect("a body");
        let parsed: serde_json::Value = serde_json::from_str(body).expect("JSON");
        assert!(parsed["token"]
            .as_str()
            .is_some_and(|token| token.len() >= 32));
    }

    /// A head that arrives in two pieces still arrives: a browser is free to send the
    /// request line and the headers in separate packets, and treating that as two requests
    /// is a setup page that loads one time in three.
    #[tokio::test]
    async fn a_request_split_across_two_packets_is_still_one_request() {
        let (port, server) = one_request_server().await;
        let mut client = TcpStream::connect(("127.0.0.1", port))
            .await
            .expect("connect");
        client
            .write_all(b"GET /status HTTP/1.1\r\n")
            .await
            .expect("write");
        tokio::time::sleep(std::time::Duration::from_millis(30)).await;
        client
            .write_all(b"Host: 127.0.0.1\r\n\r\n")
            .await
            .expect("write");
        let mut response = Vec::new();
        client.read_to_end(&mut response).await.expect("read");
        server.abort();
        let response = String::from_utf8(response).expect("utf-8");
        assert!(response.starts_with("HTTP/1.1 200 OK"), "{response}");
    }
}
