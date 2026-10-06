//! Reading files from the workspace.
//!
//! Line numbers are included because a model that needs to refer to a specific
//! line has to be able to name it, and "near the top of the file" is not a
//! usable pointer once the next edit shifts everything down by a line. They are
//! what [`super::write::edit_lines`] takes as its arguments, so the format is
//! not decoration -- it is the contract between reading and editing.
//!
//! # Path safety
//!
//! Paths are resolved against the workspace root and a resolved path outside it
//! is refused. This is the only barrier between a model and the whole disk, so
//! it is checked after symlink resolution rather than on the string the caller
//! passed -- a path that looks contained can still resolve out of the tree.
//!
//! Refusal happens here as well as in the permission policy. A gate the tool can
//! forget is not a gate, and the two can disagree about what is safe; the
//! stricter of the two applies.

use std::path::{Component, Path, PathBuf};

use serde_json::Value;

use super::ToolSpec;

/// Workspace-relative paths never resolve above the workspace.
///
/// There is no `-` escape and no absolute-path form: a model that must read
/// something outside the workspace has no way to ask for it here, which is the
/// intended behaviour. Adding one would need a deliberate decision rather than a
/// path parser that happens to accept `..`.
pub const READ_FILE: ToolSpec = ToolSpec {
    name: "read_file",
    description:
        "Read a text file from the workspace. Returns the content with line numbers, \
                  one line per numbered row. Use offset and limit to read part of a large file \
                  instead of the whole thing. Reading is preferred over guessing a file's contents.",
    parameters: r#"{
        "type": "object",
        "properties": {
            "path": {
                "type": "string",
                "description": "Path to the file, relative to the workspace root. Absolute paths are not accepted."
            },
            "offset": {
                "type": "integer",
                "description": "1-based line number to start from. Omit to start at the first line."
            },
            "limit": {
                "type": "integer",
                "description": "Maximum number of lines to return. Omit for all remaining lines."
            }
        },
        "required": ["path"],
        "additionalProperties": false
    }"#,
    effect: super::Effect::Read,
    // A file read is not a command line, so the permission policy has nothing to
    // classify here. Its safety comes from the path containment check in `run`,
    // not from inspecting text.
    command_argument: None,
    execute: |arguments, context| {
        // A filesystem read is synchronous, so the future is already complete.
        // Boxing it anyway is what keeps every tool on one signature; a model
        // may call several in one round and they must be awaited the same way.
        let _ = &context;
        Box::pin(std::future::ready(
            require_workspace().and_then(|root| run(arguments, root)),
        ))
    },
};

/// The root every relative path is resolved against.
///
/// A thin name over [`super::workspace::root`], kept because every tool calls it
/// and `workspace::root` reads as a verb where a tool wants a noun. The value is
/// the folder the user picked, not the process's launch directory -- see
/// [`super::workspace`] for why that distinction is load-bearing.
pub fn workspace() -> Option<PathBuf> {
    super::workspace::root()
}

/// The root, or the reason there isn't one.
///
/// The shape every tool wants, which is why it exists rather than each tool
/// unwrapping the `Option` itself. Turning "no workspace" into a tool *result*
/// rather than a panic is the point: the loop reports a failing tool's text back
/// to the model as its answer and carries on, so the model is told to ask the
/// user to open a folder instead of the run dying.
pub fn require_workspace() -> Result<PathBuf, String> {
    workspace().ok_or_else(|| NO_WORKSPACE.to_string())
}

/// The message a tool gives when no workspace is open.
///
/// One string for every tool, so the model is told the same thing regardless of
/// which tool it reached for. It names the fix rather than just the refusal: a
/// model told "no workspace" alone may report that to the user as though nothing
/// could be done, whereas one told a folder can be opened will say so usefully.
pub const NO_WORKSPACE: &str =
    "No workspace is open, so there is no folder to read or write in. Ask the user to open a \
     folder first. Do not guess a path and do not suggest one.";

fn run(arguments: Value, root: PathBuf) -> Result<String, String> {
    let path = arguments
        .get("path")
        .and_then(Value::as_str)
        .ok_or("read_file requires a string `path`")?;
    let offset = positive_usize(arguments.get("offset"), "offset")?.unwrap_or(1);
    // A limit the caller did not ask for is not "no limit". Reading a whole
    // 5,000-line file spends the entire context window on material the model
    // then has to skip, and the shared output cap would then cut the *middle*
    // out of it -- so the model would receive the head and the tail with a hole
    // between them and no way to ask for what was missing. A bounded default
    // that reports what it withheld is both cheaper and more honest: the model
    // can page to the region it actually wants.
    let limit =
        Some(positive_usize(arguments.get("limit"), "limit")?.unwrap_or(DEFAULT_READ_LINES));

    let resolved = resolve_within(&root, path)?;
    let body = std::fs::read_to_string(&resolved)
        .map_err(|error| format!("Could not read {}: {error}", resolved.display()))?;

    Ok(numbered_slice(&body, offset, limit))
}

/// Lines returned by a read that named no `limit`.
///
/// Roughly the same shape as the largest window a model will usefully read in
/// one call -- a long file's worth of context, rather than the whole file. Not
/// a hard limit: an explicit `limit` above this is honoured, because a caller
/// that asked for 4,000 lines meant it and the answer should be a window, not a
/// refusal.
const DEFAULT_READ_LINES: usize = 2_000;

/// List a directory's entries.
pub const LIST_DIR: ToolSpec = ToolSpec {
    name: "list_dir",
    description:
        "List the files and folders in a workspace directory. Use this to find out what is \
                  there before reading a specific file, or to see the shape of a project.",
    parameters: r#"{
        "type": "object",
        "properties": {
            "path": {
                "type": "string",
                "description": "Directory to list, relative to the workspace root. Defaults to the workspace root."
            }
        },
        "required": [],
        "additionalProperties": false
    }"#,
    effect: super::Effect::Read,
    command_argument: None,
    execute: |arguments, context| {
        let _ = &context;
        Box::pin(std::future::ready(
            require_workspace().and_then(|root| list_dir(arguments, root)),
        ))
    },
};

/// Largest number of entries one listing returns.
///
/// 100 rather than 500. Past a hundred entries a listing stops answering "what
/// is this folder for" and starts costing a turn per screenful, so the entries
/// the model actually wanted are the ones it has to go back and narrow for.
/// The cap is reported in the output ("N of M entries") and points at
/// `search_files`, so hitting it reads as a direction rather than as a wall.
const MAX_ENTRIES: usize = 100;

fn list_dir(arguments: Value, root: PathBuf) -> Result<String, String> {
    // An absent path means the workspace root, which is the common case and the
    // one a model exploring a project wants first.
    let requested = arguments
        .get("path")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|path| !path.is_empty())
        .unwrap_or(".");
    let resolved = resolve_within(&root, requested)?;

    let mut entries = std::fs::read_dir(&resolved)
        .map_err(|error| format!("Could not list {}: {error}", resolved.display()))?
        .map(|entry| {
            entry.map(|entry| {
                // A directory's size is meaningless and its link count varies, so
                // only files report one. `file_type` falls back to a read rather
                // than assuming, because guessing wrong would mark a file as a
                // folder in the one place a model is reading the shape.
                let is_dir = entry
                    .file_type()
                    .map(|kind| kind.is_dir())
                    .unwrap_or_else(|_| resolved.join(entry.file_name()).is_dir());
                let name = entry.file_name().to_string_lossy().into_owned();
                let size = (!is_dir)
                    .then(|| entry.metadata().ok().map(|meta| meta.len()))
                    .flatten();
                (name, is_dir, size)
            })
        })
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| format!("Could not list {}: {error}", resolved.display()))?;

    // Folders first, then alphabetical. A model reading this is looking for a
    // shape, and an order that shuffles between calls costs it a turn.
    entries.sort_by(|left, right| {
        right
            .1
            .cmp(&left.1)
            .then_with(|| left.0.to_lowercase().cmp(&right.0.to_lowercase()))
    });

    let total = entries.len();
    let shown = total.min(MAX_ENTRIES);
    let mut out = String::new();
    if shown == 0 {
        out.push_str("The directory is empty.");
        return Ok(out);
    }
    for (name, is_dir, size) in &entries[..shown] {
        match (is_dir, size) {
            (true, _) => out.push_str(&format!("{name}/\n")),
            (false, Some(bytes)) => out.push_str(&format!("{name}  ({bytes} bytes)\n")),
            (false, None) => out.push_str(&format!("{name}\n")),
        }
    }
    out.push_str(&format!("\n{shown} of {total} entries"));
    if total > shown {
        out.push_str(". Narrow with search_files rather than listing a larger directory.");
    }
    Ok(out)
}

/// Reads a positive `usize` from an optional argument.
///
/// Zero and negative values are rejected rather than clamped. A `limit` of zero
/// means "return nothing", which is never what a model asking for a slice meant,
/// and silently returning the whole file instead would be worse than an error.
fn positive_usize(value: Option<&Value>, field: &str) -> Result<Option<usize>, String> {
    match value {
        None => Ok(None),
        Some(Value::Number(number)) => {
            let parsed = number
                .as_u64()
                .and_then(|value| usize::try_from(value).ok())
                .ok_or_else(|| format!("`{field}` must be a positive whole number"))?;
            if parsed == 0 {
                return Err(format!("`{field}` must be at least 1"));
            }
            Ok(Some(parsed))
        }
        Some(_) => Err(format!("`{field}` must be a number")),
    }
}

/// Resolves a workspace-relative path, refusing anything that escapes.
///
/// Checks the path component-by-component *and* re-checks after symlink
/// resolution: a `link` inside the workspace pointing at `/etc` passes the first
/// test and fails the second.
pub(crate) fn resolve_within(root: &Path, requested: &str) -> Result<PathBuf, String> {
    let candidate = Path::new(requested);
    if candidate.is_absolute() || candidate.has_root() {
        return Err(format!(
            "{requested} must be relative to the workspace root, not an absolute path"
        ));
    }
    if candidate
        .components()
        .any(|component| matches!(component, Component::ParentDir))
    {
        return Err(format!(
            "{requested} points outside the workspace. Paths may not contain `..`"
        ));
    }

    let joined = root.join(candidate);
    // `canonicalize` fails when the file does not exist yet. Resolving the
    // deepest existing ancestor and re-appending the rest still catches a
    // symlinked parent directory, which is the case that matters.
    let existing = deepest_existing_ancestor(&joined);
    let canonical_root = root
        .canonicalize()
        .map_err(|error| format!("Workspace root is unreadable: {error}"))?;
    let canonical_existing = existing
        .canonicalize()
        .map_err(|error| format!("Could not resolve {}: {error}", existing.display()))?;

    if !canonical_existing.starts_with(&canonical_root) {
        return Err(format!(
            "{requested} resolves outside the workspace and was refused"
        ));
    }
    Ok(joined)
}

/// The longest prefix of `path` that exists on disk, plus the missing remainder.
fn deepest_existing_ancestor(path: &Path) -> PathBuf {
    let mut existing = path.to_path_buf();
    let mut tail: Vec<std::ffi::OsString> = Vec::new();
    // Bounded by the path's own components, so this terminates even on a path
    // that does not exist at all.
    while !existing.exists() {
        let Some(name) = existing.file_name().map(std::ffi::OsString::from) else {
            return path.to_path_buf();
        };
        tail.push(name);
        if !existing.pop() {
            return path.to_path_buf();
        }
    }
    existing
}

/// Renders a line-numbered slice of a file.
///
/// Public because [`super::write::EDIT_LINES`] reports a refused edit's current
/// lines in exactly this format: the model has to be able to read the corrected
/// numbers straight back out of the error, and a second rendering would drift
/// from the one its successful reads used.
///
/// The format is the one Codex and Claude Code use: the number, a tab, then the
/// line. Two reasons, and the second is the one that matters.
///
/// First, the number is on the **left**, where a model's own training put it.
/// The earlier rendering right-aligned the number *within its column*, which read
/// as numbers sitting on the right of the content, and every harness convention
/// contradicts it.
///
/// Second -- and this is the actual argument -- a tab-separated number is a
/// clean re-parse. Given the rows back, the exact source line is the text after
/// the first tab, with no separator character to strip out of the content. With
/// a ` | ` separator the model has to know which character is the delimiter
/// before it can trust a line it read, and a source file that legitimately
/// contains ` | ` in a table makes that guess wrong. One tab, always ours,
/// never the file's.
///
/// Left-aligned rather than padded, and deliberately: padding exists so columns
/// line up visually, but a padded number is no longer the number, and the whole
/// value of printing it is that `edit_lines` can read it straight back. The
/// transcript renders these in a monospace face, where alignment is free.
///
/// A trailing newline is dropped so the last line is not reported as an extra
/// blank one.
pub fn numbered_slice(body: &str, offset: usize, limit: Option<usize>) -> String {
    let lines: Vec<&str> = body.lines().collect();
    let start = offset.saturating_sub(1).min(lines.len());
    let end = limit
        .map(|limit| (start + limit).min(lines.len()))
        .unwrap_or(lines.len());

    let mut out = String::new();
    for (index, line) in lines[start..end].iter().enumerate() {
        out.push_str(&(start + index + 1).to_string());
        out.push('\t');
        out.push_str(line);
        out.push('\n');
    }
    if out.is_empty() {
        return if body.is_empty() {
            "The file is empty.".to_string()
        } else {
            format!(
                "The file has {len} lines, none at or after {offset}.",
                len = lines.len()
            )
        };
    }

    // Say what was withheld, because a truncated read that looks complete is
    // worse than one that admits it stopped.
    if end < lines.len() {
        out.push_str(&format!(
            "\n[showing lines {start}-{end} of {}]\n",
            lines.len(),
            start = start + 1
        ));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// A unique temporary directory for one test.
    ///
    /// The counter is what makes this safe: `cargo test` runs these in parallel
    /// threads inside one process, so a directory named only after the process
    /// id was shared by every test and each one deleted the others' files
    /// mid-run.
    fn temp_root() -> PathBuf {
        use std::sync::atomic::{AtomicU32, Ordering};
        static COUNTER: AtomicU32 = AtomicU32::new(0);
        let unique = COUNTER.fetch_add(1, Ordering::Relaxed);
        let dir =
            std::env::temp_dir().join(format!("projectz-read-{}-{unique}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create temp dir");
        dir
    }

    /// The number is the text before the first tab, and the line is everything
    /// after it.
    ///
    /// Pinned as a round-trip because it is the contract with `edit_lines`: a
    /// model that read line 43 is expected to be able to hand "line 43" straight
    /// back as an argument, and this is the half that makes that work.
    #[test]
    fn a_numbered_line_is_the_number_then_a_tab_then_the_source() {
        let rendered = numbered_slice("fn main() {}", 1, None);
        assert_eq!(rendered, "1\tfn main() {}\n");
    }

    /// The separator is a tab and only a tab.
    ///
    /// The pipe form is what this replaced, and it cost a real failure: content
    /// that legitimately contains the separator makes the split ambiguous, so
    /// the model cannot trust a line it just read. A tab cannot appear at the
    /// start of a source line in any file this app would open.
    #[test]
    fn the_separator_is_a_tab_so_the_content_is_unambiguous() {
        let rendered = numbered_slice("a | b", 1, None);
        assert_eq!(rendered, "1\ta | b\n", "the pipe came back");
        assert!(!rendered.contains('|') || rendered.contains("a | b"));
    }

    /// The number is left-aligned, not padded.
    ///
    /// Padding is there so a column of numbers lines up visually, but a padded
    /// number is no longer the number, and the entire point of printing it is
    /// that `edit_lines` reads it back. It must parse as the integer on its own.
    #[test]
    fn the_number_is_not_padded_so_it_parses_back_as_an_integer() {
        let rendered = numbered_slice("one\ntwo\nthree", 1, None);
        for line in rendered.lines() {
            let (number, _) = line.split_once('\t').expect("a tab on every row");
            number
                .parse::<usize>()
                .unwrap_or_else(|_| panic!("{number:?} is not a bare integer"));
        }
        assert!(rendered.starts_with("1\tone"), "{rendered}");
        assert!(rendered.contains("\n3\tthree"), "{rendered}");
    }

    /// The number is the real line number, not a counter restarted at the slice.
    ///
    /// A read starting at offset 50 still has to name line 50, because that is
    /// the number the model will hand back to `edit_lines`. Renumbering from 1
    /// would make every edit after a paged read target the wrong place.
    #[test]
    fn a_sliced_read_still_names_the_real_line_numbers() {
        let body = (1..=100)
            .map(|n| format!("line{n}"))
            .collect::<Vec<_>>()
            .join("\n");
        let out = numbered_slice(&body, 50, Some(2));
        assert!(out.starts_with("50\tline50"), "{out}");
        assert!(out.contains("51\tline51"), "{out}");
    }

    /// A read that withheld lines says so, naming the real range -- and the
    /// range it names is the one the numbers above it actually carry.
    #[test]
    fn the_withheld_range_matches_the_numbers_printed() {
        let body = (1..=40)
            .map(|n| format!("l{n}"))
            .collect::<Vec<_>>()
            .join("\n");
        let out = numbered_slice(&body, 5, Some(2));
        assert!(out.starts_with("5\tl5"), "{out}");
        assert!(out.contains("6\tl6"), "{out}");
        assert!(out.contains("showing lines 5-6 of 40"), "{out}");
    }

    #[test]
    fn an_unbounded_read_is_bounded_rather_than_returning_the_whole_file() {
        // The failure this prevents: an unbounded read of a large file gets cut
        // by the shared output cap, which keeps the head and the tail and drops
        // the middle. The model then has a hole in the file it cannot ask about.
        let root = temp_root();
        let body = (1..=DEFAULT_READ_LINES + 500)
            .map(|n| format!("line{n}"))
            .collect::<Vec<_>>()
            .join("\n");
        std::fs::write(root.join("big.txt"), &body).expect("write");

        let out = run(json!({ "path": "big.txt" }), root.clone()).expect("read");
        assert!(out.contains("1\tline1"), "{out}");
        assert!(
            out.contains(&format!(
                "showing lines 1-{DEFAULT_READ_LINES} of {}",
                DEFAULT_READ_LINES + 500
            )),
            "a bounded read must say what it withheld: {out}"
        );
        assert!(
            !out.contains("omitted from the middle"),
            "the shared cap cut the middle out"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn an_explicit_limit_above_the_default_is_honoured() {
        // A caller that asked for more than the default meant it. The bound is a
        // default, not a refusal.
        let root = temp_root();
        let body = (1..=DEFAULT_READ_LINES + 100)
            .map(|n| format!("line{n}"))
            .collect::<Vec<_>>()
            .join("\n");
        std::fs::write(root.join("big.txt"), &body).expect("write");

        let out = run(
            json!({ "path": "big.txt", "limit": DEFAULT_READ_LINES + 100 }),
            root.clone(),
        )
        .expect("read");
        assert!(
            out.contains(&format!(
                "{}\tline{}",
                DEFAULT_READ_LINES + 100,
                DEFAULT_READ_LINES + 100
            )),
            "{out}"
        );
        assert!(
            !out.contains("showing lines"),
            "the whole file was asked for"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn reading_past_the_default_window_works() {
        // 2,000 is the *default* when no limit is given, not a ceiling. An agent
        // that wants lines 2000-2020 asks for them by offset and gets them --
        // otherwise the bound would refuse the exact region the model had just
        // been told about.
        let root = temp_root();
        let total = DEFAULT_READ_LINES + 100;
        let body = (1..=total)
            .map(|n| format!("line{n}"))
            .collect::<Vec<_>>()
            .join("\n");
        std::fs::write(root.join("big.txt"), &body).expect("write");

        let out = run(
            json!({ "path": "big.txt", "offset": 2000, "limit": 21 }),
            root.clone(),
        )
        .expect("read");
        assert!(out.starts_with("2000\tline2000"), "{out}");
        assert!(out.contains("2020\tline2020"), "{out}");
        assert!(!out.contains("line1999"), "the window slipped backwards");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn reads_a_file_with_line_numbers() {
        let root = temp_root();
        std::fs::write(root.join("a.txt"), "one\ntwo\nthree").expect("write");
        let out = run(json!({ "path": "a.txt" }), root.clone()).expect("read");
        assert!(out.contains("1\tone"), "{out}");
        assert!(out.contains("3\tthree"), "{out}");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn an_offset_and_limit_return_just_that_window() {
        let root = temp_root();
        std::fs::write(root.join("a.txt"), "one\ntwo\nthree\nfour\nfive").expect("write");
        let out = run(
            json!({ "path": "a.txt", "offset": 2, "limit": 2 }),
            root.clone(),
        )
        .expect("read");
        assert!(out.contains("2\ttwo"), "{out}");
        assert!(out.contains("3\tthree"), "{out}");
        assert!(!out.contains("one"), "{out}");
        assert!(out.contains("showing lines 2-3 of 5"), "{out}");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn an_empty_file_says_so_rather_than_returning_nothing() {
        let root = temp_root();
        std::fs::write(root.join("empty.txt"), "").expect("write");
        let out = run(json!({ "path": "empty.txt" }), root).expect("read");
        assert!(out.contains("empty"), "{out}");
    }

    #[test]
    fn a_path_escaping_the_workspace_is_refused() {
        let root = temp_root();
        let error =
            run(json!({ "path": "../../etc/passwd" }), root.clone()).expect_err("must be refused");
        assert!(error.contains("outside"), "{error}");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn an_absolute_path_is_refused() {
        let root = temp_root();
        let error = run(json!({ "path": "/etc/passwd" }), root.clone()).expect_err("refused");
        assert!(error.contains("absolute"), "{error}");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[cfg(unix)]
    #[test]
    fn a_symlink_pointing_out_of_the_workspace_is_refused() {
        // The string-path check alone would pass this; only resolving first
        // catches it, which is why the check runs twice.
        let root = temp_root();
        let outside = temp_root().join("elsewhere");
        std::fs::create_dir_all(&outside).expect("dir");
        std::fs::write(outside.join("secret"), "sensitive").expect("write");
        std::os::unix::fs::symlink(&outside, root.join("link")).expect("symlink");

        let error =
            run(json!({ "path": "link/secret" }), root.clone()).expect_err("must be refused");
        assert!(error.contains("outside"), "{error}");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_missing_file_is_reported_without_panicking() {
        let root = temp_root();
        let error = run(json!({ "path": "nope.txt" }), root.clone()).expect_err("missing");
        assert!(error.contains("nope.txt"), "{error}");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_missing_path_argument_is_reported() {
        let root = temp_root();
        let error = run(json!({}), root.clone()).expect_err("no path");
        assert!(error.contains("path"), "{error}");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_zero_limit_is_rejected_rather_than_returning_nothing() {
        let root = temp_root();
        std::fs::write(root.join("a.txt"), "one").expect("write");
        let error =
            run(json!({ "path": "a.txt", "limit": 0 }), root.clone()).expect_err("zero limit");
        assert!(error.contains("at least 1"), "{error}");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_negative_offset_is_rejected() {
        let root = temp_root();
        std::fs::write(root.join("a.txt"), "one").expect("write");
        assert!(run(json!({ "path": "a.txt", "offset": -1 }), root.clone()).is_err());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn an_offset_past_the_end_says_so_instead_of_looking_empty() {
        let root = temp_root();
        std::fs::write(root.join("a.txt"), "one\ntwo").expect("write");
        let out = run(json!({ "path": "a.txt", "offset": 99 }), root.clone()).expect("read");
        assert!(out.contains("none at or after 99"), "{out}");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_limit_past_the_end_returns_what_exists() {
        let root = temp_root();
        std::fs::write(root.join("a.txt"), "one\ntwo").expect("write");
        let out = run(json!({ "path": "a.txt", "limit": 500 }), root.clone()).expect("read");
        assert!(out.contains("2\ttwo"), "{out}");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_listing_shows_folders_first_with_sizes_for_files() {
        let root = temp_root();
        std::fs::write(root.join("b.txt"), "hello").expect("write");
        std::fs::create_dir(root.join("a-dir")).expect("dir");
        std::fs::write(root.join("a-dir/inner.txt"), "x").expect("write");

        let out = list_dir(json!({}), root.clone()).expect("list");
        // The folder must come first, and its trailing slash is what tells a
        // model it is not a file.
        assert!(out.contains("a-dir/\n"), "{out}");
        assert!(out.contains("b.txt  (5 bytes)"), "{out}");
        let folder_at = out.find("a-dir/").expect("folder listed");
        let file_at = out.find("b.txt").expect("file listed");
        assert!(folder_at < file_at, "folders should sort first: {out}");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn listing_the_root_works_with_no_path_at_all() {
        let root = temp_root();
        std::fs::write(root.join("a.txt"), "x").expect("write");
        let out = list_dir(json!({}), root.clone()).expect("list root");
        assert!(out.contains("a.txt"), "{out}");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn an_empty_directory_says_so_instead_of_returning_nothing() {
        let root = temp_root();
        let out = list_dir(json!({}), root.clone()).expect("list");
        assert!(out.contains("empty"), "{out}");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn listing_a_path_outside_the_workspace_is_refused() {
        let root = temp_root();
        let error = list_dir(json!({ "path": "../.." }), root.clone()).expect_err("refused");
        assert!(error.contains("outside"), "{error}");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn listing_a_missing_directory_is_reported_rather_than_panicking() {
        let root = temp_root();
        let error = list_dir(json!({ "path": "nope" }), root.clone()).expect_err("missing");
        assert!(error.contains("nope"), "{error}");
        let _ = std::fs::remove_dir_all(&root);
    }
}
