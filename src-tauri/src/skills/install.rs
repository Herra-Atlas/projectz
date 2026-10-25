//! Installing a skill from a link.
//!
//! # Why this is a user action, not a tool
//!
//! A skill can carry scripts, so installing one is authorising code to run on
//! this machine later. The app's existing rule for MCP servers applies unchanged:
//! an agent that can install executable material can arrange to run whatever it
//! liked outside the prompt the user is watching. So this fetches and unpacks for
//! a button press, and the folder it writes starts **untrusted** -- its body and
//! references are usable, its `scripts/` are not until the user says so.
//!
//! # What is accepted
//!
//! A `.zip` URL, or a GitHub repository link (which is turned into that
//! repository's archive). The archive may contain one skill at any depth; the
//! shallowest `SKILL.md` is its root, so both a bare skill folder and a repo with
//! one under `skills/` work.
//!
//! # The guards
//!
//! Archive entries are the classic path-traversal surface: a name containing
//! `..`, an absolute path, a drive letter or a symlink can write outside the
//! destination. Every entry is checked here, links are skipped, and the total is
//! capped before a byte is written. The caps are deliberately small -- a skill is
//! text and a few assets, not a disk image.

use std::io::{Cursor, Read};
use std::path::{Component, Path};

use crate::skills::{self, frontmatter, Skill, SKILL_FILE};

/// Largest archive this will download.
const MAX_ARCHIVE_BYTES: usize = 16 * 1024 * 1024;

/// Largest single file after decompression.
const MAX_FILE_BYTES: u64 = 8 * 1024 * 1024;

/// Largest total after decompression.
const MAX_TOTAL_BYTES: u64 = 24 * 1024 * 1024;

/// Most entries one archive may hold.
const MAX_ENTRIES: usize = 300;

/// Fetches and installs a skill from `url`.
pub async fn install_from_url(url: &str) -> Result<Skill, String> {
    let url = normalize(url)?;
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(45))
        .user_agent("projectz-skills/1")
        .build()
        .map_err(|error| format!("Could not start the download: {error}"))?;
    let response = client
        .get(&url)
        .send()
        .await
        .map_err(|error| format!("Could not reach {url}: {error}"))?;
    let status = response.status();
    if !status.is_success() {
        return Err(format!("{url} answered {status}"));
    }
    if let Some(length) = response.content_length() {
        if length as usize > MAX_ARCHIVE_BYTES {
            return Err(format!(
                "That archive is {length} bytes; the limit is {MAX_ARCHIVE_BYTES}."
            ));
        }
    }
    let bytes = response
        .bytes()
        .await
        .map_err(|error| format!("The download failed: {error}"))?;
    if bytes.len() > MAX_ARCHIVE_BYTES {
        return Err(format!(
            "That archive is {} bytes; the limit is {MAX_ARCHIVE_BYTES}.",
            bytes.len()
        ));
    }
    install_archive(&bytes)
}

/// Installs an archive already in memory.
pub fn install_archive(bytes: &[u8]) -> Result<Skill, String> {
    install_archive_with(bytes, &skills::root())
}

/// The testable half: install into an explicit root.
pub fn install_archive_with(bytes: &[u8], root: &Path) -> Result<Skill, String> {
    if !bytes.starts_with(b"PK") {
        return Err(
            "That link did not return a zip archive. Point it at a .zip file or a GitHub repository."
                .to_string(),
        );
    }
    let mut archive = zip::ZipArchive::new(Cursor::new(bytes))
        .map_err(|error| format!("That archive could not be opened: {error}"))?;
    if archive.len() > MAX_ENTRIES {
        return Err(format!(
            "That archive holds {} files; the limit is {MAX_ENTRIES}.",
            archive.len()
        ));
    }

    // Read every entry into memory first, so the shape can be inspected before a
    // single file is written. A 16 MB archive is a bounded amount to hold.
    let mut files: Vec<(String, Vec<u8>)> = Vec::new();
    let mut total = 0u64;
    for index in 0..archive.len() {
        let mut entry = archive
            .by_index(index)
            .map_err(|error| format!("The archive entry could not be read: {error}"))?;
        let name = entry.name().replace('\\', "/");
        // A link is not content. Refused rather than followed, because following
        // one is exactly how an archive writes outside its own tree.
        if entry
            .unix_mode()
            .map(|mode| mode & 0o170000 == 0o120000)
            .unwrap_or(false)
        {
            continue;
        }
        if entry.is_dir() {
            continue;
        }
        if entry.size() > MAX_FILE_BYTES {
            return Err(format!("`{name}` is larger than a skill file may be"));
        }
        total = total.saturating_add(entry.size());
        if total > MAX_TOTAL_BYTES {
            return Err("The archive is too large once unpacked".to_string());
        }
        let mut contents = Vec::with_capacity(entry.size() as usize);
        entry
            .read_to_end(&mut contents)
            .map_err(|error| format!("Could not read `{name}`: {error}"))?;
        files.push((name, contents));
    }

    // The folder the skill sits at inside the archive, not the destination root:
    // a repo archive puts one skill under `repo-main/`, and everything outside it
    // is beside the skill rather than in it.
    let folder = skill_root(&files)?;
    let prefix = if folder.is_empty() {
        String::new()
    } else {
        format!("{folder}/")
    };

    // The header supplies identity and the trigger. Both are required: a skill
    // the model cannot decide to read is not installable.
    let markdown = files
        .iter()
        .find(|(name, _)| name == &format!("{prefix}{SKILL_FILE}"))
        .map(|(_, bytes)| bytes)
        .ok_or("The archive has no SKILL.md at the skill's root")?;
    let text = std::str::from_utf8(markdown)
        .map_err(|_| "The skill's SKILL.md is not UTF-8 text".to_string())?;
    let (header, _) = frontmatter::parse(text)?;
    let slug = if header.name.trim().is_empty() {
        // A header without a name is a plain markdown note. The folder it was
        // packaged in is the only other identity available.
        skills::slugify(folder.rsplit('/').next().unwrap_or("skill"))
    } else {
        validate_slug(&header.name)?
    };
    if header.description.trim().is_empty() {
        return Err(
            "That skill has no description, which is what the model chooses it by".to_string(),
        );
    }
    if root.join(&slug).exists() {
        return Err(format!(
            "A skill named `{slug}` is already installed. Delete or rename it first."
        ));
    }

    let destination = root.join(&slug);
    for (name, contents) in &files {
        let Some(relative) = name.strip_prefix(&prefix) else {
            // A file beside the skill rather than in it -- a README for the repo,
            // a license, another skill. Not part of this one.
            continue;
        };
        let Some(relative) = safe_relative(relative) else {
            return Err(format!(
                "`{name}` is outside the skill's folder and was refused"
            ));
        };
        if relative.is_empty() {
            continue;
        }
        let target = destination.join(&relative);
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|error| format!("Could not create {}: {error}", parent.display()))?;
        }
        crate::ai::tools::write::atomic_write(&target, contents)?;
    }
    // A skill with no SKILL.md would have been refused above; this is the guard
    // for an archive whose only copy was skipped as a link.
    if !destination.join(SKILL_FILE).is_file() {
        let _ = std::fs::remove_dir_all(&destination);
        return Err("The archive's SKILL.md was not extracted".to_string());
    }

    // Read back through the installer's own resolution rather than the
    // process-global root, so a test that installed into a temporary folder gets
    // the skill it just wrote.
    skills::resolve_at(root, &slug)
}

/// Turns a GitHub link into its archive, and refuses anything that is not HTTP.
pub fn normalize(url: &str) -> Result<String, String> {
    let url = url.trim();
    if !(url.starts_with("https://") || url.starts_with("http://")) {
        return Err("Paste an https:// link to a .zip or a GitHub repository.".to_string());
    }
    let without_scheme = url
        .trim_start_matches("https://")
        .trim_start_matches("http://");
    let mut parts = without_scheme.split('/');
    let host = parts.next().unwrap_or_default().to_lowercase();
    if host == "github.com" || host == "www.github.com" {
        let owner = parts.next().unwrap_or_default();
        let repo = parts.next().unwrap_or_default();
        if owner.is_empty() || repo.is_empty() {
            return Err("That GitHub link does not name a repository".to_string());
        }
        let repo = repo.trim_end_matches(".git");
        return Ok(format!(
            "https://github.com/{owner}/{repo}/archive/HEAD.zip"
        ));
    }
    if !url.to_lowercase().contains(".zip") {
        return Err("Paste a .zip link or a GitHub repository link.".to_string());
    }
    Ok(url.to_string())
}

/// The shallowest folder holding a `SKILL.md`, as a prefix with no trailing slash.
fn skill_root(files: &[(String, Vec<u8>)]) -> Result<String, String> {
    let mut roots = files
        .iter()
        .filter_map(|(name, _)| {
            let name = name.replace('\\', "/");
            if name == SKILL_FILE {
                Some(String::new())
            } else {
                // The separator matters: `mySKILL.md` is not a skill, and a bare
                // `strip_suffix` would treat its first two letters as the root.
                name.strip_suffix(&format!("/{SKILL_FILE}"))
                    .map(str::to_string)
            }
        })
        .collect::<Vec<_>>();
    roots.sort_by_key(|prefix| prefix.matches('/').count());
    roots.dedup();
    let Some(root) = roots.first() else {
        return Err("The archive contains no SKILL.md".to_string());
    };
    // Two skills in one archive is ambiguous: installing one and silently
    // dropping the other is worse than asking for a link to one of them.
    let same_depth = roots
        .iter()
        .take_while(|prefix| prefix.matches('/').count() == root.matches('/').count())
        .count();
    if same_depth > 1 {
        return Err(format!(
            "The archive holds {} skills. Point the link at one of them.",
            same_depth
        ));
    }
    Ok(root.clone())
}

/// A path safe to join under the destination, or `None`.
fn safe_relative(path: &str) -> Option<String> {
    let candidate = Path::new(path);
    // `is_absolute` alone is not enough on Windows: `/etc/passwd` has a root but
    // no prefix, so it is not "absolute" there while still being a rooted path.
    if candidate.is_absolute() || candidate.has_root() {
        return None;
    }
    let mut parts = Vec::new();
    for component in candidate.components() {
        match component {
            Component::Normal(part) => {
                let part = part.to_string_lossy();
                // A Windows drive prefix arrives as a normal component on Unix.
                if part.contains(':') || part == ".." {
                    return None;
                }
                parts.push(part.into_owned());
            }
            Component::CurDir => {}
            _ => return None,
        }
    }
    Some(parts.join("/"))
}

/// A slug the folder name can carry, from the spec's character set.
fn validate_slug(name: &str) -> Result<String, String> {
    let slug = name.trim();
    let valid = !slug.is_empty()
        && slug.len() <= 64
        && !slug.starts_with('-')
        && !slug.ends_with('-')
        && !slug.contains("--")
        && slug.chars().all(|character| {
            character.is_ascii_lowercase() || character.is_ascii_digit() || character == '-'
        });
    if !valid {
        return Err(format!(
            "`{name}` is not a valid skill name. The spec allows lowercase letters, digits and \
             hyphens, and the name has to match the folder."
        ));
    }
    Ok(slug.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn temp(label: &str) -> std::path::PathBuf {
        use std::sync::atomic::{AtomicU32, Ordering};
        static COUNTER: AtomicU32 = AtomicU32::new(0);
        let unique = COUNTER.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "projectz-install-{}-{label}-{unique}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).expect("temp dir");
        path
    }

    fn archive(entries: &[(&str, &str)]) -> Vec<u8> {
        let mut buffer = Cursor::new(Vec::new());
        {
            let mut writer = zip::ZipWriter::new(&mut buffer);
            let options = zip::write::SimpleFileOptions::default();
            for (name, contents) in entries {
                writer.start_file(*name, options).expect("entry");
                writer.write_all(contents.as_bytes()).expect("write");
            }
            writer.finish().expect("finish");
        }
        buffer.into_inner()
    }

    #[test]
    fn a_repository_link_becomes_an_archive_link() {
        assert_eq!(
            normalize("https://github.com/agent/skills").expect("url"),
            "https://github.com/agent/skills/archive/HEAD.zip"
        );
        assert_eq!(
            normalize("https://github.com/agent/skills/tree/main/pdf").expect("url"),
            "https://github.com/agent/skills/archive/HEAD.zip"
        );
        assert!(normalize("ftp://example.test/skill.zip").is_err());
        assert!(normalize("https://example.test/skill").is_err());
    }

    /// The traversal surface: `..`, an absolute path and a drive letter are all
    /// refused before anything is written.
    #[test]
    fn an_escaping_entry_is_refused() {
        assert!(safe_relative("../secret").is_none());
        assert!(safe_relative("/etc/passwd").is_none());
        assert!(safe_relative("C:/Windows/win.ini").is_none());
        assert!(safe_relative("./scripts/run.ps1").is_some());
        assert_eq!(
            safe_relative("scripts/run.ps1").as_deref(),
            Some("scripts/run.ps1")
        );
    }

    #[test]
    fn the_shallowest_skill_folder_is_the_root() {
        let files = vec![
            ("repo-main/SKILL.md".to_string(), Vec::new()),
            ("repo-main/scripts/run.ps1".to_string(), Vec::new()),
        ];
        assert_eq!(skill_root(&files).expect("root"), "repo-main");
    }

    #[test]
    fn two_skills_in_one_archive_are_refused() {
        let files = vec![
            ("a/SKILL.md".to_string(), Vec::new()),
            ("b/SKILL.md".to_string(), Vec::new()),
        ];
        assert!(skill_root(&files)
            .expect_err("ambiguous")
            .contains("2 skills"));
    }

    /// The whole path, against a temporary root: the body lands, the script is
    /// kept, and the label comes from the folder rather than a header field.
    #[test]
    fn an_archive_installs_as_a_skill_folder() {
        let root = temp("install");
        let bytes = archive(&[
            (
                "pdf-tools/SKILL.md",
                "---\nname: pdf-tools\ndescription: \"Use when handling PDFs\"\n---\n\nRead the PDF.",
            ),
            ("pdf-tools/scripts/extract.py", "print('hi')"),
            ("README.md", "repo readme"),
        ]);

        let skill = install_archive_with(&bytes, &root).expect("install");
        assert_eq!(skill.slug, "pdf-tools");
        assert_eq!(skill.name, "Pdf tools");
        assert!(skill.has_scripts);
        assert_eq!(skill.file_count, 1);
        assert!(root.join("pdf-tools/scripts/extract.py").is_file());
        // A file beside the skill folder is not copied into it.
        assert!(!root.join("pdf-tools/README.md").exists());
        let _ = std::fs::remove_dir_all(&root);
    }

    /// Installing the same skill twice is refused, because the second write would
    /// silently replace what the user may have edited since the first.
    #[test]
    fn a_second_install_of_the_same_skill_is_refused() {
        let root = temp("duplicate");
        let bytes = archive(&[(
            "pdf-tools/SKILL.md",
            "---\nname: pdf-tools\ndescription: \"Use when handling PDFs\"\n---\n\nBody.",
        )]);
        install_archive_with(&bytes, &root).expect("first");
        assert!(install_archive_with(&bytes, &root)
            .expect_err("second")
            .contains("already installed"));
        let _ = std::fs::remove_dir_all(&root);
    }
}
