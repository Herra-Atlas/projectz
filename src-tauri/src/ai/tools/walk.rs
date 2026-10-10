//! Walking the workspace for the tools that search it.
//!
//! `grep` and `glob` ask the same question of the tree -- *which files are worth
//! looking at?* -- and must answer it identically. A file one tool saw and the
//! other skipped would make a search and a name-match disagree about the same
//! folder, and the model would have no way to tell which was right. So the skip
//! rules live here once and both tools read them, rather than each carrying its
//! own copy that drifts.
//!
//! # Why not `walkdir`
//!
//! The traversal is small and the rules are the point, not the walking. A crate
//! that yields every entry would still need this file's skip list and deadline
//! bolted on, and the one custom behaviour that matters -- never following a
//! symlink, because following one can leave the workspace after the containment
//! check has already run -- is easier to guarantee in twenty lines than to
//! configure.

use std::ops::ControlFlow;
use std::path::Path;
use std::time::Instant;

/// Folders never descended into.
///
/// Build output and dependency trees are large, generated, and never what a
/// search for source code means. Skipping them is what keeps a search fast
/// without a timeout.
pub const SKIPPED_DIRECTORIES: &[&str] = &[
    ".git",
    "node_modules",
    "target",
    "dist",
    "build",
    ".next",
    ".venv",
    "venv",
    "__pycache__",
    ".cache",
    "vendor",
];

/// Longest a walk may run before it gives up and reports what it found.
///
/// A bound on time rather than on files, because it is a pathological tree -- a
/// symlink loop that slipped through, a directory with millions of entries --
/// that this exists to stop, and such a tree can be small in file count but huge
/// in syscalls. The deadline is checked once per directory, where a clock read is
/// amortised over the whole listing rather than paid per file.
pub const MAX_WALK_SECS: u64 = 20;

/// One visitable file: its absolute path, its workspace-relative slash path, and
/// its size in bytes.
///
/// The relative path is built here and not by the caller so every tool reports
/// paths the same way, and so the forward-slash rule -- a native separator would
/// be a path `read_file` cannot resolve on the platform this app ships on -- is
/// stated once.
pub struct FoundFile<'a> {
    pub absolute: &'a Path,
    pub relative: String,
    pub bytes: u64,
}

/// Whether the walk should keep going after a file.
///
/// A distinct type rather than a `bool` because the direction is easy to misread:
/// `Continue` reads as "keep walking", not "keep this file", and a reviewer
/// should not have to check the call site to know which.
pub type KeepGoing = ControlFlow<()>;

/// Walks every file under `from`, reporting each through `visit`.
///
/// Returns `true` when the whole tree was walked and `false` when the deadline
/// stopped it early, so the caller can say *why* a search ended rather than
/// letting a truncated result read as a complete one.
///
/// A symlink is never followed: following one can leave the workspace after the
/// containment check on the model's path has already passed. An unreadable
/// directory is skipped rather than failing the walk -- one permissions error
/// should not hide the matches elsewhere in the tree.
pub fn walk_files(
    root: &Path,
    from: &Path,
    deadline: Instant,
    visit: &mut dyn FnMut(FoundFile<'_>) -> KeepGoing,
) -> bool {
    if Instant::now() > deadline {
        return false;
    }
    let Ok(entries) = std::fs::read_dir(from) else {
        // An unreadable directory is invisible to the walk, not fatal to it.
        return true;
    };
    for entry in entries.flatten() {
        if Instant::now() > deadline {
            return false;
        }
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().into_owned();

        let Ok(kind) = entry.file_type() else {
            continue;
        };
        if kind.is_symlink() {
            continue;
        }
        if kind.is_dir() {
            if !SKIPPED_DIRECTORIES.contains(&name.as_str())
                && !walk_files(root, &path, deadline, visit)
            {
                return false;
            }
            continue;
        }

        // Read through metadata first so a file that cannot be stat'd is skipped
        // rather than reported with a size of zero.
        let Ok(meta) = entry.metadata() else {
            continue;
        };
        let file = FoundFile {
            absolute: &path,
            relative: relative_slash(root, &path),
            bytes: meta.len(),
        };
        if visit(file).is_break() {
            return true;
        }
    }
    true
}

/// A workspace-relative path with forward slashes on every platform.
///
/// A native `\` would be a path the model cannot hand back to `read_file`: the
/// two are only equal by luck on Windows, and the model copies what it is given.
pub fn relative_slash(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .components()
        .map(|component| component.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn root() -> PathBuf {
        use std::sync::atomic::{AtomicU32, Ordering};
        static COUNTER: AtomicU32 = AtomicU32::new(0);
        let unique = COUNTER.fetch_add(1, Ordering::Relaxed);
        let dir =
            std::env::temp_dir().join(format!("projectz-walk-{}-{unique}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("temp dir");
        dir
    }

    fn collect(root: &Path) -> Vec<String> {
        let mut seen = Vec::new();
        walk_files(
            root,
            root,
            Instant::now() + std::time::Duration::from_secs(30),
            &mut |file| {
                seen.push(file.relative);
                ControlFlow::Continue(())
            },
        );
        seen.sort();
        seen
    }

    #[test]
    fn every_file_below_the_root_is_reported_relative_and_slash_separated() {
        std::fs::write(root().join("a.rs"), "x").unwrap();
        let root = root();
        std::fs::create_dir_all(root.join("src/deep")).unwrap();
        std::fs::write(root.join("src/deep/a.rs"), "x").unwrap();
        assert_eq!(collect(&root), vec!["src/deep/a.rs".to_string()]);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn generated_and_dependency_trees_are_skipped() {
        let root = root();
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::write(root.join("src/a.rs"), "x").unwrap();
        for skipped in ["target", "node_modules", ".git"] {
            std::fs::create_dir_all(root.join(skipped)).unwrap();
            std::fs::write(root.join(skipped).join("b.rs"), "x").unwrap();
        }
        assert_eq!(collect(&root), vec!["src/a.rs".to_string()]);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_breaker_stops_the_walk_early() {
        let root = root();
        std::fs::write(root.join("a.rs"), "x").unwrap();
        std::fs::write(root.join("b.rs"), "x").unwrap();
        let mut seen = 0;
        walk_files(
            &root,
            &root,
            Instant::now() + std::time::Duration::from_secs(30),
            &mut |_| {
                seen += 1;
                ControlFlow::Break(())
            },
        );
        assert_eq!(seen, 1, "the breaker should stop after the first file");
        let _ = std::fs::remove_dir_all(&root);
    }
}
