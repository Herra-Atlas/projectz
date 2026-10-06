//! Aggregate usage figures for the statistics page.
//!
//! The page reports across every conversation, so the numbers are computed in SQL
//! rather than by loading sessions into the frontend. Token columns live on
//! `messages` (written by migration 3) and tool calls are identifiable by role.
//!
//! Timestamps are ISO-8601 strings (`new Date().toISOString()`), which sort
//! lexicographically. That lets SQLite group by a date prefix with `substr`
//! instead of a date function, and it avoids bucketing entirely for the
//! whole-range totals, which are single-row aggregates.

use rusqlite::{params, Connection};
use serde::Serialize;

/// Row of the token time series, one per bucket, oldest first.
#[derive(Serialize, Clone, Debug)]
pub struct TokenBucket {
    /// Bucket start as an ISO date, e.g. `2026-10-01`.
    pub bucket: String,
    pub prompt_tokens: i64,
    pub completion_tokens: i64,
}

/// One conversation's contribution, used for the per-model and activity figures.
#[derive(Serialize, Clone, Debug)]
pub struct SessionUsage {
    pub id: String,
    pub title: String,
    pub created_at: String,
    pub message_count: i64,
    pub prompt_tokens: i64,
    pub completion_tokens: i64,
}

/// A model (or local alias) with the replies it produced.
#[derive(Serialize, Clone, Debug)]
pub struct ModelUsage {
    pub model_id: String,
    pub provider_id: String,
    pub replies: i64,
    pub prompt_tokens: i64,
    pub completion_tokens: i64,
}

/// One model's tokens in one bucket, for the per-model series.
#[derive(Serialize, Clone, Debug)]
pub struct ModelBucket {
    pub bucket: String,
    pub model_id: String,
    pub provider_id: String,
    pub prompt_tokens: i64,
    pub completion_tokens: i64,
}

/// Everything the statistics page renders, for one timeframe.
#[derive(Serialize, Clone, Debug)]
pub struct UsageReport {
    pub range: String,
    /// Inclusive lower bound as an ISO date, matching `bucket` granularity.
    pub since: String,
    pub sessions: i64,
    pub user_messages: i64,
    pub assistant_messages: i64,
    pub searches: i64,
    pub prompt_tokens: i64,
    pub completion_tokens: i64,
    pub total_tokens: i64,
    /// Prompt tokens the provider reported as served from its cache, over the
    /// period.
    pub cached_tokens: i64,
    /// The prompt tokens those figures were measured against. Not the period's
    /// whole `prompt_tokens`: only the replies that reported a cache figure at
    /// all contribute, so dividing one by the other is a like-for-like ratio.
    pub cache_prompt_tokens: i64,
    /// How many replies reported a cache figure. Zero means the figure is
    /// unknown rather than zero, and the page must not present a rate at all.
    pub cache_reported_replies: i64,
    /// Per-bucket token series, oldest first. Empty when the range has no data.
    pub timeline: Vec<TokenBucket>,
    /// Sessions with their own figures, newest first.
    pub session_usage: Vec<SessionUsage>,
    /// Models with their own figures, busiest first.
    pub models: Vec<ModelUsage>,
    /// Per-model, per-bucket tokens, for the model series view.
    ///
    /// Covers only `MODEL_SERIES_LIMIT` busiest models. A gateway reporting
    /// several hundred models would otherwise produce a chart no one can read,
    /// and the tail contributes a share small enough that the aggregate view it
    /// is compared against is unchanged by dropping it.
    pub model_timeline: Vec<ModelBucket>,
}

/// How many models the per-model series plots.
///
/// A constant rather than a parameter because the number is a readability
/// limit, not a query: past roughly this many lines a chart stops being read at
/// all. Interpolated into the `LIMIT`, so it is a literal from this module and
/// never anything a caller supplied.
const MODEL_SERIES_LIMIT: usize = 6;

/// Counts a column that may be NULL, treating NULL as zero so a conversation
/// that never reported tokens contributes a zero rather than dropping out.
fn sum_or_zero(row: &rusqlite::Row<'_>, index: usize) -> rusqlite::Result<i64> {
    row.get::<_, Option<i64>>(index)
        .map(|value| value.unwrap_or(0))
}

/// Builds a report for activity on or after `since`.
///
/// `since` is an ISO date (`YYYY-MM-DD`). Bucketing happens in the timeline
/// query only, so the caller picks the bucket width through `bucket_sql`.
///
/// Every figure is read from a real column. `save_chat_session` writes the
/// per-reply metrics into `messages.prompt_tokens` / `completion_tokens` and
/// maintains `sessions.prompt_tokens` / `completion_tokens` as a rollup, so no
/// query here parses `metrics_json` and each one is a plain numeric aggregate
/// that SQLite can serve from the `created_at` index.
pub fn build_report(
    connection: &Connection,
    since: &str,
    range: &str,
    bucket_sql: &str,
) -> Result<UsageReport, String> {
    let sessions = connection
        .query_row(
            "SELECT COUNT(*) FROM sessions WHERE substr(created_at,1,10) >= ?1",
            params![since],
            |row| row.get::<_, i64>(0),
        )
        .map_err(|error| error.to_string())?;

    // Activity and search counts come from `messages`, which inherits the
    // parent's `updated_at`, so the filter is on that same column.
    let (user_messages, assistant_messages, searches) = connection
        .query_row(
            "SELECT
                 COALESCE(SUM(role = 'user'), 0),
                 COALESCE(SUM(role = 'assistant'), 0),
                 COALESCE(SUM(role = 'tool'), 0)
             FROM messages WHERE substr(created_at,1,10) >= ?1",
            params![since],
            |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, i64>(2)?,
                ))
            },
        )
        .map_err(|error| error.to_string())?;

    let (prompt_tokens, completion_tokens) = connection
        .query_row(
            "SELECT COALESCE(SUM(prompt_tokens), 0), COALESCE(SUM(completion_tokens), 0)
             FROM messages WHERE substr(created_at,1,10) >= ?1",
            params![since],
            |row| Ok((sum_or_zero(row, 0)?, sum_or_zero(row, 1)?)),
        )
        .map_err(|error| error.to_string())?;

    // `bucket_sql` is supplied by the caller rather than interpolated from
    // user input, so there is no untrusted string in this query.
    //
    // Only replies that actually reported usage are plotted, which is what
    // `completion_tokens IS NOT NULL` selects: a user message and a reply that
    // failed or was stopped leave the column empty.
    let mut timeline_statement = connection
        .prepare(&format!(
            "SELECT {bucket_sql} AS bucket,
                    COALESCE(SUM(prompt_tokens), 0),
                    COALESCE(SUM(completion_tokens), 0)
             FROM messages
             WHERE substr(created_at,1,10) >= ?1 AND completion_tokens IS NOT NULL
             GROUP BY bucket ORDER BY bucket ASC"
        ))
        .map_err(|error| error.to_string())?;
    let timeline = timeline_statement
        .query_map(params![since], |row| {
            Ok(TokenBucket {
                bucket: row.get(0)?,
                prompt_tokens: sum_or_zero(row, 1)?,
                completion_tokens: sum_or_zero(row, 2)?,
            })
        })
        .map_err(|error| error.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())?;

    // Cache hits, reported as two figures and not one rate.
    //
    // `cached_tokens IS NOT NULL` is the whole query, because the column is NULL
    // when a provider reported no cache figure at all. Folding those into either
    // side of a ratio is the mistake this avoids: "the provider did not say" and
    // "the provider cached nothing" are different facts, and averaging them
    // together reports a hit rate nobody achieved. So only replies that said
    // something are counted, and `cache_reported_replies` says how many that was
    // -- a hit rate over two replies is not the same claim as one over two
    // hundred.
    let (cached_tokens, cache_prompt_tokens, cache_reported_replies) = connection
        .query_row(
            "SELECT COALESCE(SUM(cached_tokens), 0),
                    COALESCE(SUM(prompt_tokens), 0),
                    COUNT(*)
             FROM messages
             WHERE substr(created_at,1,10) >= ?1 AND cached_tokens IS NOT NULL",
            params![since],
            |row| {
                Ok((
                    sum_or_zero(row, 0)?,
                    sum_or_zero(row, 1)?,
                    sum_or_zero(row, 2)?,
                ))
            },
        )
        .map_err(|error| error.to_string())?;

    // Token totals come from the `sessions` rollup rather than a join and sum
    // over `messages`, so this reads one row per conversation. The message
    // count still needs the join because it has no denormalised column.
    let mut session_statement = connection
        .prepare(
            "SELECT s.id, s.title, s.created_at,
                    (SELECT COUNT(*) FROM messages m WHERE m.session_id = s.id),
                    COALESCE(s.prompt_tokens, 0),
                    COALESCE(s.completion_tokens, 0)
             FROM sessions s
             WHERE substr(s.created_at,1,10) >= ?1
             ORDER BY s.created_at DESC",
        )
        .map_err(|error| error.to_string())?;
    let session_usage = session_statement
        .query_map(params![since], |row| {
            Ok(SessionUsage {
                id: row.get(0)?,
                title: row.get(1)?,
                created_at: row.get(2)?,
                message_count: sum_or_zero(row, 3)?,
                prompt_tokens: sum_or_zero(row, 4)?,
                completion_tokens: sum_or_zero(row, 5)?,
            })
        })
        .map_err(|error| error.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())?;

    let mut model_statement = connection
        .prepare(
            "SELECT model_id, COALESCE(provider_id, ''), COUNT(*),
                    COALESCE(SUM(prompt_tokens), 0),
                    COALESCE(SUM(completion_tokens), 0)
             FROM messages
             WHERE substr(created_at,1,10) >= ?1 AND model_id IS NOT NULL AND role = 'assistant'
             GROUP BY model_id, provider_id
             ORDER BY COUNT(*) DESC",
        )
        .map_err(|error| error.to_string())?;
    let models = model_statement
        .query_map(params![since], |row| {
            Ok(ModelUsage {
                model_id: row.get(0)?,
                provider_id: row.get(1)?,
                replies: sum_or_zero(row, 2)?,
                prompt_tokens: sum_or_zero(row, 3)?,
                completion_tokens: sum_or_zero(row, 4)?,
            })
        })
        .map_err(|error| error.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())?;

    // The same series as `timeline`, split by model, for the view that plots one
    // line per model instead of two totals.
    //
    // The busiest models are chosen in its own subquery rather than by trimming
    // `models`, because a model can be quiet in one bucket and loud in another;
    // picking by total usage keeps the models worth drawing and drops the long
    // tail of one-reply experiments that would otherwise each claim a colour.
    let mut model_timeline_statement = connection
        .prepare(&format!(
            "SELECT {bucket_sql} AS bucket, model_id, COALESCE(provider_id, ''),
                    COALESCE(SUM(prompt_tokens), 0),
                    COALESCE(SUM(completion_tokens), 0)
             FROM messages
             WHERE substr(created_at,1,10) >= ?1
                   AND model_id IS NOT NULL
                   AND role = 'assistant'
                   AND completion_tokens IS NOT NULL
                   AND model_id IN (
                       SELECT model_id FROM messages
                       WHERE substr(created_at,1,10) >= ?1
                             AND model_id IS NOT NULL AND role = 'assistant'
                       GROUP BY model_id
                       ORDER BY SUM(COALESCE(completion_tokens, 0)) DESC
                       LIMIT {model_series_limit}
                   )
             GROUP BY bucket, model_id, provider_id
             ORDER BY bucket ASC",
            model_series_limit = MODEL_SERIES_LIMIT,
        ))
        .map_err(|error| error.to_string())?;
    let model_timeline = model_timeline_statement
        .query_map(params![since], |row| {
            Ok(ModelBucket {
                bucket: row.get(0)?,
                model_id: row.get(1)?,
                provider_id: row.get(2)?,
                prompt_tokens: sum_or_zero(row, 3)?,
                completion_tokens: sum_or_zero(row, 4)?,
            })
        })
        .map_err(|error| error.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())?;

    Ok(UsageReport {
        range: range.to_string(),
        since: since.to_string(),
        sessions,
        user_messages,
        assistant_messages,
        searches,
        prompt_tokens,
        completion_tokens,
        total_tokens: prompt_tokens + completion_tokens,
        cached_tokens,
        cache_prompt_tokens,
        cache_reported_replies,
        timeline,
        session_usage,
        models,
        model_timeline,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::params;

    /// Schema covering only what the report queries touch, matching the real
    /// columns `save_chat_session` writes: per-reply token counts on
    /// `messages`, and the rollup on `sessions`.
    fn fixture() -> Connection {
        let connection = Connection::open_in_memory().unwrap();
        connection
            .execute_batch(
                "CREATE TABLE sessions (
                     id TEXT PRIMARY KEY,
                     title TEXT NOT NULL,
                     created_at TEXT NOT NULL,
                     prompt_tokens INTEGER,
                     completion_tokens INTEGER
                 );
                 CREATE TABLE messages (
                     id TEXT PRIMARY KEY,
                     session_id TEXT NOT NULL,
                     role TEXT NOT NULL,
                     created_at TEXT NOT NULL,
                     model_id TEXT,
                     provider_id TEXT,
                     prompt_tokens INTEGER,
                     completion_tokens INTEGER,
                     -- Nullable with no default on purpose: NULL is how a row
                     -- records that the provider reported no cache figure, and a
                     -- default of 0 would turn an unknown into a fact. See
                     -- `cache_hits_ignore_replies_that_reported_no_figure`.
                     cached_tokens INTEGER
                 );",
            )
            .unwrap();
        connection
    }

    fn insert_session(connection: &Connection, id: &str, title: &str, date: &str) {
        connection
            .execute(
                "INSERT INTO sessions (id,title,created_at) VALUES (?1,?2,?3)",
                params![id, title, date],
            )
            .unwrap();
    }

    /// Adds a message and keeps the session rollup in step, the way
    /// `save_chat_session` does inside its transaction.
    fn insert_message(
        connection: &Connection,
        id: &str,
        session: &str,
        role: &str,
        date: &str,
        model: Option<&str>,
        provider: Option<&str>,
        prompt: Option<i64>,
        completion: Option<i64>,
    ) {
        connection
            .execute(
                "INSERT INTO messages (id,session_id,role,created_at,model_id,provider_id,prompt_tokens,completion_tokens)
                 VALUES (?1,?2,?3,?4,?5,?6,?7,?8)",
                params![id, session, role, date, model, provider, prompt, completion],
            )
            .unwrap();
        // The rollup columns start NULL, and NULL + n is NULL in SQL, so they
        // are coalesced on the way in. `save_chat_session` writes the absolute
        // totals rather than incrementing, but the fixture accumulates per
        // message so it mirrors that end state.
        connection
            .execute(
                "UPDATE sessions SET
                     prompt_tokens = COALESCE(prompt_tokens, 0) + ?2,
                     completion_tokens = COALESCE(completion_tokens, 0) + ?3
                 WHERE id = ?1",
                params![session, prompt.unwrap_or(0), completion.unwrap_or(0)],
            )
            .unwrap();
    }

    /// A reply that also reported a cache figure.
    ///
    /// Separate from `insert_message` rather than an extra argument on it,
    /// because "the provider reported no cache figure" is the *normal* case for
    /// most of the suite and making every call pass `None` would say nothing.
    fn insert_cached_reply(
        connection: &Connection,
        id: &str,
        session: &str,
        date: &str,
        prompt: i64,
        completion: i64,
        cached: i64,
    ) {
        insert_message(
            connection,
            id,
            session,
            "assistant",
            date,
            Some("m"),
            Some("p"),
            Some(prompt),
            Some(completion),
        );
        connection
            .execute(
                "UPDATE messages SET cached_tokens = ?2 WHERE id = ?1",
                params![id, cached],
            )
            .unwrap();
    }

    /// A cache hit rate is only shown when a provider reported one, and it is
    /// measured only over the replies that did.
    ///
    /// A reply from a provider that reports nothing leaves NULL and must not
    /// count as a zero-hit reply: folding those in would report a rate lower than
    /// any provider achieved.
    #[test]
    fn cache_hits_ignore_replies_that_reported_no_figure() {
        let connection = fixture();
        insert_session(&connection, "s1", "Cached", "2026-10-01");
        // Two replies that reported a cache figure: 400 of 1000 prompt tokens.
        insert_cached_reply(&connection, "m1", "s1", "2026-10-01", 1000, 50, 400);
        insert_cached_reply(&connection, "m2", "s1", "2026-10-01", 1000, 50, 300);
        // One that reported nothing at all.
        insert_message(
            &connection,
            "m3",
            "s1",
            "assistant",
            "2026-10-01",
            Some("m"),
            Some("p"),
            Some(5000),
            Some(50),
        );

        let report =
            build_report(&connection, "2026-10-01", "30d", "substr(created_at,1,10)").unwrap();
        assert_eq!(report.cached_tokens, 700);
        // 2000, not 7000: the unreported reply is excluded from both sides, so
        // the rate is a like-for-like ratio rather than a diluted one.
        assert_eq!(report.cache_prompt_tokens, 2000);
        assert_eq!(report.cache_reported_replies, 2);
    }

    /// A provider that reports no cache figure yields an unknown, not a zero.
    ///
    /// The page has to be able to tell these apart: "nothing was cached" is a
    /// figure worth showing and "the provider never said" is not.
    #[test]
    fn a_provider_that_reports_no_cache_figure_reports_zero_replies_not_zero_hits() {
        let connection = fixture();
        insert_session(&connection, "s1", "Uncached", "2026-10-01");
        insert_message(
            &connection,
            "m1",
            "s1",
            "assistant",
            "2026-10-01",
            Some("m"),
            Some("p"),
            Some(1000),
            Some(50),
        );

        let report =
            build_report(&connection, "2026-10-01", "30d", "substr(created_at,1,10)").unwrap();
        assert_eq!(report.cache_reported_replies, 0);
        assert_eq!(report.cached_tokens, 0);
    }

    #[test]
    fn totals_sum_prompt_and_completion_tokens() {
        let connection = fixture();
        insert_session(&connection, "s1", "First", "2026-10-01T10:00:00.000Z");
        insert_message(
            &connection,
            "m1",
            "s1",
            "user",
            "2026-10-01T10:00:00.000Z",
            None,
            None,
            None,
            None,
        );
        insert_message(
            &connection,
            "m2",
            "s1",
            "assistant",
            "2026-10-01T10:00:05.000Z",
            Some("gpt"),
            Some("openai"),
            Some(100),
            Some(40),
        );
        insert_message(
            &connection,
            "m3",
            "s1",
            "assistant",
            "2026-10-02T10:00:05.000Z",
            Some("gpt"),
            Some("openai"),
            Some(60),
            Some(20),
        );

        let report =
            build_report(&connection, "2026-10-01", "30d", "substr(created_at,1,10)").unwrap();

        assert_eq!(report.sessions, 1);
        assert_eq!(report.user_messages, 1);
        assert_eq!(report.assistant_messages, 2);
        assert_eq!(report.prompt_tokens, 160);
        assert_eq!(report.completion_tokens, 60);
        assert_eq!(report.total_tokens, 220);
    }

    #[test]
    fn tokens_outside_the_range_are_excluded() {
        let connection = fixture();
        insert_session(&connection, "s1", "First", "2026-09-01T10:00:00.000Z");
        insert_session(&connection, "s2", "Second", "2026-10-05T10:00:00.000Z");
        insert_message(
            &connection,
            "old",
            "s1",
            "assistant",
            "2026-09-01T10:00:05.000Z",
            Some("gpt"),
            Some("openai"),
            Some(999),
            Some(999),
        );
        insert_message(
            &connection,
            "new",
            "s2",
            "assistant",
            "2026-10-05T10:00:05.000Z",
            Some("gpt"),
            Some("openai"),
            Some(10),
            Some(5),
        );

        let report =
            build_report(&connection, "2026-10-01", "7d", "substr(created_at,1,10)").unwrap();

        assert_eq!(report.sessions, 1);
        assert_eq!(report.prompt_tokens, 10);
        assert_eq!(report.completion_tokens, 5);
        assert_eq!(report.assistant_messages, 1);
    }

    #[test]
    fn timeline_buckets_by_the_supplied_expression() {
        let connection = fixture();
        insert_session(&connection, "s1", "First", "2026-10-01T10:00:00.000Z");
        insert_message(
            &connection,
            "m1",
            "s1",
            "assistant",
            "2026-10-01T09:00:00.000Z",
            Some("gpt"),
            Some("openai"),
            Some(10),
            Some(4),
        );
        insert_message(
            &connection,
            "m2",
            "s1",
            "assistant",
            "2026-10-01T18:00:00.000Z",
            Some("gpt"),
            Some("openai"),
            Some(5),
            Some(6),
        );
        insert_message(
            &connection,
            "m3",
            "s1",
            "assistant",
            "2026-10-02T11:00:00.000Z",
            Some("gpt"),
            Some("openai"),
            Some(7),
            Some(8),
        );

        let report =
            build_report(&connection, "2026-10-01", "7d", "substr(created_at,1,10)").unwrap();

        assert_eq!(report.timeline.len(), 2);
        assert_eq!(report.timeline[0].bucket, "2026-10-01");
        assert_eq!(report.timeline[0].prompt_tokens, 15);
        assert_eq!(report.timeline[0].completion_tokens, 10);
        assert_eq!(report.timeline[1].bucket, "2026-10-02");
        assert_eq!(report.timeline[1].prompt_tokens, 7);
        assert_eq!(report.timeline[1].completion_tokens, 8);
    }

    #[test]
    fn model_timeline_splits_the_series_by_model() {
        let connection = fixture();
        insert_session(&connection, "s1", "First", "2026-10-01T10:00:00.000Z");
        insert_message(
            &connection,
            "m1",
            "s1",
            "assistant",
            "2026-10-01T09:00:00.000Z",
            Some("gpt"),
            Some("openai"),
            Some(10),
            Some(4),
        );
        insert_message(
            &connection,
            "m2",
            "s1",
            "assistant",
            "2026-10-01T11:00:00.000Z",
            Some("step"),
            Some("kilo"),
            Some(20),
            Some(9),
        );

        let report =
            build_report(&connection, "2026-10-01", "7d", "substr(created_at,1,10)").unwrap();

        // The aggregate view carries one bucket holding both replies' totals,
        // while the per-model view carries one entry per model for that same
        // bucket. The two sum to the same figure, which is the point: one is the
        // same data drawn without the split.
        assert_eq!(report.timeline.len(), 1);
        assert_eq!(report.timeline[0].prompt_tokens, 30);
        assert_eq!(report.model_timeline.len(), 2);
        let by_model: Vec<(&str, i64, i64)> = report
            .model_timeline
            .iter()
            .map(|entry| {
                (
                    entry.model_id.as_str(),
                    entry.prompt_tokens,
                    entry.completion_tokens,
                )
            })
            .collect();
        assert!(by_model.contains(&("gpt", 10, 4)));
        assert!(by_model.contains(&("step", 20, 9)));
    }

    #[test]
    fn model_timeline_keeps_the_busiest_models_and_drops_the_tail() {
        let connection = fixture();
        insert_session(&connection, "s1", "First", "2026-10-01T10:00:00.000Z");
        // One more model than the limit, so the drop is actually exercised.
        for index in 0..=MODEL_SERIES_LIMIT {
            insert_message(
                &connection,
                &format!("m{index}"),
                "s1",
                "assistant",
                "2026-10-01T09:00:00.000Z",
                Some(&format!("model-{index}")),
                Some("openai"),
                Some(1),
                // Later models generate fewer tokens, so the ranking is
                // unambiguous and the quietest one is the one dropped.
                Some((index + 1) as i64),
            );
        }

        let report =
            build_report(&connection, "2026-10-01", "7d", "substr(created_at,1,10)").unwrap();

        assert_eq!(report.models.len(), MODEL_SERIES_LIMIT + 1);
        assert_eq!(report.model_timeline.len(), MODEL_SERIES_LIMIT);
        // The quietest model is the one excluded, and every kept model is
        // busier than it.
        let kept: Vec<&str> = report
            .model_timeline
            .iter()
            .map(|entry| entry.model_id.as_str())
            .collect();
        assert!(!kept.contains(&"model-0"));
        assert!(kept.contains(&"model-1"));
    }

    #[test]
    fn model_timeline_ignores_a_reply_that_reported_no_usage() {
        let connection = fixture();
        insert_session(&connection, "s1", "First", "2026-10-01T10:00:00.000Z");
        insert_message(
            &connection,
            "m1",
            "s1",
            "assistant",
            "2026-10-01T09:00:00.000Z",
            Some("gpt"),
            Some("openai"),
            Some(10),
            Some(4),
        );
        // A reply that failed or was stopped reports nothing. Counting it would
        // claim the model spent nothing in that bucket, which reads as an idle
        // period rather than a missing measurement.
        insert_message(
            &connection,
            "m2",
            "s1",
            "assistant",
            "2026-10-01T12:00:00.000Z",
            Some("other"),
            Some("openai"),
            None,
            None,
        );

        let report =
            build_report(&connection, "2026-10-01", "7d", "substr(created_at,1,10)").unwrap();

        assert_eq!(report.model_timeline.len(), 1);
        assert_eq!(report.model_timeline[0].model_id, "gpt");
    }

    #[test]
    fn messages_without_token_metrics_still_count_as_activity() {
        let connection = fixture();
        insert_session(&connection, "s1", "First", "2026-10-01T10:00:00.000Z");
        // A user message and a failed reply carry no metrics, so token columns
        // are NULL. They must still appear in the activity counts.
        insert_message(
            &connection,
            "m1",
            "s1",
            "user",
            "2026-10-01T10:00:00.000Z",
            None,
            None,
            None,
            None,
        );
        insert_message(
            &connection,
            "m2",
            "s1",
            "assistant",
            "2026-10-01T10:00:05.000Z",
            Some("gpt"),
            Some("openai"),
            None,
            None,
        );
        insert_message(
            &connection,
            "m3",
            "s1",
            "tool",
            "2026-10-01T10:00:07.000Z",
            None,
            None,
            None,
            None,
        );

        let report =
            build_report(&connection, "2026-10-01", "7d", "substr(created_at,1,10)").unwrap();

        assert_eq!(report.user_messages, 1);
        assert_eq!(report.assistant_messages, 1);
        assert_eq!(report.searches, 1);
        assert_eq!(report.prompt_tokens, 0);
        assert_eq!(report.completion_tokens, 0);
        // The timeline only plots measured replies, so it stays empty.
        assert!(report.timeline.is_empty());
    }

    /// The session figures come from the `sessions` rollup, so a report agrees
    /// with the per-message sum. The two must not be able to drift apart,
    /// because the page shows both.
    #[test]
    fn session_figures_come_from_the_rollup_and_match_the_message_totals() {
        let connection = fixture();
        insert_session(&connection, "s1", "First", "2026-10-01T10:00:00.000Z");
        insert_message(
            &connection,
            "m1",
            "s1",
            "assistant",
            "2026-10-01T10:00:05.000Z",
            Some("gpt"),
            Some("openai"),
            Some(24),
            Some(66),
        );
        insert_message(
            &connection,
            "m2",
            "s1",
            "assistant",
            "2026-10-01T10:01:05.000Z",
            Some("gpt"),
            Some("openai"),
            Some(6),
            Some(4),
        );

        let report =
            build_report(&connection, "2026-10-01", "7d", "substr(created_at,1,10)").unwrap();

        assert_eq!(report.session_usage.len(), 1);
        let session = &report.session_usage[0];
        assert_eq!(session.prompt_tokens, 30);
        assert_eq!(session.completion_tokens, 70);
        // The rollup and the direct aggregate must agree.
        assert_eq!(session.prompt_tokens, report.prompt_tokens);
        assert_eq!(session.completion_tokens, report.completion_tokens);
        assert_eq!(session.message_count, 2);
    }

    #[test]
    fn models_group_by_model_and_provider() {
        let connection = fixture();
        insert_session(&connection, "s1", "First", "2026-10-01T10:00:00.000Z");
        insert_message(
            &connection,
            "m1",
            "s1",
            "assistant",
            "2026-10-01T10:00:00.000Z",
            Some("gpt-4o"),
            Some("openai"),
            Some(10),
            Some(4),
        );
        insert_message(
            &connection,
            "m2",
            "s1",
            "assistant",
            "2026-10-01T10:01:00.000Z",
            Some("gpt-4o"),
            Some("openai"),
            Some(20),
            Some(8),
        );
        // Same model name behind a different provider is a separate row.
        insert_message(
            &connection,
            "m3",
            "s1",
            "assistant",
            "2026-10-01T10:02:00.000Z",
            Some("gpt-4o"),
            Some("groq"),
            Some(5),
            Some(2),
        );

        let report =
            build_report(&connection, "2026-10-01", "7d", "substr(created_at,1,10)").unwrap();

        assert_eq!(report.models.len(), 2);
        let openai = report
            .models
            .iter()
            .find(|m| m.provider_id == "openai")
            .unwrap();
        assert_eq!(openai.replies, 2);
        assert_eq!(openai.prompt_tokens, 30);
        assert_eq!(openai.completion_tokens, 12);
    }

    #[test]
    fn empty_database_reports_zeroes_rather_than_failing() {
        let connection = fixture();
        let report =
            build_report(&connection, "2026-10-01", "7d", "substr(created_at,1,10)").unwrap();

        assert_eq!(report.sessions, 0);
        assert_eq!(report.total_tokens, 0);
        assert!(report.timeline.is_empty());
        assert!(report.session_usage.is_empty());
        assert!(report.models.is_empty());
    }
}
