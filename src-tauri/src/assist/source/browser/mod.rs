//! Reading the browser the *user* is looking at.
//!
//! # Why this needs an extension at all
//!
//! The agent's browser is driven over the DevTools protocol (`browser/`) because this
//! app starts it, and a browser started with a debugging port can be attached to. A
//! browser the user opened themselves is a different thing entirely: it has no
//! debugging port, one cannot be opened on a process already running, and a recent
//! Chrome refuses the flag on a default profile regardless. There is no version of
//! this that reaches into someone's own browser from the outside -- so the browser
//! reaches out instead, and that is the whole reason the extension exists.
//!
//! It also solves the drawing half. A card the extension puts in the page lives *in*
//! the page: it scrolls with the paragraph it belongs to, it is clipped by the same
//! window, and it can be anchored to the exact element it is about. A window drawn on
//! top by the app could do none of that.
//!
//! # Why the transport is a socket rather than anything else
//!
//! An extension cannot open a TCP connection, and it should not be given a
//! native-messaging host -- that is a program on the user's machine launched by the
//! browser, a much larger thing to install and to trust. A WebSocket back to
//! `127.0.0.1` is the smallest door that works, it needs nothing new in the dependency
//! list (`tokio-tungstenite` is already here for the agent's browser), and it is
//! reachable only from this machine.
//!
//! # Why this port also serves a page
//!
//! Setting an extension up by hand is the worst part of anything like this: load
//! unpacked, find a token, paste it somewhere, hope. So the same port answers a plain
//! `GET` with a page that does the whole job -- it says whether the extension is
//! installed and paired, hands over the download, and carries the token in the page
//! for the extension to read once it is loaded. Opening one URL is the entire setup,
//! and the token never has to be typed.
//!
//! # Why a token, when the socket is already local
//!
//! Local is not private. Any page the user visits can open a socket to `127.0.0.1` and
//! talk to whatever answers, so a watcher listening on a fixed port with no handshake
//! is a reader of the user's screen that any website can drive. The token is what makes
//! the socket *this extension's*: it is generated on this machine and handed over by
//! the page the app itself serves. The `Origin` check on the handshake is the second
//! lock, and the third is that the status page sends no CORS header -- so a foreign
//! page cannot read the token out of a response, even though it can ask for one.

pub mod bridge;
pub mod landing;

use serde::{Deserialize, Serialize};

use super::Frame;

/// Settings key holding the bridge's pairing token.
pub const RECORD_KEY: &str = "app.assist.bridge";

/// The port the extension and the setup page are both served on.
///
/// A fixed preferred port rather than an ephemeral one, because the extension has to
/// be configured with somewhere to connect *before* it can be told otherwise: a port
/// chosen at random each launch would mean re-pairing every launch. The next port up
/// from the agent's browser -- `ai::local` holds 43127 -- and a launch that cannot have
/// it falls back to any free port and reports the one it got, which the page then
/// shows.
pub const PORT: u16 = 43128;

/// The version of the frame protocol below.
///
/// Bumped when a frame changes shape, so an extension built against an older one can
/// say which version it speaks instead of failing on a field it cannot read.
///
/// - 2: cards carry the shape of the block they answer.
/// - 3: the app tells the extension the limits it must apply before sending, and what
///   this hour has cost so far. Additive, so a 2-built extension still works -- it just
///   keeps pre-filtering with numbers of its own, which is the bug that made this a
///   frame rather than a comment.
/// - 4: blocks carry a key and cards carry it back, so a card is placed by *what* it was
///   about rather than by where that was in a frame the page has since changed.
/// - 5: blocks carry a structural hint (`code`, `table`, `image`, `heading`) and cards carry
///   the kind of help they are (`answer`, `explain`, `summarize`, `translate`, `check`).
///   Additive in both directions: an older extension ignores the hint and draws the kind's
///   word in the place it already used for the shape.
pub const PROTOCOL: u32 = 5;

/// The source id, used on the wire and in settings.
///
/// A constant rather than a trait method's return value, because the bridge has to name the
/// source on frames it builds and it has no reason to hold a value to do it.
pub const ID: &str = "browser";

/// What the extension says about itself when it connects.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct ClientInfo {
    /// The Chrome profile it is running in. Shown so a user with several can tell which
    /// of them is paired.
    #[serde(default)]
    pub profile: Option<String>,
    #[serde(default)]
    pub version: Option<String>,
}

/// A message from the extension.
#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum ClientFrame {
    /// The first frame on every connection.
    Hello {
        token: String,
        #[serde(default)]
        client: Option<ClientInfo>,
    },
    /// A look at a page: every block the extension could read, numbered by it.
    ///
    /// One frame for the whole page rather than one per block, because the page is what a
    /// person is looking at and its questions belong together -- and because the numbering is
    /// what lets the answer come back addressed to a block.
    Page {
        /// The extension's own id for this look, echoed on the answer so a card can be matched
        /// to the page state it was drawn for.
        id: String,
        frame: Frame,
    },
    /// The extension's own switch, which is the same switch as the tray's.
    SetEnabled { enabled: bool },
    /// A liveness check that also re-reads the state.
    Ping,
}

/// One card, addressed to a block.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CardFrame {
    /// The block this answers, as the extension numbered it.
    pub index: usize,
    /// The block's identity, as the extension named it.
    ///
    /// The number is a position in the frame that was sent, and a live page moves positions: an
    /// answer that arrives after something appeared above it would name a paragraph that is not the
    /// one it was about. The extension places by this and drops an answer whose key it cannot find,
    /// which turns "drawn over the wrong words" into "not drawn at all".
    pub key: String,
    pub answer: String,
    pub subject: String,
    pub confidence: f32,
    /// The kind of help this is, for the card's own title.
    pub kind: &'static str,
    /// The gate's word for the shape, for the card's own subtitle.
    pub shape: &'static str,
}

/// What the app will read, so the extension can pre-filter with its numbers.
///
/// **The point of this frame is that there is one floor, not two.** The extension reads a page
/// many times more often than the app sees one, so it must not send what the app would refuse
/// -- but it used to do that with constants of its own, which meant a user could raise the
/// app's floor, watch assist go quiet, and have no way to tell that the browser was the thing
/// ignoring them. Both numbers here are the app's, so raising one in Settings moves the wire
/// as well as the gate.
#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Limits {
    /// Shortest block worth sending, in characters.
    pub min_chars: usize,
    /// Most of a block that will be read, in characters. A longer one is cut on a character
    /// boundary before it is sent, so the cut costs nothing later.
    pub max_ask_chars: usize,
}

/// The hour's spending against its ceiling.
///
/// Sent with every frame that follows a look so the popup can say "12 of 30 this hour" without
/// asking for anything: it is the answer to "why has it gone quiet?", and the browser is where
/// the user is when the question comes up. The ceiling itself is the app's and is only ever
/// read here -- a limit the client could set is a limit a modified client could raise, and the
/// app is the thing that pays.
#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Usage {
    pub used: usize,
    pub limit: u32,
}

/// A message to the extension.
#[derive(Debug, Serialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum ServerFrame {
    /// The answer to [`ClientFrame::Hello`].
    Welcome {
        enabled: bool,
        /// Whether the app wants cards drawn. False in dry-run: the whole pipeline runs
        /// and the verdict is reported, and nothing appears on the page.
        display: bool,
        /// The version of this protocol, so a stale extension can say so rather than
        /// failing in a way the user has to guess at.
        protocol: u32,
        /// The floor and the cut the app applies, which the extension must apply too.
        limits: Limits,
        usage: Usage,
    },
    /// The result of one [`ClientFrame::Page`].
    ///
    /// Carries every outcome, not only the answers: "nothing appeared" is the commonest
    /// thing a user reports, and the only useful answer names the stage that decided and
    /// how much was looked at.
    Cards {
        id: String,
        stage: &'static str,
        /// The model's reason, when it declined.
        reason: String,
        /// How many blocks the page offered.
        screened: usize,
        /// Whether to draw them. False in dry-run, and false for anything that is not an
        /// answer.
        display: bool,
        cards: Vec<CardFrame>,
        /// What this look cost against the hour, so the popup's figure moves with the work
        /// rather than with a poll.
        usage: Usage,
    },
    /// The switch moved, pushed to every connected extension so the popup and the tray
    /// cannot disagree about whether assist is on.
    ///
    /// Also the answer to a [`ClientFrame::Ping`], which is how a popup that has just opened
    /// reads the state without owning a copy of it.
    State {
        enabled: bool,
        limits: Limits,
        usage: Usage,
    },
    /// A refusal, with the reason in the app's own words.
    Error { message: String },
}

/// The stored pairing token, generated on first read.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct BridgeRecord {
    pub token: String,
}

/// The pairing token, creating one the first time it is asked for.
///
/// Kept in the database rather than in memory so a pairing survives a restart: the page
/// hands this to the extension, and a token that changed at every launch would mean
/// pairing again every morning.
pub fn token(database: &crate::database::Database) -> String {
    if let Some(record) = database.setting::<BridgeRecord>(RECORD_KEY) {
        if !record.token.trim().is_empty() {
            return record.token;
        }
    }
    // Long and random rather than short and memorable: nobody types this, so its only
    // job is to be unguessable by a page that tries.
    let token = format!(
        "{}{}",
        uuid::Uuid::new_v4().simple(),
        uuid::Uuid::new_v4().simple()
    );
    let record = BridgeRecord {
        token: token.clone(),
    };
    if let Err(error) = database.set_setting(RECORD_KEY, &record) {
        // Reported and still returned: a token that cannot be stored is one the user has
        // to re-pair after a restart, which is a small annoyance rather than a reason to
        // leave assist unusable for this session.
        tracing::warn!(%error, "the assist pairing token could not be saved");
    }
    token
}

/// Whether a token is the one this machine issued.
///
/// Compared in full rather than by prefix, and missing rather than empty counts as no
/// token at all, so a frame that simply omits the field cannot authenticate.
pub fn token_matches(expected: &str, offered: &str) -> bool {
    !expected.is_empty() && offered == expected
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::database::Database;

    /// The first read creates the token, and every read after it returns the same one --
    /// a pairing that changed at every launch would mean pairing again every morning.
    #[test]
    fn the_token_is_created_once_and_kept() {
        let database = Database::open_in_memory();
        let first = token(&database);
        assert!(first.len() >= 32, "the token should be long");
        assert_eq!(first, token(&database));
    }

    /// An omitted or empty token never authenticates, so a frame cannot pair by leaving
    /// the field out.
    #[test]
    fn an_empty_token_never_matches() {
        assert!(!token_matches("", ""));
        assert!(!token_matches("expected", ""));
        assert!(token_matches("expected", "expected"));
        assert!(!token_matches("expected", "expect"));
    }

    /// The frames the extension and the app have to agree on, both ways round.
    #[test]
    fn the_frames_round_trip_through_their_wire_names() {
        let hello: ClientFrame = serde_json::from_str(
            r#"{"type":"hello","token":"abc","client":{"profile":"Default","version":"1.0.0"}}"#,
        )
        .expect("hello must parse");
        assert!(matches!(hello, ClientFrame::Hello { token, .. } if token == "abc"));

        let page: ClientFrame = serde_json::from_str(
            r#"{"type":"page","id":"r1","frame":{"id":"f1","url":"https://example.com","blocks":[
                {"index":1,"text":"14 - 8 =","tag":"p","rect":{"x":10,"y":20,"width":100,"height":30}}
            ]}}"#,
        )
        .expect("page must parse");
        match page {
            ClientFrame::Page { id, frame } => {
                assert_eq!(id, "r1");
                assert_eq!(frame.blocks.len(), 1);
                assert_eq!(frame.blocks[0].index, 1);
                let rect = frame.blocks[0].rect.expect("the rectangle should survive");
                assert_eq!(rect.x, 10.0);
                assert_eq!(rect.height, 30.0);
            }
            _ => panic!("wrong frame"),
        }

        let set: ClientFrame =
            serde_json::from_str(r#"{"type":"setEnabled","enabled":true}"#).expect("setEnabled");
        assert!(matches!(set, ClientFrame::SetEnabled { enabled: true }));

        let welcome = serde_json::to_value(ServerFrame::Welcome {
            enabled: true,
            display: true,
            protocol: PROTOCOL,
            limits: Limits {
                min_chars: 6,
                max_ask_chars: 1_200,
            },
            usage: Usage { used: 3, limit: 30 },
        })
        .expect("welcome must serialize");
        assert_eq!(welcome["type"], "welcome");
        assert_eq!(welcome["protocol"], 5);
        // The floor the extension must apply is the app's, in the spelling the frontend reads.
        assert_eq!(welcome["limits"]["minChars"], 6);
        assert_eq!(welcome["limits"]["maxAskChars"], 1_200);
        assert_eq!(welcome["usage"]["used"], 3);
        assert_eq!(welcome["usage"]["limit"], 30);

        // The card frame carries the block number the extension will place it by, in
        // camelCase like every other payload the frontend reads.
        let cards = serde_json::to_value(ServerFrame::Cards {
            id: "r1".to_string(),
            stage: "answered",
            reason: String::new(),
            screened: 24,
            display: true,
            usage: Usage { used: 4, limit: 30 },
            cards: vec![CardFrame {
                index: 3,
                key: "k3".to_string(),
                answer: "3.14159".to_string(),
                subject: "math".to_string(),
                confidence: 0.9,
                kind: "answer",
                shape: "question",
            }],
        })
        .expect("cards must serialize");
        assert_eq!(cards["type"], "cards");
        assert_eq!(cards["screened"], 24);
        assert_eq!(cards["cards"][0]["index"], 3);
        assert_eq!(cards["cards"][0]["kind"], "answer");
        assert_eq!(cards["cards"][0]["shape"], "question");
        assert_eq!(cards["usage"]["used"], 4);
    }

    /// A block without a rectangle is normal -- the app's own preview has nothing to
    /// point at -- and must not fail the frame it arrives in.
    #[test]
    fn a_block_without_a_rectangle_still_parses() {
        let page: ClientFrame = serde_json::from_str(
            r#"{"type":"page","id":"r1","frame":{"blocks":[{"index":1,"text":"The value of PI?"}]}}"#,
        )
        .expect("a block with no rect must parse");
        match page {
            ClientFrame::Page { frame, .. } => {
                assert_eq!(frame.blocks[0].text, "The value of PI?");
                assert!(frame.blocks[0].rect.is_none());
            }
            _ => panic!("wrong frame"),
        }
    }

    /// An unknown frame is refused rather than guessed at, so a newer extension gets an
    /// error instead of silence.
    #[test]
    fn an_unknown_frame_is_refused() {
        assert!(serde_json::from_str::<ClientFrame>(r#"{"type":"nonsense"}"#).is_err());
    }
}
