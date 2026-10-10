//! Finding files by path pattern.
//!
//! `grep` answers "where does this text appear"; this answers "where is this
//! file". Without it a model has to `list_dir` its way down a tree to find a file
//! it can already name, which costs a turn per level and still may miss one.
//!
//! It reuses [`super::walk`] and [`super::grep::Pattern`] so a pattern means the
//! same thing whether it is used to search inside files or to find them, and so
//! the skip rules -- no `node_modules`, no `target`, never follow a symlink -- are
//! the same in both.

use std::ops::ControlFlow;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use serde_json::Value;

use super::grep::Pattern;
use super::walk::{walk_files, MAX_WALK_SECS};
use super::ToolSpec;

/// Find files whose path matches a glob.
pub const GLOB: ToolSpec = ToolSpec {
    name: "glob",
    description: "Find files in the workspace by path pattern, sorted newest first. Use `**` to \
                  match across folders, for example `src/**/*.ts` or `**/*.rs`. Use this to locate \
                  a file you can name but not place; use `grep` to find files by their contents.",
    parameters: r#"{
        "type": "object",
        "properties": {
            "pattern": {
                "type": "string",
                "description": "Glob pattern to match paths against, for example `**/*.rs` or `src/**/mod.rs`. A pattern with no `/` matches file names anywhere."
            },
            "path": {
                "type": "string",
                "description": "Directory to search under, relative to the workspace root. Defaults to the whole workspace."
            }
        },
        "required": ["pattern"],
        "additionalProperties": false
    }"#,
    effect: super::Effect::Read,
    command_argument: None,
    execute: |arguments, context| {
        let _ = &context;
        Box::pin(std::future::ready(
            super::file::require_workspace().and_then(|root| find(arguments, root)),
        ))
    },
};

/// Most paths one call returns.
///
/// A ceiling rather than a page size: a model that matched more than this is
/// holding a pattern too broad to act on, and a shorter, newest-first list is more
/// useful than the whole set. The count says how many there were.
const MAX_RESULTS: usize = 500;

fn find(arguments: Value, root: PathBuf) -> Result<String, String> {
    let raw = arguments
        .get("pattern")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|pattern| !pattern.is_empty())
        .ok_or("glob requires a non-empty string `pattern`")?;
    let pattern = Pattern::new(raw)?;

    let start = arguments
        .get("path")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|path| !path.is_empty())
        .unwrap_or(".");
    let directory = super::file::resolve_within(&root, start)?;

    let deadline = Instant::now() + Duration::from_secs(MAX_WALK_SECS);
    // Collected with a modification time, because the caller wants the newest
    // files first and a walk order has no meaning to a person.
    let mut found: Vec<(std::time::SystemTime, String)> = Vec::new();
    let completed = walk_files(&root, &directory, deadline, &mut |file| {
        if !pattern.allows(&file.relative) {
            return ControlFlow::Continue(());
        }
        let modified = file
            .absolute
            .metadata()
            .and_then(|meta| meta.modified())
            .unwrap_or(std::time::UNIX_EPOCH);
        found.push((modified, file.relative));
        ControlFlow::Continue(())
    });

    // The walk visits files one at a time, so a cap is applied after sorting; the
    // deadline is what bounds the walk itself, so a huge tree still terminates.
    found.sort_by(|left, right| right.0.cmp(&left.0));

    if found.is_empty() {
        return Ok(if completed {
            format!("No files match `{raw}`.")
        } else {
            format!("No files match `{raw}` in the part of the workspace searched before the time limit.")
        });
    }

    let total = found.len();
    let shown = total.min(MAX_RESULTS);
    let mut out = found[..shown]
        .iter()
        .map(|(_, path)| path.clone())
        .collect::<Vec<_>>()
        .join("\n");
    out.push_str(&format!("\n\n{shown} of {total} files"));
    if total > shown {
        out.push_str(". Narrow the pattern to see the rest.");
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn root_with(files: &[&str]) -> PathBuf {
        use std::sync::atomic::{AtomicU32, Ordering};
        static COUNTER: AtomicU32 = AtomicU32::new(0);
        let unique = COUNTER.fetch_add(1, Ordering::Relaxed);
        let root =
            std::env::temp_dir().join(format!("projectz-glob-{}-{unique}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).expect("temp dir");
        for path in files {
            let full = root.join(path);
            if let Some(parent) = full.parent() {
                std::fs::create_dir_all(parent).expect("parent");
            }
            std::fs::write(full, "x").expect("write");
        }
        root
    }

    #[test]
    fn a_glob_finds_files_across_subtrees() {
        let root = root_with(&["src/a.rs", "src/deep/b.rs", "src/c.txt"]);
        let out = find(json!({ "pattern": "**/*.rs" }), root.clone()).expect("glob");
        assert!(out.contains("src/a.rs"), "{out}");
        assert!(out.contains("src/deep/b.rs"), "{out}");
        assert!(!out.contains("c.txt"), "{out}");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_name_only_pattern_matches_anywhere() {
        let root = root_with(&["a/mod.rs", "b/mod.rs", "b/lib.rs"]);
        let out = find(json!({ "pattern": "mod.rs" }), root.clone()).expect("glob");
        assert!(out.contains("a/mod.rs"), "{out}");
        assert!(out.contains("b/mod.rs"), "{out}");
        assert!(!out.contains("lib.rs"), "{out}");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_search_path_bounds_the_walk_and_is_refused_outside_the_workspace() {
        let root = root_with(&["src/a.rs", "other/b.rs"]);
        let out = find(json!({ "pattern": "**/*.rs", "path": "src" }), root.clone()).expect("glob");
        assert!(out.contains("src/a.rs"), "{out}");
        assert!(!out.contains("other/b.rs"), "{out}");
        assert!(
            find(json!({ "pattern": "*.rs", "path": ".." }), root.clone())
                .expect_err("escape")
                .contains("outside")
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn generated_trees_are_skipped() {
        let root = root_with(&[
            "src/a.rs",
            "target/generated.rs",
            "node_modules/pkg/index.js",
        ]);
        let out = find(json!({ "pattern": "**/*.rs" }), root.clone()).expect("glob");
        assert!(out.contains("src/a.rs"), "{out}");
        assert!(!out.contains("target"), "{out}");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn no_matches_is_reported_plainly() {
        let root = root_with(&["a.txt"]);
        assert!(find(json!({ "pattern": "**/*.rs" }), root.clone())
            .expect("glob")
            .contains("No files match"));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn an_empty_pattern_is_refused() {
        let root = root_with(&["a.rs"]);
        assert!(find(json!({ "pattern": "  " }), root.clone())
            .expect_err("empty")
            .contains("pattern"));
        let _ = std::fs::remove_dir_all(&root);
    }
}
