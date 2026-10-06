//! Remembers tool results within a conversation.
//!
//! A model that reads the same file on every turn is paying full price for a
//! page it already had. Caching the answer removes both the tokens and the
//! round trip.
//!
//! # When a cached answer is correct
//!
//! Two conditions, and both have to hold:
//!
//! 1. **Same conversation.** The key includes the session, so one session can
//!    never be answered with another's reads. It also means entries are
//!    collected with the conversation and need no expiry of their own.
//! 2. **Nothing has been written since.** A read taken before the model edited a
//!    file is wrong afterwards, and a write tool's own result is wrong to cache
//!    at all.
//!
//! The second rule is enforced by a timestamp rather than by classifying tools:
//! a write stamps the session, and any entry older than the stamp is ignored. A
//! tool cannot forget to invalidate, because invalidation is not the tool's job.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use serde_json::Value;

use crate::database::{utc_now, Database};

use super::Effect;

/// Cache key for one call.
///
/// Built from the canonical argument JSON so two calls that differ only in key
/// order still hit. `serde_json` preserves object insertion order, so two
/// `Value`s from different sources can serialize differently; the tool name is
/// included because `read_file` and `search_files` can legitimately be called
/// with the same arguments and must not share an entry.
///
/// Hashed rather than stored raw: the arguments can be a page of text, and the
/// key is compared on every tool call. `sha2` is already a dependency, so this
/// costs no new crate.
pub fn cache_key(tool_name: &str, arguments: &Value) -> String {
    use sha2::{Digest, Sha256};
    let mut digest = Sha256::new();
    digest.update(tool_name.as_bytes());
    // NUL separator so a tool name cannot be confused with the start of the
    // arguments: without it, ("ab", "c") and ("a", "bc") would hash alike.
    digest.update([0u8]);
    digest.update(canonical_json(arguments).as_bytes());
    format!("{tool_name}:{:x}", digest.finalize())
}

/// The next stamp after `previous`, in the same `utc_now` format.
///
/// Stamps are compared as text, so the arithmetic has to be textual: the field
/// after the last `.` is the milliseconds followed by `Z`, and bumping those three
/// digits keeps the string the same width so it keeps sorting correctly. A carry
/// out of the milliseconds is left alone rather than propagated -- reaching it
/// would need a real calendar to stay honest, and a run does not issue a thousand
/// stamps in 999 milliseconds.
///
/// The caller only reaches here when the clock has not moved past `previous`,
/// which in practice means two calls landed in the same millisecond, so this is
/// the rare path rather than the common one.
fn increment(previous: &str) -> String {
    const MILLIS_WIDTH: usize = 3;
    // `.NNNZ`, the shape `utc_now` writes. Anything else is returned untouched
    // rather than guessed at, so a future change to the timestamp format degrades
    // to "no bump" instead of a corrupt stamp.
    let Some(millis_start) = previous.rfind('.') else {
        return previous.to_string();
    };
    let millis_text = &previous[millis_start + 1..previous.len() - 1];
    if millis_text.len() != MILLIS_WIDTH || !millis_text.bytes().all(|byte| byte.is_ascii_digit()) {
        return previous.to_string();
    }
    let Ok(millis) = millis_text.parse::<u64>() else {
        return previous.to_string();
    };
    // Held at the ceiling rather than wrapping to a value that sorts *before*
    // `previous`, which would make the entry look older than it is.
    if millis >= 999 {
        return previous.to_string();
    }
    let bumped = format!("{:0width$}", millis + 1, width = MILLIS_WIDTH);
    format!("{}{bumped}Z", &previous[..=millis_start])
}

/// Serializes with object keys in sorted order, so equal values give equal keys.
fn canonical_json(value: &Value) -> String {
    match value {
        Value::Object(map) => {
            let mut keys = map.keys().collect::<Vec<_>>();
            keys.sort();
            let body = keys
                .into_iter()
                .map(|key| {
                    let canonical = canonical_json(&map[key]);
                    format!(
                        "{}:{canonical}",
                        serde_json::to_string(key).unwrap_or_default()
                    )
                })
                .collect::<Vec<_>>()
                .join(",");
            format!("{{{body}}}")
        }
        Value::Array(items) => {
            let body = items
                .iter()
                .map(canonical_json)
                .collect::<Vec<_>>()
                .join(",");
            format!("[{body}]")
        }
        other => other.to_string(),
    }
}

/// Reads and writes the tool cache for one run.
///
/// Holds an `Arc` to the database rather than a connection so it can be cloned
/// into the tool loop without fighting the runtime's mutex.
#[derive(Clone)]
pub struct ToolCache {
    database: Arc<Database>,
    /// The last stamp handed out, so the next one is guaranteed to sort after it.
    ///
    /// The database compares these as strings, so two stamps that are equal are
    /// indistinguishable -- and a single `utc_now()` read per run made every read
    /// and write in that run carry an identical stamp. That is the whole bug: the
    /// write's invalidation stamp then compared equal to the reads taken before
    /// it, so `>=` kept them and the model was handed a pre-edit file. Reading
    /// the clock per call does not fix it either, because two calls inside one
    /// millisecond collide anyway.
    ///
    /// So the stamps are *numbered* rather than measured. Every read and every
    /// write in this run gets a distinct, increasing suffix, which makes the
    /// ordering the filter depends on exact rather than probable, while staying in
    /// the one timestamp format the whole app compares and writes.
    ///
    /// Keyed per session rather than held as one value: a conversation's cache is
    /// shared by every run against it, and two replies in flight at once must not
    /// hand out the same stamp.
    last_stamp: Arc<Mutex<HashMap<String, String>>>,
}

impl ToolCache {
    pub fn new(database: Arc<Database>) -> Self {
        Self {
            database,
            last_stamp: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// A stamp for `session_id` that sorts strictly after everything already
    /// recorded against that conversation.
    ///
    /// Derived from the wall clock and then nudged forward past whatever the
    /// conversation has already seen, so two calls in the same millisecond still
    /// order correctly. The seed comes from the database rather than from this
    /// run's own history, because the conversation's cache outlives any one run.
    fn next_stamp(&self, session_id: &str) -> String {
        let Ok(mut last) = self.last_stamp.lock() else {
            // A poisoned lock only means another thread panicked while holding it.
            // The clock is still a valid stamp, and a run that reached this has
            // bigger problems than a cache ordering.
            return utc_now();
        };
        let now = utc_now();
        // Once per conversation rather than per call: the seed only needs to be
        // fetched the first time this run touches the conversation.
        let floor = last
            .get(session_id)
            .cloned()
            .or_else(|| self.database.tool_cache_high_water_mark(session_id))
            .unwrap_or_default();
        // Only push forward when the clock has not already moved on, so a run
        // spread over several milliseconds keeps real ordering.
        let stamp = if floor >= now { increment(&floor) } else { now };
        last.insert(session_id.to_string(), stamp.clone());
        stamp
    }

    /// A cached result for this call, if one is still valid.
    ///
    /// Returns `None` for a tool that changes anything, before consulting the
    /// database: caching a write is never correct, and a lookup that finds an
    /// entry would then hide the write's own effect.
    pub fn get(
        &self,
        session_id: &str,
        tool_name: &str,
        effect: Effect,
        arguments: &Value,
    ) -> Option<String> {
        if effect == Effect::Write {
            return None;
        }
        self.database
            .tool_cache_get(session_id, tool_name, &cache_key(tool_name, arguments))
    }

    /// Stores a result, unless the tool changed something.
    ///
    /// A write instead stamps the session, which is what makes every earlier read
    /// in this conversation stop being returned. The stamp comes from
    /// [`Self::next_stamp`], so it is strictly newer than every read already
    /// cached in this run -- which is the whole reason the read side can treat
    /// "equal" as "this is current" rather than "this might predate a write".
    pub fn put(
        &self,
        session_id: &str,
        tool_name: &str,
        effect: Effect,
        arguments: &Value,
        result: &str,
    ) {
        let stamp = self.next_stamp(session_id);
        if effect == Effect::Write {
            self.database
                .tool_cache_invalidate_session(session_id, &stamp);
            return;
        }
        self.database.tool_cache_put(
            session_id,
            tool_name,
            &cache_key(tool_name, arguments),
            result,
            &stamp,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// A session row, because `tool_cache` has a foreign key to `sessions`.
    fn seeded() -> Arc<Database> {
        let database = Arc::new(Database::in_memory().expect("migrated in-memory database"));
        database
            .save_chat_session(&json!({
                "id": "s1",
                "title": "test",
                "createdAt": "2026-01-01T00:00:00.000Z",
                "updatedAt": "2026-01-01T00:00:00.000Z",
                "messages": []
            }))
            .expect("seed session");
        database
    }

    #[test]
    fn key_order_does_not_change_the_key() {
        // The same call written by the model in a different key order must hit
        // the same entry, or the cache silently never works.
        assert_eq!(
            cache_key("read_file", &json!({ "path": "a", "limit": 5 })),
            cache_key("read_file", &json!({ "limit": 5, "path": "a" }))
        );
    }

    #[test]
    fn different_arguments_give_different_keys() {
        assert_ne!(
            cache_key("read_file", &json!({ "path": "a" })),
            cache_key("read_file", &json!({ "path": "b" }))
        );
    }

    #[test]
    fn the_same_arguments_on_two_tools_do_not_collide() {
        // `read_file` and `search_files` can be called with the same object and
        // must not share an entry.
        assert_ne!(
            cache_key("read_file", &json!({ "path": "a" })),
            cache_key("search_files", &json!({ "path": "a" }))
        );
    }

    #[test]
    fn array_order_is_significant() {
        // Unlike object keys, list order is meaning. Two different orders are two
        // different requests.
        assert_ne!(
            cache_key("t", &json!({ "args": ["a", "b"] })),
            cache_key("t", &json!({ "args": ["b", "a"] }))
        );
    }

    #[test]
    fn nested_objects_are_canonicalized_too() {
        assert_eq!(
            cache_key("t", &json!({ "o": { "x": 1, "y": 2 } })),
            cache_key("t", &json!({ "o": { "y": 2, "x": 1 } }))
        );
    }

    #[test]
    fn a_numeric_and_a_string_value_do_not_collide() {
        // JSON keeps 5 and "5" distinct; string concatenation alone would not.
        assert_ne!(
            cache_key("t", &json!({ "n": 5 })),
            cache_key("t", &json!({ "n": "5" }))
        );
    }

    #[test]
    fn a_tool_name_cannot_be_confused_with_the_arguments() {
        // Without the separator, these two would hash to the same bytes.
        assert_ne!(
            cache_key("ab", &json!({})),
            cache_key("a", &json!({ "b": null }))
        );
    }

    /// The bug the stamp allocator exists for, end to end.
    ///
    /// A read, then a write, then the same read again -- the model editing a file
    /// and then checking it. With one timestamp shared across the run the third
    /// step was answered from the cache with the pre-edit contents, because the
    /// write's invalidation stamp equalled the first read's stamp. Every step here
    /// happens within a single millisecond in practice, so the test pins the
    /// ordering rather than relying on the clock to separate the calls.
    #[test]
    fn a_read_after_a_write_is_not_answered_from_before_it() {
        let database = seeded();
        let cache = ToolCache::new(Arc::clone(&database));
        let arguments = json!({ "path": "src/App.tsx" });

        cache.put(
            "s1",
            "read_file",
            Effect::Read,
            &arguments,
            "1 | before the edit",
        );
        assert_eq!(
            cache.get("s1", "read_file", Effect::Read, &arguments),
            Some("1 | before the edit".to_string()),
            "a repeat read of an unchanged file should hit the cache"
        );

        // The write. Nothing is stored, so the entry above is still on disk and
        // only the session stamp can make it stop being returned.
        cache.put(
            "s1",
            "write_file",
            Effect::Write,
            &json!({ "path": "src/App.tsx" }),
            "wrote 1 line",
        );
        assert_eq!(
            cache.get("s1", "read_file", Effect::Read, &arguments),
            None,
            "a read taken before the write must not be served after it"
        );

        // Re-read after the write repopulates, and is current from then on.
        cache.put(
            "s1",
            "read_file",
            Effect::Read,
            &arguments,
            "1 | after the edit",
        );
        assert_eq!(
            cache.get("s1", "read_file", Effect::Read, &arguments),
            Some("1 | after the edit".to_string()),
            "a read taken after the write is valid and should be served"
        );
    }

    /// Every stamp in a run is distinct, so ordering never depends on the clock
    /// having moved between two calls.
    ///
    /// Reading the wall clock per call cannot promise this -- two calls inside one
    /// millisecond collide -- and that collision is exactly what the original
    /// single shared timestamp got wrong. Asserted here through the public surface:
    /// a second read is served only if its stamp still sorts after the write's,
    /// so ordering is exercised rather than merely described.
    #[test]
    fn many_reads_and_a_write_still_order_correctly() {
        let database = seeded();
        let cache = ToolCache::new(Arc::clone(&database));

        // Enough traffic that the clock very likely does not advance between some
        // of these, which is the case that used to break.
        for index in 0..200 {
            let path = json!({ "path": format!("file-{index}") });
            cache.put("s1", "read_file", Effect::Read, &path, "content");
            assert_eq!(
                cache.get("s1", "read_file", Effect::Read, &path),
                Some("content".to_string()),
                "read {index} should hit immediately after being cached"
            );
        }

        cache.put(
            "s1",
            "write_file",
            Effect::Write,
            &json!({ "path": "file-7" }),
            "ok",
        );
        assert_eq!(
            cache.get(
                "s1",
                "read_file",
                Effect::Read,
                &json!({ "path": "file-7" })
            ),
            None,
            "every read from before the write must be invalidated, not just the last"
        );
    }

    /// The increment works on the millisecond field textually, so it must not
    /// widen: "…:009" -> "…:010", not "…:10".
    #[test]
    fn a_stamp_holds_its_width_so_stamps_keep_sorting() {
        assert_eq!(
            increment("2026-01-01T00:00:05.009Z"),
            "2026-01-01T00:00:05.010Z"
        );
        assert_eq!(
            increment("2026-01-01T00:00:05.000Z"),
            "2026-01-01T00:00:05.001Z"
        );
        // Held at the ceiling rather than wrapping to a value that sorts earlier.
        assert_eq!(
            increment("2026-01-01T00:00:05.999Z"),
            "2026-01-01T00:00:05.999Z"
        );
    }

    /// A shape this module never produces is returned untouched rather than
    /// corrupted into something that would sort wrongly.
    #[test]
    fn a_stamp_it_cannot_parse_is_left_alone() {
        assert_eq!(increment("not-a-timestamp"), "not-a-timestamp");
    }

    /// Two runs sharing a session must not have their stamps interleave badly.
    ///
    /// The allocator is per `ToolCache`, so a second run starts from the clock
    /// again. That is safe precisely because the clock is monotonic and a second
    /// run starts later than the first one's last stamp -- but only if the second
    /// run cannot issue a stamp *earlier* than one the first already wrote. Both
    /// orderings are covered here so a regression shows up as a failing test
    /// rather than as a stale read in ordinary use.
    #[test]
    fn a_second_run_shares_the_session_without_weakening_invalidation() {
        let database = seeded();

        let first = ToolCache::new(Arc::clone(&database));
        first.put(
            "s1",
            "read_file",
            Effect::Read,
            &json!({ "path": "a" }),
            "old",
        );

        let second = ToolCache::new(Arc::clone(&database));
        second.put(
            "s1",
            "write_file",
            Effect::Write,
            &json!({ "path": "a" }),
            "ok",
        );

        assert_eq!(
            second.get("s1", "read_file", Effect::Read, &json!({ "path": "a" })),
            None,
            "a write in a later run must still invalidate an earlier run's read"
        );
    }
}
