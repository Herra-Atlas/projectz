//! Bringing the shipped skills up to date on a database that already has them.
//!
//! # The problem this solves
//!
//! A bundled skill is a row, seeded once behind a flag, so with no path to bring
//! it forward, editing a body in [`super::BUNDLED`] reached only fresh installs
//! and every existing user kept the old text forever with no way to know there
//! was a new one. Worse, a skill *removed* from the catalogue stayed on their disk
//! as a row nothing ships any more -- a skill outliving the app that made it, and
//! one that could never be updated again.
//!
//! # Why this is a reinstall, and not a merge
//!
//! On a version bump the bundled set is replaced wholesale: every row the app
//! previously seeded is deleted and the current catalogue inserted in its place.
//! A merge -- update the ids you already know about -- could never remove a skill
//! whose id has left the catalogue, because it has nothing to match, so a retired
//! skill would sit there for good. Replacing the set is the only shape that makes
//! "shipped" mean the same thing as "present".
//!
//! The cost is deliberate and it is the app's choice, not an accident: a bundled
//! skill is app-owned content, so an edit to one is replaced by the next release
//! exactly the way the bodies themselves are. Anything the user wants to keep is a
//! skill of their own -- `origin='user'` -- which this never touches.
//!
//! # What survives
//!
//! Content is replaced; a row's history is not. `use_count`, `enabled` and
//! `created_at` carry across for any id still in the catalogue. Wiping the usage
//! count on every update would erase the one signal the Skills screen ranks by,
//! and switching a skill the user turned off back on would undo a choice they made
//! on purpose. A skill the user *deleted* has no row to carry from, so it returns
//! freshly counted and enabled -- reinstalled, which is the point.

use std::collections::HashMap;

use rusqlite::params;

use crate::database::Database;

use super::BUNDLED;

/// The shipped catalogue's version.
///
/// Bumped whenever the set of skills in [`super::BUNDLED`] changes, or a body,
/// name, description or type changes in a way that should reach someone who
/// already installed the app. The seed's `INSERT OR IGNORE` adds nothing on its
/// own, so both a new skill and a removed one need a bump here to take effect.
///
/// Version history, because the number is the only record of what has shipped:
///
/// - 1 -- the original seven.
/// - 2 -- the step discipline each method body was missing.
/// - 3 -- what the code family was silent about: the read that goes stale after a
///   write, the fault a diff cannot show, the guard that is not a fix, tests that
///   pass only sometimes. Plus `Research` taught to check the workspace before the
///   web and never to cite a source it did not open.
/// - 4 -- `Running and verifying` added to the code family, and the single
///   `Writing` body split into `Documentation`, `Articles` and `Messages`, which
///   also retires the old `bundled.writing` row. The refresh became a reinstall so
///   that retirement actually takes effect.
/// - 5 -- `Code review`, `Testing` and `Commits` retired. Each is narrower than
///   the rest -- a method for one step of the work rather than for a kind of work
///   -- and a user who wants one can write their own. Seven ship.
pub const BUNDLED_VERSION: u32 = 5;

/// The version an install has before any of this existed.
///
/// Any stored value below [`BUNDLED_VERSION`] runs the reinstall; a database with
/// no stored value at all reads as this, which is what makes the first bump reach
/// it.
pub(super) const FIRST_VERSION: u32 = 1;

/// The setting holding the version the catalogue was last brought up to.
pub(super) const VERSION_KEY: &str = "app.bundled_skills_version";

/// The prefix of the settings an earlier scheme wrote to record the hash of each
/// shipped body. Nothing reads them any more -- a reinstall replaces text rather
/// than comparing against it -- so the reinstall clears them instead of leaving
/// dead state behind for a future reader to puzzle over.
const SHIPPED_HASH_PREFIX: &str = "app.bundled_skill.";

/// What the previous install kept about one shipped row: when it first appeared,
/// how much it was used, and whether the user switched it off. None of these is
/// content, which is why a reinstall preserves them.
type KeptRow = (String, i64, i64);

/// Replaces every row the app seeded with the current catalogue.
///
/// Called only when the stored version is behind [`BUNDLED_VERSION`], so a normal
/// launch does no work here.
pub(super) fn reinstall(database: &Database) -> Result<(), String> {
    let now = crate::database::utc_now();
    let mut connection = database
        .connection
        .lock()
        .map_err(|error| error.to_string())?;
    // One transaction, so a failure leaves the previous set in place and the next
    // launch simply tries again -- rather than a database missing its skills
    // because the delete landed and the insert did not.
    let transaction = connection
        .transaction()
        .map_err(|error| error.to_string())?;

    // Read before the delete, because the delete is what takes the old rows away.
    let kept = kept_rows(&transaction)?;

    // The whole bundled set goes, and the catalogue takes its place. `origin` is
    // the only predicate, so a skill the user wrote is never in scope -- and a
    // shipped skill that has left the catalogue is retired by the same statement.
    transaction
        .execute("DELETE FROM skills WHERE origin='bundled'", [])
        .map_err(|error| error.to_string())?;

    transaction
        .execute(
            "DELETE FROM settings WHERE key LIKE ?1",
            params![format!("{SHIPPED_HASH_PREFIX}%")],
        )
        .map_err(|error| error.to_string())?;

    for skill in BUNDLED {
        let (created_at, use_count, enabled) = kept
            .get(skill.id)
            .cloned()
            .unwrap_or_else(|| (now.clone(), 0, 1));
        transaction
            .execute(
                "INSERT INTO skills
                    (id,name,description,instructions,type,origin,enabled,use_count,created_at,updated_at)
                 VALUES (?1,?2,?3,?4,?5,'bundled',?6,?7,?8,?9)",
                params![
                    skill.id,
                    skill.name,
                    skill.description,
                    skill.instructions,
                    skill.skill_type,
                    enabled,
                    use_count,
                    created_at,
                    now,
                ],
            )
            .map_err(|error| error.to_string())?;
    }

    transaction.commit().map_err(|error| error.to_string())
}

/// Reads the history of every shipped row: its first appearance, its usage count
/// and whether it is switched off, keyed by id.
fn kept_rows(connection: &rusqlite::Connection) -> Result<HashMap<String, KeptRow>, String> {
    let mut statement = connection
        .prepare("SELECT id, created_at, use_count, enabled FROM skills WHERE origin='bundled'")
        .map_err(|error| error.to_string())?;
    let rows = statement
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                (
                    row.get::<_, String>(1)?,
                    row.get::<_, i64>(2)?,
                    row.get::<_, i64>(3)?,
                ),
            ))
        })
        .map_err(|error| error.to_string())?;

    let mut kept = HashMap::new();
    for row in rows {
        let (id, values) = row.map_err(|error| error.to_string())?;
        kept.insert(id, values);
    }
    Ok(kept)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::database::bundled_skills::BUNDLED;
    use crate::database::skills::{NewSkill, SkillOrigin};

    /// A seeded database pretending to be an install from the previous version:
    /// every bundled body replaced with old text and the version wound back, which
    /// is all the reinstall looks at.
    fn as_previous_version(database: &Database) {
        database.seed_bundled_skills().expect("seed");
        for skill in BUNDLED {
            database
                .update_skill(
                    skill.id,
                    &NewSkill {
                        name: skill.name.into(),
                        description: Some(skill.description.into()),
                        instructions: "The wording that shipped last time.".into(),
                        skill_type: Some(skill.skill_type.into()),
                        origin: SkillOrigin::Bundled,
                    },
                )
                .expect("older body");
        }
        database
            .set_setting(VERSION_KEY, &FIRST_VERSION)
            .expect("as an older install");
    }

    /// The bug this whole file exists for: an existing install must receive the
    /// current bodies.
    #[test]
    fn a_new_body_reaches_an_existing_install() {
        let database = Database::in_memory().expect("db");
        as_previous_version(&database);

        database.seed_bundled_skills().expect("reinstall");

        for skill in BUNDLED {
            assert_eq!(
                database
                    .skill(skill.id)
                    .expect("read")
                    .expect("row")
                    .instructions,
                skill.instructions,
                "{} was not brought up to date",
                skill.id
            );
        }
    }

    /// The reason for replacing rather than merging: a skill dropped from the
    /// catalogue has to leave the database, or it outlives the app that made it.
    #[test]
    fn a_skill_dropped_from_the_catalogue_is_removed() {
        let database = Database::in_memory().expect("db");
        as_previous_version(&database);
        // A skill an earlier release shipped and this one does not -- exactly what
        // a merge keyed on ids would leave behind for ever.
        database
            .connection
            .lock()
            .expect("connection")
            .execute(
                "INSERT INTO skills
                    (id,name,description,instructions,type,origin,enabled,use_count,created_at,updated_at)
                 VALUES ('bundled.retired','Retired',NULL,'gone','code','bundled',1,0,
                         '2020-01-01T00:00:00Z','2020-01-01T00:00:00Z')",
                [],
            )
            .expect("a stale bundled row");

        database.seed_bundled_skills().expect("reinstall");

        assert!(database.skill("bundled.retired").expect("read").is_none());
    }

    /// A bundled edit does not survive a version bump: the app owns these rows, so
    /// the next release replaces the text exactly as it replaces the bodies.
    #[test]
    fn an_edited_bundled_body_is_replaced() {
        let database = Database::in_memory().expect("db");
        as_previous_version(&database);
        let target = BUNDLED.first().expect("a shipped skill");
        database
            .update_skill(
                target.id,
                &NewSkill {
                    name: target.name.into(),
                    description: Some(target.description.into()),
                    instructions: "Rewritten by the user.".into(),
                    skill_type: Some(target.skill_type.into()),
                    origin: SkillOrigin::Bundled,
                },
            )
            .expect("the user's edit");

        database.seed_bundled_skills().expect("reinstall");

        assert_eq!(
            database
                .skill(target.id)
                .expect("read")
                .expect("row")
                .instructions,
            target.instructions
        );
    }

    /// A skill the user wrote is never in scope, whatever the reinstall does.
    #[test]
    fn a_user_skill_is_never_touched() {
        let database = Database::in_memory().expect("db");
        as_previous_version(&database);
        let mine = database
            .create_skill(&NewSkill {
                name: "My own method".into(),
                description: None,
                instructions: "Mine.".into(),
                skill_type: None,
                origin: SkillOrigin::User,
            })
            .expect("a user skill");

        database.seed_bundled_skills().expect("reinstall");

        let stored = database.skill(&mine.id).expect("read").expect("row");
        assert_eq!(stored.instructions, "Mine.");
        assert_eq!(stored.origin, SkillOrigin::User);
    }

    /// Usage, the off switch and the first-appearance date are history, not
    /// content: a reinstall replaces the text and leaves these alone.
    #[test]
    fn the_reinstall_keeps_usage_and_the_off_switch() {
        let database = Database::in_memory().expect("db");
        as_previous_version(&database);
        let target = BUNDLED.first().expect("a shipped skill");
        database.record_skill_use(target.id);
        database.record_skill_use(target.id);
        database
            .set_skill_enabled(target.id, false)
            .expect("turn off");
        let created = database
            .skill(target.id)
            .expect("read")
            .expect("row")
            .created_at;

        database.seed_bundled_skills().expect("reinstall");

        let stored = database.skill(target.id).expect("read").expect("row");
        assert_eq!(stored.use_count, 2, "the usage count was reset");
        assert!(!stored.enabled, "a skill the user turned off came back on");
        assert_eq!(
            stored.created_at, created,
            "the row lost the date it first appeared"
        );
        assert_eq!(stored.instructions, target.instructions);
    }

    /// A skill the user deleted comes back on the next version bump: a bundled
    /// skill is the app's, and an update restores the set it ships.
    #[test]
    fn the_reinstall_restores_a_deleted_skill() {
        let database = Database::in_memory().expect("db");
        as_previous_version(&database);
        let target = BUNDLED.first().expect("a shipped skill");
        database.delete_skill(target.id).expect("delete");

        database.seed_bundled_skills().expect("reinstall");

        assert_eq!(
            database
                .skill(target.id)
                .expect("read")
                .expect("row")
                .instructions,
            target.instructions
        );
    }

    /// The reinstall runs once, not on every launch.
    #[test]
    fn the_version_is_recorded_so_the_reinstall_does_not_run_again() {
        let database = Database::in_memory().expect("db");
        as_previous_version(&database);
        database.seed_bundled_skills().expect("reinstall");
        assert_eq!(
            database.setting::<u32>(VERSION_KEY),
            Some(BUNDLED_VERSION),
            "the version must be recorded, or the pass runs on every launch"
        );

        // And an edit made afterwards stays put, which is the state the version
        // check would otherwise keep re-imposing.
        let target = BUNDLED.first().expect("a shipped skill");
        database
            .update_skill(
                target.id,
                &NewSkill {
                    name: target.name.into(),
                    description: Some(target.description.into()),
                    instructions: "Mine now.".into(),
                    skill_type: Some(target.skill_type.into()),
                    origin: SkillOrigin::Bundled,
                },
            )
            .expect("edit");
        database.seed_bundled_skills().expect("again");
        assert_eq!(
            database
                .skill(target.id)
                .expect("read")
                .expect("row")
                .instructions,
            "Mine now."
        );
    }

    /// The records the old scheme wrote are cleared, so nothing is left claiming
    /// to describe the shipped text.
    #[test]
    fn the_retired_hash_records_are_cleared() {
        let database = Database::in_memory().expect("db");
        as_previous_version(&database);
        let leftover = "a-hash-from-the-old-scheme".to_string();
        database
            .set_setting("app.bundled_skill.bundled.code", &leftover)
            .expect("a leftover record");

        database.seed_bundled_skills().expect("reinstall");

        assert_eq!(
            database.setting::<String>("app.bundled_skill.bundled.code"),
            None
        );
    }
}
