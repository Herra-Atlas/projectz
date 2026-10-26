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
//! A `.zip` URL, a GitHub repository link (which is turned into that
//! repository's archive), or -- when the repository is larger than the archive
//! cap -- a link to the skill's own folder inside it. A `/tree/` link reads only
//! that folder through the contents API ([`super::github`]); a `/blob/` link
//! reads the folder holding the named file. The archive may contain one skill at
//! any depth; the shallowest `SKILL.md` is its root, so both a bare skill folder
//! and a repo with one under `skills/` work.
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

use crate::skills::{self, frontmatter, github, Skill, SKILL_FILE};

/// Largest archive, or folder fetch, this will download.
pub(super) const MAX_ARCHIVE_BYTES: usize = 16 * 1024 * 1024;

/// Largest single file after decompression.
pub(super) const MAX_FILE_BYTES: u64 = 8 * 1024 * 1024;

/// Largest total after decompression.
const MAX_TOTAL_BYTES: u64 = 24 * 1024 * 1024;

/// Most entries one archive, or folder fetch, may hold.
pub(super) const MAX_ENTRIES: usize = 300;

/// Fetches and installs a skill from `url`.
///
/// A repository link resolves through the tree, and a repository with one skill
/// installs it directly; one with several asks the caller to choose. The dialog
/// does that choosing through [`github::discover_url`] and
/// [`github::install_url_selected`]; this is the one-shot door for a link that
/// names exactly one skill, and the only door for a `.zip`.
pub async fn install_from_url(url: &str) -> Result<Skill, String> {
    let client = github::client()?;
    match github::parse(url)? {
        github::Source::Github(repository) => {
            let discovery = github::discover(&client, &repository, &|_| {}).await?;
            let [found] = discovery.skills.as_slice() else {
                return Err(format!(
                    "That repository holds {} skills. Choose one in the install dialog.",
                    discovery.skills.len()
                ));
            };
            let report = github::install_selected(
                &client,
                &repository,
                &discovery.reference,
                &[found.id.clone()],
                &|_| {},
            )
            .await?;
            let github::InstallReport { installed, failed } = report;
            installed.into_iter().next().ok_or_else(|| {
                failed
                    .into_iter()
                    .next()
                    .map(|failure| failure.message)
                    .unwrap_or_else(|| "Nothing was installed".to_string())
            })
        }
        github::Source::Zip(link) => install_zip(&client, &link).await,
    }
}

/// The archive half: a `.zip` link, downloaded and unpacked.
async fn install_zip(client: &reqwest::Client, url: &str) -> Result<Skill, String> {
    let from_github =
        url.starts_with("https://github.com/") || url.starts_with("http://github.com/");
    let response = client
        .get(url)
        .send()
        .await
        .map_err(|error| format!("Could not reach {url}: {error}"))?;
    let status = response.status();
    if !status.is_success() {
        return Err(format!("{url} answered {status}"));
    }
    if let Some(length) = response.content_length() {
        if length as usize > MAX_ARCHIVE_BYTES {
            return Err(too_large(length, from_github));
        }
    }
    let bytes = response
        .bytes()
        .await
        .map_err(|error| format!("The download failed: {error}"))?;
    if bytes.len() > MAX_ARCHIVE_BYTES {
        return Err(too_large(bytes.len() as u64, from_github));
    }
    install_archive(&bytes)
}

/// The cap, and the way past it when the link asked for a whole repository.
fn too_large(bytes: u64, from_github: bool) -> String {
    let hint = if from_github {
        " That is the whole repository; paste the skill's folder link \
         (…/tree/<branch>/<path>) so only that folder is fetched."
    } else {
        ""
    };
    format!("That archive is {bytes} bytes; the limit is {MAX_ARCHIVE_BYTES}.{hint}")
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

    install_files_with(&files, root)
}

/// Installs files already read, laid out the way an archive would be.
///
/// A GitHub folder fetch produces the same names the archive half does, so both
/// sources land here and pass through one set of checks.
pub fn install_files_with(files: &[(String, Vec<u8>)], root: &Path) -> Result<Skill, String> {
    // The folder the skill sits at inside the archive, not the destination root:
    // a repo archive puts one skill under `repo-main/`, and everything outside it
    // is beside the skill rather than in it.
    let folder = skill_root(files)?;
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
        .ok_or("The skill's folder has no SKILL.md at its root")?;
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
    for (name, contents) in files {
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
        return Err("The skill's SKILL.md was not extracted".to_string());
    }

    // Read back through the installer's own resolution rather than the
    // process-global root, so a test that installed into a temporary folder gets
    // the skill it just wrote.
    skills::resolve_at(root, &slug)
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

    /// The shape a GitHub folder fetch hands over: one `SKILL.md` under the
    /// folder's own name, and nothing else, is a whole skill.
    #[test]
    fn a_folder_holding_only_a_skill_file_installs() {
        let root = temp("body-only");
        let files = vec![(
            "brag-slim/SKILL.md".to_string(),
            b"---\nname: brag-slim\ndescription: \"Use when launching\"\n---\n\nDo it.".to_vec(),
        )];
        let skill = install_files_with(&files, &root).expect("install");
        assert_eq!(skill.slug, "brag-slim");
        assert_eq!(skill.file_count, 0);
        assert!(!skill.has_scripts);
        assert!(root.join("brag-slim/SKILL.md").is_file());
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
