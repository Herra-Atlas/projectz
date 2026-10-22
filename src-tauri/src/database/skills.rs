//! The application's half of a skill: whether it is offered, whether its scripts
//! may run, and how often it has been used.
//!
//! # Why this is not the skill any more
//!
//! A skill is now a folder on disk -- `SKILL.md` plus optional `scripts/`,
//! `references/` and `assets/` -- in the open Agent Skills layout. That is the
//! only shape that can carry a script, and it is what makes a skill portable
//! between this app and the tools that use the same format. The content lives in
//! [`crate::skills`]; this table holds the three facts that are *about* the
//! folder rather than *in* it.
//!
//! Keyed by slug, which the spec also requires the frontmatter `name` to be, so
//! the row and the folder cannot disagree about identity.
//!
//! # Why `trusted` is separate from `enabled`
//!
//! Offering a skill means the model may read its instructions. Running its
//! scripts is a different claim: a script is arbitrary code executed with the
//! user's privileges. A skill can be perfectly useful with its body and its
//! references while `scripts/` stays inert, so the two switches answer the two
//! questions a user actually asks.

use rusqlite::{params, OptionalExtension};

use crate::database::Database;

/// per-skill state the app owns.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct SkillState {
    pub slug: String,
    pub enabled: bool,
    /// Whether `scripts/` may be executed. Defaults to false.
    pub trusted: bool,
    pub use_count: i64,
    pub created_at: String,
    pub updated_at: String,
}

/// One row from the table skills used to live in.
///
/// Read once by the folder migration and never again: the table is left in place
/// as a backup, but nothing after the migration reads or writes it.
pub struct LegacySkillRow {
    pub name: String,
    pub description: Option<String>,
    pub instructions: String,
    pub skill_type: Option<String>,
    pub origin: String,
    pub enabled: bool,
    pub use_count: i64,
    pub created_at: String,
}

impl Database {
    /// The old row-backed skills, for the one-time move to folders.
    pub fn legacy_skill_rows(&self) -> Result<Vec<LegacySkillRow>, String> {
        let connection = self.connection.lock().map_err(|error| error.to_string())?;
        let mut statement = connection
            .prepare("SELECT name,description,instructions,type,origin,enabled,use_count,created_at FROM skills ORDER BY name COLLATE NOCASE")
            .map_err(|error| error.to_string())?;
        let rows = statement
            .query_map([], |row| {
                Ok(LegacySkillRow {
                    name: row.get(0)?,
                    description: row.get(1)?,
                    instructions: row.get(2)?,
                    skill_type: row.get(3)?,
                    origin: row.get(4)?,
                    enabled: row.get::<_, i64>(5)? != 0,
                    use_count: row.get(6)?,
                    created_at: row.get(7)?,
                })
            })
            .map_err(|error| error.to_string())?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|error| error.to_string())
    }

    /// Writes a state row with an old skill's history, for the folder migration.
    ///
    /// An upsert rather than an insert: a folder that somehow already has a row
    /// keeps it, because a state the user set is newer than the row being
    /// imported.
    pub fn import_skill_state(
        &self,
        slug: &str,
        enabled: bool,
        use_count: i64,
        created_at: &str,
    ) -> Result<(), String> {
        self.connection
            .lock()
            .map_err(|error| error.to_string())?
            .execute(
                "INSERT OR IGNORE INTO skill_state (slug,enabled,trusted,use_count,created_at,updated_at)
                 VALUES (?1,?2,0,?3,?4,?4)",
                params![slug, enabled, use_count.max(0), created_at],
            )
            .map_err(|error| error.to_string())?;
        Ok(())
    }

    /// Every state row, in slug order.
    ///
    /// Read once per listing so a page of skills is one query plus one directory
    /// scan, not one query per skill.
    pub fn skill_states(&self) -> Result<Vec<SkillState>, String> {
        let connection = self.connection.lock().map_err(|error| error.to_string())?;
        let mut statement = connection
            .prepare("SELECT slug,enabled,trusted,use_count,created_at,updated_at FROM skill_state ORDER BY slug COLLATE NOCASE")
            .map_err(|error| error.to_string())?;
        let rows = statement
            .query_map([], state_from_row)
            .map_err(|error| error.to_string())?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|error| error.to_string())
    }

    /// One state row, or `None` when the skill has never been seen.
    ///
    /// A folder that exists without a row is normal -- a skill copied in by hand
    /// or installed before this table existed -- and reads as enabled and
    /// untrusted rather than being refused. See [`crate::skills::list`].
    pub fn skill_state(&self, slug: &str) -> Result<Option<SkillState>, String> {
        let connection = self.connection.lock().map_err(|error| error.to_string())?;
        connection
            .query_row(
                "SELECT slug,enabled,trusted,use_count,created_at,updated_at FROM skill_state WHERE slug=?1",
                [slug],
                state_from_row,
            )
            .optional()
            .map_err(|error| error.to_string())
    }

    /// Creates the row a newly seen folder needs, keeping any history it has.
    ///
    /// `INSERT OR IGNORE` rather than an upsert: a skill that is re-seeded or
    /// re-installed must not reset a switch the user set on it.
    pub fn ensure_skill_state(&self, slug: &str) -> Result<SkillState, String> {
        let now = crate::database::utc_now();
        self.connection
            .lock()
            .map_err(|error| error.to_string())?
            .execute(
                "INSERT OR IGNORE INTO skill_state (slug,enabled,trusted,use_count,created_at,updated_at)
                 VALUES (?1,1,0,0,?2,?2)",
                params![slug, now],
            )
            .map_err(|error| error.to_string())?;
        self.skill_state(slug)?
            .ok_or_else(|| format!("Skill {slug} vanished immediately after being recorded"))
    }

    pub fn set_skill_enabled(&self, slug: &str, enabled: bool) -> Result<(), String> {
        let changed = self
            .connection
            .lock()
            .map_err(|error| error.to_string())?
            .execute(
                "UPDATE skill_state SET enabled=?2, updated_at=?3 WHERE slug=?1",
                params![slug, enabled, crate::database::utc_now()],
            )
            .map_err(|error| error.to_string())?;
        if changed == 0 {
            // A folder present without a row is an ordinary state, so the switch
            // records the row rather than reporting a missing skill.
            self.ensure_skill_state(slug)?;
            return self.set_skill_enabled(slug, enabled);
        }
        Ok(())
    }

    /// Allows or refuses `scripts/` for one skill.
    pub fn set_skill_trusted(&self, slug: &str, trusted: bool) -> Result<(), String> {
        let changed = self
            .connection
            .lock()
            .map_err(|error| error.to_string())?
            .execute(
                "UPDATE skill_state SET trusted=?2, updated_at=?3 WHERE slug=?1",
                params![slug, trusted, crate::database::utc_now()],
            )
            .map_err(|error| error.to_string())?;
        if changed == 0 {
            self.ensure_skill_state(slug)?;
            return self.set_skill_trusted(slug, trusted);
        }
        Ok(())
    }

    /// Counts a read. Silently ignores a skill that no longer exists: this runs
    /// on the reply path, and failing a reply over a usage counter would be
    /// absurd.
    pub fn record_skill_use(&self, slug: &str) {
        if let Ok(connection) = self.connection.lock() {
            let _ = connection.execute(
                "UPDATE skill_state SET use_count=use_count+1 WHERE slug=?1",
                [slug],
            );
        }
    }

    /// Forgets a deleted skill's state.
    ///
    /// Not an error when the row is absent: deleting a bundled skill may be the
    /// first thing that touches it, and the folder is about to be removed either
    /// way.
    pub fn delete_skill_state(&self, slug: &str) -> Result<(), String> {
        self.connection
            .lock()
            .map_err(|error| error.to_string())?
            .execute("DELETE FROM skill_state WHERE slug=?1", [slug])
            .map_err(|error| error.to_string())?;
        Ok(())
    }
}

fn state_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<SkillState> {
    Ok(SkillState {
        slug: row.get(0)?,
        enabled: row.get::<_, i64>(1)? != 0,
        trusted: row.get::<_, i64>(2)? != 0,
        use_count: row.get(3)?,
        created_at: row.get(4)?,
        updated_at: row.get(5)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn database() -> Database {
        Database::in_memory().expect("db")
    }

    /// A row is created on demand, enabled and untrusted. Untrusted is the
    /// deliberate default: writing a skill is not permission to execute one.
    #[test]
    fn a_new_row_is_enabled_and_untrusted() {
        let state = database().ensure_skill_state("code-review").expect("state");
        assert!(state.enabled);
        assert!(!state.trusted);
        assert_eq!(state.use_count, 0);
    }

    /// Re-seeding must not reset a user's switches or their usage history.
    #[test]
    fn ensuring_an_existing_row_keeps_its_history() {
        let database = database();
        database.ensure_skill_state("code-review").expect("first");
        database
            .set_skill_enabled("code-review", false)
            .expect("off");
        database
            .set_skill_trusted("code-review", true)
            .expect("trust");
        database.record_skill_use("code-review");

        let state = database.ensure_skill_state("code-review").expect("again");
        assert!(!state.enabled);
        assert!(state.trusted);
        assert_eq!(state.use_count, 1);
    }

    /// A switch flipped before the row exists records the row instead of
    /// reporting a skill that visibly exists on disk as missing.
    #[test]
    fn a_switch_creates_the_row_it_needs() {
        let database = database();
        database
            .set_skill_enabled("hand-copied", false)
            .expect("off");
        assert!(
            !database
                .skill_state("hand-copied")
                .expect("read")
                .expect("row")
                .enabled
        );
    }

    #[test]
    fn uses_accumulate_and_deleting_forgets_them() {
        let database = database();
        database.ensure_skill_state("x").expect("row");
        for _ in 0..3 {
            database.record_skill_use("x");
        }
        assert_eq!(
            database
                .skill_state("x")
                .expect("read")
                .expect("row")
                .use_count,
            3
        );

        database.delete_skill_state("x").expect("delete");
        assert!(database.skill_state("x").expect("read").is_none());
    }

    /// Counting a use of a skill deleted mid-reply is silent, because the reply
    /// path must not fail over a counter.
    #[test]
    fn counting_a_use_of_a_deleted_skill_is_silent() {
        database().record_skill_use("never-existed");
    }
}
