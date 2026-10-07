use std::{fs, path::PathBuf, sync::Mutex};

use rusqlite::{params, Connection, OptionalExtension};
use serde::{de::DeserializeOwned, Serialize};
use serde_json::Value;

use transcripts::SESSION_COLUMNS;

use crate::ai::{
    local::{LocalModel, LocalRuntimeSettings},
    remote::types::Endpoint,
};

pub mod skills;
pub mod statistics;
pub mod transcripts;
pub mod workspaces;

pub struct Database {
    connection: Mutex<Connection>,
}

/// The schema version a fully migrated database reports.
///
/// Every migration's `PRAGMA user_version=N` and the tests that guard the schema
/// both refer to this, so adding a migration is one edit here plus one in
/// `migrate`. Before this existed, each new migration silently broke a test that
/// had hardcoded the old number.
pub const LATEST_SCHEMA_VERSION: i64 = 12;

/// The current time as UTC ISO-8601, the format every timestamp uses.
///
/// One definition because two call sites now need it and the tool cache *compares*
/// timestamps as strings: the format sorting chronologically is what makes
/// `created_at >= cache_invalidated_at` correct, so a second copy of this in a
/// different shape would silently break invalidation rather than fail to compile.
///
/// Milliseconds with an explicit `Z`, matching what the frontend writes. The
/// cache's own stamps are derived from this rather than read from it, because two
/// calls in one millisecond would otherwise be indistinguishable -- see
/// `ToolCache::next_stamp`.
pub fn utc_now() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

impl Database {
    /// A migrated database that lives only as long as the connection.
    ///
    /// `pub(crate)` and behind `#[cfg(test)]` rather than a public constructor,
    /// because a caller reaching for an in-memory database in production code is
    /// asking for state that disappears at the end of the process -- but a test
    /// in another module needs one, and duplicating the `open_in_memory` plus
    /// `migrate` incantation at every call site is how they drift from the schema.
    #[cfg(test)]
    pub(crate) fn open_in_memory() -> Self {
        let mut connection = Connection::open_in_memory().expect("in-memory database");
        migrate(&mut connection).expect("migrations");
        Self {
            connection: Mutex::new(connection),
        }
    }

    pub fn open(path: PathBuf) -> Result<Self, String> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|error| error.to_string())?;
        }
        let connection = Connection::open(path).map_err(|error| error.to_string())?;
        connection
            .pragma_update(None, "journal_mode", "WAL")
            .map_err(|error| error.to_string())?;
        connection
            .pragma_update(None, "foreign_keys", "ON")
            .map_err(|error| error.to_string())?;
        connection
            .busy_timeout(std::time::Duration::from_secs(3))
            .map_err(|error| error.to_string())?;
        Ok(Self {
            connection: Mutex::new(connection),
        })
    }

    /// An empty, fully migrated database held in memory.
    ///
    /// Exists for tests, which need a real schema and real queries rather than a
    /// mock. `initialize` is called here so a caller cannot forget it and then
    /// fail on a missing table; the file-backed `open` deliberately leaves
    /// migration to its caller because the app controls that ordering.
    #[cfg(test)]
    pub fn in_memory() -> Result<Self, String> {
        let database = Self {
            connection: Mutex::new(
                Connection::open_in_memory().map_err(|error| error.to_string())?,
            ),
        };
        database.initialize()?;
        Ok(database)
    }

    pub fn initialize(&self) -> Result<(), String> {
        let mut connection = self.connection.lock().map_err(|error| error.to_string())?;
        migrate(&mut connection)
    }

    pub fn import_legacy(&self, data_dir: &std::path::Path) -> Result<(), String> {
        let connection = self.connection.lock().map_err(|error| error.to_string())?;
        let imported: bool = connection
            .query_row(
                "SELECT value FROM settings WHERE key = 'migration.legacy_json_imported'",
                [],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .map_err(|error| error.to_string())?
            .is_some();
        if imported {
            return Ok(());
        }
        let tx = connection
            .unchecked_transaction()
            .map_err(|error| error.to_string())?;
        if let Some(endpoints) = read_json::<Vec<Endpoint>>(&data_dir.join("endpoints.json")) {
            for endpoint in endpoints {
                tx.execute("INSERT OR IGNORE INTO providers (id, name, endpoint_url, api_key, settings_json) VALUES (?1, ?2, ?3, ?4, ?5)", params![endpoint.id, endpoint.name, endpoint.base_url, endpoint.api_key, json(&serde_json::json!({"models": endpoint.models, "enabled": endpoint.enabled}))?]).map_err(|error| error.to_string())?;
            }
        }
        if let Some(models) = read_json::<Vec<LocalModel>>(&data_dir.join("local-models.json")) {
            for model in models {
                tx.execute("INSERT OR IGNORE INTO local_models (id, name, path, size_bytes, settings_json) VALUES (?1, ?2, ?3, ?4, '{}')", params![model.id, model.name, model.path, i64::try_from(model.size_bytes).map_err(|error| error.to_string())?]).map_err(|error| error.to_string())?;
            }
        }
        if let Some(selected) = read_json::<String>(&data_dir.join("local-model-selection.json")) {
            put_setting_tx(&tx, "local.selected_model", &selected)?;
        }
        if let Some(settings) =
            read_json::<serde_json::Value>(&data_dir.join("local-runtime-settings.json"))
        {
            if settings.get("context").is_some() || settings.get("parallel").is_some() {
                put_setting_tx(&tx, "local.runtime.default", &settings)?;
            } else if let Some(object) = settings.as_object() {
                for (model_id, value) in object {
                    put_setting_tx(&tx, &format!("local.runtime.{model_id}"), value)?;
                }
            }
        }
        tx.execute("INSERT OR REPLACE INTO settings (key, value) VALUES ('migration.legacy_json_imported', 'true')", []).map_err(|error| error.to_string())?;
        tx.commit().map_err(|error| error.to_string())
    }

    pub fn list_endpoints(&self) -> Result<Vec<Endpoint>, String> {
        let connection = self.connection.lock().map_err(|error| error.to_string())?;
        // Providers are read first and their models looked up afterwards,
        // because preparing the per-provider model query inside the row
        // closure would borrow the connection twice at once.
        let mut statement = connection.prepare("SELECT id, name, endpoint_url, api_key, settings_json FROM providers ORDER BY rowid").map_err(|error| error.to_string())?;
        let providers = statement
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                ))
            })
            .map_err(|error| error.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| error.to_string())?;
        drop(statement);

        let mut model_statement = connection
            .prepare("SELECT model_id, enabled FROM provider_models WHERE provider_id=?1 ORDER BY model_id")
            .map_err(|error| error.to_string())?;

        let mut endpoints = Vec::with_capacity(providers.len());
        for (id, name, base_url, api_key, raw_settings) in providers {
            let settings: serde_json::Value =
                serde_json::from_str(&raw_settings).unwrap_or_default();
            // The model list lives in `provider_models`; the JSON copy is only
            // a fallback for a provider whose rows have not been written yet,
            // so an unbackfilled database still lists its models.
            //
            // Split by the `enabled` column rather than filtered, so Settings can
            // see a switched-off model and offer it back. The pickers read
            // `models` alone and so never show one.
            let mut models = Vec::new();
            let mut disabled_models = Vec::new();
            let rows = model_statement
                .query_map([&id], |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
                })
                .map_err(|error| error.to_string())?;
            for row in rows {
                let (model, enabled) = row.map_err(|error| error.to_string())?;
                if enabled != 0 {
                    models.push(model);
                } else {
                    disabled_models.push(model);
                }
            }
            if models.is_empty() && disabled_models.is_empty() {
                models =
                    serde_json::from_value(settings.get("models").cloned().unwrap_or_default())
                        .unwrap_or_default();
            }
            endpoints.push(Endpoint {
                id,
                name,
                base_url,
                api_key,
                models,
                disabled_models,
                enabled: settings
                    .get("enabled")
                    .and_then(|value| value.as_bool())
                    .unwrap_or(true),
            });
        }
        Ok(endpoints)
    }

    pub fn save_endpoint(&self, endpoint: &Endpoint) -> Result<(), String> {
        let connection = self.connection.lock().map_err(|error| error.to_string())?;
        let tx = connection
            .unchecked_transaction()
            .map_err(|error| error.to_string())?;
        tx.execute("INSERT INTO providers (id, name, endpoint_url, api_key, settings_json) VALUES (?1,?2,?3,?4,?5) ON CONFLICT(id) DO UPDATE SET name=excluded.name, endpoint_url=excluded.endpoint_url, api_key=excluded.api_key, settings_json=excluded.settings_json", params![endpoint.id, endpoint.name, endpoint.base_url, endpoint.api_key, json(&serde_json::json!({"models": endpoint.models, "enabled": endpoint.enabled}))?]).map_err(|error| error.to_string())?;
        // Mirror the model list into its own rows so per-model settings (enabled,
        // favorite) have somewhere to live. The JSON stays as the record of
        // what the provider reported, which `list_endpoints` falls back to.
        //
        // A new model is inserted enabled, and an existing one keeps whatever
        // `enabled` it already had. Overwriting it from `endpoint.enabled` -- the
        // *provider's* own switch -- would silently turn back on every model the
        // user had switched off each time the provider was saved, which is
        // exactly the drift the `enabled` column exists to remove. Renaming a
        // provider must not change which models are available.
        for model in &endpoint.models {
            tx.execute(
                "INSERT INTO provider_models (provider_id, model_id, enabled) VALUES (?1,?2,1)
                 ON CONFLICT(provider_id, model_id) DO NOTHING",
                params![endpoint.id, model],
            )
            .map_err(|error| error.to_string())?;
        }
        tx.commit().map_err(|error| error.to_string())?;
        Ok(())
    }

    /// Turns one remote model on or off for the pickers.
    ///
    /// Writes the `enabled` column rather than an `app.disabled_models` array.
    /// That column is what `list_endpoints` already filters on, so it is the one
    /// place this fact has to be true: an array alongside it was a second copy
    /// that the frontend had to re-apply after every read, and the two could
    /// disagree with no error from either.
    pub fn set_model_enabled(
        &self,
        endpoint_id: &str,
        model_id: &str,
        enabled: bool,
    ) -> Result<(), String> {
        let connection = self.connection.lock().map_err(|error| error.to_string())?;
        // A model the provider has not reported yet has no row. One is created
        // rather than the write being dropped, because a model added by hand in
        // Settings has no row until something records it, and refusing to enable
        // it would make a hand-added model permanently unusable.
        connection
            .execute(
                "INSERT INTO provider_models (provider_id, model_id, enabled) VALUES (?1,?2,?3)
                 ON CONFLICT(provider_id, model_id) DO UPDATE SET enabled=excluded.enabled",
                params![endpoint_id, model_id, i64::from(enabled)],
            )
            .map_err(|error| error.to_string())?;
        Ok(())
    }

    pub fn remove_endpoint(&self, id: &str) -> Result<(), String> {
        self.connection
            .lock()
            .map_err(|error| error.to_string())?
            .execute("DELETE FROM providers WHERE id=?1", [id])
            .map_err(|error| error.to_string())?;
        Ok(())
    }

    pub fn list_local_models(&self) -> Result<Vec<LocalModel>, String> {
        let connection = self.connection.lock().map_err(|error| error.to_string())?;
        let mut statement = connection
            .prepare("SELECT id, name, path, size_bytes FROM local_models ORDER BY rowid")
            .map_err(|error| error.to_string())?;
        let rows = statement
            .query_map([], |row| {
                Ok(LocalModel {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    path: row.get(2)?,
                    size_bytes: u64::try_from(row.get::<_, i64>(3)?).unwrap_or_default(),
                    // Not stored. These are read from the file itself whenever
                    // the list is asked for, so a persisted copy would go stale
                    // the moment the file moved.
                    present: true,
                    quantization: None,
                    context_length: None,
                    // Resolved by `LocalModelManager::list` from the model's own
                    // engine, and never stored on the row: the answer changes
                    // when that does, so a persisted copy would be a second
                    // thing to keep in step.
                    engine_id: None,
                })
            })
            .map_err(|error| error.to_string())?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|error| error.to_string())
    }

    pub fn save_local_model(&self, model: &LocalModel) -> Result<(), String> {
        self.connection.lock().map_err(|error| error.to_string())?.execute("INSERT INTO local_models (id,name,path,size_bytes,settings_json) VALUES (?1,?2,?3,?4,'{}') ON CONFLICT(id) DO UPDATE SET name=excluded.name,path=excluded.path,size_bytes=excluded.size_bytes", params![model.id, model.name, model.path, i64::try_from(model.size_bytes).map_err(|error| error.to_string())?]).map_err(|error| error.to_string())?;
        Ok(())
    }

    pub fn remove_local_model(&self, id: &str) -> Result<(), String> {
        let connection = self.connection.lock().map_err(|error| error.to_string())?;
        connection
            .execute("DELETE FROM local_models WHERE id=?1", [id])
            .map_err(|error| error.to_string())?;
        connection
            .execute(
                "DELETE FROM settings WHERE key=?1",
                [format!("local.runtime.{id}")],
            )
            .map_err(|error| error.to_string())?;
        // The model's engine goes with it. Left behind it would keep the
        // engine it names looking referenced -- `engines::is_referenced` counts
        // a model's engine as a reason not to uninstall -- so an engine
        // nothing can reach would be permanently unremovable.
        connection
            .execute(
                "DELETE FROM settings WHERE key=?1",
                [crate::ai::local::engines::engine_setting_key(id)],
            )
            .map_err(|error| error.to_string())?;
        Ok(())
    }

    pub fn setting<T: DeserializeOwned>(&self, key: &str) -> Option<T> {
        self.connection
            .lock()
            .ok()?
            .query_row("SELECT value FROM settings WHERE key=?1", [key], |row| {
                row.get::<_, String>(0)
            })
            .optional()
            .ok()
            .flatten()
            .and_then(|value| serde_json::from_str(&value).ok())
    }

    /// A cached tool result for this conversation, if one is still valid.
    ///
    /// Returns `None` rather than a result when the session has been written to
    /// since the entry was stored. The comparison is a plain string ordering of
    /// UTC ISO-8601 timestamps, which is why those are the format everything is
    /// stored in: lexicographic order and chronological order agree, and no date
    /// parsing runs per row.
    pub fn tool_cache_get(
        &self,
        session_id: &str,
        tool_name: &str,
        cache_key: &str,
    ) -> Option<String> {
        let connection = self.connection.lock().ok()?;
        connection
            .query_row(
                // Greater-or-equal, which is only correct because the stamps are
                // strictly increasing: `ToolCache` numbers them per run rather than
                // reading the clock, so "equal" can never mean two different events
                // and a write's stamp is always after any read taken before it.
                "SELECT result FROM tool_cache
                 WHERE session_id=?1 AND tool_name=?2 AND cache_key=?3
                   AND created_at >= COALESCE(
                     (SELECT cache_invalidated_at FROM sessions WHERE id=?1), '')
                 ",
                params![session_id, tool_name, cache_key],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .ok()
            .flatten()
    }

    /// Stores a tool result for this conversation.
    ///
    /// Replaces any existing entry rather than adding one, so a repeated call
    /// does not grow the table and the freshest read is always the one returned.
    pub fn tool_cache_put(
        &self,
        session_id: &str,
        tool_name: &str,
        cache_key: &str,
        result: &str,
        now: &str,
    ) {
        let _ = self.connection.lock().ok().map(|connection| {
            connection.execute(
                "INSERT INTO tool_cache (session_id,tool_name,cache_key,result,created_at)
                     VALUES (?1,?2,?3,?4,?5)
                     ON CONFLICT(session_id,tool_name,cache_key) DO UPDATE SET
                       result=excluded.result, created_at=excluded.created_at",
                params![session_id, tool_name, cache_key, result, now],
            )
        });
    }

    /// Marks every earlier cached read in this conversation as stale.
    ///
    /// The entries are left in place rather than deleted: the next call repopulates
    /// them, and a delete would mean walking every row for a session that may hold
    /// thousands.
    pub fn tool_cache_invalidate_session(&self, session_id: &str, now: &str) {
        let _ = self.connection.lock().ok().map(|connection| {
            connection.execute(
                "UPDATE sessions SET cache_invalidated_at=?2 WHERE id=?1",
                params![session_id, now],
            )
        });
    }

    /// The newest stamp this conversation's cache already holds.
    ///
    /// The later of the last invalidation and the newest cached entry, because
    /// either can be the highest mark in play: a write leaves no cache row of its
    /// own, so a conversation whose last action was a write has its high-water
    /// mark on the session row alone.
    ///
    /// A run seeds its stamp sequence from this. Without it a run that began
    /// before another had written -- two replies in flight against one
    /// conversation -- could hand out a stamp equal to or older than one already
    /// stored, and a write would then fail to invalidate. `MAX` does that in one
    /// read rather than a scan over the conversation's entries.
    pub fn tool_cache_high_water_mark(&self, session_id: &str) -> Option<String> {
        let connection = self.connection.lock().ok()?;
        connection
            .query_row(
                "SELECT MAX(stamp) FROM (
                   SELECT COALESCE(cache_invalidated_at, '') AS stamp
                     FROM sessions WHERE id=?1
                   UNION ALL
                   SELECT created_at AS stamp
                     FROM tool_cache WHERE session_id=?1
                 )",
                params![session_id],
                |row| row.get::<_, Option<String>>(0),
            )
            .optional()
            .ok()
            .flatten()
            .flatten()
    }

    pub fn set_setting<T: Serialize>(&self, key: &str, value: &T) -> Result<(), String> {
        let value = json(value)?;
        self.connection.lock().map_err(|error| error.to_string())?.execute("INSERT INTO settings (key,value) VALUES (?1,?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value", params![key, value]).map_err(|error| error.to_string())?;
        Ok(())
    }

    pub fn runtime_settings(&self, model_id: &str) -> LocalRuntimeSettings {
        self.setting(&format!("local.runtime.{model_id}"))
            .or_else(|| self.setting("local.runtime.default"))
            .unwrap_or_default()
    }

    pub fn save_runtime_settings(
        &self,
        model_id: &str,
        settings: &LocalRuntimeSettings,
    ) -> Result<(), String> {
        if settings.context == 0 || settings.parallel == 0 {
            return Err("Context and parallel values must be greater than zero".into());
        }
        self.set_setting(&format!("local.runtime.{model_id}"), settings)
    }

    pub fn frontend_imported(&self) -> bool {
        self.setting::<bool>("migration.frontend_localstorage_imported")
            .unwrap_or(false)
    }

    pub fn import_frontend_data(
        &self,
        sessions: &[serde_json::Value],
        general_settings: &serde_json::Value,
    ) -> Result<(), String> {
        let connection = self.connection.lock().map_err(|error| error.to_string())?;
        let tx = connection
            .unchecked_transaction()
            .map_err(|error| error.to_string())?;
        for session in sessions {
            Self::save_chat_session_tx(&tx, session)?;
        }
        let has_general = tx
            .query_row("SELECT 1 FROM settings WHERE key='app.general'", [], |_| {
                Ok(())
            })
            .optional()
            .map_err(|error| error.to_string())?
            .is_some();
        if !has_general {
            put_setting_tx(&tx, "app.general", general_settings)?;
        }
        put_setting_tx(&tx, "migration.frontend_localstorage_imported", &true)?;
        tx.commit().map_err(|error| error.to_string())
    }

    pub fn save_chat_session(&self, session: &serde_json::Value) -> Result<(), String> {
        let connection = self.connection.lock().map_err(|error| error.to_string())?;
        let tx = connection
            .unchecked_transaction()
            .map_err(|error| error.to_string())?;
        Self::save_chat_session_tx(&tx, session)?;
        tx.commit().map_err(|error| error.to_string())
    }

    fn save_chat_session_tx(
        tx: &rusqlite::Transaction<'_>,
        session: &serde_json::Value,
    ) -> Result<(), String> {
        let id = session["id"].as_str().ok_or("Session is missing its id")?;
        let model_id = session["modelId"].as_str();
        let provider_id = session["providerId"].as_str();
        let dot_color = session["dotColor"]
            .as_str()
            .filter(|color| !color.trim().is_empty());
        // `mode` and `permission` are the conversation's working settings -- Chat or
        // Agent, and how much Agent mode may do -- so they live on the session
        // rather than in component state. A conversation about a codebase is not
        // the same kind of work as a one-off question, and the mode is chosen per
        // conversation for that reason. Held in this same metadata blob rather
        // than in new columns: it is read and written with the pin and the dot,
        // and a column each would mean a migration and a wider row for two short
        // strings.
        //
        // Both default when absent, so a conversation saved before either existed
        // reads as Chat with the default permission rather than failing.
        //
        // `kind` and `parentSessionId` are what make a sub-agent's run a row in
        // the same table without it appearing in the sidebar: the list queries
        // exclude anything whose kind is `subagent`, and the Sub agents panel is
        // the only reader that asks for them. They live in this blob beside the
        // pin and the mode rather than in new columns, because they are read and
        // written with those and a column each would mean a migration for two
        // short strings. A missing `kind` is a chat, which is what every
        // conversation written before this reads as.
        let metadata = json(
            &serde_json::json!({"pinned": session["pinned"].as_bool().unwrap_or(false), "renamed": session["renamed"].as_bool().unwrap_or(false), "renamedByUser": session["renamedByUser"].as_bool().unwrap_or(false), "dotColor": dot_color, "mode": session["mode"].as_str().unwrap_or("chat"), "permission": session["permission"].as_str(), "kind": session["kind"].as_str().unwrap_or("chat"), "parentSessionId": session["parentSessionId"].as_str()}),
        )?;

        // Token counts and timing are rolled up from the messages in the same
        // transaction, so the session row never disagrees with its messages.
        let messages = session["messages"].as_array();
        let mut prompt_total: i64 = 0;
        let mut completion_total: i64 = 0;
        // Deliberately not a sum, unlike the two totals above. Every turn
        // re-reads the conversation prefix it shares with the previous turn, so
        // adding the figures together would count the same cached tokens once
        // per turn and report a hit rate no provider ever achieved. The last
        // reply's figure is the one that describes the conversation as it now
        // stands, and it is the number the statistics page should divide by the
        // last reply's prompt tokens.
        let mut cached_total: Option<i64> = None;
        if let Some(messages) = messages {
            for message in messages.iter() {
                prompt_total += message["metrics"]["prompt_tokens"].as_i64().unwrap_or(0);
                completion_total += message["metrics"]["completion_tokens"]
                    .as_i64()
                    .unwrap_or(0);
                if let Some(cached) = message["metrics"]["cached_tokens"].as_i64() {
                    cached_total = Some(cached);
                }
            }
        }

        tx.execute("INSERT INTO sessions (id,title,created_at,updated_at,model_id,provider_id,prompt_tokens,completion_tokens,cached_tokens,metadata_json) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10) ON CONFLICT(id) DO UPDATE SET title=excluded.title,updated_at=excluded.updated_at,model_id=COALESCE(excluded.model_id,sessions.model_id),provider_id=COALESCE(excluded.provider_id,sessions.provider_id),prompt_tokens=excluded.prompt_tokens,completion_tokens=excluded.completion_tokens,cached_tokens=excluded.cached_tokens,metadata_json=excluded.metadata_json", params![id, session["title"].as_str().unwrap_or("New conversation"), session["createdAt"].as_str().unwrap_or_default(), session["updatedAt"].as_str().unwrap_or_default(), model_id, provider_id, prompt_total, completion_total, cached_total, metadata]).map_err(|error| error.to_string())?;

        // Messages are upserted rather than deleted and re-inserted. A send
        // appends one or two messages, so the previous behaviour rewrote the
        // entire conversation on every reply, which is O(n) writes per send and
        // the dominant cost once a session holds a long transcript. The last
        // message is often still streaming, so a row can legitimately change
        // after it was first written; the upsert carries that update.
        // The tail-delete below keys on `ordinal`, so a save carrying fewer
        // messages than are stored removes the difference. That is sometimes
        // exactly right -- retrying a reply drops it and everything after it, and
        // `ChatPage.retry` sends the truncated conversation back deliberately --
        // so a shorter array is permitted.
        //
        // An **empty** one is not. No operation produces it: a conversation
        // cannot be created without a first message, and no edit in the app
        // removes every turn. It is what a caller that has a session row but not
        // its transcript would send, and accepting it would delete the whole
        // conversation with no error and no way back. The client now loads
        // transcripts lazily -- headers first, messages for the one open
        // conversation -- so a missing transcript is a reachable state rather
        // than a hypothetical, and refusing here is what keeps it from becoming
        // data loss.
        //
        // Refusing costs the caller a rename or a pin they can repeat; accepting
        // costs the user the conversation. An absent `messages` key is a third
        // thing again and leaves every row untouched.
        if let Some(messages) = messages {
            if messages.is_empty() && Self::stored_message_count(&tx, id)? > 0 {
                return Err(format!(
                    "Refusing to save session '{id}': no messages supplied but the session \
                     has a stored transcript. A save must carry the conversation's messages."
                ));
            }
            for (index, message) in messages.iter().enumerate() {
                let mut metrics = message["metrics"].clone();
                if let Some(values) = metrics.as_object_mut() {
                    for value in values.values_mut() {
                        if let Some(number) = value.as_f64() {
                            *value = serde_json::json!(number.round() as i64);
                        }
                    }
                }
                // The same values are written to real columns so aggregates can
                // read integers instead of parsing the JSON blob per row.
                let prompt_tokens = message["metrics"]["prompt_tokens"].as_i64();
                let completion_tokens = message["metrics"]["completion_tokens"].as_i64();
                // Only some providers report a cache figure. `None` is written as
                // NULL rather than 0 because "the provider did not say" and "the
                // provider cached nothing" are different facts, and only the
                // first one should average into a hit-rate figure.
                let cached_tokens = message["metrics"]["cached_tokens"].as_i64();
                let generation_seconds = message["metrics"]["generation_seconds"].as_f64();
                let prompt_seconds = message["metrics"]["prompt_seconds"].as_f64();
                let rate_estimated = message["metrics"]["generation_rate_estimated"].as_bool();
                // The steps, serialized whole. `startedAt` is dropped on the way
                // in: it is a local `performance.now()`/`Date.now()` reading that
                // only means something in the window that produced it, and
                // storing it would let a later render add to a step that finished
                // hours ago.
                let activity = message
                    .get("activity")
                    .filter(|steps| steps.as_array().is_some_and(|list| !list.is_empty()))
                    .map(|steps| {
                        let mut stored = steps.clone();
                        if let Some(list) = stored.as_array_mut() {
                            for step in list {
                                if let Some(object) = step.as_object_mut() {
                                    object.remove("startedAt");
                                }
                            }
                        }
                        json(&stored)
                    })
                    .transpose()?;
                tx.execute("INSERT INTO messages (id,session_id,ordinal,role,content,created_at,model_id,provider_id,metrics_json,prompt_tokens,completion_tokens,cached_tokens,ttft_ms,total_latency_ms,reasoning,tool_json,generation_seconds,prompt_seconds,generation_rate_estimated,activity_json) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18,?19,?20) ON CONFLICT(id) DO UPDATE SET content=excluded.content,model_id=excluded.model_id,provider_id=excluded.provider_id,metrics_json=excluded.metrics_json,prompt_tokens=excluded.prompt_tokens,completion_tokens=excluded.completion_tokens,cached_tokens=excluded.cached_tokens,ttft_ms=excluded.ttft_ms,total_latency_ms=excluded.total_latency_ms,reasoning=excluded.reasoning,tool_json=excluded.tool_json,generation_seconds=excluded.generation_seconds,prompt_seconds=excluded.prompt_seconds,generation_rate_estimated=excluded.generation_rate_estimated,activity_json=excluded.activity_json", params![
                    format!("{id}:{index}"),
                    id,
                    index as i64,
                    message["role"].as_str().unwrap_or("user"),
                    message["content"].as_str().unwrap_or_default(),
                    // A message's own time is not tracked separately by the
                    // frontend, so the session's updated time is the closest
                    // available value and keeps every row in a session ordered.
                    session["updatedAt"].as_str().unwrap_or_default(),
                    message["modelId"].as_str(),
                    message["providerId"].as_str(),
                    json(&metrics)?,
                    prompt_tokens,
                    completion_tokens,
                    cached_tokens,
                    prompt_seconds.map(|value| value * 1000.0),
                    prompt_seconds.zip(generation_seconds).map(|(prompt, generation)| (prompt + generation) * 1000.0),
                    message["reasoning"].as_str(),
                    message.get("tool").map(json).transpose()?,
                    generation_seconds,
                    prompt_seconds,
                    rate_estimated,
                    activity,
                ]).map_err(|error| error.to_string())?;
            }
            // A conversation can shrink (a cleared or edited transcript). Rows
            // past the end of the array no longer exist, so remove just those
            // rather than every row in the session.
            tx.execute(
                "DELETE FROM messages WHERE session_id=?1 AND ordinal >= ?2",
                params![id, messages.len() as i64],
            )
            .map_err(|error| error.to_string())?;
        }
        Ok(())
    }

    /// How many message rows a session currently has.
    ///
    /// Read inside the save transaction so the guard in `save_chat_session_tx`
    /// compares against the committed state rather than a stale read from before
    /// this write began. `COUNT(*)` rather than `MAX(ordinal)`: the tail-delete
    /// below keys on `ordinal`, and a row whose ordinal had a gap would make the
    /// max smaller than the count, so the guard would admit a shorter array.
    fn stored_message_count(tx: &rusqlite::Transaction<'_>, id: &str) -> Result<i64, String> {
        tx.query_row(
            "SELECT COUNT(*) FROM messages WHERE session_id=?1",
            [id],
            |row| row.get::<_, i64>(0),
        )
        .map_err(|error| error.to_string())
    }

    /// [`Self::stored_message_count`] outside a transaction, for tests.
    ///
    /// Test-only on purpose: production code is either inside the save
    /// transaction or reading through `list_session_messages`, and adding a
    /// public "how many messages does this session have" query for no production
    /// caller would be a second way to ask a question that already has one.
    #[cfg(test)]
    fn stored_message_count_for_test(&self, id: &str) -> Result<i64, String> {
        let connection = self.connection.lock().map_err(|error| error.to_string())?;
        connection
            .query_row(
                "SELECT COUNT(*) FROM messages WHERE session_id=?1",
                [id],
                |row| row.get::<_, i64>(0),
            )
            .map_err(|error| error.to_string())
    }

    /// Every conversation **with all of its messages**.
    ///
    /// Retained as the whole-database read, but no longer the launch path: it is
    /// `O(all messages)` and the sidebar needs none of them. Prefer
    /// [`Self::list_session_headers`] for a list and
    /// [`Self::list_session_messages`] for one conversation. See
    /// [`transcripts`] for why the split exists.
    pub fn list_chat_sessions(&self) -> Result<Vec<serde_json::Value>, String> {
        let connection = self.connection.lock().map_err(|error| error.to_string())?;
        let mut sessions = connection
            .prepare(&format!(
                "SELECT {SESSION_COLUMNS} FROM sessions \
                 WHERE COALESCE(json_extract(metadata_json, '$.kind'), 'chat') <> 'subagent' \
                 ORDER BY json_extract(metadata_json, '$.pinned') DESC, updated_at DESC"
            ))
            .map_err(|error| error.to_string())?;
        let rows = sessions
            .query_map([], transcripts::read_session_row)
            .map_err(|error| error.to_string())?;
        let mut result = Vec::new();
        for row in rows {
            // Decoded once and split in two places: the header builder takes the
            // whole tuple back, and the messages query needs the id. Decoding
            // twice would be a second row shape to keep aligned.
            let row = row.map_err(|error| error.to_string())?;
            let id = transcripts::peek_session_id(&row);
            // The shared decoder, so the full read and the per-session read cannot
            // drift into describing a stored message differently.
            let messages = Self::read_messages(&connection, &id)?;
            // `messages` is the only key the header does not have, so the shape
            // still matches what a caller of `list_chat_sessions` always got: one
            // object per conversation, transcript included.
            let mut session = transcripts::session_header(row);
            session["messages"] = Value::Array(messages);
            result.push(session);
        }
        Ok(result)
    }

    pub fn delete_chat_session(&self, id: &str) -> Result<(), String> {
        let connection = self.connection.lock().map_err(|error| error.to_string())?;
        connection
            .execute("DELETE FROM sessions WHERE id=?1", [id])
            .map_err(|error| error.to_string())?;
        // The sub-agent runs that belong to this conversation go with it. They are
        // separate rows rather than children with a foreign key, so nothing else
        // would collect them -- and a run left behind would appear in the Sub
        // agents list grouped under a conversation that no longer exists.
        connection
            .execute(
                "DELETE FROM sessions WHERE json_extract(metadata_json, '$.parentSessionId') = ?1",
                [id],
            )
            .map_err(|error| error.to_string())?;
        Ok(())
    }

    /// Removes every conversation in one transaction and reports how many went, so the
    /// caller can tell the user whether there was anything to clear. Messages cascade
    /// from their session, and providers, local models, and settings are left alone.
    pub fn clear_chat_sessions(&self) -> Result<usize, String> {
        let mut connection = self.connection.lock().map_err(|error| error.to_string())?;
        let transaction = connection
            .transaction()
            .map_err(|error| error.to_string())?;
        let removed = transaction
            .execute("DELETE FROM sessions", [])
            .map_err(|error| error.to_string())?;
        transaction.commit().map_err(|error| error.to_string())?;
        Ok(removed)
    }

    /// Decodes one stored capability row.
    ///
    /// The lists are plain JSON arrays; a value that no longer parses reads as
    /// unknown rather than failing the read, because a malformed row must not hide
    /// the context window stored beside it.
    fn capabilities_from_row(
        row: (
            Option<i64>,
            Option<String>,
            Option<String>,
            Option<i64>,
            Option<String>,
            Option<i64>,
        ),
    ) -> crate::ai::remote::capabilities::ModelCapabilities {
        use crate::ai::remote::capabilities::ModelCapabilities;
        let list = |raw: Option<String>| -> Option<Vec<String>> {
            raw.and_then(|text| serde_json::from_str::<Vec<String>>(&text).ok())
                .filter(|values| !values.is_empty())
        };
        ModelCapabilities {
            context_length: row.0,
            input_modalities: list(row.1),
            output_modalities: list(row.2),
            supports_reasoning: row.3.map(|value| value != 0),
            reasoning_values: list(row.4),
            supports_tools: row.5.map(|value| value != 0),
        }
    }

    /// Aggregate usage for the statistics page over one timeframe.
    ///
    /// The caller passes `since` and `bucket_sql` from a fixed table of known
    /// ranges (see the frontend's `TIMEFRAMES`), never from free input, so
    /// `bucket_sql` is a compile-time constant on the other end rather than
    /// user data reaching the query string.
    pub fn usage_report(
        &self,
        range: &str,
        since: &str,
        bucket_sql: &str,
    ) -> Result<statistics::UsageReport, String> {
        let connection = self.connection.lock().map_err(|error| error.to_string())?;
        // Argument order matters here: `build_report` takes the start date
        // before the label. Swapping them filters on `since = "month"`, which
        // matches nothing and makes the page look empty.
        statistics::build_report(&connection, since, range, bucket_sql)
    }

    /// Stores what each model of a provider can do.
    ///
    /// A field the provider did not report is written as NULL rather than as a
    /// false or an empty list, because "unknown" and "cannot" are different
    /// answers and the frontend needs to keep behaving normally for the first.
    pub fn save_model_capabilities(
        &self,
        provider_id: &str,
        models: &[(String, crate::ai::remote::capabilities::ModelCapabilities)],
        checked_at: &str,
    ) -> Result<(), String> {
        let connection = self.connection.lock().map_err(|error| error.to_string())?;
        let tx = connection
            .unchecked_transaction()
            .map_err(|error| error.to_string())?;
        for (model_id, capabilities) in models {
            tx.execute(
                "INSERT INTO provider_models (provider_id, model_id, enabled, context_length, input_modalities, output_modalities, supports_reasoning, reasoning_values, supports_tools, capabilities_checked_at)
                 VALUES (?1,?2,1,?3,?4,?5,?6,?7,?8,?9)
                 ON CONFLICT(provider_id, model_id) DO UPDATE SET
                     context_length=excluded.context_length,
                     input_modalities=excluded.input_modalities,
                     output_modalities=excluded.output_modalities,
                     supports_reasoning=excluded.supports_reasoning,
                     reasoning_values=excluded.reasoning_values,
                     supports_tools=excluded.supports_tools,
                     capabilities_checked_at=excluded.capabilities_checked_at",
                params![
                    provider_id,
                    model_id,
                    capabilities.context_length,
                    capabilities
                        .input_modalities
                        .as_ref()
                        .map(|values| json(values))
                        .transpose()?,
                    capabilities
                        .output_modalities
                        .as_ref()
                        .map(|values| json(values))
                        .transpose()?,
                    capabilities.supports_reasoning,
                    capabilities
                        .reasoning_values
                        .as_ref()
                        .map(|values| json(values))
                        .transpose()?,
                    capabilities.supports_tools,
                    checked_at,
                ],
            )
            .map_err(|error| error.to_string())?;
        }
        tx.commit().map_err(|error| error.to_string())
    }

    /// What a single model of a provider is known to support.
    pub fn model_capabilities(
        &self,
        provider_id: &str,
        model_id: &str,
    ) -> Result<crate::ai::remote::capabilities::ModelCapabilities, String> {
        let connection = self.connection.lock().map_err(|error| error.to_string())?;
        let row = connection
            .query_row(
                "SELECT context_length, input_modalities, output_modalities, supports_reasoning, reasoning_values, supports_tools
                 FROM provider_models WHERE provider_id=?1 AND model_id=?2",
                params![provider_id, model_id],
                |row| {
                    Ok((
                        row.get::<_, Option<i64>>(0)?,
                        row.get::<_, Option<String>>(1)?,
                        row.get::<_, Option<String>>(2)?,
                        row.get::<_, Option<i64>>(3)?,
                        row.get::<_, Option<String>>(4)?,
                        row.get::<_, Option<i64>>(5)?,
                    ))
                },
            )
            .optional()
            .map_err(|error| error.to_string())?;
        Ok(match row {
            Some(row) => Self::capabilities_from_row(row),
            // A model we have never heard of is unknown, not empty.
            None => crate::ai::remote::capabilities::ModelCapabilities::default(),
        })
    }

    /// Every model of one provider that has stored capabilities, keyed by model
    /// id.
    ///
    /// One read for the whole provider rather than one per model: a single
    /// gateway can report several hundred models, and a caller that asked per
    /// model would issue hundreds of commands at once.
    pub fn provider_capabilities(
        &self,
        provider_id: &str,
    ) -> Result<
        std::collections::HashMap<String, crate::ai::remote::capabilities::ModelCapabilities>,
        String,
    > {
        let connection = self.connection.lock().map_err(|error| error.to_string())?;
        let mut statement = connection
            .prepare(
                "SELECT model_id, context_length, input_modalities, output_modalities, supports_reasoning, reasoning_values, supports_tools
                 FROM provider_models WHERE provider_id=?1",
            )
            .map_err(|error| error.to_string())?;
        let rows = statement
            .query_map(params![provider_id], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    Self::capabilities_from_row((
                        row.get::<_, Option<i64>>(1)?,
                        row.get::<_, Option<String>>(2)?,
                        row.get::<_, Option<String>>(3)?,
                        row.get::<_, Option<i64>>(4)?,
                        row.get::<_, Option<String>>(5)?,
                        row.get::<_, Option<i64>>(6)?,
                    )),
                ))
            })
            .map_err(|error| error.to_string())?;
        let mut output = std::collections::HashMap::new();
        for row in rows {
            let (model, capabilities) = row.map_err(|error| error.to_string())?;
            output.insert(model, capabilities);
        }
        Ok(output)
    }

    pub fn table_names(&self) -> Result<Vec<String>, String> {
        let connection = self.connection.lock().map_err(|error| error.to_string())?;
        let mut statement = connection.prepare("SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%' ORDER BY name").map_err(|error| error.to_string())?;
        let names = statement
            .query_map([], |row| row.get(0))
            .map_err(|error| error.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| error.to_string())?;
        Ok(names)
    }

    pub fn table_rows(
        &self,
        table: &str,
        offset: u32,
        limit: u32,
    ) -> Result<serde_json::Value, String> {
        if limit == 0 || limit > 100 {
            return Err("Page size must be between 1 and 100 rows".into());
        }
        let allowed = self.table_names()?;
        if !allowed.iter().any(|name| name == table) {
            return Err("Unknown database table".into());
        }
        let connection = self.connection.lock().map_err(|error| error.to_string())?;
        let total: i64 = connection
            .query_row(
                &format!("SELECT COUNT(*) FROM \"{}\"", table.replace('"', "\"\"")),
                [],
                |row| row.get(0),
            )
            .map_err(|error| error.to_string())?;
        let mut statement = connection
            .prepare(&format!(
                "SELECT * FROM \"{}\" LIMIT ?1 OFFSET ?2",
                table.replace('"', "\"\"")
            ))
            .map_err(|error| error.to_string())?;
        let columns = statement
            .column_names()
            .iter()
            .map(|name| name.to_string())
            .collect::<Vec<_>>();
        let mut rows = statement
            .query(params![limit, offset])
            .map_err(|error| error.to_string())?;
        let mut output = Vec::new();
        while let Some(row) = rows.next().map_err(|error| error.to_string())? {
            let mut object = serde_json::Map::new();
            for (index, column) in columns.iter().enumerate() {
                if table == "providers" && column == "api_key" {
                    object.insert(
                        column.clone(),
                        serde_json::Value::String("••••••••".to_string()),
                    );
                    continue;
                }
                let value: rusqlite::types::Value =
                    row.get(index).map_err(|error| error.to_string())?;
                object.insert(
                    column.clone(),
                    match value {
                        rusqlite::types::Value::Null => serde_json::Value::Null,
                        rusqlite::types::Value::Integer(value) => value.into(),
                        rusqlite::types::Value::Real(value) => serde_json::json!(value),
                        rusqlite::types::Value::Text(value) => serde_json::Value::String(value),
                        rusqlite::types::Value::Blob(value) => {
                            serde_json::json!(format!("<{} bytes>", value.len()))
                        }
                    },
                );
            }
            output.push(serde_json::Value::Object(object));
        }
        Ok(serde_json::json!({"rows": output, "total": total}))
    }
}

fn migrate(connection: &mut Connection) -> Result<(), String> {
    let version: i64 = connection
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .map_err(|error| error.to_string())?;
    if version < 1 {
        let tx = connection
            .transaction()
            .map_err(|error| error.to_string())?;
        tx.execute_batch("CREATE TABLE providers (id TEXT PRIMARY KEY, name TEXT NOT NULL, endpoint_url TEXT NOT NULL, api_key TEXT NOT NULL DEFAULT '', settings_json TEXT NOT NULL DEFAULT '{}'); CREATE TABLE sessions (id TEXT PRIMARY KEY, title TEXT NOT NULL, created_at TEXT NOT NULL, updated_at TEXT NOT NULL, metadata_json TEXT NOT NULL DEFAULT '{}'); CREATE INDEX idx_sessions_updated_at ON sessions(updated_at DESC); CREATE TABLE messages (id TEXT PRIMARY KEY, session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE, ordinal INTEGER NOT NULL, role TEXT NOT NULL, content TEXT NOT NULL, created_at TEXT NOT NULL, model_id TEXT, provider_id TEXT, prompt_tokens INTEGER, completion_tokens INTEGER, cached_tokens INTEGER, ttft_ms REAL, total_latency_ms REAL, metrics_json TEXT NOT NULL DEFAULT '{}'); CREATE INDEX idx_messages_session_order ON messages(session_id, ordinal); CREATE TABLE local_models (id TEXT PRIMARY KEY, name TEXT NOT NULL, path TEXT NOT NULL UNIQUE, size_bytes INTEGER NOT NULL, settings_json TEXT NOT NULL DEFAULT '{}'); CREATE TABLE settings (key TEXT PRIMARY KEY, value TEXT NOT NULL); CREATE TABLE model_cache (provider_id TEXT PRIMARY KEY, models_json TEXT NOT NULL, refreshed_at TEXT NOT NULL); PRAGMA user_version=1;").map_err(|error| error.to_string())?;
        tx.commit().map_err(|error| error.to_string())?;
    }
    let version: i64 = connection
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .map_err(|error| error.to_string())?;
    if version < 2 {
        let tx = connection
            .transaction()
            .map_err(|error| error.to_string())?;
        tx.execute_batch("ALTER TABLE sessions ADD COLUMN model_id TEXT; ALTER TABLE sessions ADD COLUMN provider_id TEXT; ALTER TABLE sessions ADD COLUMN prompt_tokens INTEGER; ALTER TABLE sessions ADD COLUMN completion_tokens INTEGER; ALTER TABLE sessions ADD COLUMN cached_tokens INTEGER; CREATE INDEX idx_messages_provider_model ON messages(provider_id, model_id); PRAGMA user_version=2;").map_err(|error| error.to_string())?;
        tx.commit().map_err(|error| error.to_string())?;
    }
    let version: i64 = connection
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .map_err(|error| error.to_string())?;
    if version < 3 {
        let tx = connection
            .transaction()
            .map_err(|error| error.to_string())?;
        let mut statement = tx
            .prepare("SELECT id,metrics_json,ttft_ms,total_latency_ms FROM messages")
            .map_err(|error| error.to_string())?;
        let rows = statement
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, Option<f64>>(2)?,
                    row.get::<_, Option<f64>>(3)?,
                ))
            })
            .map_err(|error| error.to_string())?;
        let values = rows
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| error.to_string())?;
        drop(statement);
        for (id, raw, ttft, latency) in values {
            let mut metrics: serde_json::Value = serde_json::from_str(&raw).unwrap_or_default();
            if let Some(object) = metrics.as_object_mut() {
                for value in object.values_mut() {
                    if let Some(number) = value.as_f64() {
                        *value = serde_json::json!(number.round() as i64);
                    }
                }
            }
            tx.execute(
                "UPDATE messages SET metrics_json=?1,ttft_ms=?2,total_latency_ms=?3 WHERE id=?4",
                params![
                    json(&metrics)?,
                    ttft.map(|value| value.round()),
                    latency.map(|value| value.round()),
                    id
                ],
            )
            .map_err(|error| error.to_string())?;
        }
        tx.pragma_update(None, "user_version", 3)
            .map_err(|error| error.to_string())?;
        tx.commit().map_err(|error| error.to_string())?;
    }
    let version: i64 = connection
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .map_err(|error| error.to_string())?;
    if version < 4 {
        let tx = connection
            .transaction()
            .map_err(|error| error.to_string())?;
        tx.execute_batch("ALTER TABLE messages ADD COLUMN reasoning TEXT; PRAGMA user_version=4;")
            .map_err(|error| error.to_string())?;
        tx.commit().map_err(|error| error.to_string())?;
    }
    let version: i64 = connection
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .map_err(|error| error.to_string())?;
    if version < 5 {
        let tx = connection
            .transaction()
            .map_err(|error| error.to_string())?;
        tx.execute_batch("ALTER TABLE messages ADD COLUMN tool_json TEXT; PRAGMA user_version=5;")
            .map_err(|error| error.to_string())?;
        tx.commit().map_err(|error| error.to_string())?;
    }
    let version: i64 = connection
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .map_err(|error| error.to_string())?;
    // Per-reply metrics move out of `metrics_json` into real columns, so
    // aggregate queries read integers instead of parsing JSON per row. The JSON
    // column is kept and still written, because it carries fields that have no
    // column yet and the frontend's message shape is unchanged.
    if version < 6 {
        let tx = connection
            .transaction()
            .map_err(|error| error.to_string())?;
        tx.execute_batch(
            "ALTER TABLE messages ADD COLUMN generation_seconds REAL;
             ALTER TABLE messages ADD COLUMN prompt_seconds REAL;
             ALTER TABLE messages ADD COLUMN generation_rate_estimated INTEGER;
             CREATE INDEX idx_messages_created_at ON messages(created_at);
             CREATE INDEX idx_messages_role_created ON messages(role, created_at);
             CREATE TABLE provider_models (
                 provider_id TEXT NOT NULL REFERENCES providers(id) ON DELETE CASCADE,
                 model_id TEXT NOT NULL,
                 enabled INTEGER NOT NULL DEFAULT 1,
                 favorite INTEGER NOT NULL DEFAULT 0,
                 PRIMARY KEY (provider_id, model_id)
             );
             CREATE INDEX idx_provider_models_favorite ON provider_models(favorite);
             PRAGMA user_version=6;",
        )
        .map_err(|error| error.to_string())?;
        // Backfill the new columns from the JSON written by earlier versions, so
        // existing conversations keep their figures instead of reporting zero.
        tx.execute(
            "UPDATE messages SET
                 prompt_tokens = CAST(json_extract(metrics_json, '$.prompt_tokens') AS INTEGER),
                 completion_tokens = CAST(json_extract(metrics_json, '$.completion_tokens') AS INTEGER),
                 generation_seconds = CAST(json_extract(metrics_json, '$.generation_seconds') AS REAL),
                 prompt_seconds = CAST(json_extract(metrics_json, '$.prompt_seconds') AS REAL),
                 generation_rate_estimated = CAST(json_extract(metrics_json, '$.generation_rate_estimated') AS INTEGER)
             WHERE metrics_json IS NOT NULL AND metrics_json != 'null'",
            [],
        )
        .map_err(|error| error.to_string())?;
        // Seed `provider_models` from the `models` array inside
        // `providers.settings_json`, which is where the model list lived before.
        // The `json_type` guard skips providers that have no such key, because
        // `json_each` raises an error rather than returning no rows for it.
        tx.execute(
            "INSERT OR IGNORE INTO provider_models (provider_id, model_id, enabled)
             SELECT p.id,
                    json_each.value,
                    CASE WHEN json_extract(p.settings_json, '$.enabled') = 0 THEN 0 ELSE 1 END
             FROM providers p, json_each(p.settings_json, '$.models')
             WHERE json_type(p.settings_json, '$.models') = 'array'",
            [],
        )
        .map_err(|error| error.to_string())?;
        // The session rollup is read directly by the statistics page, so it
        // needs backfilling too. It has been written as NULL until now, which
        // is why a conversation that predates this migration would otherwise
        // report zero tokens.
        tx.execute(
            "UPDATE sessions SET
                 prompt_tokens = (
                     SELECT COALESCE(SUM(m.prompt_tokens), 0) FROM messages m
                     WHERE m.session_id = sessions.id
                 ),
                 completion_tokens = (
                     SELECT COALESCE(SUM(m.completion_tokens), 0) FROM messages m
                     WHERE m.session_id = sessions.id
                 )",
            [],
        )
        .map_err(|error| error.to_string())?;
        tx.commit().map_err(|error| error.to_string())?;
    }
    // What each model can do, learned from the provider's own `/models`
    // response rather than assumed. Two models behind one gateway differ, so
    // these sit on `provider_models` rather than on the provider.
    //
    // Every column is nullable on purpose. A provider that returns only model
    // ids leaves them NULL, and NULL means *unknown* rather than *absent*: the
    // app then behaves exactly as it did before capabilities existed, instead of
    // reading a missing answer as "this model cannot do that".
    if version < 7 {
        let tx = connection
            .transaction()
            .map_err(|error| error.to_string())?;
        tx.execute_batch(
            "ALTER TABLE provider_models ADD COLUMN context_length INTEGER;
             ALTER TABLE provider_models ADD COLUMN input_modalities TEXT;
             ALTER TABLE provider_models ADD COLUMN output_modalities TEXT;
             ALTER TABLE provider_models ADD COLUMN supports_reasoning INTEGER;
             ALTER TABLE provider_models ADD COLUMN reasoning_values TEXT;
             ALTER TABLE provider_models ADD COLUMN capabilities_checked_at TEXT;
             PRAGMA user_version=7;",
        )
        .map_err(|error| error.to_string())?;
        tx.commit().map_err(|error| error.to_string())?;
    }
    // Tool results, remembered so a model that reads the same file on twenty
    // turns pays for it once.
    //
    // Keyed by (session, tool, arguments) rather than by arguments alone, because
    // a cached `read_file` is only correct relative to the conversation that asked
    // for it: another session working a different tree must not read this one's
    // stale answer. Tying entries to the session means they are collected with
    // it and need no expiry of their own.
    //
    // `sessions.cache_invalidated_at` is the important column. Caching a
    // *write* tool's result would be a correctness bug, and caching a read taken
    // before a write is just as wrong. Rather than reasoning about which tools
    // those are on every read, a write stamps the session and every entry older
    // than the stamp is ignored. One rule, and a tool cannot forget it.
    if version < 8 {
        let tx = connection
            .transaction()
            .map_err(|error| error.to_string())?;
        tx.execute_batch(&format!(
            "CREATE TABLE tool_cache (
                 session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
                 tool_name TEXT NOT NULL,
                 cache_key TEXT NOT NULL,
                 result TEXT NOT NULL,
                 created_at TEXT NOT NULL,
                 PRIMARY KEY (session_id, tool_name, cache_key)
             );
             CREATE INDEX idx_tool_cache_created ON tool_cache(created_at);
             ALTER TABLE sessions ADD COLUMN cache_invalidated_at TEXT;
             PRAGMA user_version={LATEST_SCHEMA_VERSION};"
        ))
        .map_err(|error| error.to_string())?;
        tx.commit().map_err(|error| error.to_string())?;
    }
    // Whether a model accepts a `tools` array, learned from the provider rather
    // than assumed. Nullable like every other capability column: a provider that
    // reports only ids leaves it NULL, and NULL means *unknown*, so the app keeps
    // offering Agent mode exactly as it did before this existed.
    //
    // Worth its own column because the failure it prevents is silent. A model
    // sent a `tools` array it ignores does not error -- it just answers in prose,
    // and the user sees a mode that appears to do nothing.
    if version < 9 {
        let tx = connection
            .transaction()
            .map_err(|error| error.to_string())?;
        tx.execute_batch(&format!(
            "ALTER TABLE provider_models ADD COLUMN supports_tools INTEGER;
             PRAGMA user_version={LATEST_SCHEMA_VERSION};"
        ))
        .map_err(|error| error.to_string())?;
        tx.commit().map_err(|error| error.to_string())?;
    }
    // Reusable instructions the agent can be asked to apply: a house style, a
    // review checklist, a preference about how this project is written.
    //
    // `type` is a free-text label rather than an enum. A closed set would have to
    // predict every category a user might invent and would then need a migration
    // each time one is missing; an open string costs one column and never blocks a
    // new label. It is nullable because most skills are worth having without one,
    // and an empty string is not the same as unlabelled when filtering.
    //
    // There is deliberately no `folder_id`. Grouping was considered and dropped:
    // a hierarchy has to be right before the first skill exists, and most people
    // need two or three flat labels rather than a tree. `type` covers that, and
    // a tree can be added later without losing anything.
    //
    // `origin` records who wrote it, so a skill the agent drafted from a past
    // conversation is distinguishable from one a user wrote deliberately. That
    // distinction matters when deciding which instructions to trust.
    if version < 10 {
        let tx = connection
            .transaction()
            .map_err(|error| error.to_string())?;
        tx.execute_batch(
            "CREATE TABLE skills (
                 id TEXT PRIMARY KEY,
                 name TEXT NOT NULL,
                 description TEXT,
                 instructions TEXT NOT NULL,
                 type TEXT,
                 origin TEXT NOT NULL DEFAULT 'user',
                 enabled INTEGER NOT NULL DEFAULT 1,
                 use_count INTEGER NOT NULL DEFAULT 0,
                 created_at TEXT NOT NULL,
                 updated_at TEXT NOT NULL
             );
             -- Listing is by name over enabled skills only, which is the one query
             -- every surface performs, so it is the only one indexed.
             CREATE INDEX idx_skills_enabled_name ON skills(enabled, name);
             -- Usage is reported most-used-first, and the count is maintained by
             -- the application rather than derived from a join.
             CREATE INDEX idx_skills_use_count ON skills(use_count);
             -- Skills are grouped by type wherever they are listed. Partial, so
             -- unlabelled skills -- the majority -- do not all share one key.
             CREATE INDEX idx_skills_type ON skills(type) WHERE type IS NOT NULL;
             PRAGMA user_version=10;",
        )
        .map_err(|error| error.to_string())?;
        tx.commit().map_err(|error| error.to_string())?;
    }
    // The thoughts and tool calls behind one reply, in the order they happened.
    //
    // Its own JSON column rather than folded into `metrics_json`, because the two
    // answer different questions: metrics are what the reply *cost*, activity is
    // what the model *did*, and a conversation's transcript is the only place
    // anyone will look for the second one. A reply that read five files and then
    // answered has to still say so a week later, which is what a stored step list
    // is for.
    //
    // Nullable so a message with no steps costs nothing, and so a row written
    // before this column existed reads as "did nothing recorded" rather than
    // failing to parse.
    if version < 11 {
        let tx = connection
            .transaction()
            .map_err(|error| error.to_string())?;
        tx.execute_batch(&format!(
            "ALTER TABLE messages ADD COLUMN activity_json TEXT;
             PRAGMA user_version={LATEST_SCHEMA_VERSION};"
        ))
        .map_err(|error| error.to_string())?;
        tx.commit().map_err(|error| error.to_string())?;
    }
    // Folds `app.disabled_models` into `provider_models.enabled`, then drops
    // the setting.
    //
    // The array was a second copy of a fact the column already held, and it was
    // the copy the frontend wrote: toggling a model in Settings updated the array
    // and left the column at its seeded value, so `list_endpoints` kept serving
    // the model and every read had to re-apply the array to hide it. Two copies
    // of one setting with only one of them written is a drift that shows up as a
    // model that reappears after a restart.
    //
    // Each entry is `"endpointId:model"`, split on the *first* colon because a
    // model id may itself contain one -- `stepfun/step-3.7-flash:free` has no
    // colon but `provider:model` style ids do, and splitting on the last one
    // would rewrite the endpoint id instead.
    //
    // A model named in the array with no row is inserted as disabled rather than
    // skipped: the user was told it was off, and dropping the entry would turn
    // that back on without them asking.
    //
    // Only where the provider still exists. `provider_models.provider_id` is a
    // foreign key, so inserting a row for a removed provider fails and would roll
    // the whole migration back -- and a migration that aborts takes every other
    // change with it. There is nothing to preserve in that case: the array entry
    // names a provider that no longer exists, so there is no list for the model to
    // reappear in.
    if version < 12 {
        let tx = connection
            .transaction()
            .map_err(|error| error.to_string())?;
        let stored: Option<String> = tx
            .query_row(
                "SELECT value FROM settings WHERE key='app.disabled_models'",
                [],
                |row| row.get::<_, Option<String>>(0),
            )
            .unwrap_or(None);
        if let Some(raw) = stored {
            let entries: Vec<String> = serde_json::from_str(&raw).unwrap_or_default();
            for entry in entries {
                let Some((endpoint_id, model_id)) = entry.split_once(':') else {
                    continue;
                };
                if endpoint_id.is_empty() || model_id.is_empty() {
                    continue;
                }
                let exists: i64 = tx
                    .query_row(
                        "SELECT COUNT(*) FROM providers WHERE id=?1",
                        params![endpoint_id],
                        |row| row.get(0),
                    )
                    .unwrap_or(0);
                if exists == 0 {
                    continue;
                }
                tx.execute(
                    "INSERT INTO provider_models (provider_id, model_id, enabled) VALUES (?1,?2,0)
                     ON CONFLICT(provider_id, model_id) DO UPDATE SET enabled=0",
                    params![endpoint_id, model_id],
                )
                .map_err(|error| error.to_string())?;
            }
            tx.execute("DELETE FROM settings WHERE key='app.disabled_models'", [])
                .map_err(|error| error.to_string())?;
        }
        tx.execute_batch(&format!("PRAGMA user_version={LATEST_SCHEMA_VERSION};"))
            .map_err(|error| error.to_string())?;
        tx.commit().map_err(|error| error.to_string())?;
    }
    Ok(())
}

fn read_json<T: DeserializeOwned>(path: &std::path::Path) -> Option<T> {
    fs::read_to_string(path)
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
}
fn json<T: Serialize>(value: &T) -> Result<String, String> {
    serde_json::to_string(value).map_err(|error| error.to_string())
}
#[cfg(test)]
mod tests {
    use super::*;

    /// `usage_report` is the entry point the statistics command calls, and it
    /// forwards a start date and a label to `build_report`. Both are strings, so
    /// swapping them compiles fine and then filters on `since = "month"`, which
    /// matches no dates and leaves the page empty. This goes through
    /// `usage_report` rather than `build_report` so the wiring itself is
    /// covered rather than just the query.
    #[test]
    fn usage_report_returns_rows_for_a_real_date_range() {
        let mut connection = Connection::open_in_memory().unwrap();
        migrate(&mut connection).unwrap();
        let database = Database {
            connection: Mutex::new(connection),
        };
        database
            .save_chat_session(&serde_json::json!({
                "id": "s1", "title": "First",
                "createdAt": "2026-10-01T10:00:00.000Z", "updatedAt": "2026-10-01T10:00:05.000Z",
                "messages": [
                    {"role": "user", "content": "hi"},
                    {"role": "assistant", "content": "hello", "modelId": "gpt", "providerId": "openai",
                     "metrics": {"prompt_tokens": 23, "completion_tokens": 36}}
                ]
            }))
            .unwrap();

        let report = database
            .usage_report("month", "2026-09-02", "substr(created_at,1,10)")
            .unwrap();

        assert_eq!(report.range, "month");
        assert_eq!(report.since, "2026-09-02");
        assert_eq!(report.sessions, 1);
        assert_eq!(report.user_messages, 1);
        assert_eq!(report.assistant_messages, 1);
        assert_eq!(report.prompt_tokens, 23);
        assert_eq!(report.completion_tokens, 36);
        assert_eq!(report.timeline.len(), 1);
        assert_eq!(report.models.len(), 1);
    }

    /// A model-generated title sets `renamed` but not `renamedByUser`, because
    /// the user did not choose it. Both flags are stored in the session metadata
    /// and read back, so the overview can tell the two cases apart.
    #[test]
    fn a_generated_title_is_not_reported_as_a_manual_rename() {
        let mut connection = Connection::open_in_memory().unwrap();
        migrate(&mut connection).unwrap();
        let database = Database {
            connection: Mutex::new(connection),
        };

        database
            .save_chat_session(&serde_json::json!({
                "id": "s1", "title": "Generated name",
                "createdAt": "2026-10-01T10:00:00.000Z", "updatedAt": "2026-10-01T10:00:05.000Z",
                "renamed": true, "renamedByUser": false,
                "messages": [{"role": "user", "content": "hi"}]
            }))
            .unwrap();

        let loaded = database.list_chat_sessions().unwrap();
        assert_eq!(loaded[0]["renamed"], true);
        assert_eq!(loaded[0]["renamedByUser"], false);

        // A manual rename then sets both.
        database
            .save_chat_session(&serde_json::json!({
                "id": "s1", "title": "My own name",
                "createdAt": "2026-10-01T10:00:00.000Z", "updatedAt": "2026-10-01T10:01:00.000Z",
                "renamed": true, "renamedByUser": true,
                "messages": [{"role": "user", "content": "hi"}]
            }))
            .unwrap();

        let loaded = database.list_chat_sessions().unwrap();
        assert_eq!(loaded[0]["renamedByUser"], true);
    }

    /// The steps behind a reply survive a reload, because the transcript is the
    /// only place anyone will look for what the agent did. A reply that read
    /// three files a week ago has to still say so.
    #[test]
    fn a_replys_steps_round_trip_through_storage() {
        let database = migrated();
        database
            .save_chat_session(&serde_json::json!({
                "id": "s1", "title": "First",
                "createdAt": "2026-10-01T10:00:00.000Z", "updatedAt": "2026-10-01T10:00:09.000Z",
                "messages": [
                    {"role": "user", "content": "read it"},
                    {"role": "assistant", "content": "Here is the summary.",
                     "activity": [
                         {"kind": "thought", "text": "The user just asked me to read it.", "seconds": 2.4, "running": false},
                         {"kind": "tool", "tool": "read_file", "detail": "projectz/README.md", "label": "Read file", "outcome": "417 lines", "failed": false, "seconds": 0.12, "running": false}
                     ]}
                ]
            }))
            .unwrap();

        let loaded = database.list_chat_sessions().unwrap();
        let steps = loaded[0]["messages"][1]["activity"].as_array().unwrap();
        assert_eq!(steps.len(), 2);
        // Order is the whole point: a thought before a tool and one after it are
        // separate steps, so the stored list must keep them in sequence.
        assert_eq!(steps[0]["kind"], "thought");
        assert_eq!(steps[0]["seconds"], 2.4);
        assert_eq!(steps[1]["kind"], "tool");
        assert_eq!(steps[1]["detail"], "projectz/README.md");
    }

    /// A local clock reading means nothing after the window that produced it, so
    /// it must not be stored: a later render would otherwise keep adding to a
    /// step that finished long ago.
    #[test]
    fn a_stored_step_does_not_carry_the_local_start_clock() {
        let database = migrated();
        database
            .save_chat_session(&serde_json::json!({
                "id": "s1", "title": "First",
                "createdAt": "2026-10-01T10:00:00.000Z", "updatedAt": "2026-10-01T10:00:05.000Z",
                "messages": [{"role": "assistant", "content": "done",
                    "activity": [{"kind": "tool", "tool": "read_file", "detail": "a", "label": "Read file",
                                  "outcome": "1 line", "failed": false, "seconds": 0.2,
                                  "running": false, "startedAt": 1759300000000i64}]}]
            }))
            .unwrap();

        let loaded = database.list_chat_sessions().unwrap();
        let step = &loaded[0]["messages"][0]["activity"][0];
        assert!(step.get("startedAt").is_none(), "{step}");
        // The measured duration is kept: that one is a fact about the run.
        assert_eq!(step["seconds"], 0.2);
    }

    /// A message with no steps must not gain an empty array, and a row written
    /// before the column existed must still load.
    #[test]
    fn a_reply_with_no_steps_stores_nothing_rather_than_an_empty_list() {
        let database = migrated();
        database
            .save_chat_session(&serde_json::json!({
                "id": "s1", "title": "First",
                "createdAt": "2026-10-01T10:00:00.000Z", "updatedAt": "2026-10-01T10:00:05.000Z",
                "messages": [{"role": "assistant", "content": "plain answer"}]
            }))
            .unwrap();

        let loaded = database.list_chat_sessions().unwrap();
        assert!(loaded[0]["messages"][0]["activity"].is_null());
    }

    #[test]
    fn migrations_advance_schema_to_latest_version() {
        let mut connection = Connection::open_in_memory().unwrap();
        migrate(&mut connection).unwrap();
        migrate(&mut connection).unwrap();
        let version: i64 = connection
            .pragma_query_value(None, "user_version", |row| row.get(0))
            .unwrap();
        assert_eq!(version, LATEST_SCHEMA_VERSION);
        let table_count: i64 = connection
            .query_row("SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name IN ('providers','provider_models','sessions','messages','local_models','settings','model_cache')", [], |row| row.get(0))
            .unwrap();
        assert_eq!(table_count, 7);
    }

    /// A cached read survives being asked for twice, which is the whole point.
    #[test]
    fn a_cached_tool_result_is_returned_again() {
        let database = migrated();
        seed_session(&database, "s1");
        let key = "read_file:abc";
        database.tool_cache_put(
            "s1",
            "read_file",
            key,
            "1 | one",
            "2026-01-01T00:00:00.000Z",
        );
        assert_eq!(
            database.tool_cache_get("s1", "read_file", key),
            Some("1 | one".to_string())
        );
    }

    /// The session is part of the key: one conversation must never be answered
    /// with another's reads.
    #[test]
    fn a_cache_entry_belongs_to_one_session_only() {
        let database = migrated();
        seed_session(&database, "s1");
        seed_session(&database, "s2");
        database.tool_cache_put(
            "s1",
            "read_file",
            "k",
            "content",
            "2026-01-01T00:00:00.000Z",
        );
        assert_eq!(database.tool_cache_get("s2", "read_file", "k"), None);
    }

    /// A write invalidates every earlier read. This is the rule that keeps a
    /// cached file read from outliving the edit that made it wrong.
    #[test]
    fn a_write_stops_earlier_cached_reads_being_returned() {
        let database = migrated();
        seed_session(&database, "s1");
        let key = "read_file:a";
        database.tool_cache_put("s1", "read_file", key, "old", "2026-01-01T00:00:00.000Z");

        database.tool_cache_invalidate_session("s1", "2026-01-01T00:00:05.000Z");
        assert_eq!(database.tool_cache_get("s1", "read_file", key), None);

        // Repopulating after the write is valid again: the new read is newer
        // than the invalidation, so it is genuinely current.
        database.tool_cache_put("s1", "read_file", key, "new", "2026-01-01T00:00:06.000Z");
        assert_eq!(
            database.tool_cache_get("s1", "read_file", key),
            Some("new".to_string())
        );
    }

    /// An entry stamped at the same instant as the invalidation is kept.
    ///
    /// Sound only because `ToolCache` issues strictly increasing stamps, so an
    /// equal stamp means "written after the last write", not "written before it".
    /// The database layer has no way to tell those apart on its own, which is why
    /// the guarantee lives in the allocator rather than in this comparison.
    #[test]
    fn a_read_at_the_same_moment_as_the_write_survives() {
        let database = migrated();
        seed_session(&database, "s1");
        let at = "2026-01-01T00:00:05.000Z";
        database.tool_cache_put("s1", "read_file", "k", "content", at);
        database.tool_cache_invalidate_session("s1", at);
        assert!(database.tool_cache_get("s1", "read_file", "k").is_some());
    }

    /// Storing twice keeps the newer value rather than the first.
    #[test]
    fn storing_again_replaces_the_earlier_value() {
        let database = migrated();
        seed_session(&database, "s1");
        database.tool_cache_put("s1", "read_file", "k", "first", "2026-01-01T00:00:00.000Z");
        database.tool_cache_put("s1", "read_file", "k", "second", "2026-01-01T00:00:01.000Z");
        assert_eq!(
            database.tool_cache_get("s1", "read_file", "k"),
            Some("second".to_string())
        );
    }

    fn migrated() -> Database {
        Database::in_memory().expect("migrated in-memory database")
    }

    /// A sub-agent run lives in the same table as a conversation but appears in
    /// exactly one list. The sidebar read must not show it, and the sub-agent read
    /// must not show a conversation -- the whole reason the two queries exist apart.
    #[test]
    fn a_subagent_run_is_listed_apart_from_the_conversations() {
        let database = migrated();
        database
            .save_chat_session(&serde_json::json!({
                "id": "chat", "title": "A chat",
                "createdAt": "2026-01-01T00:00:00.000Z", "updatedAt": "2026-01-01T00:00:01.000Z",
                "messages": [{ "role": "user", "content": "hi" }]
            }))
            .unwrap();
        database
            .save_chat_session(&serde_json::json!({
                "id": "run", "title": "scan the repo",
                "createdAt": "2026-01-01T00:00:02.000Z", "updatedAt": "2026-01-01T00:00:03.000Z",
                "kind": "subagent", "parentSessionId": "chat",
                "messages": [{ "role": "user", "content": "find the entry point" }]
            }))
            .unwrap();

        let conversations = database.list_session_headers().unwrap();
        assert_eq!(conversations.len(), 1, "the sidebar must not show a run");
        assert_eq!(conversations[0]["id"], "chat");

        let runs = database.list_subagent_headers(Some("chat")).unwrap();
        assert_eq!(runs.len(), 1, "the run must be listed for its parent");
        assert_eq!(runs[0]["id"], "run");
        assert_eq!(runs[0]["kind"], "subagent");
        assert_eq!(runs[0]["parentSessionId"], "chat");

        // Naming a conversation with no runs returns none, and asking for every
        // run returns all of them.
        assert!(database
            .list_subagent_headers(Some("somewhere-else"))
            .unwrap()
            .is_empty());
        assert_eq!(database.list_subagent_headers(None).unwrap().len(), 1);
    }

    /// A conversation written before sub-agents existed has no `kind`, and must
    /// read as an ordinary chat rather than being mistaken for a run.
    #[test]
    fn a_session_without_a_kind_reads_as_a_chat() {
        let database = migrated();
        database
            .save_chat_session(&serde_json::json!({
                "id": "old", "title": "Old",
                "createdAt": "2026-01-01T00:00:00.000Z", "updatedAt": "2026-01-01T00:00:01.000Z",
                "messages": [{ "role": "user", "content": "hi" }]
            }))
            .unwrap();
        assert_eq!(database.list_session_headers().unwrap()[0]["kind"], "chat");
        assert!(database.list_subagent_headers(None).unwrap().is_empty());
    }

    /// Deleting a conversation takes its runs with it, so the panel is not left
    /// showing agents grouped under a chat that no longer exists.
    #[test]
    fn deleting_a_conversation_deletes_its_subagent_runs() {
        let database = migrated();
        for (id, kind) in [("chat", "chat"), ("run", "subagent")] {
            database
                .save_chat_session(&serde_json::json!({
                    "id": id, "title": id,
                    "createdAt": "2026-01-01T00:00:00.000Z", "updatedAt": "2026-01-01T00:00:01.000Z",
                    "kind": kind, "parentSessionId": if kind == "subagent" { Some("chat") } else { None },
                    "messages": [{ "role": "user", "content": "hi" }]
                }))
                .unwrap();
        }
        assert_eq!(database.list_subagent_headers(None).unwrap().len(), 1);

        database.delete_chat_session("chat").unwrap();

        assert!(database.list_subagent_headers(None).unwrap().is_empty());
        assert!(database.list_session_headers().unwrap().is_empty());
    }

    /// A session row, since `tool_cache` has a foreign key to it.
    fn seed_session(database: &Database, id: &str) {
        database
            .save_chat_session(&serde_json::json!({
                "id": id,
                "title": "test",
                "createdAt": "2026-01-01T00:00:00.000Z",
                "updatedAt": "2026-01-01T00:00:00.000Z",
                "messages": []
            }))
            .expect("seed session");
    }

    /// Clearing must take the messages with it. Foreign keys are only enforced when the
    /// pragma is on, so the test turns it on explicitly rather than trusting `Database`.
    #[test]
    fn clearing_sessions_removes_their_messages_and_keeps_settings() {
        let mut connection = Connection::open_in_memory().unwrap();
        migrate(&mut connection).unwrap();
        connection
            .pragma_update(None, "foreign_keys", "ON")
            .unwrap();
        connection
            .execute(
                "INSERT INTO sessions (id,title,created_at,updated_at) VALUES ('a','A','t','t'),('b','B','t','t')",
                [],
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO messages (id,session_id,ordinal,role,content,created_at) VALUES ('a:0','a',0,'user','hi','t'),('b:0','b',0,'user','yo','t')",
                [],
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO settings (key,value) VALUES ('app.general','{}')",
                [],
            )
            .unwrap();

        let connection = Database {
            connection: Mutex::new(connection),
        };
        assert_eq!(connection.clear_chat_sessions().unwrap(), 2);
        assert_eq!(connection.list_chat_sessions().unwrap().len(), 0);

        let messages: i64 = connection
            .connection
            .lock()
            .unwrap()
            .query_row("SELECT COUNT(*) FROM messages", [], |row| row.get(0))
            .unwrap();
        assert_eq!(messages, 0);
        // Settings and providers are deliberately untouched by a session clear.
        assert!(connection
            .setting::<serde_json::Value>("app.general")
            .is_some());
        assert_eq!(connection.clear_chat_sessions().unwrap(), 0);
    }

    /// Capabilities round-trip through storage, and a model the provider said
    /// nothing about reads back as unknown rather than as "cannot".
    #[test]
    fn capabilities_round_trip_and_keep_unknowns_unknown() {
        use crate::ai::remote::capabilities::ModelCapabilities;

        let mut connection = Connection::open_in_memory().unwrap();
        migrate(&mut connection).unwrap();
        connection
            .execute(
                "INSERT INTO providers (id,name,endpoint_url,api_key) VALUES ('kilo','Kilo','','')",
                [],
            )
            .unwrap();
        let database = Database {
            connection: Mutex::new(connection),
        };

        database
            .save_model_capabilities(
                "kilo",
                &[
                    (
                        "flash".to_string(),
                        ModelCapabilities {
                            context_length: Some(131_072),
                            input_modalities: Some(vec!["text".into(), "image".into()]),
                            output_modalities: Some(vec!["text".into()]),
                            supports_reasoning: Some(true),
                            reasoning_values: Some(vec!["none".into(), "low".into()]),
                            supports_tools: Some(true),
                        },
                    ),
                    // A bare entry: nothing was reported about this model.
                    ("plain".to_string(), ModelCapabilities::default()),
                ],
                "1700000000",
            )
            .unwrap();

        let flash = database.model_capabilities("kilo", "flash").unwrap();
        assert_eq!(flash.context_length, Some(131_072));
        assert_eq!(
            flash.input_modalities.as_deref(),
            Some(["text", "image"].map(String::from).as_slice())
        );
        assert_eq!(flash.supports_reasoning, Some(true));
        assert_eq!(flash.supports_tools, Some(true));
        assert_eq!(
            flash.reasoning_values.as_deref(),
            Some(["none", "low"].map(String::from).as_slice())
        );
        let plain = database.model_capabilities("kilo", "plain").unwrap();
        assert_eq!(plain.context_length, None);
        assert_eq!(plain.input_modalities, None);
        assert_eq!(plain.supports_reasoning, None);
        assert_eq!(plain.supports_tools, None);

        // A model on a provider we never inspected is unknown too.
        assert!(database
            .model_capabilities("other", "nothing")
            .unwrap()
            .context_length
            .is_none());
    }

    /// A provider saved twice keeps `favorite` while refreshing capabilities,
    /// because the refresh must not reset per-model settings the user owns.
    #[test]
    fn refreshing_capabilities_preserves_a_models_favorite_flag() {
        use crate::ai::remote::capabilities::ModelCapabilities;

        let mut connection = Connection::open_in_memory().unwrap();
        migrate(&mut connection).unwrap();
        connection
            .execute(
                "INSERT INTO providers (id,name,endpoint_url,api_key) VALUES ('kilo','Kilo','','')",
                [],
            )
            .unwrap();
        let database = Database {
            connection: Mutex::new(connection),
        };
        database
            .save_model_capabilities(
                "kilo",
                &[("flash".to_string(), ModelCapabilities::default())],
                "1700000000",
            )
            .unwrap();
        {
            let guard = database.connection.lock().unwrap();
            guard
                .execute(
                    "UPDATE provider_models SET favorite=1 WHERE provider_id='kilo' AND model_id='flash'",
                    [],
                )
                .unwrap();
        }

        database
            .save_model_capabilities(
                "kilo",
                &[(
                    "flash".to_string(),
                    ModelCapabilities {
                        context_length: Some(256_000),
                        ..Default::default()
                    },
                )],
                "1700000100",
            )
            .unwrap();

        let connection = database.connection.lock().unwrap();
        let (favorite, context): (i64, Option<i64>) = connection
            .query_row(
                "SELECT favorite, context_length FROM provider_models WHERE provider_id='kilo' AND model_id='flash'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(favorite, 1, "the favorite flag must survive a refresh");
        assert_eq!(context, Some(256_000));
    }

    /// One read per provider returns every model that has capabilities, keyed by
    /// model id, so a picker showing hundreds of models costs one command.
    #[test]
    fn provider_capabilities_returns_every_model_in_one_read() {
        use crate::ai::remote::capabilities::ModelCapabilities;

        let mut connection = Connection::open_in_memory().unwrap();
        migrate(&mut connection).unwrap();
        for id in ["kilo", "other"] {
            connection
                .execute(
                    &format!(
                        "INSERT INTO providers (id,name,endpoint_url,api_key) VALUES ('{id}','{id}','','')"
                    ),
                    [],
                )
                .unwrap();
        }
        let database = Database {
            connection: Mutex::new(connection),
        };
        database
            .save_model_capabilities(
                "kilo",
                &[
                    (
                        "a".to_string(),
                        ModelCapabilities {
                            context_length: Some(200_000),
                            input_modalities: Some(vec!["text".into(), "image".into()]),
                            supports_reasoning: Some(true),
                            ..Default::default()
                        },
                    ),
                    (
                        "b".to_string(),
                        ModelCapabilities {
                            context_length: Some(8_000),
                            ..Default::default()
                        },
                    ),
                ],
                "1700000000",
            )
            .unwrap();
        database
            .save_model_capabilities(
                "other",
                &[("c".to_string(), ModelCapabilities::default())],
                "1700000000",
            )
            .unwrap();

        let kilo = database.provider_capabilities("kilo").unwrap();
        assert_eq!(kilo.len(), 2);
        assert_eq!(kilo["a"].context_length, Some(200_000));
        assert_eq!(
            kilo["a"].input_modalities.as_deref(),
            Some(["text", "image"].map(String::from).as_slice())
        );
        assert_eq!(kilo["a"].supports_reasoning, Some(true));
        assert_eq!(kilo["b"].context_length, Some(8_000));
        // The two providers do not bleed into each other.
        let other = database.provider_capabilities("other").unwrap();
        assert_eq!(other.len(), 1);
        assert!(other.contains_key("c"));
        assert!(database
            .provider_capabilities("missing")
            .unwrap()
            .is_empty());
    }

    /// A migrated database opens cleanly and the new tables and indexes exist,
    /// so the statistics queries can rely on the columns being present.
    #[test]
    fn migrations_add_metric_columns_indexes_and_provider_models() {
        let mut connection = Connection::open_in_memory().unwrap();
        migrate(&mut connection).unwrap();

        let version: i64 = connection
            .pragma_query_value(None, "user_version", |row| row.get(0))
            .unwrap();
        assert_eq!(version, LATEST_SCHEMA_VERSION);

        // Capability columns the adaptive frontend reads.
        let provider_models: Vec<String> = connection
            .prepare("SELECT name FROM pragma_table_info('provider_models')")
            .unwrap()
            .query_map([], |row| row.get(0))
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        for expected in [
            "context_length",
            "input_modalities",
            "output_modalities",
            "supports_reasoning",
            "reasoning_values",
            "supports_tools",
            "capabilities_checked_at",
        ] {
            assert!(
                provider_models.iter().any(|name| name == expected),
                "provider_models is missing {expected}"
            );
        }

        // Every column the statistics query reads must exist.
        let columns: Vec<String> = connection
            .prepare("SELECT name FROM pragma_table_info('messages')")
            .unwrap()
            .query_map([], |row| row.get(0))
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        for expected in [
            "prompt_tokens",
            "completion_tokens",
            "generation_seconds",
            "prompt_seconds",
            "generation_rate_estimated",
            "activity_json",
        ] {
            assert!(
                columns.iter().any(|name| name == expected),
                "messages is missing {expected}"
            );
        }

        let indexes: Vec<String> = connection
            .prepare("SELECT name FROM sqlite_master WHERE type='index'")
            .unwrap()
            .query_map([], |row| row.get(0))
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        assert!(indexes.iter().any(|name| name == "idx_messages_created_at"));
        assert!(indexes
            .iter()
            .any(|name| name == "idx_provider_models_favorite"));
    }

    /// A provider saved with a model list gets a row per model, and the model
    /// list survives a round trip through the new table.
    #[test]
    fn saving_an_endpoint_records_one_row_per_model() {
        let mut connection = Connection::open_in_memory().unwrap();
        migrate(&mut connection).unwrap();
        let connection = Database {
            connection: Mutex::new(connection),
        };

        connection
            .save_endpoint(&Endpoint {
                id: "openai".into(),
                name: "OpenAI".into(),
                base_url: "https://api.openai.com/v1".into(),
                api_key: "secret".into(),
                models: vec!["gpt-4o".into(), "gpt-4o-mini".into()],
                disabled_models: vec![],
                enabled: true,
            })
            .unwrap();

        let rows: i64 = connection
            .connection
            .lock()
            .unwrap()
            .query_row(
                "SELECT COUNT(*) FROM provider_models WHERE provider_id='openai'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(rows, 2);

        let listed = connection.list_endpoints().unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].models.len(), 2);
        assert!(listed[0].models.contains(&"gpt-4o".to_string()));
    }

    /// The upsert path must append, update in place, and drop only the rows past
    /// the end of the array — never rewrite the whole transcript.
    #[test]
    fn saving_a_session_upserts_messages_and_keeps_rollups_in_step() {
        let mut connection = Connection::open_in_memory().unwrap();
        migrate(&mut connection).unwrap();
        connection
            .pragma_update(None, "foreign_keys", "ON")
            .unwrap();
        let connection = Database {
            connection: Mutex::new(connection),
        };

        let session = serde_json::json!({
            "id": "s1",
            "title": "First",
            "createdAt": "2026-10-01T10:00:00.000Z",
            "updatedAt": "2026-10-01T10:00:05.000Z",
            "messages": [
                {"role": "user", "content": "hello"},
                {"role": "assistant", "content": "partial", "modelId": "gpt", "providerId": "openai",
                 "metrics": {"prompt_tokens": 10, "completion_tokens": 5, "generation_seconds": 1.5, "prompt_seconds": 0.5}}
            ]
        });
        connection.save_chat_session(&session).unwrap();

        // A later save with more messages and an updated final reply.
        let grown = serde_json::json!({
            "id": "s1",
            "title": "First",
            "createdAt": "2026-10-01T10:00:00.000Z",
            "updatedAt": "2026-10-01T10:01:00.000Z",
            "messages": [
                {"role": "user", "content": "hello"},
                {"role": "assistant", "content": "partial", "modelId": "gpt", "providerId": "openai",
                 "metrics": {"prompt_tokens": 10, "completion_tokens": 5, "generation_seconds": 1.5, "prompt_seconds": 0.5}},
                {"role": "user", "content": "more"},
                {"role": "assistant", "content": "complete", "modelId": "gpt", "providerId": "openai",
                 "metrics": {"prompt_tokens": 20, "completion_tokens": 30, "generation_seconds": 2.0, "prompt_seconds": 1.0}}
            ]
        });
        connection.save_chat_session(&grown).unwrap();

        let loaded = connection.list_chat_sessions().unwrap();
        let messages = loaded[0]["messages"].as_array().unwrap();
        assert_eq!(messages.len(), 4);
        // The streaming reply was updated in place rather than duplicated.
        assert_eq!(messages[3]["content"], "complete");
        assert_eq!(messages[1]["content"], "partial");

        // Session rollups must match the sum of the messages.
        assert_eq!(loaded[0]["promptTokens"], 30);
        assert_eq!(loaded[0]["completionTokens"], 35);
    }

    /// A shrinking transcript must remove only the trailing rows, not the whole
    /// conversation.
    #[test]
    fn saving_a_shorter_session_removes_only_trailing_messages() {
        let mut connection = Connection::open_in_memory().unwrap();
        migrate(&mut connection).unwrap();
        connection
            .pragma_update(None, "foreign_keys", "ON")
            .unwrap();
        let connection = Database {
            connection: Mutex::new(connection),
        };

        connection
            .save_chat_session(&serde_json::json!({
                "id": "s1", "title": "First",
                "createdAt": "2026-10-01T10:00:00.000Z", "updatedAt": "2026-10-01T10:00:05.000Z",
                "messages": [
                    {"role": "user", "content": "one"},
                    {"role": "assistant", "content": "two"},
                    {"role": "user", "content": "three"}
                ]
            }))
            .unwrap();
        assert_eq!(
            connection.list_chat_sessions().unwrap()[0]["messages"]
                .as_array()
                .unwrap()
                .len(),
            3
        );

        connection
            .save_chat_session(&serde_json::json!({
                "id": "s1", "title": "First",
                "createdAt": "2026-10-01T10:00:00.000Z", "updatedAt": "2026-10-01T10:00:06.000Z",
                "messages": [{"role": "user", "content": "one"}]
            }))
            .unwrap();

        let loaded = connection.list_chat_sessions().unwrap();
        let messages = loaded[0]["messages"].as_array().unwrap();
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0]["content"], "one");
    }

    /// A save carrying no messages must not delete the conversation.
    ///
    /// Retrying a reply legitimately shortens a transcript, so a shorter array
    /// has to stay allowed -- but an *empty* one is what a caller holding a
    /// session row without its transcript would send, and the tail-delete would
    /// turn it into a silently emptied conversation. This is the guard that
    /// makes lazy transcript loading safe to introduce.
    #[test]
    fn a_save_with_no_messages_does_not_empty_a_stored_conversation() {
        let database = migrated();
        // `seed_session` writes a session with no messages, which would make the
        // guard pass by default rather than by working. A real transcript is
        // written here so the test exercises the case it is about.
        database
            .save_chat_session(&serde_json::json!({
                "id": "s1", "title": "First",
                "createdAt": "2026-10-01T10:00:00.000Z", "updatedAt": "2026-10-01T10:00:05.000Z",
                "messages": [{"role": "user", "content": "one"}]
            }))
            .unwrap();
        assert!(database.stored_message_count_for_test("s1").unwrap() > 0);

        let error = database
            .save_chat_session(&serde_json::json!({
                "id": "s1", "title": "Renamed",
                "createdAt": "2026-10-01T10:00:00.000Z", "updatedAt": "2026-10-01T10:00:09.000Z",
                "messages": []
            }))
            .unwrap_err();
        assert!(error.contains("Refusing to save session 's1'"), "{error}");

        // The conversation is intact, and the rejected title did not apply: the
        // write was refused as a whole, so it neither emptied the transcript nor
        // half-committed a rename.
        let messages = database.list_session_messages("s1").unwrap();
        assert_eq!(messages.len(), 1);
        assert_eq!(
            database.list_session_headers().unwrap()[0]["title"],
            "First"
        );
    }

    /// A brand new conversation is legitimately empty, and the guard must not
    /// refuse to create one.
    #[test]
    fn a_new_session_may_be_saved_with_no_messages_yet() {
        let database = migrated();
        database
            .save_chat_session(&serde_json::json!({
                "id": "empty", "title": "Nothing sent",
                "createdAt": "2026-10-01T10:00:00.000Z", "updatedAt": "2026-10-01T10:00:00.000Z",
                "messages": []
            }))
            .unwrap();
        assert!(database.list_session_messages("empty").unwrap().is_empty());
    }

    /// A session saved with no `messages` key at all leaves its transcript alone.
    ///
    /// Distinct from an empty array: the caller is not writing messages rather
    /// than writing none, so the tail-delete must not run.
    #[test]
    fn a_save_that_omits_messages_leaves_the_transcript_alone() {
        let database = migrated();
        database
            .save_chat_session(&serde_json::json!({
                "id": "s1", "title": "First",
                "createdAt": "2026-10-01T10:00:00.000Z", "updatedAt": "2026-10-01T10:00:05.000Z",
                "messages": [{"role": "user", "content": "one"}]
            }))
            .unwrap();
        database
            .save_chat_session(&serde_json::json!({
                "id": "s1", "title": "Renamed",
                "createdAt": "2026-10-01T10:00:00.000Z", "updatedAt": "2026-10-01T10:00:09.000Z"
            }))
            .unwrap();
        assert_eq!(database.list_session_messages("s1").unwrap().len(), 1);
        assert_eq!(
            database.list_session_headers().unwrap()[0]["title"],
            "Renamed"
        );
    }

    /// A model named in the old `app.disabled_models` array is disabled by the
    /// migration, and the setting is gone afterwards.
    ///
    /// The array used to be the only copy the frontend wrote, so a migration that
    /// dropped it without folding it in would silently re-enable every model the
    /// user had turned off.
    #[test]
    fn the_disabled_models_setting_is_folded_into_the_enabled_column() {
        let mut connection = Connection::open_in_memory().unwrap();
        migrate(&mut connection).unwrap();
        connection
            .execute_batch(
                "INSERT INTO providers (id,name,endpoint_url,api_key) VALUES ('kilo','Kilo','','');
                 INSERT INTO provider_models (provider_id, model_id, enabled) VALUES ('kilo','flash',1);",
            )
            .unwrap();

        // Claim the pre-migration version with the setting in place, then migrate
        // again the way a launch would. `put_setting_tx` takes a transaction, so
        // it is committed here: dropped uncommitted it rolls back and the
        // migration finds nothing to fold.
        let tx = connection.unchecked_transaction().unwrap();
        put_setting_tx(
            &tx,
            "app.disabled_models",
            &vec![
                "kilo:flash".to_string(),
                "kilo:stepfun/step-3.7-flash:free".to_string(),
            ],
        )
        .unwrap();
        tx.commit().unwrap();
        connection.execute_batch("PRAGMA user_version=11;").unwrap();
        migrate(&mut connection).unwrap();

        let enabled: i64 = connection
            .query_row(
                "SELECT enabled FROM provider_models WHERE provider_id='kilo' AND model_id='flash'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(enabled, 0, "an existing row should be switched off");

        // A model named in the array with no row is inserted as off, not skipped,
        // because the user was told it was off.
        let inserted: i64 = connection
            .query_row(
                "SELECT enabled FROM provider_models WHERE provider_id='kilo' AND model_id='stepfun/step-3.7-flash:free'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(
            inserted, 0,
            "a hand-added model named in the array stays off"
        );

        let remaining: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM settings WHERE key='app.disabled_models'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(remaining, 0, "the setting is dropped once folded in");

        let listed: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM provider_models WHERE provider_id='kilo' AND enabled=1",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(listed, 0);
    }

    /// A model id containing a colon survives the split, because the migration
    /// splits on the *first* one and the endpoint id is what precedes it.
    #[test]
    fn a_disabled_model_id_containing_a_colon_is_not_mangled() {
        let mut connection = Connection::open_in_memory().unwrap();
        migrate(&mut connection).unwrap();
        connection
            .execute(
                "INSERT INTO providers (id,name,endpoint_url,api_key) VALUES ('kilo','Kilo','','')",
                [],
            )
            .unwrap();
        let tx = connection.unchecked_transaction().unwrap();
        put_setting_tx(
            &tx,
            "app.disabled_models",
            &vec!["kilo:vendor:model-x".to_string()],
        )
        .unwrap();
        tx.commit().unwrap();
        connection.execute_batch("PRAGMA user_version=11;").unwrap();
        migrate(&mut connection).unwrap();

        let (provider, model): (String, String) = connection
            .query_row(
                "SELECT provider_id, model_id FROM provider_models WHERE enabled=0",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(provider, "kilo");
        assert_eq!(model, "vendor:model-x");
    }

    /// An entry naming a provider that has been removed does not abort the
    /// migration.
    ///
    /// `provider_models.provider_id` is a foreign key, so inserting for a missing
    /// provider fails. Failing there would roll back the migration transaction,
    /// which would leave `user_version` behind and re-run this on every launch
    /// forever -- so the entry is skipped rather than inserted, and the migration
    /// completes.
    #[test]
    fn a_disabled_model_for_a_removed_provider_does_not_block_the_migration() {
        let mut connection = Connection::open_in_memory().unwrap();
        migrate(&mut connection).unwrap();
        let tx = connection.unchecked_transaction().unwrap();
        put_setting_tx(
            &tx,
            "app.disabled_models",
            &vec!["gone:model-x".to_string()],
        )
        .unwrap();
        tx.commit().unwrap();
        connection.execute_batch("PRAGMA user_version=11;").unwrap();
        migrate(&mut connection).unwrap();

        let version: i64 = connection
            .pragma_query_value(None, "user_version", |row| row.get(0))
            .unwrap();
        assert_eq!(
            version, LATEST_SCHEMA_VERSION,
            "the migration completes rather than re-running forever"
        );
        let remaining: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM settings WHERE key='app.disabled_models'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(remaining, 0);
    }

    /// Turning a model off writes the column, and `list_endpoints` then omits it.
    ///
    /// This is the behaviour the array used to fake: a disabled model is absent
    /// from the list rather than present with a flag beside it, so there is
    /// nothing for a caller to forget to filter.
    #[test]
    fn setting_a_model_disabled_removes_it_from_the_provider_list() {
        let database = migrated();
        database
            .save_endpoint(&crate::ai::remote::types::Endpoint {
                id: "kilo".into(),
                name: "Kilo".into(),
                base_url: "https://example.test".into(),
                api_key: String::new(),
                models: vec!["flash".into(), "pro".into()],
                disabled_models: vec![],
                enabled: true,
            })
            .unwrap();
        assert_eq!(database.list_endpoints().unwrap()[0].models.len(), 2);

        database.set_model_enabled("kilo", "flash", false).unwrap();

        let endpoints = database.list_endpoints().unwrap();
        assert_eq!(
            endpoints[0].models,
            vec!["pro".to_string()],
            "a disabled model is absent from the list rather than flagged"
        );

        database.set_model_enabled("kilo", "flash", true).unwrap();
        assert_eq!(database.list_endpoints().unwrap()[0].models.len(), 2);
    }

    /// Saving a provider does not switch its models back on.
    ///
    /// The provider's own `enabled` flag and a model's are different facts: the
    /// first is whether the provider is used at all, the second is whether one
    /// model appears in the pickers. Writing the first over the second made every
    /// save re-enable whatever the user had turned off.
    #[test]
    fn saving_a_provider_leaves_a_switched_off_model_off() {
        let database = migrated();
        let endpoint = crate::ai::remote::types::Endpoint {
            id: "kilo".into(),
            name: "Kilo".into(),
            base_url: "https://example.test".into(),
            api_key: String::new(),
            models: vec!["flash".into(), "pro".into()],
            disabled_models: vec![],
            enabled: true,
        };
        database.save_endpoint(&endpoint).unwrap();
        database.set_model_enabled("kilo", "flash", false).unwrap();

        // Renaming and re-saving, which is what editing the provider does.
        database
            .save_endpoint(&crate::ai::remote::types::Endpoint {
                name: "Kilo Renamed".into(),
                ..endpoint.clone()
            })
            .unwrap();

        let listed = database.list_endpoints().unwrap();
        assert_eq!(listed[0].models, vec!["pro".to_string()]);
        assert_eq!(listed[0].disabled_models, vec!["flash".to_string()]);
    }

    /// A disabled model is still reported, so Settings can offer it back.
    ///
    /// Filtering it out of the only list would make it permanently unreachable:
    /// there would be no row to re-enable from, and no error saying so.
    #[test]
    fn a_disabled_model_is_reported_separately_rather_than_dropped() {
        let database = migrated();
        database
            .save_endpoint(&crate::ai::remote::types::Endpoint {
                id: "kilo".into(),
                name: "Kilo".into(),
                base_url: "https://example.test".into(),
                api_key: String::new(),
                models: vec!["flash".into(), "pro".into()],
                disabled_models: vec![],
                enabled: true,
            })
            .unwrap();
        database.set_model_enabled("kilo", "flash", false).unwrap();

        let endpoint = database.list_endpoints().unwrap().remove(0);
        assert_eq!(endpoint.models, vec!["pro".to_string()]);
        assert_eq!(endpoint.disabled_models, vec!["flash".to_string()]);
    }

    /// A conversation's mode and access level come back on the header, so
    /// reopening it restores the mode it was worked in.
    ///
    /// Both live in the session metadata rather than in component state: a mode
    /// that resets to Chat on every switch makes Agent mode something to re-pick
    /// for each conversation rather than something chosen once.
    #[test]
    fn a_session_keeps_the_mode_and_permission_it_was_worked_in() {
        let database = migrated();
        database
            .save_chat_session(&serde_json::json!({
                "id": "s1",
                "title": "Agent work",
                "createdAt": "2026-01-01T00:00:00.000Z",
                "updatedAt": "2026-01-01T00:00:00.000Z",
                "mode": "agent",
                "permission": "auto_safe",
                "messages": []
            }))
            .unwrap();

        let header = &database.list_session_headers().unwrap()[0];
        assert_eq!(header["mode"], "agent");
        assert_eq!(header["permission"], "auto_safe");
    }

    /// A conversation saved before either existed reads as Chat with no level of
    /// its own, rather than failing or inventing a third state.
    #[test]
    fn a_session_without_a_stored_mode_reads_as_chat() {
        let database = migrated();
        database
            .save_chat_session(&serde_json::json!({
                "id": "s1",
                "title": "Older",
                "createdAt": "2026-01-01T00:00:00.000Z",
                "updatedAt": "2026-01-01T00:00:00.000Z",
                "messages": []
            }))
            .unwrap();

        let header = &database.list_session_headers().unwrap()[0];
        assert_eq!(header["mode"], "chat");
        assert!(
            header["permission"].is_null(),
            "no level is asserted, so the frontend applies its own default"
        );
    }

    /// Changing the mode writes a header without touching the transcript.
    ///
    /// The guard that refuses an empty message array is what makes this worth
    /// stating: a mode change that went through the message path would have had
    /// to send a length, and the only one available was zero.
    #[test]
    fn changing_the_mode_leaves_the_transcript_alone() {
        let database = migrated();
        database
            .save_chat_session(&serde_json::json!({
                "id": "s1",
                "title": "Work",
                "createdAt": "2026-01-01T00:00:00.000Z",
                "updatedAt": "2026-01-01T00:00:00.000Z",
                "messages": [{
                    "role": "user",
                    "content": "hello",
                    "createdAt": "2026-01-01T00:00:00.000Z"
                }]
            }))
            .unwrap();

        // A header write: no `messages` key at all.
        database
            .save_chat_session(&serde_json::json!({
                "id": "s1",
                "title": "Work",
                "createdAt": "2026-01-01T00:00:00.000Z",
                "updatedAt": "2026-01-01T00:00:01.000Z",
                "mode": "agent",
                "permission": "full"
            }))
            .unwrap();

        let messages = database.list_session_messages("s1").unwrap();
        assert_eq!(messages.len(), 1, "the message survives the mode change");
        assert_eq!(messages[0]["content"], "hello");
        assert_eq!(database.list_session_headers().unwrap()[0]["mode"], "agent");
    }
}

fn put_setting_tx<T: Serialize>(
    tx: &rusqlite::Transaction<'_>,
    key: &str,
    value: &T,
) -> Result<(), String> {
    tx.execute(
        "INSERT OR REPLACE INTO settings (key,value) VALUES (?1,?2)",
        params![key, json(value)?],
    )
    .map_err(|error| error.to_string())?;
    Ok(())
}
