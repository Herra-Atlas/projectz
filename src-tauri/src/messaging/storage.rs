//! Where Telegram keeps its per-person files.
//!
//! Everything lives under the app's own data directory, not under whichever
//! project the desktop window happens to have open:
//!
//! ```text
//! <app data>/telegram/<numeric id>/profile.md
//! <app data>/telegram/<numeric id>/workspace/
//! ```
//!
//! The workspace is the agent's root for that person's messages, so one Telegram
//! id cannot browse another id's files or the host project. Profiles sit beside
//! the workspace rather than inside it, which keeps them outside every tool the
//! model can call while remaining easy for the person who owns the machine to
//! read, back up or delete.

use std::path::{Path, PathBuf};

use tauri::Manager;

use crate::messaging::Messaging;

/// The Telegram data root inside the app's data directory.
pub fn base(hub: &Messaging) -> Result<PathBuf, String> {
    hub.app()
        .path()
        .app_data_dir()
        .map(|root| root.join("telegram"))
        .map_err(|error| format!("Could not find the app data folder: {error}"))
}

/// One sender's folder. The id is validated as digits so it is one path segment.
pub fn user_root(base: &Path, user_id: &str) -> Result<PathBuf, String> {
    let id = user_id.trim();
    if id.is_empty() || !id.chars().all(|character| character.is_ascii_digit()) {
        return Err("A Telegram user id must be numeric.".to_string());
    }
    Ok(base.join(id))
}

/// The fixed agent root for one sender, created on first use.
pub async fn workspace(base: &Path, user_id: &str) -> Result<PathBuf, String> {
    let root = user_root(base, user_id)?.join("workspace");
    tokio::fs::create_dir_all(&root)
        .await
        .map_err(|error| format!("Could not prepare the Telegram workspace: {error}"))?;
    Ok(root)
}

/// The profile file for one sender, outside that sender's workspace.
pub fn profile_path(base: &Path, user_id: &str) -> Result<PathBuf, String> {
    Ok(user_root(base, user_id)?.join("profile.md"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_numeric_id_is_the_only_kind_of_folder_name() {
        let root = Path::new("base");
        assert_eq!(user_root(root, " 12345 ").expect("id"), root.join("12345"));
        assert!(user_root(root, "../123").is_err());
        assert!(user_root(root, "-100").is_err());
    }
}
