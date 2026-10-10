//! Moving and deleting files in the workspace.
//!
//! A refactor moves and removes files, and until now the only way to do either was
//! through `run_terminal` -- which is slower, needs a shell, and shows the user a
//! command line rather than the operation that was meant. These are two small
//! tools instead, both going through the same containment check as everything else
//! and both reporting what they touched.
//!
//! # Why delete is not `rm`
//!
//! There is one `delete_file`, and it deletes one thing the caller named. A
//! pattern or a flag that removed a set at once is how a model turns a mistake
//! into a disaster, and the recursive form has to be asked for explicitly. The
//! workspace root itself can never be a target: the path check refuses `..` and
//! the empty path alike, and a request for the root is refused by name.
//!
//! # Why move and rename are one tool
//!
//! They are the same operation -- a rename is a move within a directory -- so a
//! second tool would be a second way to make the same syscall, kept in step by
//! hand. The description names both words so a model reaches for it either way.

use std::path::PathBuf;

use serde_json::Value;

use super::ToolSpec;

/// Move or rename a file or folder.
pub const MOVE_FILE: ToolSpec = ToolSpec {
    name: "move_file",
    description: "Move or rename a file or folder within the workspace. Set `destination` to the \
                  new path; renaming in place is the same call. Fails rather than overwriting an \
                  existing file, so move the thing out of the way first if you mean to replace it.",
    parameters: r#"{
        "type": "object",
        "properties": {
            "path": {
                "type": "string",
                "description": "The file or folder to move, relative to the workspace root."
            },
            "destination": {
                "type": "string",
                "description": "Where to move it, relative to the workspace root. Parent folders are created if missing."
            }
        },
        "required": ["path", "destination"],
        "additionalProperties": false
    }"#,
    effect: super::Effect::Write,
    command_argument: None,
    execute: |arguments, context| {
        let _ = &context;
        Box::pin(std::future::ready(
            super::file::require_workspace().and_then(|root| move_file(arguments, root)),
        ))
    },
};

/// Delete a file, or a folder with `recursive`.
pub const DELETE_FILE: ToolSpec = ToolSpec {
    name: "delete_file",
    description: "Delete a file, or a folder when `recursive` is true. Prefer moving a file aside \
                  over deleting it when you are unsure. Deleting a folder requires `recursive` \
                  explicitly, because it removes everything inside it.",
    parameters: r#"{
        "type": "object",
        "properties": {
            "path": {
                "type": "string",
                "description": "The file or folder to delete, relative to the workspace root."
            },
            "recursive": {
                "type": "boolean",
                "description": "Required to delete a folder and everything in it. Defaults to false."
            }
        },
        "required": ["path"],
        "additionalProperties": false
    }"#,
    effect: super::Effect::Write,
    command_argument: None,
    execute: |arguments, context| {
        Box::pin(std::future::ready(
            super::file::require_workspace()
                .and_then(|root| delete_file(arguments, root, &context)),
        ))
    },
};

fn move_file(arguments: Value, root: PathBuf) -> Result<String, String> {
    let from = required_path(&arguments, "path")?;
    let to = required_path(&arguments, "destination")?;
    let source = super::file::resolve_within(&root, &from)?;
    let destination = super::file::resolve_within(&root, &to)?;

    if source == destination {
        return Err(format!("`{from}` and `{to}` are the same path."));
    }
    if !source.exists() {
        return Err(format!("`{from}` does not exist."));
    }
    // Refused rather than overwritten. A move onto an existing file is the one way
    // this tool could destroy work the user did not name, and on Windows the
    // underlying rename would fail anyway with a less useful message.
    if destination.exists() {
        return Err(format!(
            "`{to}` already exists, so `{from}` was not moved over it. Delete or move that file \
             first if you mean to replace it."
        ));
    }
    if let Some(parent) = destination.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|error| format!("Could not create {}: {error}", parent.display()))?;
    }

    match std::fs::rename(&source, &destination) {
        Ok(()) => {}
        // A rename across filesystems fails; a copy and a remove is the honest
        // fallback so the operation still means what it says. Reported only if it
        // too fails.
        Err(_) => {
            copy_then_remove(&source, &destination)?;
        }
    }
    Ok(format!("Moved `{from}` to `{to}`."))
}

/// Copies a file, then removes the source, for a cross-device move.
///
/// Only files are copied this way; a directory move across filesystems is refused,
/// because a recursive copy that fails partway leaves a half-moved tree and no way
/// to tell which half.
fn copy_then_remove(source: &std::path::Path, destination: &std::path::Path) -> Result<(), String> {
    if source.is_dir() {
        return Err(format!(
            "Could not move the folder {} (it may be on another drive). Move its contents \
             individually instead.",
            source.display()
        ));
    }
    std::fs::copy(source, destination)
        .map_err(|error| format!("Could not copy to {}: {error}", destination.display()))?;
    std::fs::remove_file(source).map_err(|error| {
        format!(
            "Copied to {} but could not remove the original: {error}",
            destination.display()
        )
    })?;
    Ok(())
}

fn delete_file(
    arguments: Value,
    root: PathBuf,
    context: &super::ToolContext,
) -> Result<String, String> {
    let requested = required_path(&arguments, "path")?;
    let target = super::file::resolve_within(&root, &requested)?;
    let recursive = arguments
        .get("recursive")
        .and_then(Value::as_bool)
        .unwrap_or(false);

    // The root itself is never a target. `resolve_within` already refuses `..`, but
    // `.` and an empty path both resolve to the root, and deleting the workspace
    // out from under the user is not an operation this tool should be able to do.
    if target == root {
        return Err("The workspace root cannot be deleted.".to_string());
    }
    let metadata = std::fs::symlink_metadata(&target)
        .map_err(|error| format!("`{requested}` does not exist: {error}"))?;

    if metadata.is_dir() {
        if !recursive {
            return Err(format!(
                "`{requested}` is a folder. Pass `recursive: true` to delete it and everything \
                 inside it."
            ));
        }
        std::fs::remove_dir_all(&target)
            .map_err(|error| format!("Could not delete the folder `{requested}`: {error}"))?;
        return Ok(format!(
            "Deleted the folder `{requested}` and its contents."
        ));
    }

    // Read the file before removing it, so the panel can show what was lost.
    // A diff of removals is the honest report of a delete, and it is what makes
    // the reply's "what changed" figure include this file. A folder is skipped --
    // there is no single file to read and no line count that would mean anything.
    let previous = std::fs::read_to_string(&target).ok();
    std::fs::remove_file(&target)
        .map_err(|error| format!("Could not delete `{requested}`: {error}"))?;
    if let Some(previous) = previous {
        context.report_diff(super::diff::diff(&previous, ""));
    }
    Ok(format!("Deleted `{requested}`."))
}

fn required_path(arguments: &Value, field: &str) -> Result<String, String> {
    arguments
        .get(field)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|path| !path.is_empty())
        .map(str::to_string)
        .ok_or_else(|| format!("A non-empty string `{field}` is required"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// A context that reports nowhere, so a delete's diff is discarded.
    fn ctx() -> super::super::ToolContext {
        super::super::ToolContext::default()
    }

    fn root() -> PathBuf {
        use std::sync::atomic::{AtomicU32, Ordering};
        static COUNTER: AtomicU32 = AtomicU32::new(0);
        let unique = COUNTER.fetch_add(1, Ordering::Relaxed);
        let dir =
            std::env::temp_dir().join(format!("projectz-paths-{}-{unique}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("temp dir");
        dir
    }

    #[test]
    fn a_file_moves_to_a_new_path() {
        let root = root();
        std::fs::write(root.join("a.txt"), "hi").expect("seed");
        move_file(
            json!({ "path": "a.txt", "destination": "sub/b.txt" }),
            root.clone(),
        )
        .expect("move");
        assert!(!root.join("a.txt").exists());
        assert_eq!(
            std::fs::read_to_string(root.join("sub/b.txt")).unwrap(),
            "hi"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn renaming_in_place_is_a_move() {
        let root = root();
        std::fs::write(root.join("old.rs"), "x").expect("seed");
        move_file(
            json!({ "path": "old.rs", "destination": "new.rs" }),
            root.clone(),
        )
        .expect("rename");
        assert!(root.join("new.rs").exists());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn moving_onto_an_existing_file_is_refused_rather_than_overwriting() {
        let root = root();
        std::fs::write(root.join("a.txt"), "a").expect("seed");
        std::fs::write(root.join("b.txt"), "b").expect("seed");
        let error = move_file(
            json!({ "path": "a.txt", "destination": "b.txt" }),
            root.clone(),
        )
        .expect_err("refused");
        assert!(error.contains("already exists"), "{error}");
        assert_eq!(std::fs::read_to_string(root.join("b.txt")).unwrap(), "b");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_move_outside_the_workspace_is_refused() {
        let root = root();
        std::fs::write(root.join("a.txt"), "a").expect("seed");
        assert!(move_file(
            json!({ "path": "a.txt", "destination": "../out.txt" }),
            root.clone(),
        )
        .expect_err("escape")
        .contains("outside"));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_missing_source_is_reported() {
        let root = root();
        assert!(move_file(
            json!({ "path": "nope.txt", "destination": "x.txt" }),
            root.clone(),
        )
        .expect_err("missing")
        .contains("does not exist"));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_file_is_deleted() {
        let root = root();
        std::fs::write(root.join("a.txt"), "x").expect("seed");
        delete_file(json!({ "path": "a.txt" }), root.clone(), &ctx()).expect("delete");
        assert!(!root.join("a.txt").exists());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_folder_needs_recursive_explicitly() {
        let root = root();
        std::fs::create_dir(root.join("dir")).expect("dir");
        std::fs::write(root.join("dir/a.txt"), "x").expect("seed");
        let error = delete_file(json!({ "path": "dir" }), root.clone(), &ctx())
            .expect_err("needs recursive");
        assert!(error.contains("recursive"), "{error}");
        assert!(root.join("dir").exists());

        delete_file(
            json!({ "path": "dir", "recursive": true }),
            root.clone(),
            &ctx(),
        )
        .expect("delete recursive");
        assert!(!root.join("dir").exists());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn the_workspace_root_cannot_be_deleted() {
        let root = root();
        let error = delete_file(
            json!({ "path": ".", "recursive": true }),
            root.clone(),
            &ctx(),
        )
        .expect_err("refused");
        assert!(error.contains("root"), "{error}");
        assert!(root.exists());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn deleting_a_missing_path_is_reported() {
        let root = root();
        assert!(delete_file(json!({ "path": "nope" }), root.clone(), &ctx())
            .expect_err("missing")
            .contains("does not exist"));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn both_tools_are_writes() {
        assert_eq!(MOVE_FILE.effect, super::super::Effect::Write);
        assert_eq!(DELETE_FILE.effect, super::super::Effect::Write);
    }
}
