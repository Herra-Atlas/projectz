//! The browser's profile: one folder, named, created on demand, deletable.
//!
//! # Why it is its own file
//!
//! The profile is the only part of this feature that *persists a user's
//! credentials*, so it gets the things persistence needs and a driver does not: a
//! name, a place on disk that can be found and explained, a way to create it, and
//! a way to destroy it. Everything else in `browser/` is a session that ends when
//! the browser closes.
//!
//! # It is not the user's browser
//!
//! A `--user-data-dir` is a browser's whole state in one folder: cookies, saved
//! logins, history, the lot. Ours is a folder under this app's data directory, so
//! what the agent signs into is not what the user is signed into, and clearing it
//! clears only the agent's own. It can never be the real Chrome profile either:
//! a recent Chrome refuses remote debugging against the default one, precisely so
//! a tool cannot attach to the browser holding a user's sessions.
//!
//! # The name is written into the browser's own state
//!
//! Chrome takes a profile's display name from `profile.info_cache` in its
//! `Local State` file. Without it the window this app opens has no name at all,
//! which is what makes it impossible to tell apart from a plain new Chrome -- and
//! what made it look as though no profile was being used. The merge is careful
//! on purpose: that file belongs to the browser, so a read that fails is a write
//! that does not happen.

use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::Serialize;

/// What this profile is called, and the name of its folder.
pub const NAME: &str = "ProjectZ";

/// The folder the profile lives in, under the app's own data directory.
///
/// `profiles/` rather than a folder per app version, because a profile is not
/// versioned state: the logins in it are meant to outlive every upgrade.
pub fn dir(app_data: &Path) -> PathBuf {
    app_data.join("profiles").join(NAME)
}

/// Where the profile lived before it had a name.
///
/// Read only by [`ensure`], to adopt it. A login the agent already made is worth
/// keeping, and a rename is cheaper than asking the user to sign in again.
pub fn legacy_dir(app_data: &Path) -> PathBuf {
    app_data.join("browser-profile")
}

/// What is on disk for this profile, as the settings row shows it.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    /// The name, which is also the browser's own name for it.
    pub name: String,
    /// Where it is, for a reader who wants to look.
    pub path: String,
    /// How much disk it is using.
    pub bytes: u64,
    /// Whether it exists yet. False before the first browser call, and that is
    /// worth saying rather than showing a size of zero.
    pub exists: bool,
}

/// What to do about a profile that may already exist under an older name.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Plan {
    /// The named folder is already there. Nothing to move.
    Ready,
    /// An older unnamed folder is there and the named one is not: adopt it.
    Adopt,
    /// Neither is there. This profile is new.
    Fresh,
}

/// Creates the profile folder, adopting an older unnamed one if there is one.
///
/// Idempotent, and called before every browser start: the folder has to exist
/// before the browser is launched, and the name has to be in place before the
/// window opens or the first launch is the one that shows no name.
pub async fn ensure(app_data: &Path) -> Result<PathBuf, String> {
    let wanted = dir(app_data);
    let legacy = legacy_dir(app_data);

    if plan(&wanted, &legacy) == Plan::Adopt {
        // A rename, not a copy: a profile is hundreds of megabytes, and a failed
        // move leaves the original where it was for the next attempt to find.
        // Falling through to `create_dir_all` either way is what makes a move
        // that could not happen a new profile rather than an error.
        if tokio::fs::rename(&legacy, &wanted).await.is_err() {
            // One retry by hand, because a rename across a directory that another
            // process still holds open on Windows fails until it lets go. The
            // failure is not fatal: the profile is recreated on next use.
            tracing::debug!("an older browser profile could not be moved; starting a new one");
        }
    }

    tokio::fs::create_dir_all(&wanted)
        .await
        .map_err(|error| format!("Could not create {}: {error}", wanted.display()))?;
    name_in_browser_state(&wanted).await;
    Ok(wanted)
}

/// The decision [`ensure`] acts on. Pure, so it can be tested without a disk.
fn plan(wanted: &Path, legacy: &Path) -> Plan {
    if wanted.exists() {
        return Plan::Ready;
    }
    if legacy.exists() {
        return Plan::Adopt;
    }
    Plan::Fresh
}

/// Tells the browser what to call this profile.
///
/// Merged into the browser's own `Local State`, never replaced: that file holds a
/// great deal this app knows nothing about. A file that exists and cannot be read
/// as JSON is left *exactly* as it is -- the failure mode of guessing at a
/// browser's own state file is worse than an unnamed window.
async fn name_in_browser_state(dir: &Path) {
    let file = dir.join("Local State");
    let mut state: serde_json::Value = match tokio::fs::read_to_string(&file).await {
        Ok(text) => match serde_json::from_str(&text) {
            Ok(state) => state,
            // Present and unreadable: hands off.
            Err(_) => return,
        },
        // Absent, which is the first run. A minimal file is what the browser
        // augments when it starts.
        Err(_) => serde_json::json!({}),
    };

    let already = state
        .get("profile")
        .and_then(|profile| profile.get("info_cache"))
        .and_then(|cache| cache.get("Default"))
        .and_then(|entry| entry.get("name"))
        .and_then(serde_json::Value::as_str);
    if already == Some(NAME) {
        return;
    }

    if !state.is_object() {
        return;
    }
    state["profile"]["info_cache"]["Default"]["name"] = serde_json::Value::String(NAME.to_string());
    if let Ok(text) = serde_json::to_string(&state) {
        let _ = tokio::fs::write(&file, text).await;
    }
}

/// What the profile is using, for the settings row.
pub async fn status(app_data: &Path) -> Status {
    let path = dir(app_data);
    let exists = path.exists();
    let bytes = match exists {
        // Off the async thread: a profile is a few thousand files, and walking
        // them is exactly the kind of work that must not stall the runtime every
        // other command shares.
        true => {
            let walked = path.clone();
            tokio::task::spawn_blocking(move || directory_size(&walked))
                .await
                .unwrap_or(0)
        }
        false => 0,
    };
    Status {
        name: NAME.to_string(),
        path: path.display().to_string(),
        bytes,
        exists,
    }
}

/// Deletes the profile and everything in it.
///
/// The path is *computed*, never taken from a caller: there is no way to point
/// this at anything but the folder this app made. The browser has to be stopped
/// first -- a profile in use is held open by a dozen processes, and a delete that
/// half-succeeds leaves a profile that looks cleared and is not.
pub async fn forget(app_data: &Path) -> Result<(), String> {
    for path in [dir(app_data), legacy_dir(app_data)] {
        if !path.exists() {
            continue;
        }
        remove_all(&path).await?;
    }
    Ok(())
}

/// Removes a folder, giving a browser that is still letting go a moment to finish.
///
/// A file still held by a process is exactly what this hits: the browser has been
/// told to stop, and the operating system releases its handles a moment after it
/// exits. The alternative to waiting is a button that reports a failure for work
/// that was going to succeed, which is worse than the wait.
async fn remove_all(path: &Path) -> Result<(), String> {
    let mut last = String::new();
    for _ in 0..5 {
        match tokio::fs::remove_dir_all(path).await {
            Ok(()) => return Ok(()),
            Err(error) => last = error.to_string(),
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    Err(format!("Could not remove {}: {last}", path.display()))
}

/// How much disk a folder is using, without following anything out of it.
fn directory_size(path: &Path) -> u64 {
    let mut total = 0;
    let mut pending = vec![path.to_path_buf()];
    while let Some(next) = pending.pop() {
        let Ok(entries) = std::fs::read_dir(&next) else {
            continue;
        };
        for entry in entries.flatten() {
            // `file_type` rather than `metadata`: it does not follow a link, so a
            // loop cannot be walked into and a link out of the profile is not
            // counted as though it were inside it.
            match entry.file_type() {
                Ok(kind) if kind.is_dir() => pending.push(entry.path()),
                Ok(kind) if kind.is_file() => {
                    total += entry.metadata().map(|m| m.len()).unwrap_or(0)
                }
                _ => {}
            }
        }
    }
    total
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_profile_is_named_and_sits_under_the_app_data_directory() {
        let data = Path::new("C:\\data");
        assert_eq!(dir(data), data.join("profiles").join("ProjectZ"));
        assert_eq!(legacy_dir(data), data.join("browser-profile"));
    }

    /// The three cases the migration has to tell apart. Getting this wrong in the
    /// direction of `Adopt` would move a profile that is already in place, and the
    /// other way would abandon the logins it holds.
    #[test]
    fn an_older_profile_is_adopted_and_an_existing_one_is_left_alone() {
        let root = std::env::temp_dir().join("projectz-profile-plan-test");
        let _ = std::fs::remove_dir_all(&root);

        let wanted = root.join("profiles").join("ProjectZ");
        let legacy = root.join("browser-profile");
        assert_eq!(plan(&wanted, &legacy), Plan::Fresh, "neither exists");

        std::fs::create_dir_all(&legacy).expect("create the old profile");
        assert_eq!(
            plan(&wanted, &legacy),
            Plan::Adopt,
            "only the old one exists"
        );

        std::fs::create_dir_all(&wanted).expect("create the named profile");
        assert_eq!(
            plan(&wanted, &legacy),
            Plan::Ready,
            "both exist: the named one wins rather than being overwritten"
        );

        let _ = std::fs::remove_dir_all(&root);
    }

    /// A profile is created on demand and the name is left in the browser's own
    /// state, so the window says which profile it is.
    #[test]
    fn ensuring_a_profile_creates_it_and_names_it() {
        let root = std::env::temp_dir().join("projectz-profile-ensure-test");
        let _ = std::fs::remove_dir_all(&root);

        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("test runtime");
        let made = runtime
            .block_on(ensure(&root))
            .expect("the profile should be created");
        assert!(made.is_dir(), "the folder was not created");

        let state: serde_json::Value = serde_json::from_str(
            &std::fs::read_to_string(made.join("Local State")).expect("state"),
        )
        .expect("valid JSON");
        assert_eq!(state["profile"]["info_cache"]["Default"]["name"], NAME);

        // Running it again changes nothing, which is what makes it safe to call
        // before every browser start.
        assert_eq!(runtime.block_on(ensure(&root)).expect("again"), made);

        let _ = std::fs::remove_dir_all(&root);
    }

    /// A `Local State` that cannot be read is not a file to guess at: the browser
    /// owns it, and a half-written one is worse than an unnamed window.
    #[test]
    fn an_unreadable_browser_state_is_left_untouched() {
        let root = std::env::temp_dir().join("projectz-profile-corrupt-test");
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).expect("create the folder");
        let file = root.join("Local State");
        std::fs::write(&file, "{ not json at all").expect("write");

        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("test runtime");
        runtime.block_on(ensure(&root)).expect("ensure");

        assert_eq!(
            std::fs::read_to_string(&file).expect("still there"),
            "{ not json at all",
            "the browser's own file was overwritten"
        );

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn forgetting_removes_the_folder_and_everything_in_it() {
        let root = std::env::temp_dir().join("projectz-profile-forget-test");
        let _ = std::fs::remove_dir_all(&root);
        let wanted = root.join("profiles").join("ProjectZ");
        std::fs::create_dir_all(wanted.join("Default")).expect("create");
        std::fs::write(wanted.join("Default").join("Cookies"), b"pretend").expect("write");

        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("test runtime");
        runtime.block_on(forget(&root)).expect("forget");

        assert!(!wanted.exists(), "the profile survived");
        // And it is not an error to forget one that is not there, which is what
        // makes the button safe to press twice.
        runtime.block_on(forget(&root)).expect("forget again");

        let _ = std::fs::remove_dir_all(&root);
    }

    /// The size the settings row shows is the size of the folder, counted without
    /// following anything out of it.
    #[test]
    fn the_size_is_the_sum_of_the_files_inside() {
        let root = std::env::temp_dir().join("projectz-profile-size-test");
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("nested")).expect("create");
        std::fs::write(root.join("a"), vec![0u8; 100]).expect("write");
        std::fs::write(root.join("nested").join("b"), vec![0u8; 40]).expect("write");

        assert_eq!(directory_size(&root), 140);

        let _ = std::fs::remove_dir_all(&root);
    }
}
