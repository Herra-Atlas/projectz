//! Turning unread conversations into small requests.
//!
//! # Only the user's own words
//!
//! The profile describes the person, so the only text that can support it is text
//! the person wrote. Assistant answers are the model's prose: they repeat the
//! request, quote it back and explain things the user never said, and they are
//! most of a transcript's characters. Reading them cost more than half of every
//! request and produced facts like "works on a quiz page" -- a task, not a trait.
//! The learner therefore reads `user` turns and nothing else.
//!
//! # Why the text is filtered before it is sent
//!
//! A turn is skipped when it cannot yield a fact: a command, a one-line reply, a
//! pasted file, a block of code. This is a cheap deterministic pass, and it is the
//! first of two: the reply is filtered again in [`super::merge`]. Together they
//! are why a small model is asked a short question instead of a dump.
//!
//! # Why batches rather than one large request
//!
//! A first run can have months of history. A batch fits comfortably in a small
//! model's window and can be committed on its own, so a failure later in the run
//! keeps the work already paid for.
//!
//! # Why the watermark is per conversation
//!
//! A run advances only the conversations whose turns it actually read. A batch
//! that failed is not marked read, and neither is a conversation the caps left
//! for next time. Re-reading a turn costs tokens and teaches nothing new.

use std::collections::HashSet;

use crate::database::Database;

/// Most conversations one run may touch.
pub const MAX_SESSIONS: usize = 32;

/// Most user turns read from one conversation in one run.
///
/// A conversation longer than this is read across runs rather than in one giant
/// request; the watermark makes that continuation exact rather than approximate.
pub const MAX_MESSAGES_PER_SESSION: usize = 80;

/// Longest text kept from one turn.
///
/// The opening of a message is where a statement about the person usually is; the
/// middle of a pasted document is not a preference.
pub const MAX_MESSAGE_CHARS: usize = 700;

/// Characters of user text in one request. Roughly 2.5k English tokens.
pub const MAX_BATCH_CHARS: usize = 10_000;

/// Most requests one run may make. A first run over a large history stops here and
/// leaves the rest for the next cadence or a manual run.
pub const MAX_BATCHES: usize = 3;

/// Visible rows read per user turn.
///
/// A conversation interleaves user and assistant messages, so a cap on user turns
/// needs a row budget twice as large plus one row to detect that there is more.
const ROWS_PER_USER_TURN: usize = 2;

/// The shortest turn worth sending -- below this it is "thanks", "ok" or "yes".
const MIN_TURN_CHARS: usize = 10;
const MIN_TURN_WORDS: usize = 3;

/// One visible user turn.
#[derive(Clone, Debug, PartialEq)]
pub struct Turn {
    pub text: String,
    /// The row's ordinal, so a batch knows exactly how far it read.
    pub ordinal: i64,
}

/// One conversation's contribution to one batch.
///
/// `read_through` is the last row read, whatever its role. It is what the
/// watermark advances to, so a conversation that ends with an assistant answer is
/// not left pending forever waiting for a user turn that already passed.
#[derive(Clone, Debug)]
pub struct SessionPart {
    pub id: String,
    pub read_through: i64,
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

    /// The furthest row read in each conversation in this batch.
    ///
    /// Deduplicated because a session is normally one part per batch, but the
    /// method does not rely on that: taking the maximum is correct either way.
    pub fn watermarks(&self) -> Vec<(String, i64)> {
        let mut marks: Vec<(String, i64)> = Vec::new();
        for session in &self.sessions {
            match marks.iter_mut().find(|(id, _)| id == &session.id) {
                Some((_, ordinal)) => *ordinal = (*ordinal).max(session.read_through),
                None => marks.push((session.id.clone(), session.read_through)),
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
    // One extra session answers "is there more?" without a second query.
    let mut pending = database.taste_pending_sessions(MAX_SESSIONS + 1)?;
    let more_sessions = pending.len() > MAX_SESSIONS;
    if more_sessions {
        pending.truncate(MAX_SESSIONS);
    }
    let mut corpus = Corpus {
        truncated: more_sessions,
        ..Corpus::default()
    };
    let mut current = Batch::default();
    let mut seen_sessions = HashSet::new();
    let row_limit = MAX_MESSAGES_PER_SESSION * ROWS_PER_USER_TURN;
    let mut deferred = false;

    for session in pending {
        if corpus.batches.len() >= MAX_BATCHES {
            corpus.truncated = true;
            break;
        }

        let rows =
            database.taste_messages_after(&session.id, session.last_ordinal, row_limit + 1)?;
        if rows.is_empty() {
            continue;
        }
        // One row beyond the budget means the conversation continues next run.
        if rows.len() > row_limit {
            corpus.truncated = true;
        }
        let rows = rows.into_iter().take(row_limit);

        let mut last_ordinal = session.last_ordinal;
        let mut kept = 0;
        let mut part_had_turns = false;

        for row in rows {
            let text = if row.role == "user" {
                clean(&row.content)
            } else {
                String::new()
            };
            if !worth_reading(&text) {
                // Not sent, but it was read: the part already in this batch follows
                // it, and a batch is watermarked to the last row it consumed.
                // `last_ordinal` moves too, so a run that read nothing but chatter
                // still does not scan those rows again.
                if let Some(part) = part_after_push(&mut current, &mut corpus.batches, &session.id)
                {
                    part.read_through = row.ordinal;
                }
                last_ordinal = row.ordinal;
                continue;
            }
            // Start a new request before this turn if it would push the current one
            // past its budget. The check is on `messages`, not `chars`, so a single
            // long turn in an empty batch is still sent rather than looping forever.
            if current.messages > 0 && current.chars + text.chars().count() > MAX_BATCH_CHARS {
                corpus.batches.push(std::mem::take(&mut current));
                if corpus.batches.len() >= MAX_BATCHES {
                    // This turn was not sent, so neither the batch nor the watermark
                    // may claim it. It is read again next run.
                    corpus.truncated = true;
                    deferred = true;
                    break;
                }
            }
            let part = match current
                .sessions
                .iter_mut()
                .find(|entry| entry.id == session.id)
            {
                Some(part) => part,
                None => {
                    current.sessions.push(SessionPart {
                        id: session.id.clone(),
                        read_through: row.ordinal,
                        turns: Vec::new(),
                    });
                    current.sessions.last_mut().expect("just added")
                }
            };
            part.read_through = row.ordinal;
            last_ordinal = row.ordinal;
            current.chars += text.chars().count();
            current.messages += 1;
            kept += 1;
            part.turns.push(Turn {
                text,
                ordinal: row.ordinal,
            });
            part_had_turns = true;

            if kept >= MAX_MESSAGES_PER_SESSION {
                corpus.truncated = true;
                break;
            }
        }

        if part_had_turns {
            seen_sessions.insert(session.id.clone());
        } else {
            // Everything unread in this conversation was skipped: empty, code, or
            // the model's own words. Advancing the watermark is what stops the next
            // run scanning it again.
            corpus
                .empty_sessions
                .push((session.id.clone(), last_ordinal));
        }

        if deferred {
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

/// The session's part, in the batch being filled or in the one just pushed.
///
/// A skipped row after a split was still read. If the session's part has already
/// moved into a finished batch, that part's watermark is the only place its
/// ordinal can be recorded -- otherwise the same assistant and chatter rows are
/// scanned again on every run.
fn part_after_push<'a>(
    current: &'a mut Batch,
    batches: &'a mut [Batch],
    id: &str,
) -> Option<&'a mut SessionPart> {
    if let Some(index) = current.sessions.iter().position(|entry| entry.id == id) {
        return current.sessions.get_mut(index);
    }
    batches
        .last_mut()?
        .sessions
        .iter_mut()
        .find(|entry| entry.id == id)
}

/// Trims a stored message to what the model is shown.
fn clean(raw: &str) -> String {
    let normalized = raw.replace("\r\n", "\n").replace('\r', "\n");
    let without_code = strip_code(&normalized);
    let text = without_code.trim();
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

/// Drops fenced blocks, which are almost always a pasted file rather than a fact.
///
/// An unclosed fence hides the rest of the message, which is the honest reading of
/// a half-pasted file: the prose before it is kept and the code is not.
fn strip_code(text: &str) -> String {
    let mut kept = String::with_capacity(text.len());
    let mut in_fence = false;
    for line in text.lines() {
        if line.trim_start().starts_with("```") || line.trim_start().starts_with("~~~") {
            in_fence = !in_fence;
            continue;
        }
        if !in_fence {
            kept.push_str(line);
            kept.push('\n');
        }
    }
    kept
}

/// Whether a cleaned turn is worth spending prompt tokens on.
///
/// Deterministic and deliberately forgiving: the model is the second filter, and
/// this one exists to keep commands, acknowledgements and code out of the request.
fn worth_reading(text: &str) -> bool {
    if text.is_empty() || text.starts_with('/') {
        return false;
    }
    if text.chars().count() < MIN_TURN_CHARS {
        return false;
    }
    // The word floor is a rule about space-separated languages. A Chinese or
    // Japanese sentence is one "word" here and would otherwise be discarded and
    // then marked read forever; the character floor above is its floor instead.
    if text.split_whitespace().count() < MIN_TURN_WORDS && text.is_ascii() {
        return false;
    }
    // A message that is mostly punctuation, markup or symbols is a paste, not a
    // statement. Half the characters have to be words for it to be read.
    let meaningful = text
        .chars()
        .filter(|character| character.is_alphanumeric())
        .count();
    meaningful * 2 >= text.chars().count()
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
        // User turns only: two from the first conversation, one from the second.
        assert_eq!(corpus.messages, 3);
        assert!(!corpus.truncated);
        assert_eq!(corpus.batches[0].sessions.len(), 2);
        // Every kept turn is a user turn by construction; this is the count the
        // page reports.
        assert_eq!(
            corpus.batches[0]
                .sessions
                .iter()
                .map(|part| part.turns.len())
                .sum::<usize>(),
            corpus.messages
        );
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
        seed(&database, "long", "2026-01-02T00:00:00.000Z", 400);
        let corpus = build(&database).expect("corpus");
        assert!(corpus.truncated, "the message cap should be reported");
        assert_eq!(corpus.messages, MAX_MESSAGES_PER_SESSION);
        let (session, ordinal) = corpus.batches[0].watermarks()[0].clone();
        assert_eq!(session, "long");
        // 400 alternating rows are 200 user turns; the cap keeps the 80th, whose
        // ordinal is 158 (every other row is the user's).
        assert_eq!(ordinal, 158);
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
        let long = "word ".repeat(MAX_MESSAGE_CHARS);
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

    /// The assistant's prose is not evidence about the user, and it is most of a
    /// transcript's characters. It must never reach the learner.
    #[test]
    fn assistant_turns_are_not_sent() {
        let database = database();
        database
            .save_chat_session(&serde_json::json!({
                "id": "s1",
                "title": "s1",
                "createdAt": "2026-01-02T00:00:00.000Z",
                "updatedAt": "2026-01-02T00:00:00.000Z",
                "messages": [
                    { "role": "user", "content": "I use Rust (programming language) for backend services." },
                    { "role": "assistant", "content": "That is a good choice for backend services, and here is why in detail." }
                ]
            }))
            .expect("seed");
        let corpus = build(&database).expect("corpus");
        assert_eq!(corpus.messages, 1);
        assert!(corpus.batches[0].sessions[0].turns[0]
            .text
            .starts_with("I use Rust"));
    }

    /// A pasted code block is a file, not a preference. The prose around it is kept.
    #[test]
    fn a_code_block_is_dropped_and_the_prose_surrounding_it_is_kept() {
        let database = database();
        database
            .save_chat_session(&serde_json::json!({
                "id": "code",
                "title": "code",
                "createdAt": "2026-01-02T00:00:00.000Z",
                "updatedAt": "2026-01-02T00:00:00.000Z",
                "messages": [{
                    "role": "user",
                    "content": "I prefer TypeScript (programming language) for app code.\n```ts\nconst x: number = 1;\n```\n"
                }]
            }))
            .expect("seed");
        let corpus = build(&database).expect("corpus");
        let text = &corpus.batches[0].sessions[0].turns[0].text;
        assert!(text.contains("prefer TypeScript"), "{text}");
        assert!(!text.contains("const x"), "{text}");
    }

    /// "thanks", "ok" and commands teach nothing, but their rows are still read so
    /// the next run does not scan them again.
    #[test]
    fn chatter_and_commands_are_skipped_and_the_watermark_advances() {
        let database = database();
        database
            .save_chat_session(&serde_json::json!({
                "id": "chatter",
                "title": "chatter",
                "createdAt": "2026-01-02T00:00:00.000Z",
                "updatedAt": "2026-01-02T00:00:00.000Z",
                "messages": [
                    { "role": "user", "content": "thanks" },
                    { "role": "assistant", "content": "You are welcome." },
                    { "role": "user", "content": "/compact" }
                ]
            }))
            .expect("seed");
        let corpus = build(&database).expect("corpus");
        assert!(corpus.batches.is_empty());
        assert_eq!(corpus.empty_sessions, vec![("chatter".to_string(), 2)]);
    }
}
