//! Where this app keeps its files.
//!
//! One place, because every one of these folders is named after the Tauri
//! identifier and a second copy of that string is a second thing to keep in step.
//! The speech assets and the downloaded models live under the same root but in
//! different folders: `voice/` is placed by the build, `models/` by the user.

use std::path::PathBuf;

/// Must match `identifier` in `tauri.conf.json`.
pub const APP_ID: &str = "com.heart.projectz";

/// The per-user root, `%APPDATA%\<identifier>` on Windows.
pub fn data_root() -> PathBuf {
    dirs::data_dir()
        // A machine with no data directory cannot have installed anything either,
        // so this only has to be a path that fails the way a wrong one does.
        .unwrap_or_else(|| PathBuf::from("."))
        .join(APP_ID)
}

/// Everything downloaded or installed for this app: `<root>/models/<kind>/<repo>/`.
pub fn models_root() -> PathBuf {
    data_root().join("models")
}

/// Speech assets the build places: `<root>/voice/`.
pub fn voice_root() -> PathBuf {
    data_root().join("voice")
}
