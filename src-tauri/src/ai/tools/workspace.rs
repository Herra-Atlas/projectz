//! The folder the agent is allowed to work in.
//!
//! # Why this is not `current_dir`
//!
//! Every filesystem tool used to resolve against `std::env::current_dir()`, which
//! is whatever directory the process was *launched* from. That is not a workspace:
//!
//! - `npm run tauri dev` from `projectz/` put the root at `projectz/`, so a
//!   relative path could not reach `projectz/src/App.tsx`.
//! - `tauri dev` from `src-tauri/` put the root one level down, and the agent's
//!   file tools and the terminal silently disagreed about which tree they were
//!   in.
//! - Double-clicking the installed `.exe` leaves the root wherever Explorer put
//!   the shortcut's working directory, so *every* file tool pointed at the wrong
//!   tree -- and, because the containment check in `resolve_within` resolves
//!   against this same root, so did the boundary that stops a path escaping it.
//!
//! The root is now a folder the user picked, held in a process-global so a tool
//! function pointer can still read it (see [`file::workspace`]). It is set once
//! at startup from the stored selection, and whenever the user picks another.
//!
//! **There is no default.** With no workspace chosen the root is `None` and every
//! tool refuses, which is deliberate on two counts: a default would put a real
//! folder behind tools the user never granted access to, and a hardcoded default
//! would be a path that does not exist on any machine but the author's.
//!
//! # Why a process-global rather than a parameter
//!
//! A tool's executor is a bare `fn(Value, ToolContext) -> impl Future`, so it
//! cannot capture the root. Threading it through `ToolContext` would work and is
//! the better shape, but it touches every tool and the registry for a value that
//! cannot change mid-run -- the root is chosen between runs, not during one.
//! A global with a setter is the smaller change with the same behaviour.

use std::path::{Path, PathBuf};
use std::sync::RwLock;

/// The folder every relative path resolves against.
///
/// `None` until the user picks one, and **that is the whole point**: there is no
/// fallback. A default root would put a real folder behind tools that the user
/// never granted access to, and on any machine other than the author's it would be
/// a path to somewhere that does not exist -- so a missing default is better than
/// a wrong one. Tools refuse until a workspace is chosen, which is an honest
/// error rather than a silent reach into the user's home directory.
static ROOT: RwLock<Option<PathBuf>> = RwLock::new(None);

/// The current workspace root, or `None` when the user has not chosen one.
///
/// `Option` rather than a fallback path because there is no safe default. The old
/// fallback to the home directory meant a user who had opened no folder still had
/// an agent that could read their home directory, and reported that folder in the
/// header as though it had been chosen.
pub fn root() -> Option<PathBuf> {
    ROOT.read().ok().and_then(|guard| guard.clone())
}

/// The current root as a string, or empty when none is chosen.
///
/// Empty rather than a placeholder path, so a caller that renders this shows
/// nothing instead of showing a folder nobody picked.
pub fn root_string() -> String {
    root()
        .map(|path| path.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// Points every tool at a new folder.
///
/// A path that does not exist, or is not a folder, is refused and the current root
/// is kept. Accepting one would leave the tools pointed at nothing, and the
/// failure would surface as every subsequent call reporting a missing file
/// rather than as the selection that caused it.
pub fn set_root(path: &Path) -> Result<PathBuf, String> {
    if !path.is_dir() {
        return Err(format!("{} is not a folder", path.display()));
    }
    // Canonicalized so the containment check compares like with like: a relative
    // root, a trailing separator, or a `\\?\` prefix would otherwise make an
    // obviously-inside path look outside.
    let resolved = path
        .canonicalize()
        .map_err(|error| format!("Could not open {}: {error}", path.display()))?;
    if let Ok(mut guard) = ROOT.write() {
        *guard = Some(resolved.clone());
    }
    Ok(resolved)
}

/// Forgets the workspace, leaving the tools unable to reach anything.
///
/// Used when the last open folder is closed. The tools then refuse every path,
/// which is the correct outcome: a user who closed their only folder asked for
/// the agent not to be working in one.
pub fn clear_root() {
    if let Ok(mut guard) = ROOT.write() {
        *guard = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The module's one lock. Every test holds it for its whole body.
    static SERIAL: std::sync::Mutex<()> = std::sync::Mutex::new(());

    /// Holds the module's lock for one test, and puts the root back afterwards.
    ///
    /// The root is a process-global and `cargo test` runs tests in parallel
    /// threads inside one process, so two tests calling `set_root` would race and
    /// one would assert against the other's folder. The counter in
    /// [`temp_root`] keeps the *folders* apart but does nothing for the shared
    /// root, so the lock is what actually isolates them.
    ///
    /// **Clearing on drop is the part that matters.** A test sets the root to a
    /// temporary folder and then deletes it, and leaving that path behind means
    /// the next test reads a root pointing at a folder that no longer exists --
    /// an intermittent failure that looks like a bug in `root()` rather than in
    /// the test. Cleared rather than reset to anything real: clearing is the
    /// neutral state and leaves no test asserting against another's choice.
    struct SerialRoot {
        lock: std::sync::MutexGuard<'static, ()>,
    }

    impl Drop for SerialRoot {
        fn drop(&mut self) {
            // `self.lock` is still held here, so this cannot race another test
            // that is waiting to set the root.
            clear_root();
        }
    }

    fn serial() -> SerialRoot {
        SerialRoot {
            lock: SERIAL
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner()),
        }
    }

    /// A unique temporary directory for one test.
    ///
    /// The counter is what makes this safe: a directory named only after the
    /// process id would be shared by every test, each deleting the others' files.
    fn temp_root(label: &str) -> PathBuf {
        use std::sync::atomic::{AtomicU32, Ordering};
        static COUNTER: AtomicU32 = AtomicU32::new(0);
        let unique = COUNTER.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "projectz-ws-{}-{label}-{unique}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).expect("temp dir");
        path
    }

    /// The root is the folder that was chosen, not the launch directory. This is
    /// the whole point of the file: `current_dir()` is the app's own directory,
    /// which is almost never the folder the user meant.
    #[test]
    fn the_root_is_the_chosen_folder_rather_than_the_launch_directory() {
        let _guard = serial();
        let folder = temp_root("chosen");
        set_root(&folder).expect("set root");
        assert_eq!(root(), Some(folder.canonicalize().expect("canonical")));
        let _ = std::fs::remove_dir_all(&folder);
    }

    /// **There is no default.** With nothing chosen the root is `None`, and every
    /// tool refuses rather than reaching into some fallback folder.
    ///
    /// This is the rule the whole file turns on, so it is asserted directly: a
    /// fallback path here would be a real folder behind tools the user never
    /// granted access to, and on any machine but the author's it would name
    /// somewhere that does not exist.
    #[test]
    fn no_folder_open_means_no_root_at_all() {
        let _guard = serial();
        clear_root();
        assert_eq!(root(), None);
        assert_eq!(root_string(), "");
    }

    /// Closing the last folder puts the tools back to refusing, rather than
    /// leaving them pointed at the folder the user just removed.
    #[test]
    fn clearing_puts_the_tools_back_to_having_no_workspace() {
        let _guard = serial();
        let folder = temp_root("cleared");
        set_root(&folder).expect("set root");
        assert!(root().is_some());
        clear_root();
        assert_eq!(root(), None);
        let _ = std::fs::remove_dir_all(&folder);
    }

    /// A folder that does not exist, or is a file, is refused rather than
    /// accepted. Accepting one would leave every tool pointed at nothing and the
    /// failure would surface as a missing file rather than as the selection.
    #[test]
    fn a_missing_or_non_folder_root_is_refused_and_the_current_one_kept() {
        let _guard = serial();
        let folder = temp_root("kept");
        set_root(&folder).expect("set root");
        let before = root();

        let missing = folder.join("not-here");
        assert!(set_root(&missing)
            .expect_err("missing folder")
            .contains("not a folder"));

        let file = folder.join("a.txt");
        std::fs::write(&file, "x").expect("write");
        assert!(set_root(&file)
            .expect_err("a file")
            .contains("not a folder"));

        // Refusing must not clear the working root.
        assert_eq!(root(), before);
        let _ = std::fs::remove_dir_all(&folder);
    }

    /// The root is stored canonicalized, so the containment check compares like
    /// with like. A path carrying a trailing separator or a relative segment
    /// would otherwise make an obviously-inside path look outside.
    #[test]
    fn the_root_is_canonicalized_so_containment_compares_like_with_like() {
        let _guard = serial();
        let folder = temp_root("canonical");
        set_root(&folder).expect("set root");
        let chosen = root().expect("a root was just set");
        assert!(chosen.is_absolute(), "{} is not absolute", chosen.display());
        assert!(
            chosen
                .components()
                .all(|component| component.as_os_str() != "."),
            "root still carries a relative segment: {}",
            chosen.display()
        );
        let _ = std::fs::remove_dir_all(&folder);
    }

    /// A root inside another folder must not be mistaken for a path escaping it.
    /// The containment check in `resolve_within` is the security boundary, and it
    /// compares against this value, so a root that is not canonicalized would let
    /// the two disagree.
    #[test]
    fn a_root_with_a_nested_folder_resolves_inside_it() {
        let _guard = serial();
        let outer = temp_root("outer");
        let inner = outer.join("inner");
        std::fs::create_dir_all(&inner).expect("inner");
        set_root(&inner).expect("set root");
        assert!(root()
            .expect("a root was just set")
            .starts_with(inner.canonicalize().expect("canonical")));
        let _ = std::fs::remove_dir_all(&outer);
    }
}
