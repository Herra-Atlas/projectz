//! What the taste learner remembers.
//!
//! # Four small stores, one fact each
//!
//! - `taste_items` is the profile as it stands: one self-contained fact per row.
//! - `taste_sessions` is how far the learner has read each conversation. It is a
//!   watermark rather than a cursor over rows because a run may fail halfway and
//!   the batches that succeeded must not be paid for again.
//! - `taste_runs` is history: what a run was given, what it changed, what it cost
//!   and what it wrote. A report is worth keeping even when the profile did not
//!   change, because "nothing was learned" is a result.
//!
//! Nothing here renders or interprets a fact. The section names, operation rules
//! and prompt rendering live in `taste/`, so the database stays a store and the
//! learner stays the only place with an opinion about meaning.

use rusqlite::{params, OptionalExtension};

use super::Database;

/// One stored fact.
///
/// `section` is a plain string rather than an enum here: the database is not
/// allowed to reject a row the learner might still be able to use, and an
/// unknown section is dropped by the merge rather than failing a whole run.
#[derive(Clone, Debug, PartialEq)]
pub struct TasteItemRow {
    pub id: i64,
    pub section: String,
    pub text: String,
    pub confidence: f32,
    pub created_at: String,
    pub updated_at: String,
    pub last_seen_at: String,
}

/// One conversation that still has unread turns.
#[derive(Clone, Debug)]
pub struct TasteSessionRow {
    pub id: String,
    pub title: String,
    pub updated_at: String,
    /// `chat`, `agent` or `job`. Stored only so the prompt can say what kind of
    /// conversation a turn came from; sub-agent transcripts are excluded by the
    /// query, because they are the model talking to itself rather than the user.
    pub kind: String,
    /// The last ordinal already learned from this conversation.
    pub last_ordinal: i64,
}

/// One visible turn, as the learner reads it.
#[derive(Clone, Debug)]
pub struct TasteMessageRow {
    pub ordinal: i64,
    pub role: String,
    pub content: String,
}

/// One finished run.
#[derive(Clone, Debug)]
pub struct TasteRunRow {
    pub id: i64,
    pub started_at: String,
    pub finished_at: Option<String>,
    pub status: String,
    pub reason: Option<String>,
    pub model_id: Option<String>,
    pub provider_id: Option<String>,
    pub sessions_read: i64,
    pub messages_read: i64,
    pub prompt_tokens: Option<i64>,
    pub completion_tokens: Option<i64>,
    pub added: i64,
    pub updated: i64,
    pub removed: i64,
    pub report: Option<String>,
    pub document: Option<String>,
}

/// A run about to be written.
///
/// Separate from [`TasteRunRow`] so the learner cannot set an id or a finish time
/// by accident. `finished_at` is stamped by [`Database::record_taste_run`]: a row
/// is only written once the run is over.
#[derive(Clone, Debug)]
pub struct NewTasteRun {
    pub started_at: String,
    pub status: String,
    pub reason: Option<String>,
    pub model_id: Option<String>,
    pub provider_id: Option<String>,
    pub sessions_read: i64,
    pub messages_read: i64,
    pub prompt_tokens: Option<i64>,
    pub completion_tokens: Option<i64>,
    pub added: i64,
    pub updated: i64,
    pub removed: i64,
    pub report: Option<String>,
    pub document: Option<String>,
}

const ITEM_COLUMNS: &str = "id,section,text,confidence,created_at,updated_at,last_seen_at";

const RUN_COLUMNS: &str = "id,started_at,finished_at,status,reason,model_id,provider_id,\
                           sessions_read,messages_read,prompt_tokens,completion_tokens,\
                           added,updated,removed,report,document";

impl Database {
    /// The profile as it stands, oldest first.
    ///
    /// `id` order is the stable order the renderer uses: the prompt bytes must not
    /// move between requests just because a fact was reinforced, or the provider's
    /// cached prefix would be thrown away for nothing.
    pub fn taste_items(&self) -> Result<Vec<TasteItemRow>, String> {
        let connection = self.connection.lock().map_err(|error| error.to_string())?;
        let mut statement = connection
            .prepare(&format!(
                "SELECT {ITEM_COLUMNS} FROM taste_items ORDER BY id"
            ))
            .map_err(|error| error.to_string())?;
        let rows = statement
            .query_map([], |row| {
                Ok(TasteItemRow {
                    id: row.get(0)?,
                    section: row.get(1)?,
                    text: row.get(2)?,
                    confidence: row.get::<_, f64>(3)? as f32,
                    created_at: row.get(4)?,
                    updated_at: row.get(5)?,
                    last_seen_at: row.get(6)?,
                })
            })
            .map_err(|error| error.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| error.to_string())?;
        Ok(rows)
    }

    /// Replaces the whole profile in one transaction.
    ///
    /// The merge is a small in-memory operation over at most a few dozen rows, so
    /// writing the result as a whole is simpler and safer than diffing it: a
    /// half-applied operation set is exactly the state a failed run must not
    /// leave behind. Ids are explicit and assigned by the learner, which is what
    /// keeps an item's identity stable across a rewrite.
    pub fn replace_taste_items(&self, items: &[TasteItemRow]) -> Result<(), String> {
        let connection = self.connection.lock().map_err(|error| error.to_string())?;
        let tx = connection
            .unchecked_transaction()
            .map_err(|error| error.to_string())?;
        tx.execute("DELETE FROM taste_items", [])
            .map_err(|error| error.to_string())?;
        for item in items {
            if item.id <= 0 {
                return Err("A taste item needs a positive id".to_string());
            }
            tx.execute(
                "INSERT INTO taste_items (id,section,text,confidence,created_at,updated_at,last_seen_at)
                 VALUES (?1,?2,?3,?4,?5,?6,?7)",
                params![
                    item.id,
                    item.section,
                    item.text,
                    f64::from(item.confidence),
                    item.created_at,
                    item.updated_at,
                    item.last_seen_at,
                ],
            )
            .map_err(|error| error.to_string())?;
        }
        tx.commit().map_err(|error| error.to_string())
    }

    /// Rewrites one fact's text.
    ///
    /// Manual editing is the user confirming the fact, so the confidence is raised
    /// to at least 0.9 rather than left at whatever an inference guessed. The
    /// watermark table is untouched: correcting a fact is not a reason to re-read
    /// the conversations it came from.
    pub fn update_taste_item(&self, id: i64, text: &str) -> Result<(), String> {
        let now = super::utc_now();
        let changed = self
            .connection
            .lock()
            .map_err(|error| error.to_string())?
            .execute(
                "UPDATE taste_items
                 SET text=?2, confidence=MAX(confidence, 0.9), updated_at=?3, last_seen_at=?3
                 WHERE id=?1",
                params![id, text, now],
            )
            .map_err(|error| error.to_string())?;
        if changed == 0 {
            return Err(format!("No taste item with id {id}"));
        }
        Ok(())
    }

    /// Deletes one fact.
    ///
    /// The watermarks are deliberately unchanged: a fact the user removed should
    /// not be quietly re-learned from conversations already read. A future
    /// conversation can still bring it back, which is the honest behaviour.
    pub fn delete_taste_item(&self, id: i64) -> Result<(), String> {
        let changed = self
            .connection
            .lock()
            .map_err(|error| error.to_string())?
            .execute("DELETE FROM taste_items WHERE id=?1", [id])
            .map_err(|error| error.to_string())?;
        if changed == 0 {
            return Err(format!("No taste item with id {id}"));
        }
        Ok(())
    }

    /// Advances one conversation's watermark.
    ///
    /// Called once per batch that succeeded, not once per run: a run that dies
    /// after its second batch of six must keep the two it already paid for.
    pub fn set_taste_watermark(&self, session_id: &str, last_ordinal: i64) -> Result<(), String> {
        let now = super::utc_now();
        self.connection
            .lock()
            .map_err(|error| error.to_string())?
            .execute(
                "INSERT INTO taste_sessions (session_id,last_ordinal,updated_at)
                 VALUES (?1,?2,?3)
                 ON CONFLICT(session_id) DO UPDATE SET
                   last_ordinal=MAX(taste_sessions.last_ordinal,excluded.last_ordinal),
                   updated_at=excluded.updated_at",
                params![session_id, last_ordinal, now],
            )
            .map_err(|error| error.to_string())?;
        Ok(())
    }

    /// Conversations with at least one visible turn the learner has not read.
    ///
    /// Newest first, because when a first run has more history than one run may
    /// read, the recent conversations are the ones that describe the user as they
    /// are now. A session already read to its end is absent, so "nothing to do" is
    /// an empty list rather than a scan over every message.
    pub fn taste_pending_sessions(&self, limit: usize) -> Result<Vec<TasteSessionRow>, String> {
        let connection = self.connection.lock().map_err(|error| error.to_string())?;
        let mut statement = connection
            .prepare(
                "SELECT s.id, s.title, s.updated_at,
                        COALESCE(json_extract(s.metadata_json, '$.kind'), 'chat'),
                        COALESCE(t.last_ordinal, -1)
                 FROM sessions s
                 LEFT JOIN taste_sessions t ON t.session_id = s.id
                 WHERE COALESCE(json_extract(s.metadata_json, '$.kind'), 'chat') <> 'subagent'
                   AND EXISTS (
                       SELECT 1 FROM messages m
                       WHERE m.session_id = s.id
                         AND m.ordinal > COALESCE(t.last_ordinal, -1)
                         -- NULL is what a message written before the column
                         -- existed reads as; it was visible then and still is.
                         AND COALESCE(m.hidden, 0) = 0
                         AND m.role IN ('user', 'assistant')
                   )
                 ORDER BY s.updated_at DESC
                 LIMIT ?1",
            )
            .map_err(|error| error.to_string())?;
        let rows = statement
            .query_map([limit as i64], |row| {
                Ok(TasteSessionRow {
                    id: row.get(0)?,
                    title: row.get(1)?,
                    updated_at: row.get(2)?,
                    kind: row.get(3)?,
                    last_ordinal: row.get(4)?,
                })
            })
            .map_err(|error| error.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| error.to_string())?;
        Ok(rows)
    }

    /// Visible turns after a watermark, oldest first.
    ///
    /// `hidden` turns are excluded because the model receives them as context but
    /// the user never wrote them; reasoning and tool payloads have no column here
    /// at all, so they cannot leak into the profile by being read through the
    /// wrong query.
    pub fn taste_messages_after(
        &self,
        session_id: &str,
        after_ordinal: i64,
        limit: usize,
    ) -> Result<Vec<TasteMessageRow>, String> {
        let connection = self.connection.lock().map_err(|error| error.to_string())?;
        let mut statement = connection
            .prepare(
                "SELECT ordinal, role, content FROM messages
                 WHERE session_id = ?1 AND ordinal > ?2 AND COALESCE(hidden, 0) = 0
                   AND role IN ('user', 'assistant')
                 ORDER BY ordinal LIMIT ?3",
            )
            .map_err(|error| error.to_string())?;
        let rows = statement
            .query_map(params![session_id, after_ordinal, limit as i64], |row| {
                Ok(TasteMessageRow {
                    ordinal: row.get(0)?,
                    role: row.get(1)?,
                    content: row.get(2)?,
                })
            })
            .map_err(|error| error.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| error.to_string())?;
        Ok(rows)
    }

    /// Writes a finished run and returns its id.
    pub fn record_taste_run(&self, run: &NewTasteRun) -> Result<i64, String> {
        let connection = self.connection.lock().map_err(|error| error.to_string())?;
        connection
            .execute(
                "INSERT INTO taste_runs
                   (started_at,finished_at,status,reason,model_id,provider_id,sessions_read,
                    messages_read,prompt_tokens,completion_tokens,added,updated,removed,report,document)
                 VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15)",
                params![
                    run.started_at,
                    super::utc_now(),
                    run.status,
                    run.reason,
                    run.model_id,
                    run.provider_id,
                    run.sessions_read,
                    run.messages_read,
                    run.prompt_tokens,
                    run.completion_tokens,
                    run.added,
                    run.updated,
                    run.removed,
                    run.report,
                    run.document,
                ],
            )
            .map_err(|error| error.to_string())?;
        Ok(connection.last_insert_rowid())
    }

    /// The most recent run, or `None` before the first one.
    pub fn latest_taste_run(&self) -> Result<Option<TasteRunRow>, String> {
        let connection = self.connection.lock().map_err(|error| error.to_string())?;
        connection
            .query_row(
                &format!("SELECT {RUN_COLUMNS} FROM taste_runs ORDER BY id DESC LIMIT 1"),
                [],
                taste_run_from_row,
            )
            .optional()
            .map_err(|error| error.to_string())
    }

    /// Runs newest first, for the settings page's history.
    pub fn taste_runs(&self, limit: i64) -> Result<Vec<TasteRunRow>, String> {
        let connection = self.connection.lock().map_err(|error| error.to_string())?;
        let mut statement = connection
            .prepare(&format!(
                "SELECT {RUN_COLUMNS} FROM taste_runs ORDER BY id DESC LIMIT ?1"
            ))
            .map_err(|error| error.to_string())?;
        let rows = statement
            .query_map([limit], taste_run_from_row)
            .map_err(|error| error.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| error.to_string())?;
        Ok(rows)
    }

    /// Forgets what was learned and how far it was read.
    ///
    /// Runs are deliberately kept: they are the record of what the feature did,
    /// and deleting the profile is not a request to forget that it once ran.
    pub fn reset_taste(&self) -> Result<(), String> {
        let connection = self.connection.lock().map_err(|error| error.to_string())?;
        let tx = connection
            .unchecked_transaction()
            .map_err(|error| error.to_string())?;
        tx.execute("DELETE FROM taste_items", [])
            .map_err(|error| error.to_string())?;
        tx.execute("DELETE FROM taste_sessions", [])
            .map_err(|error| error.to_string())?;
        tx.commit().map_err(|error| error.to_string())
    }
}

fn taste_run_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<TasteRunRow> {
    Ok(TasteRunRow {
        id: row.get(0)?,
        started_at: row.get(1)?,
        finished_at: row.get(2)?,
        status: row.get(3)?,
        reason: row.get(4)?,
        model_id: row.get(5)?,
        provider_id: row.get(6)?,
        sessions_read: row.get(7)?,
        messages_read: row.get(8)?,
        prompt_tokens: row.get(9)?,
        completion_tokens: row.get(10)?,
        added: row.get(11)?,
        updated: row.get(12)?,
        removed: row.get(13)?,
        report: row.get(14)?,
        document: row.get(15)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn database() -> Database {
        Database::in_memory().expect("database")
    }

    fn item(id: i64, text: &str) -> TasteItemRow {
        TasteItemRow {
            id,
            section: "work".into(),
            text: text.into(),
            confidence: 0.8,
            created_at: "2026-01-01T00:00:00.000Z".into(),
            updated_at: "2026-01-01T00:00:00.000Z".into(),
            last_seen_at: "2026-01-01T00:00:00.000Z".into(),
        }
    }

    fn seed_session(database: &Database, id: &str, updated: &str, messages: &[(&str, &str)]) {
        let rows: Vec<serde_json::Value> = messages
            .iter()
            .map(|(role, content)| serde_json::json!({ "role": role, "content": content }))
            .collect();
        database
            .save_chat_session(&serde_json::json!({
                "id": id,
                "title": id,
                "createdAt": updated,
                "updatedAt": updated,
                "messages": rows,
            }))
            .expect("seed");
    }

    #[test]
    fn items_round_trip_in_id_order() {
        let database = database();
        database
            .replace_taste_items(&[item(2, "second"), item(1, "first")])
            .expect("save");
        let read = database.taste_items().expect("read");
        assert_eq!(read.len(), 2);
        assert_eq!(read[0].id, 1);
        assert_eq!(read[0].text, "first");
        assert_eq!(read[1].id, 2);
    }

    /// A manual edit is the user confirming the fact, so it also raises the
    /// confidence rather than leaving an inference's guess in place.
    #[test]
    fn editing_a_fact_changes_its_text_and_raises_confidence() {
        let database = database();
        let mut stored = item(1, "An inferred guess about the user.");
        stored.confidence = 0.4;
        database.replace_taste_items(&[stored]).expect("save");
        database
            .update_taste_item(1, "The user confirmed this fact themselves.")
            .expect("update");
        let read = database.taste_items().expect("read");
        assert_eq!(read[0].text, "The user confirmed this fact themselves.");
        assert_eq!(read[0].confidence, 0.9);
    }

    #[test]
    fn deleting_a_fact_removes_it_and_reports_an_unknown_id() {
        let database = database();
        database
            .replace_taste_items(&[item(1, "A fact worth removing.")])
            .expect("save");
        database.delete_taste_item(1).expect("delete");
        assert!(database.taste_items().expect("read").is_empty());
        assert!(database.delete_taste_item(99).is_err());
    }

    #[test]
    fn replacing_the_profile_removes_what_is_gone() {
        let database = database();
        database
            .replace_taste_items(&[item(1, "kept"), item(2, "gone")])
            .expect("first");
        database
            .replace_taste_items(&[item(1, "kept")])
            .expect("second");
        let read = database.taste_items().expect("read");
        assert_eq!(read.len(), 1);
        assert_eq!(read[0].text, "kept");
    }

    /// A first run reads only visible turns, and the next run does not read them
    /// again. This is the whole reason the watermark exists.
    #[test]
    fn a_watermark_stops_a_turn_being_read_twice() {
        let database = database();
        seed_session(
            &database,
            "s1",
            "2026-01-02T00:00:00.000Z",
            &[("user", "hello"), ("assistant", "hi")],
        );

        let pending = database.taste_pending_sessions(10).expect("pending");
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].last_ordinal, -1);

        let messages = database
            .taste_messages_after("s1", -1, 100)
            .expect("messages");
        assert_eq!(messages.len(), 2);
        database
            .set_taste_watermark("s1", messages[1].ordinal)
            .expect("watermark");

        assert!(database
            .taste_pending_sessions(10)
            .expect("pending")
            .is_empty());
        assert!(database
            .taste_messages_after("s1", 1, 100)
            .expect("messages")
            .is_empty());
    }

    /// A sub-agent's transcript is the model talking to itself, not the user.
    #[test]
    fn subagent_transcripts_are_not_learned_from() {
        let database = database();
        seed_session(
            &database,
            "chat",
            "2026-01-02T00:00:00.000Z",
            &[("user", "hello")],
        );
        database
            .save_chat_session(&serde_json::json!({
                "id": "run",
                "title": "run",
                "createdAt": "2026-01-02T00:00:00.000Z",
                "updatedAt": "2026-01-02T00:00:00.000Z",
                "kind": "subagent",
                "messages": [{ "role": "user", "content": "find it" }],
            }))
            .expect("seed run");
        let pending = database.taste_pending_sessions(10).expect("pending");
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].id, "chat");
    }

    /// A hidden turn is sent to the model but was not written by the user, so it
    /// must not become a fact about them.
    #[test]
    fn hidden_turns_are_not_learned_from() {
        let database = database();
        seed_session(
            &database,
            "s1",
            "2026-01-02T00:00:00.000Z",
            &[("user", "hello")],
        );
        database
            .save_chat_session(&serde_json::json!({
                "id": "s1",
                "title": "s1",
                "createdAt": "2026-01-02T00:00:00.000Z",
                "updatedAt": "2026-01-02T00:00:00.000Z",
                "messages": [
                    { "role": "user", "content": "hello" },
                    { "role": "assistant", "content": "a hidden report", "hidden": true }
                ],
            }))
            .expect("save");
        let messages = database
            .taste_messages_after("s1", -1, 100)
            .expect("messages");
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].content, "hello");
    }

    /// Messages written before `hidden` existed have NULL there. They were
    /// visible then, and a profile that skipped them would ignore the oldest half
    /// of a long-time user's history.
    #[test]
    fn a_legacy_message_without_a_hidden_flag_is_still_read() {
        let database = database();
        seed_session(
            &database,
            "s1",
            "2026-01-02T00:00:00.000Z",
            &[("user", "hello"), ("assistant", "hi")],
        );
        database
            .connection
            .lock()
            .expect("lock")
            .execute("UPDATE messages SET hidden=NULL WHERE session_id='s1'", [])
            .expect("legacy rows");
        let messages = database
            .taste_messages_after("s1", -1, 100)
            .expect("messages");
        assert_eq!(messages.len(), 2);
    }

    #[test]
    fn a_run_records_its_figures_and_snapshot() {
        let database = database();
        let id = database
            .record_taste_run(&NewTasteRun {
                started_at: "2026-01-01T00:00:00.000Z".into(),
                status: "completed".into(),
                reason: None,
                model_id: Some("gpt".into()),
                provider_id: Some("ep".into()),
                sessions_read: 2,
                messages_read: 12,
                prompt_tokens: Some(900),
                completion_tokens: Some(120),
                added: 3,
                updated: 1,
                removed: 1,
                report: Some("Learned three facts.".into()),
                document: Some("[work]\n- Prefers Rust.".into()),
            })
            .expect("record");
        let latest = database.latest_taste_run().expect("latest").expect("row");
        assert_eq!(latest.id, id);
        assert_eq!(latest.status, "completed");
        assert_eq!(latest.added, 3);
        assert_eq!(latest.messages_read, 12);
        assert_eq!(
            latest.document.as_deref(),
            Some("[work]\n- Prefers Rust.".into())
        );
        assert!(latest.finished_at.is_some());
    }

    /// Resetting the profile keeps history: it is the record of what the feature
    /// did, and forgetting a preference is not a request to forget that it ran.
    #[test]
    fn reset_clears_the_profile_and_watermarks_but_not_history() {
        let database = database();
        seed_session(
            &database,
            "s1",
            "2026-01-02T00:00:00.000Z",
            &[("user", "hello")],
        );
        database
            .replace_taste_items(&[item(1, "a fact")])
            .expect("items");
        database.set_taste_watermark("s1", 0).expect("watermark");
        database
            .record_taste_run(&NewTasteRun {
                started_at: "2026-01-01T00:00:00.000Z".into(),
                status: "completed".into(),
                reason: None,
                model_id: None,
                provider_id: None,
                sessions_read: 1,
                messages_read: 1,
                prompt_tokens: None,
                completion_tokens: None,
                added: 1,
                updated: 0,
                removed: 0,
                report: None,
                document: None,
            })
            .expect("run");

        database.reset_taste().expect("reset");
        assert!(database.taste_items().expect("items").is_empty());
        // The watermark is gone, so the same conversation is pending again.
        assert_eq!(
            database.taste_pending_sessions(10).expect("pending").len(),
            1
        );
        assert_eq!(database.taste_runs(10).expect("runs").len(), 1);
    }
}
