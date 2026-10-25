//! Per-sender answer profiles, written with `/profile`.
//!
//! A Telegram user id is the identity the allowlist already trusts, so it is also
//! the right key for "answer me like this". The profile is a small Markdown file
//! beside that id's workspace, not a database row: it is easy to inspect, back up
//! or delete with the rest of the app's data, and a page save cannot erase it.

use std::path::Path;

use crate::messaging::storage;

/// The most text one profile may hold.
///
/// A profile rides in every request from that sender, so it is a prompt they pay
/// for on each message rather than an unlimited note.
pub const MAX_PROFILE_CHARS: usize = 2_000;

/// One profile, if the sender wrote one.
pub async fn get(base: &Path, user_id: &str) -> Option<String> {
    let path = storage::profile_path(base, user_id).ok()?;
    tokio::fs::read_to_string(path)
        .await
        .ok()
        .map(|text| text.trim().to_string())
        .filter(|text| !text.is_empty())
}

/// Stores a profile, trimmed and bounded, and returns what was stored.
pub async fn set(base: &Path, user_id: &str, text: &str) -> Result<String, String> {
    let text = text.trim();
    if text.is_empty() {
        return Err("A profile needs some text. Use /profile clear to remove it.".to_string());
    }
    let stored: String = text.chars().take(MAX_PROFILE_CHARS).collect();
    let path = storage::profile_path(base, user_id)?;
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|error| format!("Could not prepare the profile: {error}"))?;
    }
    tokio::fs::write(&path, &stored)
        .await
        .map_err(|error| format!("Could not save the profile: {error}"))?;
    Ok(stored)
}

/// Removes one sender's profile.
pub async fn clear(base: &Path, user_id: &str) -> Result<(), String> {
    let path = storage::profile_path(base, user_id)?;
    match tokio::fs::remove_file(path).await {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(format!("Could not remove the profile: {error}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_base() -> std::path::PathBuf {
        std::env::temp_dir().join(format!("projectz-profiles-{}", uuid::Uuid::new_v4()))
    }

    #[tokio::test]
    async fn a_profile_round_trips_per_sender_and_stays_out_of_the_workspace() {
        let base = temp_base();
        set(&base, "1", "  Short answers. No lists.  ")
            .await
            .expect("set");
        set(&base, "2", "Explain more.").await.expect("set");
        assert_eq!(
            get(&base, "1").await.as_deref(),
            Some("Short answers. No lists.")
        );
        assert_eq!(get(&base, "2").await.as_deref(), Some("Explain more."));
        assert!(storage::workspace(&base, "1")
            .await
            .expect("workspace")
            .is_dir());
        assert!(!storage::workspace(&base, "1")
            .await
            .expect("workspace")
            .join("profile.md")
            .exists());
        let _ = std::fs::remove_dir_all(base);
    }

    #[tokio::test]
    async fn clearing_removes_only_that_sender() {
        let base = temp_base();
        set(&base, "1", "one").await.expect("set");
        set(&base, "2", "two").await.expect("set");
        clear(&base, "1").await.expect("clear");
        assert!(get(&base, "1").await.is_none());
        assert_eq!(get(&base, "2").await.as_deref(), Some("two"));
        let _ = std::fs::remove_dir_all(base);
    }

    #[tokio::test]
    async fn a_profile_is_bounded_in_characters_not_bytes() {
        let base = temp_base();
        let stored = set(&base, "1", &"ä".repeat(MAX_PROFILE_CHARS + 10))
            .await
            .expect("set");
        assert_eq!(stored.chars().count(), MAX_PROFILE_CHARS);
        let _ = std::fs::remove_dir_all(base);
    }
}
