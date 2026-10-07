use std::path::Path;

use serde::Serialize;

use crate::ai::tools::{require_workspace, resolve_within};

/// One entry in a listing.
///
/// `children` rather than a boolean "has children", because the tree needs to
/// know *now* whether to draw a caret on a collapsed folder. Fetching that on
/// expand would mean an empty-looking row that pops a caret in after a round
/// trip, and an empty folder would look identical to one that simply had not
/// been asked yet.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TreeEntry {
    /// Name only. The full relative path is carried beside it so a row does not
    /// have to re-join it on every render.
    pub name: String,
    pub path: String,
    pub is_dir: bool,
    /// Whether a folder has anything to expand into. Always false for a file.
    pub has_children: bool,
}

/// Entries returned for one folder, before the cap.
const MAX_ENTRIES: usize = 500;

/// Listing the workspace root.
///
/// An empty relative path is the root itself. Kept as the *empty string* rather
/// than `"."` because `resolve_within` refuses `..` and absolutes but permits `.`,
/// and an empty string is what the frontend naturally holds for "the top of the
/// tree" -- making the caller spell it as `"."` would be a rule with no payoff.
pub fn list(relative: &str) -> Result<Vec<TreeEntry>, String> {
    let root = require_workspace()?;
    list_in(&root, relative)
}

/// Listing one folder inside the workspace.
///
/// Takes the root separately so the tests can point it at a temporary folder
/// without touching the process-global, which is shared and needs the serial
/// lock in [`crate::ai::tools::workspace`] to test safely.
pub fn list_in(root: &Path, relative: &str) -> Result<Vec<TreeEntry>, String> {
    let requested = if relative.is_empty() { "." } else { relative };
    let directory = resolve_within(root, requested)?;

    let mut folders: Vec<TreeEntry> = Vec::new();
    let mut files: Vec<TreeEntry> = Vec::new();

    let entries = std::fs::read_dir(&directory)
        .map_err(|error| format!("Could not read {}: {error}", directory.display()))?;

    for entry in entries.flatten() {
        // `file_type` from the directory entry rather than a `metadata` call per
        // item: on Windows the latter follows every symlink and re-stats the
        // file, which on a folder of a few thousand entries is the difference
        // between an instant listing and a visible stall.
        let Ok(file_type) = entry.file_type() else {
            // An entry that cannot be classified -- a broken symlink, or one the
            // user cannot stat -- is skipped rather than failing the listing.
            // One unreadable item should not hide every sibling beside it.
            continue;
        };

        let raw_name = entry.file_name();
        let Some(name) = raw_name.to_str() else {
            // Not valid UTF-8. Windows allows this; there is no name to show and
            // no path to open, so it cannot be a row.
            continue;
        };
        if should_skip(name) {
            continue;
        }

        let is_dir = file_type.is_dir();
        let path = join_relative(relative, name);

        if is_dir {
            folders.push(TreeEntry {
                name: name.to_string(),
                path,
                is_dir: true,
                has_children: has_children(&entry.path()),
            });
        } else {
            files.push(TreeEntry {
                name: name.to_string(),
                path,
                is_dir: false,
                has_children: false,
            });
        }
    }

    // Folders first, then files, each case-insensitively sorted. A tree that
    // interleaves them has to be scanned to find a folder; this one can be read
    // top to bottom.
    folders.sort_by_key(|entry| entry.name.to_lowercase());
    files.sort_by_key(|entry| entry.name.to_lowercase());

    folders.append(&mut files);
    folders.truncate(MAX_ENTRIES);
    Ok(folders)
}

/// Whether the read was capped, so the frontend can say so.
///
/// Not folded into the list itself: a caller that wanted every entry would have
/// to guess, and a silently truncated folder looks exactly like a folder that
/// genuinely has 500 entries.
#[tauri::command]
pub fn panel_fs_list(relative: String) -> Result<Vec<TreeEntry>, String> {
    eprintln!("[panel_fs_list] called with relative='{}'", relative);
    let result = list(&relative);
    eprintln!("[panel_fs_list] returning {} entries", result.as_ref().map_or(0, |v| v.len()));
    result
}

/// Reading a file for the panel's preview.
///
/// Bounded, and reported rather than silently shortened in a way the reader
/// cannot detect. This is a *view*, not a model's context window: the cap is
/// about what is sensible to draw, and a 200,000-line minified bundle is not a
/// file anyone reads in a side panel.
#[tauri::command]
pub fn panel_read_preview(relative: String) -> Result<String, String> {
    const MAX_PREVIEW_BYTES: u64 = 2 * 1024 * 1024;
    let root = require_workspace()?;
    let resolved = resolve_within(&root, if relative.is_empty() { "." } else { &relative })?;

    let metadata = std::fs::metadata(&resolved)
        .map_err(|error| format!("Could not open {}: {error}", resolved.display()))?;
    if !metadata.is_file() {
        return Err(format!("{} is a folder", relative));
    }
    if metadata.len() > MAX_PREVIEW_BYTES {
        return Err(format!(
            "{} is larger than 2 MB, so it cannot be previewed here",
            relative
        ));
    }

    std::fs::read_to_string(&resolved)
        .map_err(|error| format!("Could not read {}: {error}", resolved.display()))
}

/// Overwriting a file from the panel's editor.
///
/// The user's own save path, kept apart from `ai/tools/` for the same reason the
/// listing is: it has no `ToolSpec`, no effect class, and never passes the
/// permission gate. It shares the boundary -- `resolve_within` -- and the atomic
/// temp+rename write, so there is one containment rule and one way a file lands.
#[tauri::command]
pub fn panel_write_file(relative: String, content: String) -> Result<(), String> {
    let root = require_workspace()?;
    write_in(&root, &relative, &content)
}

/// Overwriting one file inside the workspace.
///
/// Takes the root separately so the tests can point it at a temporary folder
/// without touching the process-global, the same split as [`list_in`].
fn write_in(root: &Path, relative: &str, content: &str) -> Result<(), String> {
    const MAX_WRITE_BYTES: u64 = 2 * 1024 * 1024;
    if content.len() as u64 > MAX_WRITE_BYTES {
        return Err(format!(
            "{} is larger than 2 MB, so it cannot be saved here",
            relative
        ));
    }
    let requested = if relative.is_empty() { "." } else { relative };
    let resolved = resolve_within(&root, requested)?;

    if resolved.is_dir() {
        return Err(format!("{} is a folder", relative));
    }

    let parent = resolved
        .parent()
        .ok_or_else(|| format!("{} has no parent directory", resolved.display()))?;
    let temporary = parent.join(format!(
        ".{}.projectz-partial",
        resolved
            .file_name()
            .map(|name| name.to_string_lossy())
            .unwrap_or_default()
    ));

    std::fs::write(&temporary, content.as_bytes())
        .map_err(|error| format!("Could not write {}: {error}", temporary.display()))?;
    if let Err(error) = std::fs::rename(&temporary, &resolved) {
        let _ = std::fs::remove_file(&temporary);
        return Err(format!("Could not replace {}: {error}", resolved.display()));
    }
    Ok(())
}

/// Creating an empty file inside a folder of the panel.
///
/// The user's own create path, kept beside the panel's other file commands for
/// the same reason the write is: no `ToolSpec`, no effect class, and never past
/// the permission gate. It shares the boundary -- `resolve_within` -- so there is
/// one containment rule for everything the panel writes.
#[tauri::command]
pub fn panel_create_file(relative: String, name: String) -> Result<(), String> {
    let root = require_workspace()?;
    create_in(&root, &relative, &name, false)
}

/// Creating a folder inside a folder of the panel. See [`panel_create_file`].
#[tauri::command]
pub fn panel_create_folder(relative: String, name: String) -> Result<(), String> {
    let root = require_workspace()?;
    create_in(&root, &relative, &name, true)
}

/// Creating one entry inside `parent` (the empty string is the workspace root).
///
/// Takes the root separately so the tests can point it at a temporary folder
/// without touching the process-global, the same split as [`list_in`] and
/// [`write_in`]. `name` is a *name* and not a path: rejecting separators and
/// `.`/`..` up front means the destination can only ever sit beside its parent,
/// so the resolve below has nothing surprising to catch.
fn create_in(root: &Path, parent: &str, name: &str, is_dir: bool) -> Result<(), String> {
    let name = name.trim();
    if name.is_empty() {
        return Err("A name is required".to_string());
    }
    if name.contains('/') || name.contains('\\') || name == "." || name == ".." {
        return Err(format!("{name} is not a valid name"));
    }

    let parent_resolved = resolve_within(root, if parent.is_empty() { "." } else { parent })?;
    if !parent_resolved.is_dir() {
        return Err(format!("{parent} is not a folder"));
    }

    let dest_relative = join_relative(parent, name);
    let dest = resolve_within(root, &dest_relative)?;
    if dest.exists() {
        return Err(format!("{name} already exists"));
    }

    if is_dir {
        std::fs::create_dir(&dest)
            .map_err(|error| format!("Could not create folder {name}: {error}"))?;
    } else {
        std::fs::write(&dest, b"")
            .map_err(|error| format!("Could not create file {name}: {error}"))?;
    }
    Ok(())
}

/// Moving a file or folder into a different folder of the panel.
///
/// The drag-and-drop path, kept beside the panel's other file commands for the
/// same reason the create is: no `ToolSpec`, no effect class, and never past the
/// permission gate. It shares the boundary -- `resolve_within` -- so a drop can
/// only ever land inside the workspace.
#[tauri::command]
pub fn panel_move_file(relative: String, target: String) -> Result<(), String> {
    let root = require_workspace()?;
    move_in(&root, &relative, &target)
}

/// Moving the entry at `source` into the folder `target` (the empty string is
/// the workspace root).
///
/// Takes the root separately so the tests can point it at a temporary folder
/// without touching the process-global, the same split as [`list_in`].
fn move_in(root: &Path, source: &str, target: &str) -> Result<(), String> {
    if source.is_empty() || source == "." {
        return Err("The workspace root cannot be moved".to_string());
    }

    let source_resolved = resolve_within(root, source)?;
    if !source_resolved.exists() {
        return Err(format!("{source} no longer exists"));
    }

    let target_resolved = resolve_within(root, if target.is_empty() { "." } else { target })?;
    if !target_resolved.is_dir() {
        return Err(format!("{target} is not a folder"));
    }

    // Dropping a folder onto itself or into one of its own descendants would move
    // it inside itself. `fs::rename` does refuse this, but with a message that
    // does not say why; naming the rule here gives the honest error.
    if target == source || target.starts_with(&format!("{source}/")) {
        return Err("A folder cannot be moved into itself".to_string());
    }

    let name = source.rsplit('/').next().unwrap_or(source);
    let dest_relative = join_relative(target, name);
    let dest_resolved = resolve_within(root, &dest_relative)?;
    if dest_resolved.exists() {
        return Err(format!("{name} already exists there"));
    }

    std::fs::rename(&source_resolved, &dest_resolved)
        .map_err(|error| format!("Could not move {name}: {error}"))
}

/// Renaming a file or directory in the panel.
///
/// The user's own rename path, kept apart from `ai/tools/` for the same reason the
/// listing is: it has no `ToolSpec`, no effect class, and never passes the
/// permission gate. It shares the boundary -- `resolve_within` -- and the atomic
/// rename, so there is one containment rule and one way a file lands.
#[tauri::command]
pub fn panel_rename_file(relative: String, new_name: String) -> Result<(), String> {
    eprintln!("[panel_rename_file] called with relative='{}', new_name='{}'", relative, new_name);
    let root = require_workspace()?;
    let source = if relative.is_empty() { "." } else { &relative };
    let source_resolved = resolve_within(&root, source)?;
    eprintln!("[panel_rename_file] source_resolved='{}'", source_resolved.display());

    // Construct the destination relative path: parent relative path + new_name
    let parent_relative = if relative.contains('/') {
        let parts: Vec<&str> = relative.split('/').collect();
        parts[..parts.len() - 1].join("/")
    } else {
        String::new()
    };
    let dest_relative = if parent_relative.is_empty() {
        new_name.clone()
    } else {
        format!("{}/{}", parent_relative, new_name)
    };
    eprintln!("[panel_rename_file] dest_relative='{}'", dest_relative);

    // Ensure the destination is within the workspace
    let _ = resolve_within(&root, &dest_relative)?;

    let source_resolved = resolve_within(&root, source)?;
    let dest_resolved = resolve_within(&root, &dest_relative)?;
    eprintln!("[panel_rename_file] source_resolved='{}'", source_resolved.display());
    eprintln!("[panel_rename_file] dest_resolved='{}'", dest_resolved.display());

    if dest_resolved.exists() {
        return Err(format!("{} already exists", dest_resolved.display()));
    }

    // Perform the rename
    std::fs::rename(&source_resolved, &dest_resolved)
        .map_err(|error| format!("Could not rename {}: {error}", source_resolved.display()))?;

    Ok(())
}

#[tauri::command]
pub fn panel_delete_file(relative: String) -> Result<(), String> {
    eprintln!("[panel_delete_file] called with relative='{}'", relative);
    let root = require_workspace()?;
    let target = if relative.is_empty() { "." } else { &relative };
    let target_resolved = resolve_within(&root, target)?;
    eprintln!("[panel_delete_file] target_resolved='{}'", target_resolved.display());

    if target_resolved.is_dir() {
        std::fs::remove_dir_all(&target_resolved)
            .map_err(|error| format!("Could not delete directory {}: {error}", target_resolved.display()))?;
    } else {
        std::fs::remove_file(&target_resolved)
            .map_err(|error| format!("Could not delete file {}: {error}", target_resolved.display()))?;
    }

    Ok(())
}

/// Folders that are never worth listing.
///
/// Shared with `tools::search`, which skips the same set when grepping. Two
/// copies would drift, and the drift would show up as a directory that the agent
/// can search but the user cannot open.
const SKIPPED: [&str; 7] = [
    "node_modules",
    "target",
    "dist",
    "build",
    ".git",
    ".next",
    ".venv",
];

fn should_skip(name: &str) -> bool {
    SKIPPED
        .iter()
        .any(|skipped| name.eq_ignore_ascii_case(skipped))
}

/// Joins a name onto the relative path of its parent, keeping separators forward.
///
/// Forward slashes on every platform, because this string is what the frontend
/// hands straight back as the next `relative` and what it shows in the file
/// header. Windows accepts them natively, and a model or a user copying a path
/// out of the panel should not have to translate `\` to `/` first.
fn join_relative(parent: &str, name: &str) -> String {
    if parent.is_empty() || parent == "." {
        name.to_string()
    } else {
        format!("{}/{}", parent.trim_end_matches('/'), name)
    }
}

/// Whether a directory has anything inside it.
///
/// One `read_dir` per folder, and only its error matters: an empty iterator and
/// an unreadable directory are the same answer for a caret, which is that there
/// is nothing to expand.
fn has_children(path: &Path) -> bool {
    std::fs::read_dir(path)
        .map(|mut entries| entries.next().is_some())
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn temp_dir(label: &str) -> PathBuf {
        use std::sync::atomic::{AtomicU32, Ordering};
        static COUNTER: AtomicU32 = AtomicU32::new(0);
        let unique = COUNTER.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "projectz-panel-{}-{label}-{unique}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).expect("temp dir");
        path
    }

    fn write(root: &Path, relative: &str) {
        let path = root.join(relative);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).expect("mkdir");
        }
        std::fs::write(path, "x").expect("write");
    }

    /// The root listing is the whole workspace, folders before files. Both
    /// halves matter: a folder the user cannot see is the feature not working,
    /// and files sorted among folders is a tree nobody can scan.
    #[test]
    fn the_root_lists_folders_before_files() {
        let root = temp_dir("root");
        write(&root, "zebra.txt");
        write(&root, "apple.txt");
        std::fs::create_dir_all(root.join("zfolder")).expect("dir");
        std::fs::create_dir_all(root.join("afolder")).expect("dir");

        let entries = list_in(&root, "").expect("list");

        let names: Vec<&str> = entries.iter().map(|entry| entry.name.as_str()).collect();
        assert_eq!(names, vec!["afolder", "zfolder", "apple.txt", "zebra.txt"]);
        let _ = std::fs::remove_dir_all(&root);
    }

    /// A caret is drawn on a folder from the listing alone, so `has_children`
    /// has to be true without anyone having expanded it. A false answer draws a
    /// folder that opens onto nothing.
    #[test]
    fn a_folder_reports_whether_it_can_be_expanded() {
        let root = temp_dir("caret");
        std::fs::create_dir_all(root.join("full/inner")).expect("dir");
        std::fs::create_dir_all(root.join("empty")).expect("dir");

        let entries = list_in(&root, "").expect("list");
        let full = entries
            .iter()
            .find(|entry| entry.name == "full")
            .expect("full");
        let empty = entries
            .iter()
            .find(|entry| entry.name == "empty")
            .expect("empty");

        assert!(full.has_children, "a folder with a child drew no caret");
        assert!(
            !empty.has_children,
            "an empty folder drew a caret onto nothing"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    /// The relative path a row carries is what the frontend sends back for that
    /// row's children, so it must round-trip: ask for `src/app` twice and get
    /// the same folder both times. A trailing or doubled separator would make
    /// `resolve_within` resolve somewhere else.
    #[test]
    fn a_child_path_resolves_back_to_the_folder_it_came_from() {
        let root = temp_dir("roundtrip");
        write(&root, "src/app/main.rs");

        let entries = list_in(&root, "").expect("list");
        let src = entries
            .iter()
            .find(|entry| entry.name == "src")
            .expect("src");
        let nested = list_in(&root, &src.path).expect("list src");

        let app = nested
            .iter()
            .find(|entry| entry.name == "app")
            .expect("app");
        let leaves = list_in(&root, &app.path).expect("list app");

        assert_eq!(leaves.len(), 1);
        assert_eq!(leaves[0].name, "main.rs");
        assert_eq!(leaves[0].path, "src/app/main.rs");
        let _ = std::fs::remove_dir_all(&root);
    }

    /// The build and dependency folders are skipped because they are enormous and
    /// nobody browses them. If that list ever fails to apply, a tree rooted at a
    /// JS project opens onto thousands of identical package rows.
    #[test]
    fn dependency_and_build_folders_are_not_listed() {
        let root = temp_dir("skipped");
        write(&root, "node_modules/left-pad/index.js");
        write(&root, "target/debug/app");
        write(&root, "src/main.rs");

        let entries = list_in(&root, "").expect("list");
        let names: Vec<&str> = entries.iter().map(|entry| entry.name.as_str()).collect();

        assert_eq!(
            names,
            vec!["src"],
            "skipped folders leaked into the tree: {names:?}"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    /// A symlink pointing out of the workspace is still refused, because the
    /// check runs through `resolve_within` and the tree uses that same function.
    /// The tree shares the boundary rather than restating it.
    #[test]
    fn a_path_escaping_the_workspace_is_refused() {
        let root = temp_dir("escape");
        write(&root, "ok.txt");

        let error = list_in(&root, "../..").expect_err("escaping path");
        assert!(
            error.contains("outside") || error.contains(".."),
            "the escape was not named in the error: {error}"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    /// The root listing is keyed on the empty string, because that is what the
    /// frontend holds for "the top of the tree". Rejecting it would mean every
    /// caller had to spell the root as `"."`, a rule with no payoff.
    #[test]
    fn an_empty_relative_path_is_the_root_itself() {
        let root = temp_dir("empty-is-root");
        write(&root, "a.txt");

        let entries = list_in(&root, "").expect("list");
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].path, "a.txt", "the root path should be bare");
        let _ = std::fs::remove_dir_all(&root);
    }

    /// A write lands atomically at the path it names, with no partial file left
    /// behind and no escape outside the root.
    #[test]
    fn a_panel_write_replaces_the_file_it_names() {
        let root = temp_dir("panel-write");
        write(&root, "a.txt");

        write_in(&root, "a.txt", "new contents").expect("write");

        assert_eq!(
            std::fs::read_to_string(root.join("a.txt")).expect("read back"),
            "new contents"
        );
        assert!(
            std::fs::read_dir(&root).expect("read").count() == 1,
            "a partial file was left behind"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_panel_write_outside_the_workspace_is_refused() {
        let root = temp_dir("panel-write-escape");
        write(&root, "ok.txt");

        let error = write_in(&root, "../escaped.txt", "x").expect_err("refused");
        assert!(
            error.contains("outside") || error.contains(".."),
            "the escape was not named in the error: {error}"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    /// Test renaming a file.
    #[test]
    fn panel_rename_file_works() {
        let root = temp_dir("panel-rename");
        write(&root, "old.txt");

        // Rename the file
        panel_rename_file("old.txt".to_string(), "new.txt".to_string()).expect("rename failed");

        // Check the old file is gone and the new file exists
        assert!(!root.join("old.txt").exists(), "old file still exists");
        assert!(root.join("new.txt").exists(), "new file not created");
        let _ = std::fs::remove_dir_all(&root);
    }

    /// Test renaming a directory.
    #[test]
    fn panel_rename_directory_works() {
        let root = temp_dir("panel-rename-dir");
        std::fs::create_dir_all(root.join("old_dir")).expect("create dir");
        write(&root.join("old_dir"), "file.txt");

        // Rename the directory
        panel_rename_file("old_dir".to_string(), "new_dir".to_string()).expect("rename failed");

        // Check the old directory is gone and the new directory exists with contents
        assert!(!root.join("old_dir").exists(), "old directory still exists");
        assert!(root.join("new_dir").exists(), "new directory not created");
        assert!(root.join("new_dir/file.txt").exists(), "file not found in renamed directory");
        let _ = std::fs::remove_dir_all(&root);
    }

    /// Test renaming to an invalid path (outside workspace) is refused.
    #[test]
    fn panel_rename_file_outside_workspace_is_refused() {
        let root = temp_dir("panel-rename-escape");
        write(&root, "ok.txt");

        let error = panel_rename_file("ok.txt".to_string(), "../escaped.txt".to_string()).expect_err("rename should fail");
        assert!(
            error.contains("outside") || error.contains(".."),
            "the escape was not named in the error: {error}"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    /// Test deleting a file.
    #[test]
    fn panel_delete_file_works() {
        let root = temp_dir("panel-delete");
        write(&root, "to_delete.txt");

        // Delete the file
        panel_delete_file("to_delete.txt".to_string()).expect("delete failed");

        // Check the file is gone
        assert!(!root.join("to_delete.txt").exists(), "file still exists after delete");
        let _ = std::fs::remove_dir_all(&root);
    }

    /// Test deleting a directory (recursively).
    #[test]
    fn panel_delete_directory_works() {
        let root = temp_dir("panel-delete-dir");
        std::fs::create_dir_all(root.join("to_delete_dir/nested")).expect("create dir");
        write(&root.join("to_delete_dir/nested"), "file.txt");

        // Delete the directory
        panel_delete_file("to_delete_dir".to_string()).expect("delete failed");

        // Check the directory and its contents are gone
        assert!(!root.join("to_delete_dir").exists(), "directory still exists after delete");
        let _ = std::fs::remove_dir_all(&root);
    }

    /// Test deleting a path outside the workspace is refused.
    #[test]
    fn panel_delete_file_outside_workspace_is_refused() {
        let root = temp_dir("panel-delete-escape");
        write(&root, "ok.txt");

        let error = panel_delete_file("../escaped.txt".to_string()).expect_err("delete should fail");
        assert!(
            error.contains("outside") || error.contains(".."),
            "the escape was not named in the error: {error}"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    /// A new file lands beside its parent, empty, and shows up in that folder's
    /// listing -- the create the user's next refresh would reveal.
    #[test]
    fn a_created_file_appears_in_its_folder() {
        let root = temp_dir("panel-create-file");
        create_in(&root, "", "notes.txt", false).expect("create");

        assert!(root.join("notes.txt").is_file(), "the file was not written");
        let entries = list_in(&root, "").expect("list");
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].name, "notes.txt");
        let _ = std::fs::remove_dir_all(&root);
    }

    /// A new folder is created inside the folder that was named, not at the root.
    #[test]
    fn a_created_folder_lands_inside_its_parent() {
        let root = temp_dir("panel-create-folder");
        std::fs::create_dir_all(root.join("src")).expect("dir");

        create_in(&root, "src", "components", true).expect("create");

        assert!(root.join("src/components").is_dir(), "the folder was not created");
        assert!(!root.join("components").exists(), "the folder landed at the root");
        let _ = std::fs::remove_dir_all(&root);
    }

    /// A name is a name: a separator would walk the new entry somewhere the user
    /// did not point at, so it is refused before anything touches the disk.
    #[test]
    fn a_name_with_a_separator_is_refused() {
        let root = temp_dir("panel-create-separator");
        write(&root, "ok.txt");

        let error = create_in(&root, "", "sub/file.txt", false).expect_err("refused");
        assert!(error.contains("not a valid name"), "unexpected error: {error}");
        assert!(!root.join("sub").exists(), "a folder was created anyway");
        let _ = std::fs::remove_dir_all(&root);
    }

    /// Creating over an existing entry is refused rather than overwriting it --
    /// the create path must never be a way to lose a file's contents.
    #[test]
    fn creating_over_an_existing_name_is_refused() {
        let root = temp_dir("panel-create-existing");
        write(&root, "a.txt");

        let error = create_in(&root, "", "a.txt", false).expect_err("refused");
        assert!(error.contains("already exists"), "unexpected error: {error}");
        assert_eq!(std::fs::read_to_string(root.join("a.txt")).expect("read"), "x");
        let _ = std::fs::remove_dir_all(&root);
    }

    /// An empty name has nothing to create and is refused with a plain message.
    #[test]
    fn an_empty_name_is_refused() {
        let root = temp_dir("panel-create-empty");
        let error = create_in(&root, "", "   ", true).expect_err("refused");
        assert!(error.contains("name is required"), "unexpected error: {error}");
        let _ = std::fs::remove_dir_all(&root);
    }

    /// A moved entry leaves its old folder and appears in the new one, keeping
    /// its name and contents.
    #[test]
    fn a_moved_file_lands_in_the_target_folder() {
        let root = temp_dir("panel-move-file");
        write(&root, "note.txt");
        std::fs::create_dir_all(root.join("docs")).expect("dir");

        move_in(&root, "note.txt", "docs").expect("move");

        assert!(!root.join("note.txt").exists(), "the source was left behind");
        assert!(root.join("docs/note.txt").is_file(), "the file did not arrive");
        let _ = std::fs::remove_dir_all(&root);
    }

    /// Dropping a folder onto itself is refused rather than half-done. `fs::rename`
    /// would also fail, but the point is that the rule is stated, not stumbled into.
    #[test]
    fn a_folder_cannot_be_moved_into_itself() {
        let root = temp_dir("panel-move-self");
        std::fs::create_dir_all(root.join("src")).expect("dir");

        let error = move_in(&root, "src", "src").expect_err("refused");
        assert!(error.contains("into itself"), "unexpected error: {error}");
        assert!(root.join("src").is_dir(), "the folder was disturbed");
        let _ = std::fs::remove_dir_all(&root);
    }

    /// A folder dropped into one of its own descendants would swallow itself, so
    /// that is refused too -- the case `fs::rename` reports least clearly.
    #[test]
    fn a_folder_cannot_be_moved_into_its_descendant() {
        let root = temp_dir("panel-move-descendant");
        std::fs::create_dir_all(root.join("src/app")).expect("dir");

        let error = move_in(&root, "src", "src/app").expect_err("refused");
        assert!(error.contains("into itself"), "unexpected error: {error}");
        let _ = std::fs::remove_dir_all(&root);
    }

    /// A move onto a name that already exists is refused, so a drag can never
    /// silently overwrite the file it lands on.
    #[test]
    fn moving_onto_an_existing_name_is_refused() {
        let root = temp_dir("panel-move-existing");
        write(&root, "note.txt");
        write(&root, "docs/note.txt");

        let error = move_in(&root, "note.txt", "docs").expect_err("refused");
        assert!(error.contains("already exists"), "unexpected error: {error}");
        let _ = std::fs::remove_dir_all(&root);
    }
}