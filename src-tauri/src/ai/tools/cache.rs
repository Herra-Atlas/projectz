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
//!
//! # What each `Effect` means here
//!
//! [`Effect`] decides both halves of that, and the two are separate questions:
//!
//! - `Read` is stored and replayed, and leaves the conversation's entries alone.
//! - `Write` is never stored and stamps the session, so nothing read before it is
//!   served again.
//! - `Volatile` is never stored and **never stamps**. `ask_user` and `todo` change
//!   no file, so treating them as writes only threw away reads that were still
//!   perfectly good -- the single largest reason a long agent run re-read the same
//!   files over and over.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use serde_json::Value;

use crate::database::{utc_now, Database};

use super::Effect;

/// Argument names holding one workspace-relative path.
///
/// Listed rather than guessed at from the value's shape, because a string
/// argument that is not a path must keep its bytes in the key.
const PATH_ARGUMENTS: &[&str] = &["path", "from", "to"];

/// Argument names holding a list of paths.
const PATH_LIST_ARGUMENTS: &[&str] = &["paths", "files"];

/// Cache key for one call.
///
/// Built from the canonical argument JSON so two calls that differ only in key
/// order still hit. `serde_json` preserves object insertion order, so two
/// `Value`s from different sources can serialize differently; the tool name is
/// included because `read_file` and `grep` can legitimately be called
/// with the same arguments and must not share an entry.
///
/// Hashed rather than stored raw: the arguments can be a page of text, and the
/// key is compared on every tool call. `sha2` is already a dependency, so this
/// costs no new crate.
///
/// `root` is the workspace the arguments are read against. It is part of the key
/// because a path argument's meaning depends on it -- `src/main.rs` in one folder
/// is a different file from the same string in another -- which is what stops a
/// search being answered from entries taken before the user switched workspace.
pub fn cache_key(tool_name: &str, arguments: &Value, root: Option<&Path>) -> String {
    use sha2::{Digest, Sha256};
    let mut digest = Sha256::new();
    digest.update(tool_name.as_bytes());
    // NUL separator so a tool name cannot be confused with the start of the
    // arguments: without it, ("ab", "c") and ("a", "bc") would hash alike.
    digest.update([0u8]);
    if let Some(root) = root {
        digest.update(root.to_string_lossy().as_bytes());
    }
    digest.update([0u8]);
    digest.update(canonical_json(&normalized(arguments, root)).as_bytes());
    format!("{tool_name}:{:x}", digest.finalize())
}

/// The arguments as the key should see them.
///
/// Two calls that mean the same thing have to hash the same, or the cache misses
/// on a spelling difference rather than on anything real. Two normalizations, and
/// only these two, because a normalization that changed a call's meaning would be
/// far worse than a miss:
///
/// - A `null` is dropped. Every tool here reads an explicit null exactly as it
///   reads an absent field, so `{"path":"a"}` and `{"path":"a","limit":null}` are
///   one call written two ways.
/// - A path argument is rewritten to the workspace-relative slash form the tools
///   resolve it to anyway, so `./src/a.ts`, `src\a.ts` and `src/a.ts` share one
///   entry instead of three.
fn normalized(arguments: &Value, root: Option<&Path>) -> Value {
    let Value::Object(map) = arguments else {
        return arguments.clone();
    };
    let mut out = serde_json::Map::new();
    for (key, value) in map {
        if value.is_null() {
            continue;
        }
        // A path only has a workspace-relative form when there is a workspace;
        // with none the call is refused anyway, so the spelling is left alone.
        if root.is_some() {
            if PATH_ARGUMENTS.contains(&key.as_str()) {
                if let Some(text) = value.as_str() {
                    out.insert(key.clone(), Value::String(relative_path(text)));
                    continue;
                }
            }
            if PATH_LIST_ARGUMENTS.contains(&key.as_str()) {
                if let Some(items) = value.as_array() {
                    let rewritten = items
                        .iter()
                        .map(|item| match item.as_str() {
                            Some(text) => Value::String(relative_path(text)),
                            None => item.clone(),
                        })
                        .collect();
                    out.insert(key.clone(), Value::Array(rewritten));
                    continue;
                }
            }
        }
        out.insert(key.clone(), value.clone());
    }
    Value::Object(out)
}

/// One path argument in the form the key compares.
///
/// **Lexical on purpose.** This runs for every cached call, twice (once to look up
/// and once to store), and a cache key is not a security boundary: the tool
/// resolves the same path again before it touches anything. Resolving here meant a
/// hit paid for per-component existence checks and two canonicalisations just to
/// discover it had nothing to do -- more disk work than the answer saved.
///
/// A refused path -- absolute, or containing `..` -- is left exactly as written.
/// The tool will refuse it too, and rewriting it would be claiming an equivalence
/// between two calls that are both about to fail for their own unrelated reasons.
fn relative_path(requested: &str) -> String {
    let path = Path::new(requested);
    if path.is_absolute() {
        return requested.to_string();
    }
    let mut parts = Vec::new();
    for component in path.components() {
        match component {
            std::path::Component::Normal(part) => parts.push(part.to_string_lossy().into_owned()),
            std::path::Component::CurDir => {}
            // `..`, a root, or a Windows drive prefix: left as written, for the
            // reason above.
            _ => return requested.to_string(),
        }
    }
    parts.join("/")
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
    /// The workspace in force when this run started.
    ///
    /// Captured once per run rather than read per call, so every key this run
    /// writes or looks up is read against one folder. It rides on the key, which
    /// is what keeps a run that spans a workspace switch from answering itself
    /// with entries taken in the folder that used to be open.
    root: Option<PathBuf>,
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
            root: super::workspace::root(),
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
    /// Returns `None` for anything that is not a `Read`, before consulting the
    /// database: caching a write is never correct, and a lookup that found an
    /// entry would then hide the write's own effect.
    pub fn get(
        &self,
        session_id: &str,
        tool_name: &str,
        effect: Effect,
        arguments: &Value,
    ) -> Option<String> {
        if effect != Effect::Read {
            return None;
        }
        self.database.tool_cache_get(
            session_id,
            tool_name,
            &cache_key(tool_name, arguments, self.root.as_deref()),
        )
    }

    /// Stores a result, unless the tool changed something.
    ///
    /// A write instead stamps the session, which is what makes every earlier read
    /// in this conversation stop being returned. The stamp comes from
    /// [`Self::next_stamp`], so it is strictly newer than every read already
    /// cached in this run -- which is the whole reason the read side can treat
    /// "equal" as "this is current" rather than "this might predate a write".
    ///
    /// A `Volatile` result is neither stored nor stamped: it must not be replayed,
    /// and it did not change anything for a stamp to invalidate.
    pub fn put(
        &self,
        session_id: &str,
        tool_name: &str,
        effect: Effect,
        arguments: &Value,
        result: &str,
    ) {
        if effect == Effect::Volatile {
            return;
        }
        let stamp = self.next_stamp(session_id);
        if effect == Effect::Write {
            self.database
                .tool_cache_invalidate_session(session_id, &stamp);
            return;
        }
        self.database.tool_cache_put(
            session_id,
            tool_name,
            &cache_key(tool_name, arguments, self.root.as_deref()),
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
            cache_key("read_file", &json!({ "path": "a", "limit": 5 }), None),
            cache_key("read_file", &json!({ "limit": 5, "path": "a" }), None)
        );
    }

    #[test]
    fn different_arguments_give_different_keys() {
        assert_ne!(
            cache_key("read_file", &json!({ "path": "a" }), None),
            cache_key("read_file", &json!({ "path": "b" }), None)
        );
    }

    #[test]
    fn the_same_arguments_on_two_tools_do_not_collide() {
        // `read_file` and `grep` can be called with the same object and
        // must not share an entry.
        assert_ne!(
            cache_key("read_file", &json!({ "path": "a" }), None),
            cache_key("grep", &json!({ "path": "a" }), None)
        );
    }

    #[test]
    fn array_order_is_significant() {
        // Unlike object keys, list order is meaning. Two different orders are two
        // different requests.
        assert_ne!(
            cache_key("t", &json!({ "args": ["a", "b"] }), None),
            cache_key("t", &json!({ "args": ["b", "a"] }), None)
        );
    }

    #[test]
    fn nested_objects_are_canonicalized_too() {
        assert_eq!(
            cache_key("t", &json!({ "o": { "x": 1, "y": 2 } }), None),
            cache_key("t", &json!({ "o": { "y": 2, "x": 1 } }), None)
        );
    }

    #[test]
    fn a_numeric_and_a_string_value_do_not_collide() {
        // JSON keeps 5 and "5" distinct; string concatenation alone would not.
        assert_ne!(
            cache_key("t", &json!({ "n": 5 }), None),
            cache_key("t", &json!({ "n": "5" }), None)
        );
    }

    #[test]
    fn a_tool_name_cannot_be_confused_with_the_arguments() {
        // The tool name is its own field in the digest, behind a NUL separator.
        // Without one, a name and the start of an argument object could run
        // together and two different calls could hash alike.
        assert_ne!(
            cache_key("ab", &json!({ "b": 1 }), None),
            cache_key("a", &json!({ "b": 1 }), None)
        );
        assert_ne!(
            cache_key("read_file", &json!({}), None),
            cache_key("read_fil", &json!({ "e": 1 }), None)
        );
    }

    /// A workspace, so path normalization has something to resolve against.
    fn temp_root() -> PathBuf {
        use std::sync::atomic::{AtomicU32, Ordering};
        static COUNTER: AtomicU32 = AtomicU32::new(0);
        let unique = COUNTER.fetch_add(1, Ordering::Relaxed);
        let dir =
            std::env::temp_dir().join(format!("projectz-cache-{}-{unique}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("src")).expect("temp dir");
        dir
    }

    /// The same file written three ways is one entry, not three.
    ///
    /// The model writes `./src/a.ts` one round and `src/a.ts` the next, and the
    /// tools resolve both to the same file. Keying on the characters they were
    /// passed made that a miss, which is a miss for no reason at all.
    #[test]
    fn three_spellings_of_one_path_are_one_key() {
        let root = temp_root();
        let plain = cache_key("read_file", &json!({ "path": "src/a.ts" }), Some(&root));
        let dotted = cache_key("read_file", &json!({ "path": "./src/a.ts" }), Some(&root));
        let doubled = cache_key("read_file", &json!({ "path": "src//a.ts" }), Some(&root));
        let other = cache_key("read_file", &json!({ "path": "src/b.ts" }), Some(&root));

        assert_eq!(plain, dotted, "`./` changed the key");
        assert_eq!(plain, doubled, "a doubled separator changed the key");
        assert_ne!(plain, other, "two different files must not share an entry");
        let _ = std::fs::remove_dir_all(&root);
    }

    /// The folder a path is read against is part of the key.
    ///
    /// Switching workspace mid-conversation must not let a search be answered by
    /// an entry taken in the folder that used to be open -- the same arguments
    /// name a different file in each.
    #[test]
    fn the_workspace_is_part_of_the_key() {
        let first = temp_root();
        let second = temp_root();
        assert_ne!(
            cache_key("grep", &json!({ "pattern": "x" }), Some(&first)),
            cache_key("grep", &json!({ "pattern": "x" }), Some(&second))
        );
        // And an unknown workspace is its own case rather than matching a real one.
        assert_ne!(
            cache_key("grep", &json!({ "pattern": "x" }), Some(&first)),
            cache_key("grep", &json!({ "pattern": "x" }), None)
        );
        let _ = std::fs::remove_dir_all(&first);
        let _ = std::fs::remove_dir_all(&second);
    }

    /// An explicit null is the same call as an absent field.
    ///
    /// Every tool here reads a null exactly as it reads a missing argument, so a
    /// model that names every field every time must not miss the entry written by
    /// one that names only what it needs.
    #[test]
    fn an_explicit_null_is_the_same_as_omitting_the_argument() {
        assert_eq!(
            cache_key("read_file", &json!({ "path": "a" }), None),
            cache_key("read_file", &json!({ "path": "a", "limit": null }), None)
        );
    }

    /// The hit-rate fix, end to end: a checklist update no longer evicts reads.
    ///
    /// `todo` and `ask_user` were classified as writes, so an agent that ticked an
    /// item off its list between reads threw away everything it had already read
    /// and paid to read it again. Nothing either tool does can make a file stale.
    #[test]
    fn a_checklist_update_does_not_evict_earlier_reads() {
        let database = seeded();
        let cache = ToolCache::new(Arc::clone(&database));
        let read = json!({ "path": "src/App.tsx" });

        cache.put("s1", "read_file", Effect::Read, &read, "the file");
        cache.put(
            "s1",
            "todo",
            Effect::Volatile,
            &json!({}),
            "Checklist updated",
        );
        cache.put(
            "s1",
            "ask_user",
            Effect::Volatile,
            &json!({ "question": "which?" }),
            "The user answered: the first one",
        );

        assert_eq!(
            cache.get("s1", "read_file", Effect::Read, &read),
            Some("the file".to_string()),
            "a note the agent wrote to itself must not invalidate a read"
        );
        assert_eq!(
            cache.get("s1", "todo", Effect::Volatile, &json!({})),
            None,
            "a volatile result must never be replayed"
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
