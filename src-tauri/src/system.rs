//! What this installation is, and what it is using.
//!
//! Everything here is measured rather than remembered: folder sizes are walked
//! when the page asks, the database size is the file's own, and the build date is
//! the one baked in at compile time. A page that reports a cached figure would
//! disagree with the disk the moment someone deleted a model by hand.
//!
//! The reset is deliberately the only destructive command in the app. It exists
//! because "clear everything and start again" is a real thing to want, and it
//! names exactly what it removes on the page rather than surprising anyone later.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde::Serialize;
use tauri::State;

use crate::app_paths::{data_root, models_root, voice_root};
use crate::database::Database;

/// The date this binary was compiled, from `build.rs`.
pub const BUILD_DATE: &str = env!("BUILD_DATE");

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SystemInfo {
    pub version: String,
    pub build_date: String,
    pub data_dir: String,
    pub models_dir: String,
    pub engines_dir: String,
    pub voice_dir: String,
    pub database_path: String,
    pub database_bytes: u64,
    pub models_bytes: u64,
    pub engines_bytes: u64,
    pub voice_bytes: u64,
}

/// The database file, in the same place `lib.rs` opens it.
fn database_path() -> PathBuf {
    data_root().join("projectz.sqlite3")
}

pub fn engines_dir() -> PathBuf {
    data_root().join("engines")
}

/// A folder's size, walked. Symlinks are followed once by `metadata`, and a
/// broken one contributes nothing.
fn size_of(path: &Path) -> u64 {
    let Ok(entries) = std::fs::read_dir(path) else {
        return 0;
    };
    entries
        .flatten()
        .map(|entry| {
            let path = entry.path();
            entry.metadata().map_or(0, |meta| {
                if meta.is_dir() {
                    size_of(&path)
                } else {
                    meta.len()
                }
            })
        })
        .sum()
}

#[tauri::command]
pub fn system_info() -> SystemInfo {
    let database = database_path();
    SystemInfo {
        version: env!("CARGO_PKG_VERSION").to_string(),
        build_date: BUILD_DATE.to_string(),
        data_dir: data_root().display().to_string(),
        models_dir: models_root().display().to_string(),
        engines_dir: engines_dir().display().to_string(),
        voice_dir: voice_root().display().to_string(),
        database_bytes: std::fs::metadata(&database)
            .map(|meta| meta.len())
            .unwrap_or(0),
        database_path: database.display().to_string(),
        models_bytes: size_of(&models_root()),
        engines_bytes: size_of(&engines_dir()),
        voice_bytes: size_of(&voice_root()),
    }
}

/// Removes everything the app has downloaded or accumulated.
///
/// Files first: a locked engine (a running server) fails here with the folder
/// named, which is a better answer than clearing the history and then failing to
/// remove the models. What is *not* removed is deliberate: providers, their keys
/// and the settings are configuration, not data, and losing an API key because
/// someone wanted their disk back would be a much worse outcome than the space
/// it saves.
#[tauri::command]
pub async fn system_clear(database: State<'_, Arc<Database>>) -> Result<(), String> {
    remove_tree(&models_root())?;
    remove_tree(&voice_root())?;
    remove_tree(&engines_dir())?;
    database
        .clear_chat_sessions()
        .map_err(|error| format!("Could not clear the conversations: {error}"))?;
    database
        .reset_taste()
        .map_err(|error| format!("Could not forget the tasted profile: {error}"))?;
    Ok(())
}

/// Everything on disk for one kind. Its own command so the page can offer them
/// one at a time: someone reclaiming space usually wants the models gone and the
/// conversations kept.
#[tauri::command]
pub fn system_clear_models() -> Result<(), String> {
    remove_tree(&models_root())
}

#[tauri::command]
pub fn system_clear_engines() -> Result<(), String> {
    remove_tree(&engines_dir())
}

#[tauri::command]
pub fn system_clear_sessions(database: State<'_, Arc<Database>>) -> Result<(), String> {
    database
        .clear_chat_sessions()
        .map(|_removed| ())
        .map_err(|error| format!("Could not clear the conversations: {error}"))
}

#[tauri::command]
pub fn system_forget_taste(database: State<'_, Arc<Database>>) -> Result<(), String> {
    database
        .reset_taste()
        .map_err(|error| format!("Could not forget the tasted profile: {error}"))
}

/// Removes a folder and everything in it, naming it when it cannot.
fn remove_tree(directory: &Path) -> Result<(), String> {
    if !directory.exists() {
        return Ok(());
    }
    std::fs::remove_dir_all(directory).map_err(|error| {
        format!(
            "Could not remove {}. Close anything using it and try again: {error}",
            directory.display()
        )
    })
}
