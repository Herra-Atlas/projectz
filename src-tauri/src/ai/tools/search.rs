//! Finding text across the workspace.
//!
//! The tool a model reaches for instead of shelling out to `grep`, which is why
//! it exists rather than relying on `run_terminal`: a search is a read, so it
//! needs no approval, and its output is already shaped for a model.
//!
//! # Why not shell out to ripgrep
//!
//! `rg` is faster, but it may not be installed, and shelling out would route
//! every search through the permission gate as a shell command. Walking the
//! tree here keeps search a `Read` tool and keeps its behaviour identical
//! everywhere.

use std::path::PathBuf;

use serde_json::Value;

use super::ToolSpec;

/// Search the workspace for text.
pub const SEARCH_FILES: ToolSpec = ToolSpec {
    name: "search_files",
    description: "Search files in the workspace for text. Returns matching lines with their file path \
                  and line number. Use this to find where something is defined or used before reading \
                  a whole file.",
    parameters: r#"{
        "type": "object",
        "properties": {
            "pattern": {
                "type": "string",
                "description": "Text to look for. Matched literally, not as a regular expression."
            },
            "path": {
                "type": "string",
                "description": "Directory to search, relative to the workspace root. Defaults to the whole workspace."
            },
            "glob": {
                "type": "string",
                "description": "Only search files whose name ends with this, for example `.rs` or `.tsx`."
            },
            "max_results": {
                "type": "integer",
                "description": "Maximum matching lines to return. Defaults to 100, capped at 500."
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
            super::file::require_workspace()
                .and_then(|root| search(arguments, root)),
        ))
    },
};

/// Default and ceiling on reported matches.
///
/// The ceiling matters more than the default: an unbounded search over a large
/// tree produces megabytes of matches, and a model given those reads none of
/// them. Stopping early and saying so is more useful than the full set.
const DEFAULT_MAX_RESULTS: usize = 100;
const MAX_RESULTS: usize = 500;

/// Folders never descended into.
///
/// Build output and dependency trees are large, generated, and never what a
/// search for source code means. Skipping them is what keeps a search fast
/// without a timeout.
const SKIPPED_DIRECTORIES: &[&str] = &[
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

/// Files larger than this are skipped.
///
/// A single large binary or lockfile can hold a megabyte on one line, which would
/// make a match line useless rather than informative.
const MAX_FILE_BYTES: u64 = 2 * 1024 * 1024;

/// Longest a search may walk before it gives up and reports what it found.
const MAX_WALK_SECS: u64 = 20;

fn search(arguments: Value, root: PathBuf) -> Result<String, String> {
    let pattern = arguments
        .get("pattern")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|pattern| !pattern.is_empty())
        .ok_or("search_files requires a non-empty string `pattern`")?
        .to_lowercase();
    let start = arguments
        .get("path")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|path| !path.is_empty())
        .unwrap_or(".");
    // `ends_with` rather than a glob: a model asking for `.rs` means extension,
    // and teaching it to write `*.rs` instead would be a worse interface.
    let suffix = arguments
        .get("glob")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|glob| !glob.is_empty())
        .map(|glob| glob.trim_start_matches("*").to_lowercase());
    let limit = arguments
        .get("max_results")
        .and_then(Value::as_u64)
        .map(|value| value.clamp(1, MAX_RESULTS as u64) as usize)
        .unwrap_or(DEFAULT_MAX_RESULTS);

    let root = super::file::resolve_within(&root, start)?;
    let started = std::time::Instant::now();
    let mut matches = Vec::new();
    let mut scanned = 0usize;
    let mut truncated = false;
    let mut timed_out = false;

    walk(
        &root,
        &root,
        &pattern,
        suffix.as_deref(),
        limit,
        &mut matches,
        &mut scanned,
        &mut truncated,
        &mut timed_out,
        started,
    );

    if matches.is_empty() {
        return Ok(if timed_out {
            format!(
                "No matches for `{pattern}` in the part of the workspace searched before the \
                 time limit. Narrow the search with `path` or `glob`."
            )
        } else {
            format!("No matches for `{pattern}` in {scanned} files.")
        });
    }

    let mut out = matches.join("\n");
    out.push_str(&format!(
        "\n\n{} matching lines in {scanned} files",
        matches.len()
    ));
    if truncated || timed_out {
        // Saying why it stopped is what lets the model narrow the search instead
        // of concluding the code it wants is not there.
        out.push_str(if timed_out {
            " (search stopped at the time limit; narrow it with `path` or `glob`)"
        } else {
            " (more matches exist; narrow the search or raise `max_results`)"
        });
    }
    Ok(out)
}

/// Walks `directory`, recording matches, stopping at the limit or the deadline.
///
/// Takes both the original root and the current directory so a reported path is
/// workspace-relative. Passing the absolute path back to a model would make it
/// guess the workspace root on its next call.
#[allow(clippy::too_many_arguments)]
fn walk(
    root: &std::path::Path,
    directory: &std::path::Path,
    pattern: &str,
    suffix: Option<&str>,
    limit: usize,
    matches: &mut Vec<String>,
    scanned: &mut usize,
    truncated: &mut bool,
    timed_out: &mut bool,
    started: std::time::Instant,
) {
    if *truncated || *timed_out {
        return;
    }
    // Checked once per directory rather than once per file: this is the bound
    // that stops a deep tree from running unbounded, and a per-file clock read
    // would cost more than the walk.
    if started.elapsed() > std::time::Duration::from_secs(MAX_WALK_SECS) {
        *timed_out = true;
        return;
    }

    let Ok(entries) = std::fs::read_dir(directory) else {
        // An unreadable directory is skipped rather than failing the search: a
        // permissions error on one folder should not hide matches elsewhere.
        return;
    };
    for entry in entries.flatten() {
        if *truncated || *timed_out {
            return;
        }
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().into_owned();

        // A symlink is not followed. Following one can leave the workspace, and
        // the containment check has already run on the path the model gave.
        let Ok(kind) = entry.file_type() else {
            continue;
        };
        if kind.is_symlink() {
            continue;
        }
        if kind.is_dir() {
            if !SKIPPED_DIRECTORIES.contains(&name.as_str()) {
                walk(
                    root, &path, pattern, suffix, limit, matches, scanned, truncated, timed_out,
                    started,
                );
            }
            continue;
        }

        if suffix.is_some_and(|suffix| !name.to_lowercase().ends_with(suffix)) {
            continue;
        }
        // Read through metadata first: this is what keeps a huge lockfile from
        // being loaded to be skipped.
        match entry.metadata() {
            Ok(meta) if meta.len() <= MAX_FILE_BYTES => {}
            _ => continue,
        }
        let Ok(body) = std::fs::read_to_string(&path) else {
            continue;
        };
        *scanned += 1;

        // Reported with forward slashes on every platform. A native separator
        // would be a path `read_file` cannot resolve, because the model will copy
        // it straight back and `Path` on Windows treats `a/b` and `a\b` as the
        // same only by luck.
        let relative = path
            .strip_prefix(root)
            .unwrap_or(&path)
            .components()
            .map(|component| component.as_os_str().to_string_lossy())
            .collect::<Vec<_>>()
            .join("/");
        for (index, line) in body.lines().enumerate() {
            if !line.to_lowercase().contains(pattern) {
                continue;
            }
            if matches.len() >= limit {
                *truncated = true;
                return;
            }
            // Trimmed because a minified file's single line can be megabytes,
            // and one match should still be a readable sentence.
            let trimmed = line.trim();
            let shortened: String = trimmed.chars().take(400).collect();
            matches.push(format!(
                "{relative}:{}: {}",
                index + 1,
                if shortened.len() < trimmed.len() {
                    format!("{shortened}…")
                } else {
                    shortened
                }
            ));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn root_with(files: &[(&str, &str)]) -> PathBuf {
        use std::sync::atomic::{AtomicU32, Ordering};
        static COUNTER: AtomicU32 = AtomicU32::new(0);
        let unique = COUNTER.fetch_add(1, Ordering::Relaxed);
        let root =
            std::env::temp_dir().join(format!("projectz-search-{}-{unique}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).expect("temp dir");
        for (path, body) in files {
            let full = root.join(path);
            if let Some(parent) = full.parent() {
                std::fs::create_dir_all(parent).expect("parent");
            }
            std::fs::write(full, body).expect("write");
        }
        root
    }

    #[test]
    fn a_match_is_reported_with_its_path_and_line() {
        let root = root_with(&[("a.rs", "fn main() {\n    println!(\"hello\");\n}")]);
        let out = search(json!({ "pattern": "hello" }), root.clone()).expect("search");
        // The line number is what lets the model jump straight to it.
        assert!(out.contains("a.rs:2:"), "{out}");
        assert!(out.contains("1 matching line"), "{out}");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_search_is_case_insensitive_by_default() {
        let root = root_with(&[("a.rs", "const NAME: &str = \"x\";")]);
        let out = search(json!({ "pattern": "name" }), root.clone()).expect("search");
        assert!(out.contains("a.rs:1:"), "{out}");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn the_pattern_is_literal_rather_than_a_regex() {
        // A regex engine here would silently match nothing for a pattern like
        // `fn foo(` and the model would wrongly conclude the code is absent.
        let root = root_with(&[("a.rs", "let re = r\"^\\d+$\";")]);
        let out = search(json!({ "pattern": "^\\d+$" }), root.clone()).expect("search");
        assert!(out.contains("a.rs:1:"), "{out}");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_glob_narrows_the_search_by_suffix() {
        let root = root_with(&[("a.rs", "needle"), ("b.txt", "needle")]);
        let only_rust =
            search(json!({ "pattern": "needle", "glob": ".rs" }), root.clone()).expect("search");
        assert!(only_rust.contains("a.rs"), "{only_rust}");
        assert!(!only_rust.contains("b.txt"), "{only_rust}");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_star_glob_is_accepted_as_the_same_suffix() {
        // Models write `*.rs` unprompted; rejecting it would cost a turn.
        let root = root_with(&[("a.rs", "needle"), ("b.txt", "needle")]);
        let out =
            search(json!({ "pattern": "needle", "glob": "*.rs" }), root.clone()).expect("search");
        assert!(!out.contains("b.txt"), "{out}");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_nested_file_is_found_and_reported_relative_to_the_workspace() {
        let root = root_with(&[("src/deep/a.rs", "needle")]);
        let out = search(json!({ "pattern": "needle" }), root.clone()).expect("search");
        // Forward slashes on every platform. A native `\` here would be a path
        // the model cannot hand back to `read_file`, and the tool would be
        // useless on exactly the platform this app ships on.
        assert!(out.contains("src/deep/a.rs:1:"), "{out}");
        assert!(
            !out.contains('\\'),
            "path should use forward slashes: {out}"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn generated_and_dependency_trees_are_skipped() {
        // Without this, a search walks target/ and node_modules/ and returns
        // matches from build output instead of source.
        let root = root_with(&[
            ("src/a.rs", "needle"),
            ("target/debug/build.rs", "needle"),
            ("node_modules/pkg/index.js", "needle"),
        ]);
        let out = search(json!({ "pattern": "needle" }), root.clone()).expect("search");
        assert!(out.contains("src/a.rs"), "{out}");
        assert!(!out.contains("target"), "{out}");
        assert!(!out.contains("node_modules"), "{out}");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn results_are_capped_and_say_that_more_exist() {
        let root = root_with(&[("a.rs", "needle\nneedle\nneedle\nneedle")]);
        let out = search(
            json!({ "pattern": "needle", "max_results": 2 }),
            root.clone(),
        )
        .expect("search");
        assert_eq!(out.matches("a.rs:").count(), 2, "{out}");
        // Silence here would let the model conclude the fourth line does not exist.
        assert!(out.contains("more matches exist"), "{out}");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn no_matches_says_how_much_was_searched() {
        let root = root_with(&[("a.rs", "nothing here")]);
        let out = search(json!({ "pattern": "absent" }), root.clone()).expect("search");
        assert!(out.contains("No matches"), "{out}");
        assert!(out.contains("1 files"), "{out}");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn an_empty_pattern_is_rejected_rather_than_matching_everything() {
        let root = root_with(&[("a.rs", "x")]);
        assert!(search(json!({ "pattern": "  " }), root.clone())
            .expect_err("empty")
            .contains("pattern"));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_search_path_outside_the_workspace_is_refused() {
        let root = root_with(&[("a.rs", "x")]);
        assert!(
            search(json!({ "pattern": "x", "path": "../.." }), root.clone())
                .expect_err("escape")
                .contains("outside")
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_very_long_match_line_is_shortened() {
        let root = root_with(&[("a.rs", &format!("needle{}", "x".repeat(1000)))]);
        let out = search(json!({ "pattern": "needle" }), root.clone()).expect("search");
        assert!(out.contains('…'), "{out}");
        assert!(out.len() < 1000, "the match line was not shortened");
        let _ = std::fs::remove_dir_all(&root);
    }
}
