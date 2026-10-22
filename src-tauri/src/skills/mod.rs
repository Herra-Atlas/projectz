//! Skills as folders on disk, in the open Agent Skills layout.
//!
//! # The shape
//!
//! ```text
//! <data>/skills/<slug>/
//!     SKILL.md          # required: frontmatter + instructions
//!     scripts/          # optional: executable code
//!     references/       # optional: material the model reads on demand
//!     assets/           # optional: templates, images, data
//! ```
//!
//! The same slug directory may also exist under `<data>/bundled-skills/`, the
//! set the app ships. The user root wins when both exist, which is what lets an
//! edit to a bundled skill be a local override rather than a fork the user has
//! to manage: [`editable_dir`] copies the shipped folder across on first write.
//!
//! # One content store, one state store
//!
//! Everything the skill *is* lives in the folder. What the app knows *about* it
//! -- enabled, scripts trusted, how often read -- lives in `skill_state`, keyed
//! by the same slug. Content copied in by hand therefore works with no row: it
//! reads as enabled and untrusted, which are the safe defaults.
//!
//! # Where the agent can reach
//!
//! These roots are outside the workspace, and deliberately so: the workspace is
//! the user's project and a skill is the app's material. The agent reaches skills
//! only through `skill_read` / `skill_write` / `skill_run`, each of which resolves
//! inside one skill folder using the same containment check the file tools use
//! ([`crate::ai::tools::resolve_within`]). `read_file` is not widened.

pub mod bundled;
pub mod frontmatter;
pub mod install;

use std::collections::HashMap;
use std::path::{Component, Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::database::skills::SkillState;
use crate::database::Database;

/// The file every skill directory must contain.
pub const SKILL_FILE: &str = "SKILL.md";

/// Largest `SKILL.md` this app will read.
///
/// Generous for prose and small enough that a folder labelled a skill cannot be
/// used to make a listing read a whole disk.
const MAX_BODY_BYTES: u64 = 1_000_000;

/// Largest supporting file a read or write will carry.
///
/// Text only, and one file per call: the tool result has its own cap, so a larger
/// ceiling here would only be truncated later. Binary assets are copied rather
/// than transferred as text.
const MAX_SUPPORTING_BYTES: u64 = 512 * 1024;

/// Largest asset [`copy_file`] will bring in from the workspace.
const MAX_ASSET_BYTES: u64 = 8 * 1024 * 1024;

/// How deep a skill's own tree is walked.
const MAX_DEPTH: usize = 4;

/// How many files one manifest reports.
const MAX_FILES: usize = 200;

/// Extensions that make a file something `skill_run` could execute.
const SCRIPT_EXTENSIONS: &[&str] = &[
    "ps1", "py", "js", "mjs", "cjs", "sh", "bash", "cmd", "bat", "exe", "rb", "pl", "ts",
];

/// Who wrote a skill.
///
/// The distinction is unchanged from the row-backed version: a shipped or
/// user-written skill was deliberate, an agent-drafted one was inferred and may
/// encode an assumption nobody checked. Storage moved; the meaning did not.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SkillOrigin {
    #[default]
    User,
    Generated,
    Bundled,
}

impl SkillOrigin {
    pub fn from_str(value: &str) -> Self {
        match value {
            "generated" => Self::Generated,
            "bundled" => Self::Bundled,
            _ => Self::User,
        }
    }
}

/// One skill as the UI and the model see it.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Skill {
    /// The folder name and the spec's `name`.
    pub slug: String,
    /// The human label, from `metadata.title` or the slug.
    pub name: String,
    pub description: Option<String>,
    pub skill_type: Option<String>,
    pub origin: SkillOrigin,
    pub enabled: bool,
    pub trusted: bool,
    pub use_count: i64,
    /// Supporting files, excluding `SKILL.md`.
    pub file_count: usize,
    /// Whether any of them is executable material.
    pub has_scripts: bool,
    pub created_at: String,
    pub updated_at: String,
}

/// One supporting file in a skill.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct SkillFile {
    /// Relative to the skill root, with `/` separators on every platform.
    pub path: String,
    pub size: u64,
    pub is_script: bool,
}

/// A skill plus the parts a listing does not need.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SkillDetail {
    pub skill: Skill,
    pub instructions: String,
    pub files: Vec<SkillFile>,
}

/// What the create/edit form sends.
#[derive(Clone, Debug, Deserialize)]
pub struct SkillDraft {
    /// The display name. The slug is derived from it on create and kept after.
    pub name: String,
    pub description: Option<String>,
    /// The markdown body.
    pub instructions: String,
    pub skill_type: Option<String>,
}

/// The two roots a scan reads.
#[derive(Clone, Debug)]
pub struct Roots {
    pub user: PathBuf,
    pub bundled: PathBuf,
}

/// Where skills live for this installation.
pub fn roots() -> Roots {
    Roots {
        user: crate::app_paths::skills_root(),
        bundled: crate::app_paths::bundled_skills_root(),
    }
}

/// Every skill, enabled or not, user root first.
pub fn list(database: &Database) -> Result<Vec<Skill>, String> {
    list_with(database, &roots())
}

/// Only the skills a run may be offered.
pub fn enabled(database: &Database) -> Result<Vec<Skill>, String> {
    Ok(list(database)?
        .into_iter()
        .filter(|skill| skill.enabled)
        .collect())
}

/// One skill by slug, or by the label the index printed.
pub fn resolve(database: &Database, requested: &str) -> Result<Skill, String> {
    resolve_with(database, requested, &roots())
}

fn resolve_with(database: &Database, requested: &str, roots: &Roots) -> Result<Skill, String> {
    let skills = list_with(database, roots)?;
    let wanted = requested.trim();
    let bare = strip_type_suffix(wanted);
    skills
        .iter()
        .find(|skill| skill.slug.eq_ignore_ascii_case(bare))
        .or_else(|| {
            skills
                .iter()
                .find(|skill| skill.name.eq_ignore_ascii_case(bare))
        })
        .cloned()
        .ok_or_else(|| {
            let available = skills
                .iter()
                .map(|skill| skill.name.clone())
                .collect::<Vec<_>>()
                .join(", ");
            format!("No skill named `{requested}`. Available: {available}")
        })
}

/// One skill with its instructions and files.
pub fn get(database: &Database, slug: &str) -> Result<SkillDetail, String> {
    get_with(database, slug, &roots())
}

pub(crate) fn get_with(
    database: &Database,
    slug: &str,
    roots: &Roots,
) -> Result<SkillDetail, String> {
    let skill = resolve_with(database, slug, roots)?;
    let dir = directory_of_in(&skill, roots)?;
    let (_, instructions) = frontmatter::read(&dir.join(SKILL_FILE))?;
    let files = manifest(&dir);
    Ok(SkillDetail {
        skill,
        instructions,
        files,
    })
}

/// Creates a folder for a new skill.
///
/// The slug is derived once and then kept: a rename changes the label, not the
/// address, so a link, a state row and a running conversation all keep pointing
/// at the same skill.
pub fn create(
    database: &Database,
    draft: &SkillDraft,
    origin: SkillOrigin,
) -> Result<Skill, String> {
    let roots = roots();
    let name = required(&draft.name, "name")?;
    let body = required(&draft.instructions, "instructions")?;
    let slug = unique_slug(&roots.user, &slugify(&name));
    let header = frontmatter::Frontmatter {
        name: slug.clone(),
        title: Some(name.clone()),
        description: clean(&draft.description).unwrap_or_default(),
        skill_type: clean(&draft.skill_type),
        origin: (origin == SkillOrigin::Generated).then_some(SkillOrigin::Generated),
    };
    let dir = roots.user.join(&slug);
    std::fs::create_dir_all(&dir)
        .map_err(|error| format!("Could not create {}: {error}", dir.display()))?;
    write_markdown(&dir, &frontmatter::render(&header, &body))?;
    database.ensure_skill_state(&slug)?;
    get(database, &slug).map(|detail| detail.skill)
}

/// Replaces a skill's editable text, keeping its slug and history.
pub fn update(database: &Database, slug: &str, draft: &SkillDraft) -> Result<Skill, String> {
    let skill = resolve(database, slug)?;
    let name = required(&draft.name, "name")?;
    let body = required(&draft.instructions, "instructions")?;
    let dir = editable_dir(&skill)?;
    let (existing, _) = frontmatter::read(&dir.join(SKILL_FILE))?;
    // An agent-drafted skill stays marked as such until it is re-created from
    // scratch; the label describes where the text came from, not who last typed.
    let origin = existing
        .origin
        .filter(|origin| *origin == SkillOrigin::Generated);
    let header = frontmatter::Frontmatter {
        name: skill.slug.clone(),
        title: Some(name),
        description: clean(&draft.description).unwrap_or_default(),
        skill_type: clean(&draft.skill_type),
        origin,
    };
    write_markdown(&dir, &frontmatter::render(&header, &body))?;
    database.ensure_skill_state(&skill.slug)?;
    get(database, &skill.slug).map(|detail| detail.skill)
}

/// Removes a skill's folder.
///
/// A user copy goes first, and only it: if the skill was a fork of a shipped
/// one, deleting the edit reveals the original rather than destroying both. A
/// bundled skill with no user copy is removed from the shipped root too; the
/// next catalogue version restores it, which is the same contract the row-backed
/// set had. The state row goes with whichever folder was removed.
pub fn delete(database: &Database, slug: &str) -> Result<(), String> {
    let skill = resolve(database, slug)?;
    let roots = roots();
    let user = roots.user.join(&skill.slug);
    let target = if user.is_dir() {
        user
    } else {
        roots.bundled.join(&skill.slug)
    };
    if target.is_dir() {
        std::fs::remove_dir_all(&target)
            .map_err(|error| format!("Could not remove {}: {error}", target.display()))?;
    }
    database.delete_skill_state(&skill.slug)
}

/// Writes a text file beside `SKILL.md`.
pub fn write_file(
    database: &Database,
    slug: &str,
    path: &str,
    content: &str,
) -> Result<SkillFile, String> {
    let skill = resolve(database, slug)?;
    if content.len() as u64 > MAX_SUPPORTING_BYTES {
        return Err(format!(
            "That file is {} bytes; a single supporting file is limited to {}.",
            content.len(),
            MAX_SUPPORTING_BYTES
        ));
    }
    let dir = editable_dir(&skill)?;
    let relative = relative_path(path)?;
    let target = resolve_inside(&dir, &relative)?;
    if let Some(parent) = target.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|error| format!("Could not create {}: {error}", parent.display()))?;
    }
    crate::ai::tools::write::atomic_write(&target, content.as_bytes())?;
    file_entry(&dir, &relative)
}

/// Copies a workspace file into a skill.
///
/// The way binary material arrives: an image, a template, a PDF. `content`
/// cannot carry bytes honestly, and base64 would inflate every asset by a third
/// only to decode it again.
pub fn copy_file(
    database: &Database,
    slug: &str,
    path: &str,
    from_workspace: &str,
) -> Result<SkillFile, String> {
    let skill = resolve(database, slug)?;
    let workspace = crate::ai::tools::require_workspace()?;
    let source = crate::ai::tools::resolve_within(&workspace, from_workspace)?;
    let size = std::fs::metadata(&source)
        .map_err(|error| format!("Could not read {}: {error}", source.display()))?
        .len();
    if size > MAX_ASSET_BYTES {
        return Err(format!(
            "{} is {size} bytes; an asset is limited to {MAX_ASSET_BYTES}.",
            source.display()
        ));
    }
    let bytes = std::fs::read(&source)
        .map_err(|error| format!("Could not read {}: {error}", source.display()))?;
    let dir = editable_dir(&skill)?;
    let relative = relative_path(path)?;
    let target = resolve_inside(&dir, &relative)?;
    if let Some(parent) = target.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|error| format!("Could not create {}: {error}", parent.display()))?;
    }
    crate::ai::tools::write::atomic_write(&target, &bytes)?;
    file_entry(&dir, &relative)
}

/// Reads one supporting file back as text.
pub fn read_file(database: &Database, slug: &str, path: &str) -> Result<String, String> {
    let skill = resolve(database, slug)?;
    let dir = directory_of(&skill)?;
    let relative = relative_path(path)?;
    let target = resolve_inside(&dir, &relative)?;
    let size = std::fs::metadata(&target)
        .map_err(|error| format!("Could not read {}: {error}", target.display()))?
        .len();
    if size > MAX_SUPPORTING_BYTES {
        return Err(format!(
            "`{relative}` is {size} bytes; this tool reads one file at a time and is limited to \
             {MAX_SUPPORTING_BYTES}. Read part of it with grep, or run a script that uses it."
        ));
    }
    match std::fs::read_to_string(&target) {
        Ok(text) => Ok(text),
        Err(error) if error.kind() == std::io::ErrorKind::InvalidData => Ok(format!(
            "[{relative} is not UTF-8 text — {size} bytes of binary. It can be copied, moved or \
             used by a script, but not read as text.]"
        )),
        Err(error) => Err(format!("Could not read {relative}: {error}")),
    }
}

/// Removes one supporting file.
pub fn delete_file(database: &Database, slug: &str, path: &str) -> Result<(), String> {
    let skill = resolve(database, slug)?;
    let dir = editable_dir(&skill)?;
    let relative = relative_path(path)?;
    let target = resolve_inside(&dir, &relative)?;
    if target.is_dir() {
        return Err(format!("`{relative}` is a folder, not a file"));
    }
    std::fs::remove_file(&target).map_err(|error| format!("Could not remove {relative}: {error}"))
}

/// Whether a skill's scripts may run.
pub fn trusted(database: &Database, slug: &str) -> Result<bool, String> {
    Ok(database
        .skill_state(slug)?
        .map(|state| state.trusted)
        .unwrap_or(false))
}

/// Resolves a script inside a skill, ready to execute.
///
/// Returned as a pair so the caller can use the skill's slug and the file's real
/// path without resolving twice. A path that is not a script, or not in this
/// skill, is refused before anything is spawned.
pub fn script_path(
    database: &Database,
    slug: &str,
    path: &str,
) -> Result<(Skill, PathBuf), String> {
    let skill = resolve(database, slug)?;
    let dir = directory_of(&skill)?;
    let relative = relative_path(path)?;
    if !is_script(&relative) {
        return Err(format!(
            "`{relative}` does not look like a script. A script has a known extension \
             (.ps1, .py, .js, .sh, .cmd, .bat, .exe) or lives under `scripts/`."
        ));
    }
    let target = resolve_inside(&dir, &relative)?;
    if !target.is_file() {
        return Err(format!("`{relative}` is not in `{}`", skill.name));
    }
    Ok((skill, target))
}

/// The folder a skill's files live in, for reading.
pub fn directory_of(skill: &Skill) -> Result<PathBuf, String> {
    directory_of_in(skill, &roots())
}

fn directory_of_in(skill: &Skill, roots: &Roots) -> Result<PathBuf, String> {
    let user = roots.user.join(&skill.slug);
    if user.is_dir() {
        return Ok(user);
    }
    let bundled = roots.bundled.join(&skill.slug);
    if bundled.is_dir() {
        return Ok(bundled);
    }
    Err(format!("The folder for `{}` is gone", skill.slug))
}

/// The folder a write should land in.
///
/// A bundled skill is copied into the user root the first time it is edited, so
/// the app's own material is never modified in place and the user's version wins
/// from then on. A delete of the user folder reveals the shipped one again.
fn editable_dir(skill: &Skill) -> Result<PathBuf, String> {
    let roots = roots();
    let user = roots.user.join(&skill.slug);
    if user.is_dir() {
        return Ok(user);
    }
    let bundled = roots.bundled.join(&skill.slug);
    if !bundled.is_dir() {
        return Err(format!("The folder for `{}` is gone", skill.slug));
    }
    copy_tree(&bundled, &user)?;
    Ok(user)
}

/// The supporting files of a skill, sorted by path.
pub fn manifest(dir: &Path) -> Vec<SkillFile> {
    let mut files = Vec::new();
    walk(dir, dir, 0, &mut files);
    files.sort_by(|left, right| left.path.cmp(&right.path));
    files
}

fn walk(root: &Path, dir: &Path, depth: usize, files: &mut Vec<SkillFile>) {
    if depth > MAX_DEPTH || files.len() >= MAX_FILES {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let mut entries: Vec<_> = entries.flatten().collect();
    entries.sort_by_key(|entry| entry.file_name());
    for entry in entries {
        if files.len() >= MAX_FILES {
            return;
        }
        let path = entry.path();
        let Ok(kind) = entry.file_type() else {
            continue;
        };
        if kind.is_symlink() {
            // A link is not part of the skill: following one would put a file
            // outside the folder in the manifest and in reach of a read.
            continue;
        }
        if kind.is_dir() {
            walk(root, &path, depth + 1, files);
            continue;
        }
        let Ok(relative) = path.strip_prefix(root) else {
            continue;
        };
        let relative = slash(relative);
        if relative == SKILL_FILE {
            continue;
        }
        let size = entry.metadata().map(|meta| meta.len()).unwrap_or(0);
        files.push(SkillFile {
            is_script: is_script(&relative),
            path: relative,
            size,
        });
    }
}

fn file_entry(dir: &Path, relative: &str) -> Result<SkillFile, String> {
    let target = resolve_inside(dir, relative)?;
    let size = std::fs::metadata(&target)
        .map_err(|error| format!("Could not stat {relative}: {error}"))?
        .len();
    Ok(SkillFile {
        path: relative.to_string(),
        size,
        is_script: is_script(relative),
    })
}

/// Whether a path names something executable.
pub fn is_script(path: &str) -> bool {
    Path::new(path)
        .extension()
        .and_then(|extension| extension.to_str())
        .map(|extension| {
            SCRIPT_EXTENSIONS
                .iter()
                .any(|known| known.eq_ignore_ascii_case(extension))
        })
        .unwrap_or(false)
        || path.replace('\\', "/").starts_with("scripts/")
}

/// Normalizes and validates a path inside one skill folder.
fn relative_path(path: &str) -> Result<String, String> {
    let trimmed = path.trim().replace('\\', "/");
    if trimmed.is_empty() {
        return Err("A file path is required".to_string());
    }
    let candidate = Path::new(&trimmed);
    if candidate.is_absolute() || candidate.has_root() {
        return Err(format!("`{path}` must be relative to the skill folder"));
    }
    if candidate.components().any(|component| {
        matches!(
            component,
            Component::ParentDir | Component::RootDir | Component::Prefix(_)
        )
    }) {
        return Err(format!("`{path}` points outside the skill folder"));
    }
    let normalized = candidate
        .components()
        .filter_map(|component| match component {
            Component::Normal(part) => Some(part.to_string_lossy().into_owned()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("/");
    if normalized.eq_ignore_ascii_case(SKILL_FILE) {
        return Err(
            "SKILL.md is edited through the skill's own form, not as a supporting file".to_string(),
        );
    }
    Ok(normalized)
}

/// Resolves a relative path inside a skill, refusing anything that escapes.
///
/// The same containment check the workspace tools use, with the skill folder as
/// the root. One implementation because two would be two things to keep correct.
fn resolve_inside(dir: &Path, relative: &str) -> Result<PathBuf, String> {
    crate::ai::tools::resolve_within(dir, relative)
}

/// A slug for a display name: lowercase, digits and single hyphens.
pub fn slugify(name: &str) -> String {
    let mut slug = String::new();
    let mut last_hyphen = false;
    for character in name.trim().chars().flat_map(char::to_lowercase) {
        if character.is_ascii_alphanumeric() {
            slug.push(character);
            last_hyphen = false;
        } else if !last_hyphen && !slug.is_empty() {
            slug.push('-');
            last_hyphen = true;
        }
    }
    let slug = slug.trim_matches('-').to_string();
    let slug = if slug.len() > 64 {
        slug[..64].trim_end_matches('-').to_string()
    } else {
        slug
    };
    if slug.is_empty() {
        "skill".to_string()
    } else {
        slug
    }
}

/// A slug no folder in the user root is using.
fn unique_slug(root: &Path, base: &str) -> String {
    if !root.join(base).exists() {
        return base.to_string();
    }
    for suffix in 2..1000 {
        let candidate = format!("{base}-{suffix}");
        if !root.join(&candidate).exists() {
            return candidate;
        }
    }
    format!("{base}-{}", uuid::Uuid::new_v4())
}

fn write_markdown(dir: &Path, text: &str) -> Result<(), String> {
    std::fs::create_dir_all(dir)
        .map_err(|error| format!("Could not create {}: {error}", dir.display()))?;
    crate::ai::tools::write::atomic_write(&dir.join(SKILL_FILE), text.as_bytes())
}

/// Copies a folder tree, refusing links and enforcing the same size budget a
/// listing does. Used only to fork a bundled skill into the user root.
fn copy_tree(from: &Path, to: &Path) -> Result<(), String> {
    std::fs::create_dir_all(to)
        .map_err(|error| format!("Could not create {}: {error}", to.display()))?;
    for entry in std::fs::read_dir(from)
        .map_err(|error| format!("Could not read {}: {error}", from.display()))?
        .flatten()
    {
        let source = entry.path();
        let destination = to.join(entry.file_name());
        let kind = entry
            .file_type()
            .map_err(|error| format!("Could not read {}: {error}", source.display()))?;
        if kind.is_symlink() {
            continue;
        }
        if kind.is_dir() {
            copy_tree(&source, &destination)?;
        } else {
            let bytes = std::fs::read(&source)
                .map_err(|error| format!("Could not read {}: {error}", source.display()))?;
            if bytes.len() as u64 > MAX_ASSET_BYTES {
                return Err(format!("{} is too large to copy", source.display()));
            }
            crate::ai::tools::write::atomic_write(&destination, &bytes)?;
        }
    }
    Ok(())
}

/// Scans two roots. Crate-visible because the seed has to inspect the roots it
/// just wrote rather than the installation's real ones.
pub(crate) fn list_with(database: &Database, roots: &Roots) -> Result<Vec<Skill>, String> {
    let states = database
        .skill_states()
        .map_err(|error| format!("Could not read skill state: {error}"))?
        .into_iter()
        .map(|state| (state.slug.clone(), state))
        .collect::<HashMap<_, _>>();

    let mut skills: HashMap<String, Skill> = HashMap::new();
    // Bundled first, so a user folder of the same slug replaces it below.
    for (root, origin) in [(&roots.bundled, true), (&roots.user, false)] {
        for skill in scan(root, origin, &states) {
            skills.insert(skill.slug.clone(), skill);
        }
    }
    let mut skills = skills.into_values().collect::<Vec<_>>();
    // Enabled first, then by label, then by slug. The final tie-break matters:
    // the values came out of a `HashMap`, so two skills with the same label would
    // otherwise swap places between launches -- and the model's index is a cached
    // prefix that must be byte-identical from one request to the next.
    skills.sort_by(|left, right| {
        right
            .enabled
            .cmp(&left.enabled)
            .then_with(|| left.name.to_lowercase().cmp(&right.name.to_lowercase()))
            .then_with(|| left.slug.cmp(&right.slug))
    });
    Ok(skills)
}

/// Reads one root, skipping folders that are not skills.
///
/// A malformed `SKILL.md` is logged and skipped rather than failing the list: one
/// broken skill must not make every other skill disappear.
fn scan(root: &Path, bundled: bool, states: &HashMap<String, SkillState>) -> Vec<Skill> {
    let Ok(entries) = std::fs::read_dir(root) else {
        return Vec::new();
    };
    let mut skills = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        // A symlinked directory is not a skill: following it would read a folder
        // outside the root the user installed into.
        if !entry.file_type().map(|kind| kind.is_dir()).unwrap_or(false) {
            continue;
        }
        let Some(slug) = path.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        let markdown = path.join(SKILL_FILE);
        if !markdown.is_file() {
            continue;
        }
        let Ok(size) = std::fs::metadata(&markdown).map(|meta| meta.len()) else {
            continue;
        };
        if size > MAX_BODY_BYTES {
            tracing::warn!(%slug, size, "skill SKILL.md is too large; skipped");
            continue;
        }
        // The index needs the header only. Reading the whole body on every
        // request would make a listing cost whatever the instructions weigh.
        let header = match frontmatter::read_header(&markdown) {
            Ok(header) => header,
            Err(error) => {
                tracing::warn!(%slug, %error, "skill SKILL.md could not be read");
                continue;
            }
        };
        let state = states.get(slug);
        let files = manifest(&path);
        let modified = std::fs::metadata(&markdown)
            .and_then(|meta| meta.modified())
            .ok()
            .map(timestamp)
            .unwrap_or_default();
        let display = header.display_name();
        skills.push(Skill {
            slug: slug.to_string(),
            // The folder is the address even when the header disagrees, which is
            // what keeps a hand-edited name from making a skill unreachable.
            name: if display.trim().is_empty() {
                slug.to_string()
            } else {
                display
            },
            description: clean_text(&header.description),
            skill_type: header.skill_type,
            origin: if bundled {
                SkillOrigin::Bundled
            } else {
                header.origin.unwrap_or(SkillOrigin::User)
            },
            enabled: state.map(|state| state.enabled).unwrap_or(true),
            trusted: state.map(|state| state.trusted).unwrap_or(false),
            use_count: state.map(|state| state.use_count).unwrap_or(0),
            file_count: files.len(),
            has_scripts: files.iter().any(|file| file.is_script),
            created_at: state
                .map(|state| state.created_at.clone())
                .unwrap_or_else(|| modified.clone()),
            updated_at: modified,
        });
    }
    skills
}

fn timestamp(time: std::time::SystemTime) -> String {
    chrono::DateTime::<chrono::Utc>::from(time).to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

fn strip_type_suffix(label: &str) -> &str {
    let trimmed = label.trim();
    if let Some(start) = trimmed.rfind('[') {
        if trimmed.ends_with(']') && start > 0 {
            return trimmed[..start].trim();
        }
    }
    trimmed
}

fn clean(value: &Option<String>) -> Option<String> {
    value
        .as_deref()
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .map(str::to_string)
}

fn clean_text(value: &str) -> Option<String> {
    let value = value.trim();
    (!value.is_empty()).then(|| value.to_string())
}

fn required(value: &str, field: &str) -> Result<String, String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err(format!("A skill needs a `{field}`"));
    }
    let limit = if field == "name" { 120 } else { 20_000 };
    if trimmed.chars().count() > limit {
        return Err(format!("`{field}` is longer than {limit} characters"));
    }
    Ok(trimmed.to_string())
}

fn slash(path: &Path) -> String {
    path.components()
        .filter_map(|component| match component {
            Component::Normal(part) => Some(part.to_string_lossy().into_owned()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("/")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(label: &str) -> PathBuf {
        use std::sync::atomic::{AtomicU32, Ordering};
        static COUNTER: AtomicU32 = AtomicU32::new(0);
        let unique = COUNTER.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "projectz-skills-{}-{label}-{unique}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).expect("temp dir");
        path
    }

    fn write_skill(root: &Path, slug: &str, title: &str) {
        let dir = root.join(slug);
        std::fs::create_dir_all(dir.join("scripts")).expect("dir");
        std::fs::create_dir_all(dir.join("references")).expect("refs");
        std::fs::write(
            dir.join(SKILL_FILE),
            format!("---\nname: {slug}\ndescription: \"when {title}\"\nmetadata:\n  title: {title}\n  type: code\n---\n\nDo it.\n"),
        )
        .expect("write");
        std::fs::write(dir.join("scripts/run.ps1"), "Write-Output hi").expect("script");
        std::fs::write(dir.join("references/notes.md"), "notes").expect("ref");
    }

    #[test]
    fn a_slug_is_lowercase_and_single_hyphens() {
        assert_eq!(slugify("Code Review!"), "code-review");
        assert_eq!(slugify("  A -- B  "), "a-b");
        assert_eq!(slugify(""), "skill");
        assert_eq!(slugify("---"), "skill");
    }

    /// The container check is the security boundary: no absolute path, no `..`,
    /// and `SKILL.md` is not a supporting file.
    #[test]
    fn a_path_outside_the_skill_folder_is_refused() {
        assert!(relative_path("../secret").is_err());
        assert!(relative_path("C:\\Windows\\win.ini").is_err());
        assert!(relative_path("/etc/passwd").is_err());
        assert!(relative_path("SKILL.md").is_err());
        assert_eq!(
            relative_path("references\\notes.md").expect("relative"),
            "references/notes.md"
        );
    }

    #[test]
    fn a_manifest_excludes_the_skill_file_and_marks_scripts() {
        let root = temp("manifest");
        write_skill(&root, "code-review", "Code review");
        let files = manifest(&root.join("code-review"));
        let paths = files
            .iter()
            .map(|file| file.path.as_str())
            .collect::<Vec<_>>();
        assert_eq!(paths, vec!["references/notes.md", "scripts/run.ps1"]);
        assert!(
            files
                .iter()
                .find(|file| file.path.ends_with("run.ps1"))
                .expect("script")
                .is_script
        );
        assert!(
            !files
                .iter()
                .find(|file| file.path.ends_with("notes.md"))
                .expect("ref")
                .is_script
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    /// The label printed by the index is `name [type]`; a model that copies the
    /// whole label must still find the skill.
    #[test]
    fn a_copied_index_label_resolves_to_its_slug() {
        assert_eq!(strip_type_suffix("Code review [code]"), "Code review");
        assert_eq!(strip_type_suffix("Code review"), "Code review");
        assert_eq!(strip_type_suffix("[code]"), "[code]");
    }

    #[test]
    fn the_last_write_wins_when_both_roots_hold_a_slug() {
        let user = temp("user");
        let bundled = temp("bundled");
        write_skill(&user, "code-review", "Mine");
        write_skill(&bundled, "code-review", "Shipped");
        let database = Database::in_memory().expect("db");
        let listed = list_with(
            &database,
            &Roots {
                user: user.clone(),
                bundled: bundled.clone(),
            },
        )
        .expect("list");
        assert_eq!(listed.len(), 1);
        // The user root is scanned second, so its copy replaces the shipped one --
        // including the label it carries.
        assert_eq!(listed[0].name, "Mine");
        assert_eq!(listed[0].origin, SkillOrigin::User);
        let _ = std::fs::remove_dir_all(&user);
        let _ = std::fs::remove_dir_all(&bundled);
    }

    #[test]
    fn a_new_row_reads_as_enabled_and_untrusted() {
        let root = temp("state");
        write_skill(&root, "code-review", "Code review");
        let database = Database::in_memory().expect("db");
        let listed = list_with(
            &database,
            &Roots {
                user: root.clone(),
                bundled: root.join("none"),
            },
        )
        .expect("list");
        assert!(listed[0].enabled);
        assert!(!listed[0].trusted);
        assert_eq!(listed[0].file_count, 2);
        assert!(listed[0].has_scripts);
        let _ = std::fs::remove_dir_all(&root);
    }
}
