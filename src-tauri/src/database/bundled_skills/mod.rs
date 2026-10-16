//! The skills the app ships with.
//!
//! # What a bundled skill is
//!
//! An ordinary row. It is written into the same `skills` table, shown on the
//! same Skills screen and read by the agent with `skill_read` exactly like one
//! the user wrote; nothing downstream has to know where it came from, and only
//! `origin` records it so the screen can label it.
//!
//! # Why the seed runs once, and how the text is kept current anyway
//!
//! The seed is idempotent by id -- a stable `bundled.*` id and `INSERT OR IGNORE`
//! -- but it runs **once**, and a setting records that it did, so a re-seed on
//! every launch cannot keep handing back a row the user removed.
//!
//! That left the other half unsolved, and it bit: editing a body here changed
//! nothing for anyone who had already installed the app, because the flag
//! short-circuits the seed forever. [`BUNDLED_VERSION`] is what fixes it. A
//! version bump runs [`refresh`], which **reinstalls the shipped set**: every row
//! the app previously seeded is deleted and the current catalogue inserted in its
//! place. So a body edited here reaches every install, a skill dropped from
//! [`BUNDLED`] disappears instead of lingering as a row nothing ships any more,
//! and a new one appears -- none of which a merge keyed on ids could do, since a
//! merge can only see the ids it already knows about.
//!
//! It is deliberately not a schema migration. The table's shape does not change
//! and no existing row moves, and a migration would make every unit test that
//! builds an in-memory database start with these rows in it -- a cost paid by
//! tests that have nothing to do with the feature.

mod refresh;

use rusqlite::{params, Connection};

use crate::database::Database;

pub use refresh::BUNDLED_VERSION;

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
/// **A method, and the step discipline that goes with it.** Every body that
/// involves looking around a codebase says what *not* to re-do: read once, batch
/// what is independent, and stop when the work is verified. The agent is spending
/// real time per round, and the difference between a good method and a good method
/// run three times is the user's afternoon.
///
/// **Written for a general-purpose app.** A reply may be a summary, a piece of
/// research, a document or a personal task, so what ships here is a method per
/// *kind of work* rather than rules about code. That is the whole reason these
/// exist: the Agent prompt has to stay true for every kind of request, and
/// anything that only fits one of them belongs here, where it is loaded when it
/// applies and costs nothing when it does not.
///
/// **Two of a kind where the work differs.** `bundled.code` and
/// `bundled.debugging` both touch a codebase, but writing a feature and finding a
/// fault are different methods, and separating them lets the model read the one
/// that fits instead of a compromise covering both.
///
/// **A family, split by kind of work.** The writing side is one family --
/// `bundled.documentation`, `bundled.articles` and `bundled.messages` -- because
/// prose is not one craft. A page that is looked things up in, a piece that
/// argues a point and a note to a person want different structures and different
/// rules, and one body covering all three could only ever state what survives
/// being true of all three, which is the least useful part of each.
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
Read once. A file you have already read does not need reading again unless something has written to \
it since -- and your own edit counts, so the lines you just changed are the one place worth a second \
look rather than assuming what landed. The files a change touches can usually be opened together, \
so work out the whole shape of the edit first and then make it: a change split across six rounds of \
read, edit, read, edit costs the user six waits and produces a worse result than one that was \
thought through once.\n\n\
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
report, and never claim something works without the run that shows it. Then stop: once they pass, \
the change is done, and re-reading the file or looking for further work is time the user did not \
ask to spend. Nothing else is part of the change -- no documentation, configuration or new file the \
task did not ask for -- and a comment belongs where the code cannot show its own reason, not where \
it would restate what the next line does.",
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
do, so a wrong expectation becomes visible instead of silent. When a run proves nothing about the \
difference you are testing, stop running it -- repeating a command that already failed, unchanged, \
is the most expensive way to learn nothing.\n\n\
A guard, a retry or a `catch` that swallows the error is not a fix. It silences the report of a \
failure whose cause is still there, and the next person meets it somewhere harder to read. And when \
the cause is outside the code -- a dependency's behaviour, the environment, a configuration value -- \
say that and name it, rather than adding code that works around something you cannot fix from \
here.\n\n\
When you find it, fix the cause and not the symptom. Then look for the same mistake elsewhere -- \
the same call, the same assumption, a sibling file -- and add a test that fails before the fix and \
passes after. Finish by stating in a sentence what was actually wrong, because the next person will \
read it before they read the diff.",
    },
    BundledSkill {
        id: "bundled.running",
        name: "Running and verifying",
        skill_type: "code",
        description: "Use when running, building, testing or verifying the project in this workspace: \
before claiming something works, after making a change, or when the user asks to run, check or demo \
the app.",
        instructions: "Running the project means making it actually run and having seen it do so -- \
not inferring it from clean code.\n\n\
Read before you run. Find out how this project is meant to run from the project itself: a README, a \
Makefile, a `scripts` block in package.json or pyproject.toml or Cargo.toml, a CI file. Never assume \
a command -- the project tells you. When two candidate commands exist, prefer the most specific one \
(for example `npm run dev` over `node index.js`).\n\n\
Check the environment before blaming the code. Wrong working directory, missing dependency, wrong \
interpreter or Cargo or Node version, unsaved file, stale build cache -- these fail with error \
messages that look like bugs but are not. On Windows (PowerShell 5.1): no `&&`, use `;`, or `cmd /c` \
for a native chain.\n\n\
Watch real output, not the exit code. A process that exits 0 with the traceback hidden in stderr is \
a failure anyway, so read the output for tracebacks and warnings whether or not the exit code is 0. \
When output is missing, wait and re-read rather than assuming there was none.\n\n\
Fail fast, then fix the root cause. The first error is the real one; do not \"fix\" what is only a \
cascade of one earlier failure. Prefer reading the failing file over guessing at it. One fix per \
attempt, and re-run to watch it work. When a fix takes more than two attempts, step back and re-read \
the surrounding code instead of patching a third guess.\n\n\
Verification is watching behaviour, not compiling. Proof that something works is watching the right \
thing happen: output text appear, a route answer, a test pass with the count it should have, a file \
hold the change you made. When the check is a long-running server, run it in the background and \
confirm it starts cleanly -- the banner printed, the port open, no crash in the first seconds -- then \
stop it when you are done rather than leaving it up.\n\n\
Run the checks the project already has. A project with tests, linters or a build step owns those \
checks: run them rather than inventing a lighter substitute -- and when they are already failing \
before you touched anything, say so, so it is not mistaken for a regression you caused.\n\n\
Stop when verified. Once the process runs cleanly and the checks the project owns pass, the task is \
done; do not keep probing for more to check. When the user only asked for it to run, run it, confirm \
it, and report.",
    },
    BundledSkill {
        id: "bundled.research",
        name: "Research",
        skill_type: "research",
        description: "Use when a question needs checking against sources rather than answering from memory: the user's own material, a document, a paper, or the web.",
        instructions: "Answer from sources rather than memory whenever the answer can be checked. \
Search first, then open the pages that look authoritative and read them rather than trusting a \
snippet: a snippet is chosen for relevance to a query, not for what it proves. When the question is \
about the user's own material -- their notes, their project, a file or folder they named -- look in \
the workspace before the web, because the answer is usually already on the disk and a search would \
only be guessing at it. A page that cannot be read -- a paywall, a blocked request, a dead link -- \
is reported as unreadable and never inferred from its title.\n\n\
Cross-check a fact against a second source before stating it as settled, and say when sources \
disagree instead of picking one silently. Prefer primary sources -- the documentation, the paper, \
the project's own repository -- over summaries of them, because a summary is someone else's \
compression of exactly the details you need.\n\n\
One good primary source beats five summaries. Open what you need, and stop searching once the \
question is answered: a search that keeps going after the answer is found turns a paragraph into a \
bibliography, and the reader wanted the paragraph.\n\n\
Separate what you verified from what you are inferring, and name the source for each non-obvious \
claim. Every source you name has to be one you actually opened: never cite a page you did not read, \
and never assemble a URL that merely looks right, because a reference nobody can follow is worse \
than no answer -- it is the one mistake the reader cannot catch. Date a claim whose truth depends on \
when it was made. When you cannot confirm something, say what you could not confirm and what it \
would take to find out, rather than filling the gap with the most plausible answer.",
    },
    BundledSkill {
        id: "bundled.documentation",
        name: "Documentation",
        skill_type: "writing",
        description: "Use when writing or updating documentation: a README, a guide, reference \
material, a changelog or release notes.",
        instructions: "Write for the reader who is looking something up, not reading front to back. \
That reader arrives with one question and leaves when it is answered, so build the page to be \
searched and skimmed: headings that name what they answer, sections in the order a new reader needs \
them, and a table where the content is a set of options or parameters rather than a paragraph that \
buries them.\n\n\
Say what the thing does and how to use it before how it works. A reader who cannot get started does \
not need the rationale first, and one who can will scroll to what they came for. State the default, \
the version a behaviour applies to, and what happens when the input is wrong -- the failure mode is \
documentation, not a footnote.\n\n\
Every example must run exactly as written: real names and a whole command, not a fragment with a \
placeholder where the reader has to guess the shape. An example that needs editing before it works \
is one that gets mistyped, and the reader blames the tool rather than the page. Keep one name for one \
concept and use it throughout; a page that calls the same thing three things teaches the reader that \
the names do not matter.\n\n\
Document the reason the code cannot show -- a constraint that is not obvious, a decision that looks \
wrong until you know why -- and leave what the signature already states to the code. Accuracy beats \
completeness beats polish: a missing page is a gap the reader can see, and a page that is \
confidently wrong is worse than no page at all.\n\n\
Update the documentation the change touches, and only that. Then stop: the page is done when a reader \
who does not know the system can follow it without a question it should already have answered.",
    },
    BundledSkill {
        id: "bundled.articles",
        name: "Articles",
        skill_type: "writing",
        description: "Use when writing long-form prose that argues or explains: an article, a blog \
post, an essay or a talk.",
        instructions: "One piece makes one point. Settle what it is before you start and cut anything \
that does not serve it, because a second idea is a second piece and the one that stays is the one \
the reader came for. State the point early, so a reader who stops after the first paragraph still \
carries away the thing worth having.\n\n\
Earn attention with substance, not with a preamble: no scene-setting about the state of the world, \
no restating the title, no promise of what you are about to say. Then build, with each section \
answering the question the last one raised. Write the seams between sections rather than leaving a \
heading to bridge them -- a heading is a signpost for a reader who comes back, not a substitute for \
the sentence you skipped.\n\n\
Prefer the concrete to the abstract. A named example, a figure, a specific case carries an argument \
that \"many people find\" does not. Check what someone could check, and say what you could not verify \
rather than rounding it up to certainty; a piece is only as strong as the weakest fact it asserts.\n\n\
Draft in one pass and edit in a second -- cutting, tightening, reordering -- rather than rewriting a \
sentence at a time across many turns. Read it back once, in order, as the intended reader would, and \
fix what makes them stumble. Stop when the point is made and supported: a piece that ends by \
repeating itself was finished a paragraph earlier.",
    },
    BundledSkill {
        id: "bundled.messages",
        name: "Messages",
        skill_type: "writing",
        description: "Use when writing short correspondence: an email, a chat reply, a status update \
or an announcement.",
        instructions: "Lead with the reason you are writing. The ask, the answer or the news goes in \
the first line, because that is the line a reader uses to decide whether this is for them. Do not \
open with pleasantries they must read past to reach the point, and do not close by offering further \
help -- a message that ends where it stops being useful is one that respects their time.\n\n\
Give exactly the context the reader needs to act and no more. Name what they have to do, by when, \
and the one thing they need to know in order to do it; leave the history of how you got there, which \
is yours and not theirs. When there is more than one thing, make it a list rather than a paragraph, \
so no item can be skimmed past.\n\n\
Match the register to the reader and the reason: a note to a colleague is warmer and shorter than an \
announcement to a group, and a message about a problem leads with the problem. Plain, active, \
specific -- \"the deploy failed at 14:00 and I have rolled it back\" rather than \"there were some \
issues we are looking into\". Apologise once if an apology is owed, then say what you are doing about \
it; a second apology is about you, not them.\n\n\
Write it in one pass and send it. Do not draft a long message and then a summary of it.",
    },
];

/// The setting that records the seed has run.
///
/// Read before the insert and written after it. A database that has never been
/// seeded has no value here, which is exactly the case that seeds.
const SEEDED_KEY: &str = "app.bundled_skills_seeded";

impl Database {
    /// Writes the shipped skills on a fresh database, and reinstalls the shipped
    /// set on one that has already been seeded and is behind the catalogue.
    ///
    /// Safe to call on every startup: the flag short-circuits the insert, the
    /// version short-circuits the reinstall, and the fresh-seed inserts ignore an
    /// id that is already there. A failure is reported rather than swallowed,
    /// because a database that cannot take these rows has a problem worth
    /// surfacing.
    pub fn seed_bundled_skills(&self) -> Result<(), String> {
        if self.setting::<bool>(SEEDED_KEY).unwrap_or(false) {
            // Already seeded, so the only remaining question is whether the
            // catalogue has moved since. A database seeded before this existed has
            // no version and reads as 1, which is what makes the first bump reach
            // it.
            let stored = self
                .setting::<u32>(refresh::VERSION_KEY)
                .unwrap_or(refresh::FIRST_VERSION);
            if stored >= BUNDLED_VERSION {
                return Ok(());
            }
            refresh::reinstall(self)?;
            return self.set_setting(refresh::VERSION_KEY, &BUNDLED_VERSION);
        }

        let now = crate::database::utc_now();
        {
            let connection = self.connection.lock().map_err(|error| error.to_string())?;
            insert_all(&connection, &now)?;
        }
        // Written last, so a failure part-way through the insert leaves the flag
        // unset and the seed runs again next launch rather than half-done forever.
        self.set_setting(SEEDED_KEY, &true)?;
        // The state of the catalogue just written, so the first later version bump
        // sees this install is behind and runs the reinstall.
        self.set_setting(refresh::VERSION_KEY, &BUNDLED_VERSION)
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

        // A second call is a no-op -- the flag and the version short-circuit it,
        // and the stable ids would ignore the rows even without them.
        database.seed_bundled_skills().expect("seed again");
        assert_eq!(
            database.enabled_skills().expect("after twice").len(),
            BUNDLED.len()
        );
    }

    /// A deletion holds until the catalogue moves. The seed runs once rather than
    /// being re-applied on every launch, so deleting a shipped skill is not undone
    /// by simply starting the app again -- only a version bump reinstalls the set,
    /// and that case is covered in `refresh.rs`.
    #[test]
    fn a_deleted_bundled_skill_does_not_come_back_before_a_version_bump() {
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

    /// The rules a future edit must not quietly drop.
    ///
    /// A body is prose and most of it can be reworded freely. These are the
    /// specific failures each one exists to prevent -- the ones a model gets wrong
    /// by default -- so they are pinned by name rather than left to survive on
    /// good intentions.
    #[test]
    fn the_load_bearing_rule_survives_in_each_body() {
        let body = |id: &str| {
            BUNDLED
                .iter()
                .find(|skill| skill.id == id)
                .expect("shipped")
                .instructions
        };
        // A read taken before a write is out of date, including your own.
        assert!(body("bundled.code").contains("your own edit counts"));
        // A guard is not a fix.
        assert!(body("bundled.debugging").contains("is not a fix"));
        // Never cite what you did not open.
        assert!(
            body("bundled.research").contains("never cite a page you did not read"),
            "the research skill lost the citation rule"
        );
        // The user's own material before the web.
        assert!(body("bundled.research").contains("workspace before the web"));
        // Running it is not compiling it, and a cause is found before it is fixed.
        assert!(
            body("bundled.running").contains("watching behaviour, not compiling"),
            "the running skill lost the verify-by-behaviour rule"
        );
        assert!(body("bundled.running").contains("fix the root cause"));
        // Work out how to run it from the project, not from memory.
        assert!(body("bundled.running").contains("the project tells you"));
        // Documentation is judged by whether an example runs as written.
        assert!(body("bundled.documentation").contains("run exactly as written"));
        // A piece argues one point, and is not padded with a preamble.
        assert!(body("bundled.articles").contains("One piece makes one point"));
        // A message leads with the reason it was written.
        assert!(body("bundled.messages").contains("Lead with the reason you are writing"));
    }

    /// The ids are the identity, so two skills must not share one -- a duplicate
    /// would silently drop a skill from the catalogue at seed time.
    #[test]
    fn no_two_bundled_skills_share_an_id() {
        let mut ids = BUNDLED.iter().map(|skill| skill.id).collect::<Vec<_>>();
        ids.sort_unstable();
        let total = ids.len();
        ids.dedup();
        assert_eq!(ids.len(), total, "duplicate id in the shipped catalogue");
    }
}
