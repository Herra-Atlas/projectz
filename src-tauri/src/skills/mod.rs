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
//! # One store, and it is the disk
//!
//! There is no table. A skill is its folder, and every fact about it is read from
//! that folder: the title is the folder name, the trigger is the description in
//! `SKILL.md`, and whether it is offered is whether its folder is there. That is
//! what lets the Skills page hand the folder to the Files panel and get out of
//! the way -- an edit made in any editor is an edit, with no row to keep in step
//! and no save to press.
//!
//! The app used to keep `skill_state` (enabled, trusted, use count) beside the
//! folder, and a second `bundled-skills/` root it seeded itself. Both are gone.
//! Every state field was a second answer to a question the folder already
//! answered, and each had a way to go stale; the shipped set now lives in the
//! repository -- `bundled-skills/` beside the README -- for a person to copy in.
//!
//! # Where the agent can reach
//!
//! These roots are outside the workspace, and deliberately so: the workspace is
//! the user's project and a skill is the app's material. The agent reaches skills
//! only through `skill_read` / `skill_write` / `skill_run`, each of which resolves
//! inside one skill folder using the same containment check the file tools use
//! ([`crate::ai::tools::resolve_within`]). `read_file` is not widened.

pub mod frontmatter;
pub mod github;
pub mod install;

use std::path::{Component, Path, PathBuf};

use serde::{Deserialize, Serialize};

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

/// One skill as the UI and the model see it.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Skill {
    /// The folder name and the spec's `name`. The skill's only identity.
    pub slug: String,
    /// The label to draw, made from the folder name.
    pub name: String,
    pub description: Option<String>,
    /// Supporting files, excluding `SKILL.md`.
    pub file_count: usize,
    /// Whether any of them is executable material.
    pub has_scripts: bool,
    /// Whether the skill carries a `references/` folder.
    pub has_references: bool,
    /// Whether the skill carries an `assets/` (or `templates/`) folder.
    pub has_assets: bool,
}

/// One supporting file in a skill.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct SkillFile {
    /// Relative to the skill root, with `/` separators on every platform.
    pub path: String,
    pub size: u64,
    pub is_script: bool,
}

/// What a create/update writes.
///
/// The description is the only header field left to edit, because the name is
/// the folder and the body is the rest of the file.
#[derive(Clone, Debug, Deserialize)]
pub struct SkillDraft {
    /// The display name. The slug is derived from it on create and kept after.
    pub name: String,
    pub description: Option<String>,
    /// The markdown body.
    pub instructions: String,
}

/// The folder every skill lives in.
pub fn root() -> PathBuf {
    crate::app_paths::skills_root()
}

/// Every skill folder, by label.
pub fn list() -> Result<Vec<Skill>, String> {
    let mut skills = scan(&root());
    // By label, then slug. The final tie-break matters: two folders can carry
    // the same label (`a-b` and `a_b` both humanize to `A b`), and the model's
    // index is a cached prefix that must be byte-identical between requests.
    skills.sort_by(|left, right| {
        left.name
            .to_lowercase()
            .cmp(&right.name.to_lowercase())
            .then_with(|| left.slug.cmp(&right.slug))
    });
    Ok(skills)
}

/// One skill by slug, or by the label it prints.
///
/// A trailing `[type]` is tolerated because that is how an older index labelled a
/// skill, and a transcript from before the field was removed still names one.
pub fn resolve(requested: &str) -> Result<Skill, String> {
    resolve_at(&root(), requested)
}

/// The same resolution against a named root, for the installer and the tests.
pub fn resolve_at(root: &Path, requested: &str) -> Result<Skill, String> {
    let skills = scan(root);
    let wanted = strip_type_suffix(requested.trim());
    skills
        .iter()
        .find(|skill| skill.slug.eq_ignore_ascii_case(wanted))
        .or_else(|| {
            skills
                .iter()
                .find(|skill| skill.name.eq_ignore_ascii_case(wanted))
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

/// One skill's body and files.
pub fn get(slug: &str) -> Result<(Skill, String, Vec<SkillFile>), String> {
    let skill = resolve(slug)?;
    let dir = directory_of(&skill)?;
    let (_, instructions) = frontmatter::read(&dir.join(SKILL_FILE))?;
    Ok((skill, instructions, manifest(&dir)))
}

/// Creates a folder for a new skill.
///
/// The slug is derived once and then kept: a rename changes the label, not the
/// address, so a link and a running conversation both keep pointing at the same
/// skill.
pub fn create(draft: &SkillDraft) -> Result<Skill, String> {
    let root = root();
    let name = required(&draft.name, "name")?;
    let body = required(&draft.instructions, "instructions")?;
    let slug = unique_slug(&root, &slugify(&name));
    let dir = root.join(&slug);
    std::fs::create_dir_all(&dir)
        .map_err(|error| format!("Could not create {}: {error}", dir.display()))?;
    write_markdown(
        &dir,
        &frontmatter::render(
            &slug,
            clean(&draft.description).as_deref().unwrap_or(""),
            &body,
        ),
    )?;
    resolve(&slug)
}

/// Replaces a skill's text, keeping its slug and files.
pub fn update(slug: &str, draft: &SkillDraft) -> Result<Skill, String> {
    let skill = resolve(slug)?;
    let body = required(&draft.instructions, "instructions")?;
    let dir = directory_of(&skill)?;
    write_markdown(
        &dir,
        &frontmatter::render(
            &skill.slug,
            clean(&draft.description).as_deref().unwrap_or(""),
            &body,
        ),
    )?;
    resolve(&skill.slug)
}

/// Removes a skill's folder.
pub fn delete(slug: &str) -> Result<(), String> {
    let skill = resolve(slug)?;
    let dir = directory_of(&skill)?;
    std::fs::remove_dir_all(&dir)
        .map_err(|error| format!("Could not remove {}: {error}", dir.display()))
}

/// Writes a text file beside `SKILL.md`.
pub fn write_file(slug: &str, path: &str, content: &str) -> Result<SkillFile, String> {
    let skill = resolve(slug)?;
    if content.len() as u64 > MAX_SUPPORTING_BYTES {
        return Err(format!(
            "That file is {} bytes; a single supporting file is limited to {}.",
            content.len(),
            MAX_SUPPORTING_BYTES
        ));
    }
    let dir = directory_of(&skill)?;
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
pub fn copy_file(slug: &str, path: &str, from_workspace: &str) -> Result<SkillFile, String> {
    let skill = resolve(slug)?;
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
    let dir = directory_of(&skill)?;
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
pub fn read_file(slug: &str, path: &str) -> Result<String, String> {
    let skill = resolve(slug)?;
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
pub fn delete_file(slug: &str, path: &str) -> Result<(), String> {
    let skill = resolve(slug)?;
    let dir = directory_of(&skill)?;
    let relative = relative_path(path)?;
    let target = resolve_inside(&dir, &relative)?;
    if target.is_dir() {
        return Err(format!("`{relative}` is a folder, not a file"));
    }
    std::fs::remove_file(&target).map_err(|error| format!("Could not remove {relative}: {error}"))
}

/// Resolves a script inside a skill, ready to execute.
///
/// Returned as a pair so the caller can use the skill's slug and the file's real
/// path without resolving twice. A path that is not a script, or not in this
/// skill, is refused before anything is spawned.
pub fn script_path(slug: &str, path: &str) -> Result<(Skill, PathBuf), String> {
    let skill = resolve(slug)?;
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

/// The folder a skill lives in.
pub fn directory_of(skill: &Skill) -> Result<PathBuf, String> {
    let dir = root().join(&skill.slug);
    if dir.is_dir() {
        Ok(dir)
    } else {
        Err(format!("The folder for `{}` is gone", skill.slug))
    }
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

/// A slug no folder in the root is using.
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

/// Reads one root, skipping folders that are not skills.
///
/// A malformed `SKILL.md` is logged and skipped rather than failing the list: one
/// broken skill must not make every other skill disappear.
pub fn scan(root: &Path) -> Vec<Skill> {
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
        let files = manifest(&path);
        let under = |folder: &str| files.iter().any(|file| file.path.starts_with(folder));
        skills.push(Skill {
            slug: slug.to_string(),
            // The folder is the address *and* the label.
            name: frontmatter::humanize(slug),
            description: clean_text(&header.description),
            file_count: files.len(),
            has_scripts: files.iter().any(|file| file.is_script),
            has_references: under("references/"),
            has_assets: under("assets/") || under("templates/"),
        });
    }
    skills
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

    fn write_skill(root: &Path, slug: &str, description: &str) {
        let dir = root.join(slug);
        std::fs::create_dir_all(dir.join("scripts")).expect("dir");
        std::fs::create_dir_all(dir.join("references")).expect("refs");
        std::fs::write(
            dir.join(SKILL_FILE),
            format!("---\nname: {slug}\ndescription: \"{description}\"\n---\n\nDo it.\n"),
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
        write_skill(&root, "code-review", "when reviewing");
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

    /// A folder is the whole skill: the label is the folder's name, and the
    /// folders beside `SKILL.md` are what the row's glyphs report.
    #[test]
    fn a_folder_reads_as_one_skill() {
        let root = temp("scan");
        write_skill(&root, "code-review", "when reviewing");
        let listed = scan(&root);
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].slug, "code-review");
        assert_eq!(listed[0].name, "Code review");
        assert_eq!(listed[0].description.as_deref(), Some("when reviewing"));
        assert_eq!(listed[0].file_count, 2);
        assert!(listed[0].has_scripts);
        assert!(listed[0].has_references);
        assert!(!listed[0].has_assets);
        let _ = std::fs::remove_dir_all(&root);
    }

    /// A folder with no `SKILL.md` is not a skill, and neither is a file.
    #[test]
    fn a_folder_without_a_skill_file_is_skipped() {
        let root = temp("skip");
        std::fs::create_dir_all(root.join("notes")).expect("dir");
        std::fs::write(root.join("loose.md"), "no").expect("write");
        assert!(scan(&root).is_empty());
        let _ = std::fs::remove_dir_all(&root);
    }

    /// The label an older index printed was `name [type]`; a transcript that
    /// copies it must still find the skill.
    #[test]
    fn a_copied_index_label_resolves_to_its_slug() {
        assert_eq!(strip_type_suffix("Code review [code]"), "Code review");
        assert_eq!(strip_type_suffix("Code review"), "Code review");
        assert_eq!(strip_type_suffix("[code]"), "[code]");
    }

    #[test]
    fn resolve_matches_a_slug_or_a_label() {
        let root = temp("resolve");
        write_skill(&root, "code-review", "when reviewing");
        assert_eq!(
            resolve_at(&root, "code-review").expect("slug").slug,
            "code-review"
        );
        assert_eq!(
            resolve_at(&root, "Code review").expect("label").slug,
            "code-review"
        );
        assert_eq!(
            resolve_at(&root, "Code review [code]")
                .expect("legacy label")
                .slug,
            "code-review"
        );
        let _ = std::fs::remove_dir_all(&root);
    }
}
