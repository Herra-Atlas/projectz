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
/// **Few and good on purpose.** The whole catalogue rides in every Agent request
/// as the list the model picks from, so each extra entry both spends tokens and
/// dilutes attention on the ones that matter. These are the kinds of work the
/// model is asked for most often, and each body states the method rather than the
/// obvious -- a skill that only says "write good code" is a skill that costs
/// tokens to say nothing.
///
/// **A trigger, not a summary.** `description` is the only text the model sees
/// before deciding to read a body, so it must answer "when does this apply",
/// which is why every one of them starts "Use when".
///
/// **Two of a kind where the work differs.** `bundled.code` and
/// `bundled.debugging` both touch a codebase, but writing a feature and finding a
/// fault are different methods, and separating them lets the model read the one
/// that fits instead of a compromise covering both.
pub const BUNDLED: &[BundledSkill] = &[
    BundledSkill {
        id: "bundled.code",
        name: "Code changes",
        skill_type: "code",
        description: "Use when writing new code or editing, refactoring or removing existing code in this workspace.",
        instructions: "Read before you write. Open the file and its neighbours, and match the \
conventions already there -- naming, formatting, file layout, error handling -- rather than \
importing a style of your own. The goal is a change that reads as though it was always meant to be \
there.\n\n\
Make the smallest change that does the job. Do not reformat, rename or restructure anything the \
task did not ask for, and leave no commented-out code, dead branches or scaffolding behind. Prefer \
editing an existing function to adding a second that nearly duplicates it, and prefer deleting code \
to adding it.\n\n\
Keep the shape of the codebase: one concern per file, a function that does one thing, a name that \
says what it holds. If the change you are making really wants to touch more than the task \
described, say so and stop rather than expanding the work on your own.\n\n\
Handle the cases that can actually occur -- empty input, a missing file, a failed request, a \
malformed response -- at the boundary where the data enters, and let the code inside trust its own \
invariants. Do not add a guard for a case that cannot happen; it is noise that hides the real ones.\n\n\
After changing code, run the project's own checks: formatter, type check, tests. Fix what they \
report, and never claim something works without the run that shows it.",
    },
    BundledSkill {
        id: "bundled.code-review",
        name: "Code review",
        skill_type: "code",
        description: "Use when reviewing a diff, a pull request or existing code for problems.",
        instructions: "Review for correctness first, then clarity, then style, and report a finding \
only when you can name the input or the condition that makes it wrong. Do not pad a review with \
preferences: a reviewer who lists favourites teaches the reader to skim.\n\n\
Work through, in order: does it do what it claims; are the edge cases handled (empty, one, many, \
missing, concurrent, malformed); can it fail silently; is there a simpler way to say the same \
thing; and does it match the code around it.\n\n\
For each finding, name the file and the line, state the problem in one sentence, and give the \
smallest fix that resolves it. Separate what is blocking from what is worth fixing from what is \
optional, and say which is which, so the author knows what to do now and what to do later.\n\n\
If you find nothing, say so plainly. An invented finding to fill the space is worse than a short \
review, because it costs the author time to disprove.",
    },
    BundledSkill {
        id: "bundled.debugging",
        name: "Debugging",
        skill_type: "code",
        description: "Use when something is broken, failing, or behaving differently from what was expected.",
        instructions: "Find the cause before changing anything. Read the error, then the code that \
produced it, then the code that called that. A fix applied before the cause is known is a guess \
that hides the next failure rather than removing this one.\n\n\
Reproduce it first. State the exact input or the exact step that makes it happen, and run it. A bug \
you cannot make happen on demand is a bug you cannot confirm you have fixed.\n\n\
Narrow by halving. Disable, comment out or revert half of what is involved, see which half still \
fails, and repeat on that half. Change one thing at a time, and say what you expect each change to \
do, so a wrong expectation becomes visible instead of silent.\n\n\
When you find it, fix the cause and not the symptom. Then look for the same mistake elsewhere -- \
the same call, the same assumption, a sibling file -- and add a test that fails before the fix and \
passes after. Finish by stating in a sentence what was actually wrong, because the next person will \
read it before they read the diff.",
    },
    BundledSkill {
        id: "bundled.testing",
        name: "Testing",
        skill_type: "code",
        description: "Use when writing or changing tests, or when deciding how behaviour should be verified.",
        instructions: "Test behaviour, not implementation. A test that breaks when a private function \
is renamed is a test that will be deleted the first time it is inconvenient, taking its coverage \
with it. Assert on what the caller can see.\n\n\
Name each test for the case it covers, and keep each one small: one behaviour, arranged so the \
reader sees the input and the expected result without scrolling. A test whose name does not say \
what it proves is a test nobody will maintain.\n\n\
Cover the boundaries first -- empty, one, many, missing, malformed, repeated, concurrent -- because \
that is where the faults live. Then the happy path. A test that only checks the happy path proves \
the code runs, not that it is right.\n\n\
Run the suite before and after your change. A new test must fail against the old code and pass \
against the new; if it passes both, it does not test the change. Prefer a real dependency to a \
mock of one, and when a mock is unavoidable, make it fail the same way the real thing does.",
    },
    BundledSkill {
        id: "bundled.commits",
        name: "Commits",
        skill_type: "code",
        description: "Use when writing a commit message, or describing a change to be committed.",
        instructions: "Write the subject as what the change does, in the imperative, and keep it under \
about seventy characters: \"Fix cache invalidation on write\", not \"Fixed\" or \"fixing stuff\". A \
reader scanning a log should be able to tell one change from another by its subject alone.\n\n\
Use the body for the why, never the what -- the diff already shows what changed. Explain the \
problem that existed and the decision that resolved it, name the alternative you rejected and why, \
and write down anything a future reader would otherwise have to rediscover from the code.\n\n\
Keep one change per commit. If the body needs the word \"also\", it is probably two commits. Do not \
list the files, and do not restate the diff in prose.\n\n\
Match the repository's existing convention for format, tense, scope and trailers before inventing \
one of your own.",
    },
    BundledSkill {
        id: "bundled.research",
        name: "Research",
        skill_type: "research",
        description: "Use when a question needs facts or sources from outside the workspace.",
        instructions: "Answer from sources rather than memory whenever the answer can be checked. \
Search first, then open the pages that look authoritative and read them rather than trusting a \
snippet. A snippet is chosen for relevance to a query, not for what it proves.\n\n\
Cross-check a fact against a second source before stating it as settled, and say when sources \
disagree instead of picking one silently. Prefer primary sources -- the documentation, the paper, \
the project's own repository -- over summaries of them, because a summary is someone else's \
compression of exactly the details you need.\n\n\
Separate what you verified from what you are inferring, and name the source for each non-obvious \
claim. When you cannot confirm something, say what you could not confirm and what it would take to \
find out, rather than filling the gap with the most plausible answer.",
    },
    BundledSkill {
        id: "bundled.writing",
        name: "Writing",
        skill_type: "writing",
        description:
            "Use when drafting or editing prose: documentation, articles, messages, release notes.",
        instructions: "Write plainly and directly. Prefer the short word, the active voice and the \
concrete noun, and cut any sentence that does not add meaning. If a sentence can be deleted and the \
paragraph still makes sense, delete it.\n\n\
Lead with the point and put the detail after it. One idea per paragraph. Use a list only when the \
items are genuinely parallel, not as a way to avoid writing the connecting sentences.\n\n\
Match the register the reader expects: terse for reference and release notes, warmer for a message \
to a person, precise for a specification. Say what you mean in terms the reader already knows, and \
define a term the first time you use it rather than the third.\n\n\
Do not open by restating the request, and do not close by offering to help. Start at the first \
thing worth reading and stop when you have said it.",
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
        assert_eq!(
            database.enabled_skills().expect("after").len(),
            BUNDLED.len()
        );

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
            assert!(
                skill.id.starts_with("bundled."),
                "unstable id: {}",
                skill.id
            );
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
