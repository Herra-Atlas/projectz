//! The skills the app ships with.
//!
//! # What a bundled skill is
//!
//! An ordinary row. It is written into the same `skills` table, shown on the
//! same Skills screen and read by the agent with `skill_read` exactly like one
//! the user wrote; nothing downstream has to know where it came from, and only
//! `origin` records it so the screen can label it.
//!
//! # Why once, behind a flag, and not a migration
//!
//! The seed is idempotent by id -- a stable `bundled.*` id and `INSERT OR IGNORE`
//! -- but it runs **once**, and a setting records that it did. Deleting a skill
//! the app shipped is therefore final, the same as deleting one the user wrote; a
//! re-seed on every launch would keep handing back a row they removed.
//!
//! It is deliberately not a schema migration. The table's shape does not change
//! and no existing row moves, and a migration would make every unit test that
//! builds an in-memory database start with these rows in it -- a cost paid by
//! tests that have nothing to do with the feature.

use rusqlite::{params, Connection};

use crate::database::Database;

/// One skill the app ships.
pub struct BundledSkill {
    /// Stable id, so running the seed again can never insert a second copy.
    pub id: &'static str,
    pub name: &'static str,
    /// The grouping shown on the Skills screen and in the model's catalogue.
    pub skill_type: &'static str,
    /// A trigger -- *when* to read this -- rather than a summary of what is
    /// inside, because this is the only text the model chooses from.
    pub description: &'static str,
    pub instructions: &'static str,
}

/// The shipped catalogue.
///
/// Few and good on purpose: the whole catalogue rides in every Agent request as
/// the list the model picks from, so each extra entry both spends tokens and
/// dilutes attention on the ones that matter.
pub const BUNDLED: &[BundledSkill] = &[
    BundledSkill {
        id: "bundled.code",
        name: "Code changes",
        skill_type: "code",
        description: "Use when writing, editing, refactoring or debugging code in this workspace.",
        instructions: "Follow the conventions already in the code: match the surrounding naming, \
formatting and file layout rather than importing a style of your own, and read a file and its \
neighbours before changing it.\n\n\
Make the smallest change that does the job. Do not reformat, rename or restructure anything the \
task did not ask for, and leave no commented-out code or scaffolding behind.\n\n\
After changing code, run the project's own checks -- formatter, type check, tests -- and fix what \
they report. Do not claim something works without evidence.\n\n\
Prefer editing an existing function to adding a second one that does nearly the same thing, and \
prefer deleting code to adding it.",
    },
    BundledSkill {
        id: "bundled.code-review",
        name: "Code review",
        skill_type: "code",
        description: "Use when reviewing a diff, pull request or existing code for problems.",
        instructions: "Review for correctness first, then clarity, then style, and report a \
finding only when you can name the input or condition that makes it wrong -- do not pad the \
review with preferences.\n\n\
Check, in order: does it do what it claims; are the edge cases handled (empty, one, many, \
missing, concurrent); can it fail silently; is there a simpler way; does it match the code around \
it.\n\n\
Name the file and line, state the problem, and give the smallest fix. Separate what is blocking \
from what is worth fixing from what is optional. If you find nothing, say so plainly.",
    },
    BundledSkill {
        id: "bundled.research",
        name: "Research",
        skill_type: "research",
        description: "Use when a question needs facts or sources from outside the workspace.",
        instructions: "Answer from sources rather than memory whenever the answer can be \
checked. Search first, then open the pages that look authoritative and read them rather than \
trusting a snippet.\n\n\
Cross-check a fact against a second source before stating it as settled, and say when sources \
disagree. Prefer primary sources -- documentation, papers, the project's own repository -- over \
summaries of them.\n\n\
Separate what you verified from what you are inferring. Name the source for each non-obvious \
claim, and say when you could not confirm something rather than filling the gap.",
    },
    BundledSkill {
        id: "bundled.writing",
        name: "Writing",
        skill_type: "writing",
        description:
            "Use when drafting or editing prose: documentation, articles, messages, release notes.",
        instructions: "Write plainly and directly. Prefer the short word, the active voice and \
the concrete noun, and cut any sentence that does not add meaning.\n\n\
Lead with the point and put the detail after it. One idea per paragraph, and a list only when the \
items are genuinely parallel.\n\n\
Match the register the reader expects: terse for reference and release notes, warmer for a \
message to a person. Do not open by restating the request or close by offering help.",
    },
];

/// The setting that records the seed has run.
///
/// Read before the insert and written after it. A database that has never been
/// seeded has no value here, which is exactly the case that seeds.
const SEEDED_KEY: &str = "app.bundled_skills_seeded";

impl Database {
    /// Writes the shipped skills, once ever.
    ///
    /// Safe to call on every startup: the flag short-circuits it, and the inserts
    /// ignore an id that is already there. A failure is reported rather than
    /// swallowed, because a database that cannot take these rows is a database
    /// with a problem worth surfacing.
    pub fn seed_bundled_skills(&self) -> Result<(), String> {
        if self.setting::<bool>(SEEDED_KEY).unwrap_or(false) {
            return Ok(());
        }
        let now = crate::database::utc_now();
        {
            let connection = self.connection.lock().map_err(|error| error.to_string())?;
            insert_all(&connection, &now)?;
        }
        // Written last, so a failure part-way through the insert leaves the flag
        // unset and the seed runs again next launch rather than half-done forever.
        self.set_setting(SEEDED_KEY, &true)
    }
}

/// Inserts every bundled skill, ignoring any id already present.
fn insert_all(connection: &Connection, now: &str) -> Result<(), String> {
    for skill in BUNDLED {
        connection
            .execute(
                "INSERT OR IGNORE INTO skills
                    (id,name,description,instructions,type,origin,enabled,use_count,created_at,updated_at)
                 VALUES (?1,?2,?3,?4,?5,'bundled',1,0,?6,?6)",
                params![
                    skill.id,
                    skill.name,
                    skill.description,
                    skill.instructions,
                    skill.skill_type,
                    now,
                ],
            )
            .map_err(|error| error.to_string())?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seeding_adds_every_bundled_skill_and_only_once() {
        let database = Database::in_memory().expect("db");
        assert!(
            database.enabled_skills().expect("before").is_empty(),
            "an unseeded database should have no skills"
        );

        database.seed_bundled_skills().expect("seed");
        assert_eq!(database.enabled_skills().expect("after").len(), BUNDLED.len());

        // A second call is a no-op -- the flag short-circuits it, and the stable
        // ids would ignore the rows even without it.
        database.seed_bundled_skills().expect("seed again");
        assert_eq!(
            database.enabled_skills().expect("after twice").len(),
            BUNDLED.len()
        );
    }

    /// Deleting a shipped skill is final, which is the whole reason the seed
    /// runs once rather than being re-applied on every launch.
    #[test]
    fn a_deleted_bundled_skill_does_not_come_back() {
        let database = Database::in_memory().expect("db");
        database.seed_bundled_skills().expect("seed");
        let first = database
            .enabled_skills()
            .expect("after")
            .into_iter()
            .next()
            .expect("a seeded skill");

        database.delete_skill(&first.id).expect("delete");
        database.seed_bundled_skills().expect("seed again");

        assert!(database.skill(&first.id).expect("read").is_none());
    }

    /// The catalogue is data, so its shape is asserted: a stable, namespaced id, a
    /// type to group by, and a description written as a trigger rather than a
    /// summary. A summary here is the one thing that makes the model skip a skill
    /// it should have read.
    #[test]
    fn every_bundled_skill_is_valid_and_written_as_a_trigger() {
        assert!(!BUNDLED.is_empty(), "the app ships no skills");
        for skill in BUNDLED {
            assert!(skill.id.starts_with("bundled."), "unstable id: {}", skill.id);
            assert!(!skill.name.trim().is_empty(), "{}", skill.id);
            assert!(!skill.skill_type.trim().is_empty(), "{}", skill.id);
            assert!(
                skill.description.to_lowercase().starts_with("use when"),
                "{} does not read as a trigger: {}",
                skill.id,
                skill.description
            );
            // The table's own limit, checked here so an over-long seed fails a
            // test rather than becoming a row validation would refuse.
            assert!(skill.instructions.chars().count() <= 8_000, "{}", skill.id);
        }
    }
}
