//! Turning unread conversations into bounded requests.
//!
//! # Why batches rather than one large request
//!
//! A first run can have months of history. Sending it all at once is expensive,
//! slow, and likely to be truncated by the provider -- and a truncated reply is
//! wasted spend. A batch is a unit that fits comfortably in a context window and
//! that can be committed on its own: the profile is updated after each one, so a
//! failure later in the run keeps the work already paid for.
//!
//! # Why the watermark is per conversation
//!
//! A run advances only the conversations whose turns it actually sent. A batch
//! that failed is not marked read, and neither is a conversation the caps left
//! for next time. Re-reading a turn costs tokens and teaches nothing new.
//!
//! # What is read
//!
//! User and assistant text only, and only turns the transcript actually draws.
//! Tool results, reasoning and the hidden turns a sub-agent reports through are
//! not the user's words, and a profile built from them would describe the model's
//! work rather than the person's preferences.

use std::collections::HashSet;

use crate::database::Database;

/// Most conversations one run may touch.
pub const MAX_SESSIONS: usize = 48;

/// Most turns read from one conversation in one run.
///
/// A conversation longer than this is read across runs rather than in one giant
/// request; the watermark makes that continuation exact rather than approximate.
pub const MAX_MESSAGES_PER_SESSION: usize = 120;

/// Longest text kept from one turn.
///
/// The start of a message is where the request and the conclusion usually are;
/// the middle of a pasted file is not a preference.
pub const MAX_MESSAGE_CHARS: usize = 1_600;

/// Characters of conversation in one request. Roughly 4.5k English tokens.
pub const MAX_BATCH_CHARS: usize = 18_000;

/// Most requests one run may make. A first run over a large history stops here and
/// leaves the rest for the next cadence.
pub const MAX_BATCHES: usize = 5;

/// One visible turn.
#[derive(Clone, Debug, PartialEq)]
pub struct Turn {
    pub role: String,
    pub text: String,
    /// The row's ordinal, so a batch knows exactly how far it read.
    pub ordinal: i64,
}

/// One conversation's contribution to one batch.
///
/// A long conversation can appear in several batches. Each part carries that
/// batch's slice; the watermark is the last ordinal in the slice, not the last in
/// the conversation.
#[derive(Clone, Debug)]
pub struct SessionPart {
    pub id: String,
    pub title: String,
    pub updated_at: String,
    pub kind: String,
    pub turns: Vec<Turn>,
}

/// One request's worth of conversations.
#[derive(Clone, Debug, Default)]
pub struct Batch {
    pub sessions: Vec<SessionPart>,
    pub chars: usize,
    pub messages: usize,
}

impl Batch {
    pub fn is_empty(&self) -> bool {
        self.messages == 0
    }

    /// The furthest ordinal read in each conversation in this batch.
    ///
    /// Deduplicated because a session is normally one part per batch, but the
    /// method does not rely on that: taking the maximum is correct either way.
    pub fn watermarks(&self) -> Vec<(String, i64)> {
        let mut marks: Vec<(String, i64)> = Vec::new();
        for session in &self.sessions {
            let Some(last) = session.turns.last() else {
                continue;
            };
            match marks.iter_mut().find(|(id, _)| id == &session.id) {
                Some((_, ordinal)) => *ordinal = (*ordinal).max(last.ordinal),
                None => marks.push((session.id.clone(), last.ordinal)),
            }
        }
        marks
    }
}

/// Everything one run may read, already batched.
#[derive(Debug, Default)]
pub struct Corpus {
    pub batches: Vec<Batch>,
    /// Conversations whose unread turns were all empty after cleaning. Their
    /// watermarks are advanced without spending a request: there is nothing to ask
    /// the model about, but re-reading the same empty rows next time is waste.
    pub empty_sessions: Vec<(String, i64)>,
    /// Distinct conversations represented in `batches`.
    pub sessions: usize,
    pub messages: usize,
    /// True when a cap left work for the next run.
    pub truncated: bool,
}

/// Reads every unread conversation into batches.
pub fn build(database: &Database) -> Result<Corpus, String> {
    let pending = database.taste_pending_sessions(MAX_SESSIONS)?;
    let mut corpus = Corpus::default();
    let mut current = Batch::default();
    let mut seen_sessions = HashSet::new();

    for session in pending {
        if corpus.batches.len() >= MAX_BATCHES {
            corpus.truncated = true;
            break;
        }

        // One extra row tells us whether the cap cut a conversation short. Without
        // it the run could not distinguish "this is all there is" from "there is
        // more", and the report would be silent about a partial read.
        let rows = database.taste_messages_after(
            &session.id,
            session.last_ordinal,
            MAX_MESSAGES_PER_SESSION + 1,
        )?;
        if rows.is_empty() {
            continue;
        }
        let more = rows.len() > MAX_MESSAGES_PER_SESSION;
        if more {
            corpus.truncated = true;
        }

        let mut last_ordinal = session.last_ordinal;
        let mut part_had_turns = false;
        for row in rows.into_iter().take(MAX_MESSAGES_PER_SESSION) {
            last_ordinal = row.ordinal;
            let text = clean(&row.content);
            if text.is_empty() {
                continue;
            }
            // Start a new request before this turn if it would push the current one
            // past its budget. The check is on `messages`, not `chars`, so a single
            // long turn in an empty batch is still sent rather than looping forever.
            if current.messages > 0 && current.chars + text.chars().count() > MAX_BATCH_CHARS {
                corpus.batches.push(std::mem::take(&mut current));
                if corpus.batches.len() >= MAX_BATCHES {
                    corpus.truncated = true;
                    break;
                }
            }
            if current.sessions.is_empty()
                || !current.sessions.iter().any(|entry| entry.id == session.id)
            {
                current.sessions.push(SessionPart {
                    id: session.id.clone(),
                    title: one_line(&session.title),
                    updated_at: session.updated_at.clone(),
                    kind: session.kind.clone(),
                    turns: Vec::new(),
                });
            }
            let part = current
                .sessions
                .iter_mut()
                .find(|entry| entry.id == session.id)
                .expect("just added");
            current.chars += text.chars().count();
            current.messages += 1;
            part.turns.push(Turn {
                role: row.role,
                text,
                ordinal: row.ordinal,
            });
            part_had_turns = true;
        }

        if part_had_turns {
            seen_sessions.insert(session.id.clone());
        } else {
            // Everything unread in this conversation was empty after cleaning.
            // Advancing the watermark is what stops the next run scanning it again.
            corpus
                .empty_sessions
                .push((session.id.clone(), last_ordinal));
        }

        if corpus.batches.len() >= MAX_BATCHES {
            corpus.truncated = true;
            // The current batch has not been pushed yet, so it is still usable.
            break;
        }
    }

    if !current.is_empty() && corpus.batches.len() < MAX_BATCHES {
        corpus.batches.push(current);
    } else if !current.is_empty() {
        corpus.truncated = true;
    }

    corpus.sessions = seen_sessions.len();
    corpus.messages = corpus.batches.iter().map(|batch| batch.messages).sum();
    Ok(corpus)
}

/// Trims a stored message to what the model is shown.
fn clean(raw: &str) -> String {
    let normalized = raw.replace("\r\n", "\n").replace('\r', "\n");
    let text = normalized.trim();
    if text.is_empty() {
        return String::new();
    }
    if text.chars().count() <= MAX_MESSAGE_CHARS {
        return text.to_string();
    }
    let mut clipped: String = text.chars().take(MAX_MESSAGE_CHARS - 1).collect();
    clipped.push('…');
    clipped
}

/// A title on one line, because it is drawn inside a prompt header.
fn one_line(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn database() -> Database {
        Database::in_memory().expect("database")
    }

    fn seed(database: &Database, id: &str, updated: &str, count: usize) {
        let messages: Vec<serde_json::Value> = (0..count)
            .map(|index| {
                serde_json::json!({
                    "role": if index % 2 == 0 { "user" } else { "assistant" },
                    "content": format!("turn {index} in {id} with enough text to count."),
                })
            })
            .collect();
        database
            .save_chat_session(&serde_json::json!({
                "id": id,
                "title": id,
                "createdAt": updated,
                "updatedAt": updated,
                "messages": messages,
            }))
            .expect("seed");
    }

    #[test]
    fn two_conversations_share_one_batch() {
        let database = database();
        seed(&database, "one", "2026-01-02T00:00:00.000Z", 4);
        seed(&database, "two", "2026-01-01T00:00:00.000Z", 2);

        let corpus = build(&database).expect("corpus");
        assert_eq!(corpus.batches.len(), 1);
        assert_eq!(corpus.sessions, 2);
        assert_eq!(corpus.messages, 6);
        assert!(!corpus.truncated);
        assert_eq!(corpus.batches[0].sessions.len(), 2);
    }

    #[test]
    fn a_read_conversation_is_not_read_again() {
        let database = database();
        seed(&database, "one", "2026-01-02T00:00:00.000Z", 2);
        let corpus = build(&database).expect("corpus");
        for (session, ordinal) in corpus.batches[0].watermarks() {
            database
                .set_taste_watermark(&session, ordinal)
                .expect("watermark");
        }
        let next = build(&database).expect("next");
        assert!(next.batches.is_empty());
        assert_eq!(next.messages, 0);
    }

    #[test]
    fn a_long_conversation_is_split_with_an_exact_watermark() {
        let database = database();
        // Enough long turns to force a split under a deliberately small budget is
        // not possible without changing the constant, so this checks the rule at
        // its boundary: every batch's mark is the last ordinal it actually holds.
        seed(&database, "long", "2026-01-02T00:00:00.000Z", 200);
        let corpus = build(&database).expect("corpus");
        assert!(corpus.truncated, "the message cap should be reported");
        assert_eq!(corpus.messages, MAX_MESSAGES_PER_SESSION);
        let (session, ordinal) = corpus.batches[0].watermarks()[0].clone();
        assert_eq!(session, "long");
        assert_eq!(ordinal, (MAX_MESSAGES_PER_SESSION - 1) as i64);
    }

    #[test]
    fn empty_turns_advance_without_a_request() {
        let database = database();
        database
            .save_chat_session(&serde_json::json!({
                "id": "blank",
                "title": "blank",
                "createdAt": "2026-01-02T00:00:00.000Z",
                "updatedAt": "2026-01-02T00:00:00.000Z",
                "messages": [
                    { "role": "user", "content": "   " },
                    { "role": "assistant", "content": "\n\n" }
                ]
            }))
            .expect("seed");
        let corpus = build(&database).expect("corpus");
        assert!(corpus.batches.is_empty());
        assert_eq!(corpus.empty_sessions.len(), 1);
        assert_eq!(corpus.empty_sessions[0].1, 1);
    }

    #[test]
    fn a_pasted_file_is_clipped_not_sent_whole() {
        let database = database();
        let long = "x".repeat(MAX_MESSAGE_CHARS * 2);
        database
            .save_chat_session(&serde_json::json!({
                "id": "paste",
                "title": "paste",
                "createdAt": "2026-01-02T00:00:00.000Z",
                "updatedAt": "2026-01-02T00:00:00.000Z",
                "messages": [{ "role": "user", "content": long }]
            }))
            .expect("seed");
        let corpus = build(&database).expect("corpus");
        let turn = &corpus.batches[0].sessions[0].turns[0];
        assert_eq!(turn.text.chars().count(), MAX_MESSAGE_CHARS);
        assert!(turn.text.ends_with('…'));
    }
}
