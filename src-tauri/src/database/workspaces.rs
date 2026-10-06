//! The folders the user has opened, and which one is selected.
//!
//! # Why a setting rather than a table
//!
//! This is app-wide state rather than an entity with a lifecycle, so it lives in
//! `settings` under `app.workspaces` like `app.general` and `app.preferences` do.
//! The shape is deliberately tiny -- a list of absolute paths and which is
//! selected -- because a workspace has no metadata worth storing beyond its path:
//! the folder name is the last path segment, so a name column would be a second
//! copy of something the path already says.
//!
//! # Why the *selected* root is not read from here per call
//!
//! [`crate::ai::tools::workspace`] holds the root in a process-global because a
//! tool executor is a bare function pointer and cannot capture it. This module is
//! the durable record; the global is the runtime mirror. They are set together by
//! [`AiRuntime::select_workspace`] so the two cannot drift, which is why nothing
//! here is allowed to write the setting without going through the runtime.

use serde::{Deserialize, Serialize};

/// The saved workspaces and the current selection.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Workspaces {
    /// Absolute paths, most recently opened first.
    ///
    /// Order is the display order: the picker shows the newest at the top, which
    /// is the folder the user most likely wants. De-duplicated on insert rather
    /// than on display, so the list cannot grow on every reopen.
    pub paths: Vec<String>,
    /// The path every relative tool path resolves against.
    ///
    /// `None` before anything is chosen, and the tools then fall back to the
    /// user's home directory rather than to the app's launch directory -- see
    /// `ai::tools::workspace` for why that fallback matters.
    #[serde(default)]
    pub selected: Option<String>,
}

/// The settings key this lives under.
pub const WORKSPACES_KEY: &str = "app.workspaces";

impl Workspaces {
    /// Adds a folder to the top of the list and selects it.
    ///
    /// Returns the stored value so a caller can persist exactly what was kept.
    /// A folder already in the list is moved to the top rather than duplicated,
    /// because a list with the same folder twice would offer it twice.
    pub fn add(&mut self, path: &str) -> &Self {
        self.paths.retain(|existing| existing != path);
        self.paths.insert(0, path.to_string());
        self.selected = Some(path.to_string());
        self
    }

    /// Drops a folder, moving the selection elsewhere if it was the chosen one.
    ///
    /// The selection moves to the top of what is left rather than to `None`,
    /// because leaving the app with no root at all would silently point every
    /// tool at the home directory with nothing on screen saying so.
    pub fn remove(&mut self, path: &str) -> &Self {
        self.paths.retain(|existing| existing != path);
        if self.selected.as_deref() == Some(path) {
            self.selected = self.paths.first().cloned();
        }
        self
    }

    /// Selects a folder already in the list.
    ///
    /// Refuses a path that is not listed rather than adding it. Selecting is not
    /// opening: an unlisted path would leave the picker and the selection
    /// disagreeing about what exists, and the next open would add it anyway.
    pub fn select(&mut self, path: &str) -> Result<&Self, String> {
        if !self.paths.iter().any(|existing| existing == path) {
            return Err(format!("{path} is not an open workspace"));
        }
        self.selected = Some(path.to_string());
        Ok(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_new_folder_goes_to_the_top_and_is_selected() {
        let mut workspaces = Workspaces::default();
        workspaces.add("F:\\Vscode");
        workspaces.add("C:\\work");
        // Most recent first, because that is the order the picker shows.
        assert_eq!(workspaces.paths, vec!["C:\\work", "F:\\Vscode"]);
        assert_eq!(workspaces.selected.as_deref(), Some("C:\\work"));
    }

    /// Reopening a folder must not duplicate it. A list offering the same folder
    /// twice would offer it twice in the picker, and the user would have no way
    /// to tell the entries apart.
    #[test]
    fn reopening_a_folder_moves_it_to_the_top_instead_of_duplicating_it() {
        let mut workspaces = Workspaces::default();
        workspaces.add("F:\\Vscode");
        workspaces.add("C:\\work");
        workspaces.add("F:\\Vscode");
        assert_eq!(workspaces.paths, vec!["F:\\Vscode", "C:\\work"]);
        assert_eq!(workspaces.selected.as_deref(), Some("F:\\Vscode"));
    }

    /// Removing the chosen folder moves the selection rather than clearing it.
    /// Leaving `None` would point every tool at the home directory with nothing
    /// on screen saying the workspace had gone.
    #[test]
    fn removing_the_selected_folder_moves_the_selection_rather_than_clearing_it() {
        let mut workspaces = Workspaces::default();
        workspaces.add("F:\\Vscode");
        workspaces.add("C:\\work");
        workspaces.remove("C:\\work");
        assert_eq!(workspaces.selected.as_deref(), Some("F:\\Vscode"));
        assert_eq!(workspaces.paths, vec!["F:\\Vscode"]);

        // Removing the last one has nothing left to fall back to.
        workspaces.remove("F:\\Vscode");
        assert_eq!(workspaces.selected, None);
        assert!(workspaces.paths.is_empty());
    }

    /// Removing a folder that was not selected leaves the selection alone.
    #[test]
    fn removing_another_folder_leaves_the_selection_alone() {
        let mut workspaces = Workspaces::default();
        workspaces.add("F:\\Vscode");
        workspaces.add("C:\\work");
        workspaces.remove("F:\\Vscode");
        assert_eq!(workspaces.selected.as_deref(), Some("C:\\work"));
    }

    /// Selecting is not opening. An unlisted path would leave the picker and the
    /// selection disagreeing about what exists.
    #[test]
    fn selecting_a_folder_that_was_never_opened_is_refused() {
        let mut workspaces = Workspaces::default();
        workspaces.add("F:\\Vscode");
        let error = workspaces.select("C:\\elsewhere").expect_err("not listed");
        assert!(error.contains("not an open workspace"), "{error}");
        assert_eq!(workspaces.selected.as_deref(), Some("F:\\Vscode"));
    }

    /// A stored list without a selection must not fail to read, because that is
    /// exactly what a database written before a workspace was ever chosen holds.
    #[test]
    fn a_stored_list_without_a_selection_still_reads() {
        let parsed: Workspaces = serde_json::from_str(r#"{"paths":["F:\\Vscode"]}"#).expect("read");
        assert_eq!(parsed.selected, None);
        assert_eq!(parsed.paths, vec!["F:\\Vscode"]);
    }
}
