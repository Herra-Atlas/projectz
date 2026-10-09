//! Reading conversations back.
//!
//! Split out of `mod.rs` because these are the two reads the sidebar and the
//! chat page make, and keeping them together makes the split between "what a
//! list of sessions needs" and "what one conversation needs" easy to read as a
//! pair.
//!
//! **Why there are two reads at all.** `list_chat_sessions` used to return every
//! conversation with every message, and the frontend loaded it once at launch.
//! The sidebar only draws titles, but the payload carried whole transcripts --
//! content, metrics, reasoning, tool rows, and the activity-panel steps for every
//! reply in the database -- across the IPC bridge and into React state. Cost was
//! `O(all history)` for an `O(1)` need, and no index changes that: the problem is
//! the bytes crossing the bridge, not the rows being read.
//!
//! `list_session_headers` returns what a list actually draws, and
//! `list_session_messages` returns one conversation. The token rollup on
//! `sessions` is read by the header, so the statistics-adjacent figures keep
//! working without touching `messages`.
//!
//! **`list_chat_sessions` is kept** as the full read. The frontend import path
//! and the database tests still use it, and a second full-read implementation
//! would be a second thing to keep correct.

use rusqlite::{params, Connection};
use serde_json::{json, Value};

use super::Database;

/// The columns every session listing reads, in the order the row decoders below
/// expect them.
///
/// Held as one constant so the header query and the full query cannot drift
/// into reading the same table with different column lists and different
/// ordinals, which is the kind of mismatch that compiles and returns nonsense.
///
/// Public because the full read in `mod.rs` builds its own query from the same
/// list; two copies of a column list is exactly what the note above is about.
pub const SESSION_COLUMNS: &str = "id,title,created_at,updated_at,model_id,provider_id,prompt_tokens,completion_tokens,cached_tokens,metadata_json";

impl Database {
    /// Every conversation, with no messages.
    ///
    /// The launch read. Cheap in proportion to the number of conversations rather
    /// than to the number of messages, which is what makes it the right one to
    /// call on startup.
    pub fn list_session_headers(&self) -> Result<Vec<Value>, String> {
        let connection = self.connection.lock().map_err(|error| error.to_string())?;
        let mut statement = connection
            .prepare(&format!(
                "SELECT {SESSION_COLUMNS} FROM sessions \
                 WHERE COALESCE(json_extract(metadata_json, '$.kind'), 'chat') <> 'subagent' \
                 ORDER BY \
                     CASE WHEN COALESCE(json_extract(metadata_json, '$.kind'), 'chat') = 'job' THEN 1 ELSE 0 END, \
                     json_extract(metadata_json, '$.pinned') DESC, \
                     updated_at DESC"
            ))
            .map_err(|error| error.to_string())?;
        let rows = statement
            .query_map([], read_session_row)
            .map_err(|error| error.to_string())?;
        let mut result = Vec::new();
        for row in rows {
            result.push(session_header(row.map_err(|error| error.to_string())?));
        }
        Ok(result)
    }

    /// The sub-agent runs, newest first, optionally for one parent conversation.
    ///
    /// The mirror of [`Self::list_session_headers`]: that read excludes what this
    /// one selects, so a run appears in exactly one list. `parent` narrows to the
    /// conversation that spawned them, which is what the panel wants when a
    /// conversation is open -- the agents it started, not every agent ever run.
    ///
    /// No messages. The panel lists titles first and fetches a transcript when one
    /// is opened, exactly as the chat sidebar does.
    pub fn list_subagent_headers(&self, parent: Option<&str>) -> Result<Vec<Value>, String> {
        let connection = self.connection.lock().map_err(|error| error.to_string())?;
        let mut statement = connection
            .prepare(&format!(
                "SELECT {SESSION_COLUMNS} FROM sessions \
                 WHERE json_extract(metadata_json, '$.kind') = 'subagent' \
                   AND (?1 IS NULL OR json_extract(metadata_json, '$.parentSessionId') = ?1) \
                 ORDER BY updated_at DESC"
            ))
            .map_err(|error| error.to_string())?;
        let rows = statement
            .query_map(params![parent], read_session_row)
            .map_err(|error| error.to_string())?;
        let mut result = Vec::new();
        for row in rows {
            result.push(session_header(row.map_err(|error| error.to_string())?));
        }
        Ok(result)
    }

    /// One conversation's messages, oldest first.
    ///
    /// Returns an empty array for a session that has none, which is the correct
    /// answer rather than an error: a conversation created but never sent to is a
    /// real state, and the chat page shows an empty transcript for it.
    ///
    /// `ordinal` is the ordering and is never assumed to be dense, so a row left
    /// behind by an earlier truncation cannot shift a message's position.
    pub fn list_session_messages(&self, session_id: &str) -> Result<Vec<Value>, String> {
        let connection = self.connection.lock().map_err(|error| error.to_string())?;
        Self::read_messages(&connection, session_id)
    }

    /// The `messages` query, shared by the per-session read and the full read so
    /// the two cannot decode a row differently.
    ///
    /// Decoding is tolerant on purpose. A row with no steps, a blob that no
    /// longer parses, or metrics that are not an object reads as "not recorded"
    /// rather than failing: the panel and the figures are additions to the
    /// transcript, and losing one reply's steps or metrics must never cost the
    /// user the conversation they came from.
    /// Private to the crate: only the two reads above should decode messages,
    /// so there is exactly one description of a stored message.
    pub(crate) fn read_messages(
        connection: &Connection,
        session_id: &str,
    ) -> Result<Vec<Value>, String> {
        let mut statement = connection
            .prepare(
                "SELECT role,content,model_id,provider_id,metrics_json,reasoning,tool_json,activity_json,hidden \
                 FROM messages WHERE session_id=?1 ORDER BY ordinal",
            )
            .map_err(|error| error.to_string())?;
        let rows = statement
            .query_map([session_id], |row| {
                let metrics: Option<String> = row.get(4)?;
                let metrics: Value = metrics
                    .and_then(|raw| serde_json::from_str(&raw).ok())
                    .unwrap_or(Value::Null);
                let tool: Option<String> = row.get(6)?;
                let tool: Option<Value> = tool.and_then(|raw| serde_json::from_str(&raw).ok());
                // A blob that no longer parses reads as no steps rather than
                // failing the whole load, for the reason given above.
                let activity: Option<String> = row.get(7)?;
                let activity: Option<Value> = activity
                    .and_then(|raw| serde_json::from_str(&raw).ok())
                    .filter(|value: &Value| value.as_array().is_some_and(|list| !list.is_empty()));
                // Kept in the model's context but not drawn; see the `hidden` note
                // in the schema. Stored as 0/1 and read back as a plain boolean.
                let hidden = row.get::<_, Option<i64>>(8)?.unwrap_or(0) != 0;
                Ok(json!({
                    "role": row.get::<_, String>(0)?,
                    "content": row.get::<_, String>(1)?,
                    "reasoning": row.get::<_, Option<String>>(5)?,
                    "tool": tool,
                    "activity": activity,
                    "hidden": hidden,
                    "modelId": row.get::<_, Option<String>>(2)?,
                    "providerId": row.get::<_, Option<String>>(3)?,
                    "metrics": metrics,
                }))
            })
            .map_err(|error| error.to_string())?;
        let mut messages = Vec::new();
        for row in rows {
            messages.push(row.map_err(|error| error.to_string())?);
        }
        Ok(messages)
    }
}

/// One session row, in [`SESSION_COLUMNS`] order.
///
/// `Clone` so the full read can borrow the id for the messages query and still
/// hand the tuple to [`session_header`], rather than decoding the row twice or
/// widening this into a struct that only two call sites would use.
type SessionRow = (
    String,
    String,
    String,
    String,
    Option<String>,
    Option<String>,
    Option<i64>,
    Option<i64>,
    Option<i64>,
    String,
);

/// The id from a decoded row, without consuming it.
pub fn peek_session_id(row: &SessionRow) -> String {
    row.0.clone()
}

/// Decodes a session row without its messages.
pub fn read_session_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<SessionRow> {
    Ok((
        row.get(0)?,
        row.get(1)?,
        row.get(2)?,
        row.get(3)?,
        row.get(4)?,
        row.get(5)?,
        row.get(6)?,
        row.get(7)?,
        row.get(8)?,
        row.get(9)?,
    ))
}

/// The header shape: the session row with its metadata unpacked, and no
/// `messages` key at all.
///
/// The key is **absent rather than null**, which is what lets the frontend tell a
/// header from a full session without a second field. A consumer that reads
/// `session.messages` on a header gets `undefined` rather than an empty
/// transcript, so the mistake is visible instead of rendering as an empty chat.
pub fn session_header(row: SessionRow) -> Value {
    let (
        id,
        title,
        created,
        updated,
        model_id,
        provider_id,
        prompt_tokens,
        completion_tokens,
        cached_tokens,
        metadata_json,
    ) = row;
    // A blob that no longer parses leaves every flag at its default rather than
    // failing the load: a session with unreadable metadata is still a session,
    // and refusing to list it would hide a conversation the user can see.
    let metadata: Value = serde_json::from_str(&metadata_json).unwrap_or_default();
    json!({
        "id": id,
        "title": title,
        "createdAt": created,
        "updatedAt": updated,
        "modelId": model_id,
        "providerId": provider_id,
        "promptTokens": prompt_tokens,
        "completionTokens": completion_tokens,
        "cachedTokens": cached_tokens,
        "pinned": metadata.get("pinned").and_then(Value::as_bool).unwrap_or(false),
        "renamed": metadata.get("renamed").and_then(Value::as_bool).unwrap_or(false),
        "renamedByUser": metadata.get("renamedByUser").and_then(Value::as_bool).unwrap_or(false),
        "dotColor": metadata.get("dotColor").and_then(Value::as_str),
        // Chat or Agent, and how much Agent mode may do. Absent reads as Chat,
        // which is what a conversation saved before these existed should do --
        // the permission level has its own default rather than being stored as a
        // third state here.
        "mode": metadata.get("mode").and_then(Value::as_str).unwrap_or("chat"),
        "permission": metadata.get("permission").and_then(Value::as_str),
        // `chat` unless the row says otherwise, so every conversation written
        // before sub-agents existed reads as an ordinary chat. `parentSessionId`
        // is absent for a chat, which is what the panel uses to group runs under
        // the conversation that started them.
        "kind": metadata.get("kind").and_then(Value::as_str).unwrap_or("chat"),
        "parentSessionId": metadata.get("parentSessionId").and_then(Value::as_str),
    })
}
