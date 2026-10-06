//! Changing files in the workspace.
//!
//! Three tools rather than one, and the split matters:
//!
//! - [`WRITE_FILE`] replaces a whole file. For a new file, or one small enough
//!   to hold in the model's head.
//! - [`EDIT_FILE`] changes one exact run of text, when the model has the text and
//!   not the numbers -- the shape you get from a search hit rather than a read.
//! - [`EDIT_LINES`] changes a numbered range, which is what [`super::file::READ_FILE`]
//!   produces.
//!
//! A model that wants to touch three lines should not have to reproduce the
//! other four hundred, because a file it reproduces from memory is a file it
//! will get subtly wrong. [`EDIT_LINES`] is the cheapest of the three for that
//! case: the model already has the numbers, so there is nothing to quote back.
//!
//! All three are `Write` effect, which means their results are never cached and
//! running one invalidates every cached read from earlier in the conversation. That last
//! part is what stops a model reading a file, editing it, and then being handed
//! the pre-edit contents from the cache.
//!
//! # Atomicity
//!
//! Edits go through a temporary file in the same directory and are then renamed
//! over the target. A write that fails partway leaves the original intact rather
//! than a half-written file that still parses.
//!
//! # Reading back what was edited
//!
//! Every edit returns the changed lines, numbered in the same format
//! [`super::file::READ_FILE`] uses. It costs a handful of tokens and it closes
//! the loop: a model that wrote lines 43-48 knows immediately whether the result
//! is what it meant, without spending a whole extra round trip to re-read and
//! find out. A small local model is the one that needs this most -- it is the one
//! that would otherwise edit blind and either repeat the edit or report success
//! for something it got wrong.

use std::path::{Path, PathBuf};

use serde_json::Value;

use super::ToolSpec;

/// Replace a file's entire contents.
pub const WRITE_FILE: ToolSpec = ToolSpec {
    name: "write_file",
    description: "Write text to a file, replacing its current contents. Creates the file and any missing \
                  parent folders. Use this for a new file; use edit_file to change part of an existing one \
                  so you do not have to reproduce the rest.",
    parameters: r#"{
        "type": "object",
        "properties": {
            "path": {
                "type": "string",
                "description": "Path to the file, relative to the workspace root."
            },
            "content": {
                "type": "string",
                "description": "The full new contents of the file."
            }
        },
        "required": ["path", "content"],
        "additionalProperties": false
    }"#,
    effect: super::Effect::Write,
    command_argument: None,
    execute: |arguments, context| {
        Box::pin(std::future::ready(
            super::file::require_workspace().and_then(|root| write(arguments, root, &context)),
        ))
    },
};

/// Replace one exact run of text in a file.
pub const EDIT_FILE: ToolSpec = ToolSpec {
    name: "edit_file",
    description: "Replace an exact piece of text in a file. The old text must appear exactly as given; \
                  set replace_all to change every occurrence. Read the file first so the old text matches \
                  exactly, including its whitespace. Prefer edit_lines when you know which lines to change.",
    parameters: r#"{
        "type": "object",
        "properties": {
            "path": {
                "type": "string",
                "description": "Path to the file, relative to the workspace root."
            },
            "old_text": {
                "type": "string",
                "description": "The exact text to replace, including indentation."
            },
            "new_text": {
                "type": "string",
                "description": "The text to put in its place."
            },
            "replace_all": {
                "type": "boolean",
                "description": "Replace every occurrence instead of requiring exactly one."
            }
        },
        "required": ["path", "old_text", "new_text"],
        "additionalProperties": false
    }"#,
    effect: super::Effect::Write,
    command_argument: None,
    execute: |arguments, context| {
        Box::pin(std::future::ready(
            super::file::require_workspace().and_then(|root| edit(arguments, root, &context)),
        ))
    },
};

/// Replace a numbered range of lines.
///
/// The tool a read is designed to feed: `read_file` returns numbers, and this
/// takes them back. That round trip is the point -- a model does not have to
/// quote the file's text at all, so it cannot get the whitespace wrong, and it
/// cannot be refused for an ambiguous match when it meant one specific place.
///
/// The cost of that safety is staleness, so the range is verified against the
/// text the model last saw rather than trusted blindly. `expected_text` is
/// optional: pass the lines you read and a file that has changed underneath you
/// is refused rather than overwritten, which is the failure mode that silently
/// discards someone else's work. Omit it and the edit applies to the lines as
/// they are now.
///
/// The result is numbered the same way a read is, so the model sees what it
/// wrote without another round trip.
pub const EDIT_LINES: ToolSpec = ToolSpec {
    name: "edit_lines",
    description: "Replace a range of lines in a file, given 1-based line numbers from read_file. \
                  Prefer this over edit_file: it does not require reproducing the file's text, so \
                  it cannot fail on whitespace. Include surrounding unchanged lines in new_text to \
                  place the edit accurately. Pass expected_text from the read to make the edit \
                  refuse if the file changed underneath you.",
    parameters: r#"{
        "type": "object",
        "properties": {
            "path": {
                "type": "string",
                "description": "Path to the file, relative to the workspace root."
            },
            "start_line": {
                "type": "integer",
                "description": "First line to replace, 1-based and inclusive."
            },
            "end_line": {
                "type": "integer",
                "description": "Last line to replace, 1-based and inclusive. Omit to replace just start_line."
            },
            "new_text": {
                "type": "string",
                "description": "The text to put in that range. Include any unchanged lines from the start and end of the range so they are not lost."
            },
            "expected_text": {
                "type": "string",
                "description": "Optional. The lines as you last read them, without their line numbers. The edit is refused if they no longer match."
            }
        },
        "required": ["path", "start_line", "new_text"],
        "additionalProperties": false
    }"#,
    effect: super::Effect::Write,
    command_argument: None,
    execute: |arguments, context| {
        Box::pin(std::future::ready(
            super::file::require_workspace().and_then(|root| edit_lines(arguments, root, &context)),
        ))
    },
};

fn edit_lines(
    arguments: Value,
    root: PathBuf,
    context: &super::ToolContext,
) -> Result<String, String> {
    let path = required_path(&arguments)?;
    let start = line_number(&arguments, "start_line")?;
    let end = match arguments.get("end_line") {
        None | Some(Value::Null) => start,
        Some(_) => line_number(&arguments, "end_line")?,
    };
    if end < start {
        return Err(format!(
            "`end_line` ({end}) is before `start_line` ({start}). Lines run from 1 upward."
        ));
    }

    // An empty `new_text` is a deletion and that is a legitimate thing to want, so
    // unlike `old_text` it is not rejected here. What it cannot do is delete
    // nothing, which would be a no-op the model reported as a change.
    let new_text = string_of(&arguments, "new_text")?;

    let resolved = super::file::resolve_within(&root, &path)?;
    let body = std::fs::read_to_string(&resolved)
        .map_err(|error| format!("Could not read {path}: {error}"))?;
    let lines: Vec<&str> = body.lines().collect();

    // Past the end is refused rather than clamped. Clamping would write the
    // replacement over a line the model never named, and an off-by-one from a
    // stale read is exactly the case that silently destroys the wrong code.
    if start == 0 || start > lines.len() {
        return Err(format!(
            "`start_line` is {start}, but {path} has {} lines. Re-read the file to get the \
             current line numbers, since an edit earlier in the file shifts them.",
            lines.len()
        ));
    }
    // One past the last line is allowed and means "to the end": that is how a
    // model truncates a file, and refusing it would leave no way to do so.
    if end > lines.len() {
        return Err(format!(
            "`end_line` is {end}, but {path} has {} lines. Re-read the file to get the current \
             line numbers, since an edit earlier in the file shifts them.",
            lines.len()
        ));
    }

    if let Some(expected) = arguments.get("expected_text").and_then(Value::as_str) {
        verify_unchanged(&path, &lines, start, end, expected)?;
    }

    // Whether the file ended with a newline, captured before the rebuild because
    // `join` can never preserve it.
    let ended_with_newline = body.ends_with('\n') || body.is_empty();

    let replaced: Vec<&str> = new_text.lines().collect();
    let mut updated: Vec<String> = Vec::with_capacity(lines.len() + replaced.len());
    updated.extend(lines[..start - 1].iter().map(|line| line.to_string()));
    updated.extend(replaced.iter().map(|line| line.to_string()));
    updated.extend(lines[end..].iter().map(|line| line.to_string()));
    let mut body = updated.join("\n");
    // The trailing newline is preserved rather than assumed, because losing it is
    // not a no-op: it shows as a "\ No newline at end of file" line in every
    // future diff of this file, and most tooling will keep re-adding it. A
    // `new_text` that itself ends in one wins, since that is the model saying
    // the file should end that way.
    if (ended_with_newline || new_text.ends_with('\n')) && !body.is_empty() {
        body.push('\n');
    }

    atomic_write(&resolved, body.as_bytes())?;
    context.report_diff(super::diff::diff(&lines.join("\n"), &body));

    // Read back what landed, numbered as a read numbers it. Closes the loop for
    // the model without a second round trip, which is what a small local model
    // needs most and a large one barely notices.
    let written: Vec<&str> = new_text.lines().collect();
    let mut result = format!(
        "Replaced lines {start}-{end} in {path} with {} line{}.",
        written.len(),
        if written.len() == 1 { "" } else { "s" }
    );
    result.push('\n');
    for (index, line) in written.iter().enumerate() {
        result.push_str(&format!("{}\t{}\n", start + index, line));
    }
    let grew_by = written.len() as isize - (end - start + 1) as isize;
    if grew_by != 0 {
        // Worth stating because line numbers *after* the edit have moved, and a
        // model that edits again from stale numbers will hit the wrong place.
        // The sign is spelled out rather than left to `{:+}` because a shrink
        // reported as a bare positive number reads as a growth -- which would
        // send the model looking in the wrong direction entirely.
        let sign = match grew_by.cmp(&0) {
            std::cmp::Ordering::Greater => "+",
            std::cmp::Ordering::Less => "-",
            std::cmp::Ordering::Equal => "",
        };
        result.push_str(&format!(
            "[the file is now {total} lines; lines after {end} have shifted by {sign}{moved}]\n",
            total = updated.len(),
            moved = grew_by.abs()
        ));
    }
    Ok(result)
}

/// Refuses an edit whose range no longer holds what the model read.
///
/// The staleness check. Without it, a file changed by anything -- the user, a
/// formatter, another tool in the same run -- since the model read it would be
/// overwritten at numbers that now point somewhere else, and the model would be
/// told the edit succeeded. This is the one failure in the tool layer that loses
/// work the user can see, so it is checked rather than assumed.
fn verify_unchanged(
    path: &str,
    lines: &[&str],
    start: usize,
    end: usize,
    expected: &str,
) -> Result<(), String> {
    if lines[start - 1..end].join("\n") == expected {
        return Ok(());
    }
    Err(format!(
        "Lines {start}-{end} of {path} no longer match what you read, so the edit was refused. \
         The file changed underneath this conversation. Re-read it and retry. \
         The lines are now:\n{}",
        // Sliced from the *whole* file rather than from the range text alone:
        // numbering is offset-based, so handing it a string that starts at line
        // `start` and asking for line `start` reports nothing at all -- and a
        // refusal that shows no lines is a refusal the model cannot act on.
        super::file::numbered_slice(&lines.join("\n"), start, Some(end - start + 1))
    ))
}

/// Reads a 1-based line number, rejecting zero and negatives.
///
/// Zero is the interesting rejection: it is what a model computing
/// `end - start + 1` or a 0-based index by hand lands on, and treating it as
/// "the first line" would edit the wrong place rather than fail.
fn line_number(arguments: &Value, field: &str) -> Result<usize, String> {
    let number = arguments
        .get(field)
        .and_then(Value::as_u64)
        .ok_or_else(|| format!("`{field}` is required and must be a whole number"))?;
    if number == 0 {
        return Err(format!("`{field}` is 1-based, so it starts at 1, not 0"));
    }
    usize::try_from(number).map_err(|_| format!("`{field}` is too large"))
}

fn string_of(arguments: &Value, field: &str) -> Result<String, String> {
    arguments
        .get(field)
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| format!("A string `{field}` is required"))
}

fn write(arguments: Value, root: PathBuf, context: &super::ToolContext) -> Result<String, String> {
    let path = required_path(&arguments)?;
    let content = string_of(&arguments, "content")?;
    let resolved = super::file::resolve_within(&root, &path)?;

    let existed = resolved.exists();
    if let Some(parent) = resolved.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|error| format!("Could not create {}: {error}", parent.display()))?;
    }
    // Read before writing, not after: once the file is replaced this text is the
    // only record of what was there, and it is what the panel's diff needs.
    let previous = if existed {
        std::fs::read_to_string(&resolved).unwrap_or_default()
    } else {
        String::new()
    };
    let previous_size = file_length(&resolved).unwrap_or(0);
    atomic_write(&resolved, content.as_bytes())?;
    context.report_diff(super::diff::diff(&previous, &content));

    Ok(if existed {
        format!("Wrote {previous_size} bytes to {path}, replacing its contents.")
    } else {
        format!("Created {path}.")
    })
}

fn edit(arguments: Value, root: PathBuf, context: &super::ToolContext) -> Result<String, String> {
    let path = required_path(&arguments)?;
    let old_text = string_of(&arguments, "old_text")?;
    let new_text = string_of(&arguments, "new_text")?;
    // An empty needle would match at every position and the count would be the
    // file's length in bytes. Reported plainly rather than as a match count.
    if old_text.is_empty() {
        return Err("`old_text` cannot be empty; there would be nothing to identify".into());
    }
    let replace_all = arguments
        .get("replace_all")
        .and_then(Value::as_bool)
        .unwrap_or(false);

    let resolved = super::file::resolve_within(&root, &path)?;
    let body = std::fs::read_to_string(&resolved)
        .map_err(|error| format!("Could not read {path}: {error}"))?;

    let occurrences = body.matches(&old_text).count();
    if occurrences == 0 {
        return Err(format!(
            "`old_text` was not found in {path}. Read the file and copy the text exactly, \
             including its indentation."
        ));
    }
    // Ambiguity is refused rather than guessed. Picking the first of two matches
    // is how a model silently edits the wrong function, and it is invisible in
    // the diff it reports back.
    if occurrences > 1 && !replace_all {
        return Err(format!(
            "`old_text` appears {occurrences} times in {path}. Include more surrounding text to \
             identify one, or set replace_all to change all of them."
        ));
    }

    let updated = if replace_all {
        body.replace(&old_text, &new_text)
    } else {
        body.replacen(&old_text, &new_text, 1)
    };
    atomic_write(&resolved, updated.as_bytes())?;
    context.report_diff(super::diff::diff(&body, &updated));

    Ok(format!(
        "Replaced {} occurrence{} in {path}.",
        if replace_all { occurrences } else { 1 },
        if replace_all && occurrences != 1 {
            "s"
        } else {
            ""
        }
    ))
}

fn required_path(arguments: &Value) -> Result<String, String> {
    arguments
        .get("path")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|path| !path.is_empty())
        .map(str::to_string)
        .ok_or_else(|| "A non-empty string `path` is required".to_string())
}

fn file_length(path: &Path) -> Result<u64, String> {
    std::fs::metadata(path)
        .map(|meta| meta.len())
        .map_err(|error| format!("Could not stat {}: {error}", path.display()))
}

/// Writes through a sibling temporary file and renames it over the target.
///
/// The temporary file shares a directory with the target so the rename stays on
/// one filesystem and is therefore atomic; a temporary file in the system temp
/// directory could be a different volume, where rename silently copies instead.
fn atomic_write(target: &Path, contents: &[u8]) -> Result<(), String> {
    let parent = target
        .parent()
        .ok_or_else(|| format!("{} has no parent directory", target.display()))?;
    let temporary = parent.join(format!(
        ".{}.projectz-partial",
        target
            .file_name()
            .map(|name| name.to_string_lossy())
            .unwrap_or_default()
    ));

    std::fs::write(&temporary, contents)
        .map_err(|error| format!("Could not write {}: {error}", temporary.display()))?;
    // Every exit path from here removes the temporary file, including the
    // failed-rename one: a leftover `.file.projectz-partial` would be picked up
    // by the user's own searches and is not theirs to clean up.
    if let Err(error) = std::fs::rename(&temporary, target) {
        let _ = std::fs::remove_file(&temporary);
        return Err(format!("Could not replace {}: {error}", target.display()));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// A context that reports nowhere, so a test can call a write the way the
    /// registry does without threading a sink through every case.
    ///
    /// The diff it would have reported is not what these tests are about -- they
    /// are about what landed on disk -- and [`super::super::diff`] has its own
    /// tests for the diff itself.
    fn discarding() -> super::super::ToolContext {
        super::super::ToolContext::default()
    }

    fn root() -> PathBuf {
        use std::sync::atomic::{AtomicU32, Ordering};
        static COUNTER: AtomicU32 = AtomicU32::new(0);
        let unique = COUNTER.fetch_add(1, Ordering::Relaxed);
        let dir =
            std::env::temp_dir().join(format!("projectz-write-{}-{unique}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("temp dir");
        dir
    }

    fn body_of(root: &Path, path: &str) -> String {
        std::fs::read_to_string(root.join(path)).expect("read back")
    }

    #[test]
    fn writing_a_new_file_creates_it_with_its_parents() {
        let root = root();
        let out = write(
            json!({ "path": "src/deep/a.rs", "content": "fn main() {}" }),
            root.clone(),
            &discarding(),
        )
        .expect("write");
        assert!(out.contains("Created"), "{out}");
        assert_eq!(body_of(&root, "src/deep/a.rs"), "fn main() {}");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn writing_an_existing_file_replaces_it_and_says_so() {
        let root = root();
        std::fs::write(root.join("a.txt"), "old").expect("seed");
        let out = write(
            json!({ "path": "a.txt", "content": "new" }),
            root.clone(),
            &discarding(),
        )
        .expect("write");
        assert!(out.contains("replacing"), "{out}");
        assert_eq!(body_of(&root, "a.txt"), "new");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_write_outside_the_workspace_is_refused() {
        let root = root();
        let error = write(
            json!({ "path": "../escaped.txt", "content": "x" }),
            root.clone(),
            &discarding(),
        )
        .expect_err("refused");
        assert!(error.contains("outside"), "{error}");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn an_edit_replaces_exactly_one_occurrence() {
        let root = root();
        std::fs::write(root.join("a.rs"), "let a = 1;\nlet b = 2;\n").expect("seed");
        edit(
            json!({ "path": "a.rs", "old_text": "let b = 2;", "new_text": "let b = 3;" }),
            root.clone(),
            &discarding(),
        )
        .expect("edit");
        assert_eq!(body_of(&root, "a.rs"), "let a = 1;\nlet b = 3;\n");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn an_ambiguous_edit_is_refused_rather_than_guessed() {
        // The whole reason `edit_file` exists: picking one of two matches would
        // change the wrong line and report success.
        let root = root();
        std::fs::write(root.join("a.rs"), "x\nx\n").expect("seed");
        let error = edit(
            json!({ "path": "a.rs", "old_text": "x", "new_text": "y" }),
            root.clone(),
            &discarding(),
        )
        .expect_err("ambiguous");
        assert!(error.contains("2 times"), "{error}");
        assert_eq!(body_of(&root, "a.rs"), "x\nx\n");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn replace_all_changes_every_occurrence_when_asked() {
        let root = root();
        std::fs::write(root.join("a.rs"), "x\nx\nx\n").expect("seed");
        let out = edit(
            json!({ "path": "a.rs", "old_text": "x", "new_text": "y", "replace_all": true }),
            root.clone(),
            &discarding(),
        )
        .expect("edit");
        assert!(out.contains("3 occurrences"), "{out}");
        assert_eq!(body_of(&root, "a.rs"), "y\ny\ny\n");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn text_that_is_not_there_is_reported_with_what_to_do() {
        let root = root();
        std::fs::write(root.join("a.rs"), "hello").expect("seed");
        let error = edit(
            json!({ "path": "a.rs", "old_text": "goodbye", "new_text": "x" }),
            root.clone(),
            &discarding(),
        )
        .expect_err("absent");
        assert!(error.contains("not found"), "{error}");
        // Telling the model to re-read turns a dead end into another turn.
        assert!(error.contains("Read the file"), "{error}");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn an_empty_old_text_is_refused() {
        let root = root();
        std::fs::write(root.join("a.rs"), "hello").expect("seed");
        assert!(edit(
            json!({ "path": "a.rs", "old_text": "", "new_text": "x" }),
            root.clone(),
            &discarding(),
        )
        .expect_err("empty")
        .contains("nothing"));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_match_inside_indented_text_is_still_found() {
        // Substring, not line-anchored: a model that omits the leading
        // indentation still gets the edit it meant, and including the
        // indentation is what disambiguates two otherwise identical lines.
        let root = root();
        std::fs::write(root.join("a.rs"), "fn a() {\n    let x = 1;\n}").expect("seed");
        edit(
            json!({ "path": "a.rs", "old_text": "let x = 1;", "new_text": "let x = 2;" }),
            root.clone(),
            &discarding(),
        )
        .expect("edit");
        assert_eq!(body_of(&root, "a.rs"), "fn a() {\n    let x = 2;\n}");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn including_the_indentation_disambiguates_two_identical_lines() {
        // The reason exact whitespace matters, stated as a working case rather
        // than a failure: the same text at two indent levels is two matches, and
        // naming one of them selects one.
        let root = root();
        std::fs::write(root.join("a.rs"), "    let x = 1;\n        let x = 1;\n").expect("seed");
        let out = edit(
            json!({ "path": "a.rs", "old_text": "        let x = 1;", "new_text": "        let x = 2;" }),
            root.clone(),
            &discarding(),
        )
        .expect("edit");
        assert!(out.contains("1 occurrence"), "{out}");
        assert_eq!(
            body_of(&root, "a.rs"),
            "    let x = 1;\n        let x = 2;\n"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_failed_write_leaves_no_partial_file_behind() {
        // The temporary file is the thing a user would find in their own searches
        // if an error path missed it.
        let root = root();
        let target = root.join("a.txt");
        let error = atomic_write(&root.join("missing-dir/a.txt"), b"x").expect_err("fails");
        assert!(error.contains("Could not write"), "{error}");
        assert!(!target.exists());
        assert_eq!(
            std::fs::read_dir(&root).expect("read").count(),
            0,
            "a partial file was left behind"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_line_range_is_replaced_and_read_back_numbered() {
        let root = root();
        std::fs::write(root.join("a.ts"), "one\ntwo\nthree\nfour\n").expect("seed");
        let out = edit_lines(
            json!({ "path": "a.ts", "start_line": 2, "end_line": 3, "new_text": "TWO\nTHREE" }),
            root.clone(),
            &discarding(),
        )
        .expect("edit");
        assert_eq!(body_of(&root, "a.ts"), "one\nTWO\nTHREE\nfour\n");
        // The read-back is the whole point: the model sees what landed without
        // spending a round trip, and it is numbered as a read numbers it.
        assert!(
            out.contains("Replaced lines 2-3 in a.ts with 2 lines"),
            "{out}"
        );
        assert!(out.contains("2\tTWO"), "{out}");
        assert!(out.contains("3\tTHREE"), "{out}");
        let _ = std::fs::remove_dir_all(&root);
    }

    /// Omitting `end_line` edits exactly one line rather than the rest of the
    /// file. A model that means "this line" says one number, and the tool must
    /// not read that as "from here to the end".
    #[test]
    fn a_start_line_alone_replaces_only_that_line() {
        let root = root();
        std::fs::write(root.join("a.ts"), "one\ntwo\nthree\n").expect("seed");
        edit_lines(
            json!({ "path": "a.ts", "start_line": 2, "new_text": "TWO" }),
            root.clone(),
            &discarding(),
        )
        .expect("edit");
        assert_eq!(body_of(&root, "a.ts"), "one\nTWO\nthree\n");
        let _ = std::fs::remove_dir_all(&root);
    }

    /// The replacement may be longer than the range it replaces. That is the
    /// normal case for adding a function or a block, and a tool that assumed a
    /// 1:1 line count could not express it.
    #[test]
    fn a_range_can_grow_and_the_shift_is_reported() {
        let root = root();
        std::fs::write(root.join("a.ts"), "one\ntwo\nthree\n").expect("seed");
        let out = edit_lines(
            json!({ "path": "a.ts", "start_line": 2, "new_text": "A\nB\nC\nD" }),
            root.clone(),
            &discarding(),
        )
        .expect("edit");
        assert_eq!(body_of(&root, "a.ts"), "one\nA\nB\nC\nD\nthree\n");
        // Stated because line numbers after the edit have moved, and a model
        // editing again from stale numbers hits the wrong place.
        assert!(out.contains("shifted by +3"), "{out}");
        assert!(out.contains("now 6 lines"), "{out}");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_shrinking_range_reports_the_negative_shift() {
        let root = root();
        std::fs::write(root.join("a.ts"), "one\ntwo\nthree\nfour\n").expect("seed");
        let out = edit_lines(
            json!({ "path": "a.ts", "start_line": 2, "end_line": 4, "new_text": "TWO" }),
            root.clone(),
            &discarding(),
        )
        .expect("edit");
        assert_eq!(body_of(&root, "a.ts"), "one\nTWO\n");
        assert!(out.contains("shifted by -2"), "{out}");
        let _ = std::fs::remove_dir_all(&root);
    }

    /// An empty `new_text` deletes the range, which is a legitimate thing to
    /// want. It is `edit_file`'s `old_text` that must be non-empty -- an empty
    /// needle there has nothing to identify.
    #[test]
    fn an_empty_new_text_deletes_the_range() {
        let root = root();
        std::fs::write(root.join("a.ts"), "one\ntwo\nthree\n").expect("seed");
        edit_lines(
            json!({ "path": "a.ts", "start_line": 2, "end_line": 2, "new_text": "" }),
            root.clone(),
            &discarding(),
        )
        .expect("edit");
        assert_eq!(body_of(&root, "a.ts"), "one\nthree\n");
        let _ = std::fs::remove_dir_all(&root);
    }

    /// Refuses rather than clamps. This is the one failure in the tool layer
    /// that loses work the user can see, and it comes from a stale read after an
    /// earlier edit shifted the numbers.
    #[test]
    fn a_line_past_the_end_is_refused_rather_than_clamped() {
        let root = root();
        std::fs::write(root.join("a.ts"), "one\ntwo\n").expect("seed");
        let error = edit_lines(
            json!({ "path": "a.ts", "start_line": 9, "new_text": "x" }),
            root.clone(),
            &discarding(),
        )
        .expect_err("past the end");
        assert!(error.contains("has 2 lines"), "{error}");
        // Tells the model how to recover rather than only what went wrong.
        assert!(error.contains("Re-read"), "{error}");
        assert_eq!(body_of(&root, "a.ts"), "one\ntwo\n");
        let _ = std::fs::remove_dir_all(&root);
    }

    /// Line numbers are 1-based. A model computing `end - start + 1` or copying
    /// a 0-based index lands on zero, and treating that as "the first line"
    /// would edit the wrong place rather than fail.
    #[test]
    fn a_zero_line_number_is_refused_rather_than_read_as_the_first_line() {
        let root = root();
        std::fs::write(root.join("a.ts"), "one\ntwo\n").expect("seed");
        let error = edit_lines(
            json!({ "path": "a.ts", "start_line": 0, "new_text": "x" }),
            root.clone(),
            &discarding(),
        )
        .expect_err("zero");
        assert!(error.contains("1-based"), "{error}");
        assert_eq!(body_of(&root, "a.ts"), "one\ntwo\n");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_negative_line_number_is_refused() {
        let root = root();
        std::fs::write(root.join("a.ts"), "one\n").expect("seed");
        assert!(edit_lines(
            json!({ "path": "a.ts", "start_line": -1, "new_text": "x" }),
            root.clone(),
            &discarding(),
        )
        .expect_err("negative")
        .contains("whole number"));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn an_end_before_the_start_is_refused() {
        let root = root();
        std::fs::write(root.join("a.ts"), "one\ntwo\n").expect("seed");
        assert!(edit_lines(
            json!({ "path": "a.ts", "start_line": 3, "end_line": 1, "new_text": "x" }),
            root.clone(),
            &discarding(),
        )
        .expect_err("reversed")
        .contains("before"));
        let _ = std::fs::remove_dir_all(&root);
    }

    /// The staleness check. Without it, a file changed by anything since the
    /// model read it -- the user, a formatter, another tool -- would be
    /// overwritten at numbers that now point somewhere else, and the model would
    /// be told it succeeded.
    #[test]
    fn a_stale_edit_is_refused_and_shows_the_current_lines() {
        let root = root();
        std::fs::write(root.join("a.ts"), "one\ntwo\nthree\n").expect("seed");
        let error = edit_lines(
            json!({
                "path": "a.ts",
                "start_line": 2,
                "new_text": "TWO",
                "expected_text": "something else"
            }),
            root.clone(),
            &discarding(),
        )
        .expect_err("stale");
        assert!(error.contains("no longer match"), "{error}");
        assert!(error.contains("Re-read"), "{error}");
        // Numbered as a read numbers it, so the corrected numbers come straight
        // back out of the error.
        assert!(error.contains("2\ttwo"), "{error}");
        assert_eq!(body_of(&root, "a.ts"), "one\ntwo\nthree\n");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_matching_expected_text_lets_the_edit_through() {
        let root = root();
        std::fs::write(root.join("a.ts"), "one\ntwo\nthree\n").expect("seed");
        edit_lines(
            json!({
                "path": "a.ts",
                "start_line": 2,
                "end_line": 2,
                "new_text": "TWO",
                "expected_text": "two"
            }),
            root.clone(),
            &discarding(),
        )
        .expect("edit");
        assert_eq!(body_of(&root, "a.ts"), "one\nTWO\nthree\n");
        let _ = std::fs::remove_dir_all(&root);
    }

    /// `expected_text` spanning a multi-line range is compared as one block,
    /// which is the shape a read gives the model.
    #[test]
    fn a_multi_line_expected_text_is_compared_as_a_block() {
        let root = root();
        std::fs::write(root.join("a.ts"), "one\ntwo\nthree\nfour\n").expect("seed");
        edit_lines(
            json!({
                "path": "a.ts",
                "start_line": 2,
                "end_line": 3,
                "new_text": "X",
                "expected_text": "two\nthree"
            }),
            root.clone(),
            &discarding(),
        )
        .expect("edit");
        assert_eq!(body_of(&root, "a.ts"), "one\nX\nfour\n");
        let _ = std::fs::remove_dir_all(&root);
    }

    /// A file ending without a newline keeps ending without one. Losing it is
    /// not a no-op: it shows as "\ No newline at end of file" in every future
    /// diff, and most tooling keeps re-adding it.
    #[test]
    fn a_missing_trailing_newline_is_not_invented() {
        let root = root();
        std::fs::write(root.join("a.txt"), "one\ntwo").expect("seed");
        edit_lines(
            json!({ "path": "a.txt", "start_line": 1, "new_text": "ONE" }),
            root.clone(),
            &discarding(),
        )
        .expect("edit");
        assert_eq!(body_of(&root, "a.txt"), "ONE\ntwo");
        let _ = std::fs::remove_dir_all(&root);
    }

    /// And a file that had one keeps it, which `join` alone would not do.
    #[test]
    fn an_existing_trailing_newline_is_preserved() {
        let root = root();
        std::fs::write(root.join("a.txt"), "one\ntwo\n").expect("seed");
        edit_lines(
            json!({ "path": "a.txt", "start_line": 1, "new_text": "ONE" }),
            root.clone(),
            &discarding(),
        )
        .expect("edit");
        assert_eq!(body_of(&root, "a.txt"), "ONE\ntwo\n");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_line_edit_outside_the_workspace_is_refused() {
        let root = root();
        let error = edit_lines(
            json!({ "path": "../escaped.txt", "start_line": 1, "new_text": "x" }),
            root.clone(),
            &discarding(),
        )
        .expect_err("refused");
        assert!(error.contains("outside"), "{error}");
        let _ = std::fs::remove_dir_all(&root);
    }

    /// Both editors are `Write`, which is what triggers approval and cache
    /// invalidation. Getting it wrong would cache an edit's own result and hand
    /// the model the pre-edit contents on its next read.
    #[test]
    fn all_three_write_tools_are_marked_as_writes() {
        for spec in [WRITE_FILE, EDIT_FILE, EDIT_LINES] {
            assert_eq!(spec.effect, super::super::Effect::Write, "{}", spec.name);
        }
    }

    /// The names are stable because they are the cache key: renaming one
    /// silently invalidates every provider's cached prefix for every existing
    /// conversation.
    #[test]
    fn the_write_tool_names_are_stable() {
        assert_eq!(WRITE_FILE.name, "write_file");
        assert_eq!(EDIT_FILE.name, "edit_file");
        assert_eq!(EDIT_LINES.name, "edit_lines");
    }
}
