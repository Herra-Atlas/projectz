//! Realtime awareness of the workspace on behalf of the Files panel.
//!
//! The tree is fetched on demand, which made it a snapshot: a file the user
//! (or an editor, a build, git) changed after the panel read a folder stayed
//! invisible until something re-read it. This watches the workspace root
//! recursively and tells the panel which folders' listings have changed, so an
//! already-open tree follows what the disk actually holds.
//!
//! One watcher, repointed in place, rather than one per workspace -- a workspace
//! switch must not leave a handle watching the folder the user just left. The
//! root is re-read each loop turn (`ai::tools::workspace::root()` is a lock read,
//! not a query), so a change needs no hook planted in every workspace writer.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::Duration;

use notify::{Event, RecommendedWatcher, RecursiveMode, Watcher};
use serde::Serialize;
use tauri::{AppHandle, Emitter};

use super::fs::should_skip;
use crate::ai::tools::workspace;

/// How long the wait is between the loop's other duties.
///
/// The same number does two jobs: it is the granularity at which the current
/// workspace is re-checked, and the window a burst of writes is folded into
/// before anything reaches the frontend.
const TICK: Duration = Duration::from_millis(250);

/// The event name the tree listens for.
pub const EVENT: &str = "panel_fs_changed";

/// The folders whose listings changed, workspace-relative and forward-slashed.
///
/// An array (even though a full tree refresh of the *root* could serve) because
/// the tree refreshes folders it has already drawn and leaves the rest alone:
/// every open folder keeps its state, and a folder nobody expanded costs no IPC.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WatchPayload {
    /// `""` means the workspace root.
    pub folders: Vec<String>,
}

/// Starts the watcher for the life of the app.
///
/// Its own thread, like the scheduler and the taste loop: the watcher parks on a
/// channel receive, so idle cost is one parked thread and it never borrows the
/// app or runtime a command may be using.
pub fn start(app: AppHandle) {
    std::thread::spawn(move || watch_loop(app));
}

fn watch_loop(app: AppHandle) {
    let (tx, rx) = mpsc::channel::<Result<Event, notify::Error>>();
    // What the handle is pointed at, which is how a workspace change is seen
    // without a hook in every place a root can be set or cleared.
    let mut watched: Option<PathBuf> = None;
    // The live watch, held between re-points: nothing reads it, because the
    // watcher is *kept alive* rather than read -- dropping it is what un-watches
    // a folder the user has left. The underscore is the story of that value.
    let mut _watcher: Option<RecommendedWatcher> = None;

    loop {
        if watched != workspace::root() {
            // Forwarding the same sender: events from the new workspace reach
            // the same loop that is dropping the old handle.
            match workspace::root() {
                Some(root) => {
                    _watcher = watch_root(&tx, &root);
                    // The root is recorded whatever the watch did, so a backend
                    // that cannot watch does not re-attempt every tick: the
                    // tree keeps working by hand rather than churning.
                    watched = Some(root);
                }
                None => {
                    _watcher = None;
                    watched = None;
                }
            }
        }

        // One sit-down between chores is also one debounce window: whatever lands
        // is gathered with everything already queued, so a save that produces a
        // dozen events reports the folders that changed once, not twelve times.
        let mut folders = BTreeSet::new();
        if let Ok(Ok(event)) = rx.recv_timeout(TICK) {
            // The root in force now, not the one being seen from here on out: a
            // workspace switch mid-batch is rarer than the cost of caring, and a
            // folder computed against the wrong root is only ever lost to the
            // dedup, never sent outside the workspace.
            let root = watched.clone();
            if let Some(root) = root {
                record(&root, &event, &mut folders);
                while let Ok(more) = rx.try_recv() {
                    if let Ok(more) = more {
                        record(&root, &more, &mut folders);
                    }
                }
            }
        }

        if !folders.is_empty() {
            let _ = app.emit(
                EVENT,
                &WatchPayload {
                    folders: folders.into_iter().collect(),
                },
            );
        }
    }
}

fn watch_root(
    tx: &mpsc::Sender<Result<Event, notify::Error>>,
    root: &Path,
) -> Option<RecommendedWatcher> {
    let mut watcher = match notify::recommended_watcher(tx.clone()) {
        Ok(watcher) => watcher,
        Err(_) => return None,
    };
    match watcher.watch(root, RecursiveMode::Recursive) {
        Ok(()) => Some(watcher),
        Err(_) => None,
    }
}

/// Adds the folders `event` touches, if any span the workspace and matter.
fn record(root: &Path, event: &Event, folders: &mut BTreeSet<String>) {
    for path in &event.paths {
        if let Some(folder) = relative_folder(root, path) {
            folders.insert(folder);
        }
    }
}

/// The path in its ordinary form, without Windows' extended-length wrapper.
///
/// Canonical roots and watcher reports both carry `\\?\C:\...` (and
/// `\\?\UNC\server\...` for network shares), and a component comparison would
/// otherwise be decided by *spelling* rather than location -- `VerbatimDisk`
/// and `Disk` are different variants, so one written form against the other is
/// always a `None`. Folding the wrapper off both sides makes the compare a
/// question of where the path is, which is the only thing that matters here.
fn plain(path: &Path) -> String {
    let text = path.as_os_str().to_string_lossy();
    if let Some(rest) = text.strip_prefix(r"\\?\UNC\") {
        return format!(r"\\{rest}");
    }
    if let Some(rest) = text.strip_prefix(r"\\?\") {
        return rest.to_owned();
    }
    text.into()
}

/// The folder whose listing the change belongs in: the changed item's parent as
/// a workspace-relative, forward-slashed path. `None` when the path is the root
/// itself, is outside the workspace, or sits under a folder the tree never
/// draws.
fn relative_folder(root: &Path, path: &Path) -> Option<String> {
    let plain_root = plain(root);
    let plain_path = plain(path);
    let mut root_parts = Path::new(&plain_root).components().collect::<Vec<_>>();
    let mut path_parts = Path::new(&plain_path).components().collect::<Vec<_>>();

    while let Some(root_part) = root_parts.first() {
        if path_parts.first() != Some(root_part) {
            return None;
        }
        path_parts.remove(0);
        root_parts.remove(0);
    }
    // Empty means the watched root itself was the changed path. Its listing only
    // changes through the entries in it, which arrive as `root/<name>`, so there
    // is nothing to name without a changed item.
    let Some(_) = path_parts.first() else {
        return None;
    };

    if path_parts
        .iter()
        .any(|part| part.as_os_str().to_str().is_some_and(should_skip))
    {
        return None;
    }

    // The parent of the touching path: everything but its last component is a
    // folder along the way, so all of them but the last are the parent's path.
    let parent_len = path_parts.len() - 1;
    if parent_len == 0 {
        return Some(String::new());
    }
    Some(
        path_parts[..parent_len]
            .iter()
            .map(|part| part.as_os_str().to_string_lossy().replace('\\', "/"))
            .collect::<Vec<_>>()
            .join("/"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A unique temporary directory for one test, the shape `workspace`'s tests
    /// use: a per-process counter, so parallel tests never share one folder and
    /// the roots do not exist and are never written -- `relative_folder` reads
    /// no paths, it only walks components.
    fn temp_root(label: &str) -> PathBuf {
        use std::sync::atomic::{AtomicU32, Ordering};
        static COUNTER: AtomicU32 = AtomicU32::new(0);
        let unique = COUNTER.fetch_add(1, Ordering::Relaxed);
        std::env::temp_dir().join(format!(
            "projectz-watch-{}-{label}-{unique}",
            std::process::id()
        ))
    }

    /// A file dropped straight into the root refreshes the root, which is the
    /// empty string.
    #[test]
    fn a_file_at_the_root_names_the_root() {
        let root = temp_root("plain");
        assert_eq!(
            relative_folder(&root, &root.join("readme.md")),
            Some("".into())
        );
    }

    /// A change deeper in names the folder that holds it, forward-slashed.
    #[test]
    fn a_nested_change_names_its_folder() {
        let root = temp_root("nested");
        assert_eq!(
            relative_folder(&root, &root.join("src").join("pages").join("app.tsx")),
            Some("src/pages".into())
        );
    }

    /// A path that does not sit under the root is dropped. Windows may hand an
    /// event about the OS's own temp churn; a map against the wrong root would
    /// be a hideous refresh, an outside path is only ever a None.
    #[test]
    fn a_path_outside_the_root_is_dropped() {
        let root = temp_root("outside");
        let sibling = std::env::temp_dir().join("projectz-not-the-workspace");
        assert_eq!(relative_folder(&root, &sibling), None);
    }

    /// Churn under a folder the tree refuses to draw costs no refresh: the
    /// listing would not have shown it.
    #[test]
    fn a_change_under_a_skipped_folder_is_dropped() {
        let root = temp_root("skipped");
        assert_eq!(
            relative_folder(
                &root,
                &root.join("node_modules").join("left-pad").join("index.js")
            ),
            None
        );
    }

    /// The extended-length form matches the canonical root. This is the shape a
    /// Windows watcher actually reports, and the prefix would otherwise strand
    /// every event as None.
    #[cfg(windows)]
    #[test]
    fn the_windows_extended_prefix_matches_the_canonical_root() {
        let root = temp_root("ext");
        let extended = PathBuf::from(format!(r"\\?\{}", root.display()));
        assert_eq!(
            relative_folder(&root, &extended.join("notes.txt")),
            Some("".into())
        );
    }
}
