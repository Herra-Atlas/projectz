//! Writing the shipped catalogue into its folder root.
//!
//! # The two jobs
//!
//! 1. **Keep the shipped set current.** A version recorded in settings says which
//!    catalogue this install has. If it is behind, the whole bundled root is
//!    rewritten: every edited shipped skill is replaced and one that left the
//!    catalogue disappears. That is the same contract the row-backed set had,
//!    and it is deliberate -- the app owns this content, so a user who wants to
//!    keep an edit writes their own skill (or edits a copy, which the first save
//!    makes for them).
//! 2. **Move the old rows to folders, once.** The table the app used to store
//!    skills in is still on disk. Its user-written and agent-drafted rows become
//!    real `SKILL.md` files, carrying their enabled flag and use count; its
//!    bundled rows are skipped because the catalogue above recreates them.
//!
//! Both steps are safe to run on every launch: the version short-circuits the
//! first, the migration flag the second, and `INSERT OR IGNORE` on the state.

use std::collections::HashSet;
use std::path::Path;

use crate::database::bundled_skills::BUNDLED;
use crate::database::Database;
use crate::skills::frontmatter::{self, Frontmatter};
use crate::skills::{Roots, SkillOrigin, SKILL_FILE};

/// The shipped catalogue's version.
///
/// Bumped whenever a body, name, description or type changes in a way that
/// should reach an install that already has the app. Version history, because
/// the number is the only record of what has shipped:
///
/// - 5 -- the row-backed set: seven skills, `Research` taught to check the
///   workspace first, `Running and verifying` added, writing split in three.
/// - 6 -- skills are folders. Same seven bodies; the move itself is the change,
///   so every install gets its `SKILL.md` files on the first launch after this.
pub const BUNDLED_VERSION: u32 = 6;

const VERSION_KEY: &str = "app.bundled_skills_version";
const LEGACY_MIGRATION_KEY: &str = "app.skills_folders_version";
const LEGACY_MIGRATION_VERSION: u32 = 1;

/// Writes the shipped set, then migrates anything the old table still holds.
pub fn seed(database: &Database) -> Result<(), String> {
    let roots = crate::skills::roots();
    seed_with(database, &roots)
}

/// The same, against roots a test can point at a temporary folder.
pub fn seed_with(database: &Database, roots: &Roots) -> Result<(), String> {
    let stored = database.setting::<u32>(VERSION_KEY).unwrap_or_default();
    if stored < BUNDLED_VERSION {
        write_catalogue(database, &roots.bundled, &roots.user)?;
        // Written last: a failure part-way through the write leaves the version
        // behind, so the next launch tries again rather than accepting a
        // half-written set.
        database.set_setting(VERSION_KEY, &BUNDLED_VERSION)?;
    }
    migrate_legacy_rows(database, &roots.user)?;
    ensure_states(database, roots)?;
    Ok(())
}

/// Rewrites the bundled root from the catalogue.
fn write_catalogue(database: &Database, root: &Path, user_root: &Path) -> Result<(), String> {
    std::fs::create_dir_all(root)
        .map_err(|error| format!("Could not create {}: {error}", root.display()))?;

    // Captured before the rewrite so a retired slug's state row can go with its
    // folder. Reading afterwards would see only what is still there.
    let existing = directories(root)?;
    let shipped = BUNDLED
        .iter()
        .map(|skill| skill.id.to_string())
        .collect::<HashSet<_>>();

    for slug in existing.difference(&shipped) {
        let path = root.join(slug);
        if path.is_dir() {
            let _ = std::fs::remove_dir_all(&path);
        }
        // The state row is keyed by slug alone, and the same slug may also be a
        // user's own skill (they wrote one called `writing` after a shipped skill
        // of that name was retired). Deleting the row here would reset a switch
        // on a folder this pass never touched.
        if !user_root.join(slug).is_dir() {
            let _ = database.delete_skill_state(slug);
        }
    }

    for skill in BUNDLED {
        let dir = root.join(skill.id);
        std::fs::create_dir_all(&dir)
            .map_err(|error| format!("Could not create {}: {error}", dir.display()))?;
        let header = Frontmatter {
            name: skill.id.to_string(),
            title: Some(skill.name.to_string()),
            description: skill.description.to_string(),
            skill_type: Some(skill.skill_type.to_string()),
            // The root is what marks these; a `metadata.origin` would be a second
            // account of the same fact.
            origin: None,
        };
        let text = frontmatter::render(&header, skill.instructions);
        crate::ai::tools::write::atomic_write(&dir.join(SKILL_FILE), text.as_bytes())?;
    }
    Ok(())
}

/// Moves the old table's user and generated rows into `SKILL.md` files.
fn migrate_legacy_rows(database: &Database, user_root: &Path) -> Result<(), String> {
    if database
        .setting::<u32>(LEGACY_MIGRATION_KEY)
        .unwrap_or_default()
        >= LEGACY_MIGRATION_VERSION
    {
        return Ok(());
    }

    let rows = database.legacy_skill_rows()?;
    std::fs::create_dir_all(user_root)
        .map_err(|error| format!("Could not create {}: {error}", user_root.display()))?;
    // Slugs already used in this run, so two old rows whose names normalize to
    // one slug do not overwrite each other.
    let mut used = directories(user_root)?;

    for row in rows {
        // The catalogue is the source of truth for these, and a bundled row would
        // otherwise be written into the user root by the migration.
        if row.origin == "bundled" {
            continue;
        }
        let base = crate::skills::slugify(&row.name);
        let slug = unique(&mut used, &base);
        let dir = user_root.join(&slug);
        let header = Frontmatter {
            name: slug.clone(),
            title: Some(row.name.clone()),
            description: row.description.unwrap_or_default(),
            skill_type: row.skill_type,
            origin: (row.origin == "generated").then_some(SkillOrigin::Generated),
        };
        let text = frontmatter::render(&header, &row.instructions);
        std::fs::create_dir_all(&dir)
            .map_err(|error| format!("Could not create {}: {error}", dir.display()))?;
        crate::ai::tools::write::atomic_write(&dir.join(SKILL_FILE), text.as_bytes())?;
        database.import_skill_state(&slug, row.enabled, row.use_count, &row.created_at)?;
    }

    database.set_setting(LEGACY_MIGRATION_KEY, &LEGACY_MIGRATION_VERSION)
}

/// Makes sure every folder in either root has a state row.
///
/// A folder copied in by hand is an ordinary way to install a skill. It reads as
/// enabled and untrusted; recording that once means the switches on the page have
/// a row to update rather than having to special-case a missing one.
fn ensure_states(database: &Database, roots: &Roots) -> Result<(), String> {
    let states = database
        .skill_states()?
        .into_iter()
        .map(|state| state.slug)
        .collect::<HashSet<_>>();
    for skill in crate::skills::list_with(database, roots)? {
        if !states.contains(&skill.slug) {
            database.ensure_skill_state(&skill.slug)?;
        }
    }
    Ok(())
}

fn directories(root: &Path) -> Result<HashSet<String>, String> {
    let mut names = HashSet::new();
    let Ok(entries) = std::fs::read_dir(root) else {
        return Ok(names);
    };
    for entry in entries.flatten() {
        if entry.file_type().map(|kind| kind.is_dir()).unwrap_or(false) {
            if let Some(name) = entry.file_name().to_str() {
                names.insert(name.to_string());
            }
        }
    }
    Ok(names)
}

fn unique(used: &mut HashSet<String>, base: &str) -> String {
    if used.insert(base.to_string()) {
        return base.to_string();
    }
    for suffix in 2..1000 {
        let candidate = format!("{base}-{suffix}");
        if used.insert(candidate.clone()) {
            return candidate;
        }
    }
    format!("{base}-{}", uuid::Uuid::new_v4())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(label: &str) -> std::path::PathBuf {
        use std::sync::atomic::{AtomicU32, Ordering};
        static COUNTER: AtomicU32 = AtomicU32::new(0);
        let unique = COUNTER.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "projectz-bundled-{}-{label}-{unique}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).expect("temp dir");
        path
    }

    fn roots() -> Roots {
        let base = temp("roots");
        Roots {
            user: base.join("skills"),
            bundled: base.join("bundled"),
        }
    }

    /// The shipped set reaches disk, with one folder per catalogue entry.
    #[test]
    fn the_catalogue_is_written_as_folders() {
        let database = Database::in_memory().expect("db");
        let roots = roots();
        seed_with(&database, &roots).expect("seed");

        for skill in BUNDLED {
            let file = roots.bundled.join(skill.id).join(SKILL_FILE);
            assert!(file.is_file(), "{} was not written", skill.id);
            let (header, body) = frontmatter::read(&file).expect("read");
            assert_eq!(header.name, skill.id);
            assert_eq!(header.display_name(), skill.name);
            let head = skill.instructions.chars().take(40).collect::<String>();
            assert!(body.contains(&head), "{} lost its body", skill.id);
        }
        let _ = std::fs::remove_dir_all(roots.bundled.parent().expect("parent"));
    }

    /// A version bump rewrites a body and removes a skill that left the
    /// catalogue, which is the reason the set is replaced rather than merged.
    #[test]
    fn a_version_bump_replaces_the_shipped_tree() {
        let database = Database::in_memory().expect("db");
        let roots = roots();
        seed_with(&database, &roots).expect("seed");

        let first = roots.bundled.join(BUNDLED[0].id).join(SKILL_FILE);
        std::fs::write(&first, "---\nname: stale\n---\nold").expect("edit");
        let retired = roots.bundled.join("retired-skill");
        std::fs::create_dir_all(&retired).expect("dir");
        std::fs::write(retired.join(SKILL_FILE), "old").expect("write");

        database.set_setting(VERSION_KEY, &1u32).expect("wind back");
        seed_with(&database, &roots).expect("reseed");

        let (header, _) = frontmatter::read(&first).expect("read");
        assert_eq!(header.name, BUNDLED[0].id);
        assert!(!retired.exists(), "a retired skill was left behind");
        let _ = std::fs::remove_dir_all(roots.bundled.parent().expect("parent"));
    }

    /// The migration is one-time: running the seed again must not recreate a
    /// skill the user deleted from their own root.
    #[test]
    fn a_deleted_user_skill_is_not_recreated_by_a_later_seed() {
        let database = Database::in_memory().expect("db");
        let roots = roots();
        seed_with(&database, &roots).expect("seed");

        let mine = roots.user.join("mine");
        std::fs::create_dir_all(&mine).expect("dir");
        std::fs::write(
            mine.join(SKILL_FILE),
            "---\nname: mine\ndescription: \"when mine\"\n---\n\nMine.",
        )
        .expect("write");
        database.ensure_skill_state("mine").expect("state");

        std::fs::remove_dir_all(&mine).expect("delete");
        seed_with(&database, &roots).expect("again");
        assert!(!mine.exists(), "a user skill came back");
        let _ = std::fs::remove_dir_all(roots.bundled.parent().expect("parent"));
    }

    /// Every folder that exists gets a state row, including one copied in by
    /// hand, so the page has something to update.
    #[test]
    fn a_hand_copied_folder_gets_state() {
        let database = Database::in_memory().expect("db");
        let roots = roots();
        let dir = roots.user.join("hand-copied");
        std::fs::create_dir_all(&dir).expect("dir");
        std::fs::write(
            dir.join(SKILL_FILE),
            "---\nname: hand-copied\ndescription: \"when copied\"\n---\n\nBody.",
        )
        .expect("write");

        seed_with(&database, &roots).expect("seed");

        let state = database
            .skill_state("hand-copied")
            .expect("read")
            .expect("row");
        assert!(state.enabled);
        assert!(!state.trusted);
        let _ = std::fs::remove_dir_all(roots.bundled.parent().expect("parent"));
    }
}
