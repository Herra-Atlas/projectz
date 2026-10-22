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

/// Skills the user installs or writes: `<root>/skills/<slug>/SKILL.md`.
///
/// A folder per skill, in the open Agent Skills layout, so a skill can carry the
/// scripts and references that make it more than a paragraph. The agent reaches
/// these through the skill tools rather than `read_file`, which is confined to
/// the workspace -- see [`crate::skills`].
pub fn skills_root() -> PathBuf {
    data_root().join("skills")
}

/// The set the app ships: `<root>/bundled-skills/<slug>/SKILL.md`.
///
/// A second root rather than a flag on a row, because a version bump replaces
/// this whole tree and must never be able to overwrite something the user wrote.
/// The index reads both roots; a user skill of the same slug wins.
pub fn bundled_skills_root() -> PathBuf {
    data_root().join("bundled-skills")
}

/// Scratch space for programs this app starts: `<root>/tmp/`.
///
/// A child process inherits its parent's working directory, and a dev build of a
/// Tauri app runs from the source tree -- so a server that writes an unasked-for
/// file would put it in the repository. Each MCP server gets a folder under here
/// and is started in it instead. See [`crate::mcp::scratch_dir`].
pub fn tmp_root() -> PathBuf {
    data_root().join("tmp")
}
