//! The audio.cpp side of speech synthesis.
//!
//! A thin bridge on purpose: the engine's server, config and HTTP client live in
//! `audiocpp/`, and this file is only where a resolved audio voice is handed the
//! request the engine expects. Nothing here knows about settings or the player.
//!
//! What a voice belongs to is decided by its folder: `models/audio` is this
//! engine's territory, exactly as `models/tts` is Piper's, so one `Path::starts_with`
//! is the whole of the disagreement there could ever be.

use super::super::audiocpp::families::{self, Model};
use crate::database::Database;

/// Whether a stored voice id is one of this engine's voices.
pub fn owns_path(path: &str) -> bool {
    let root = crate::app_paths::models_root().join("audio");
    std::path::Path::new(path).starts_with(root)
}

/// The installed voices, which are the engine's own TTS models.
pub fn voices() -> Vec<Model> {
    families::installed_for(families::Task::Tts)
}

/// Synthesizes `text` with one installed model.
///
/// The result is raw 16-bit mono PCM with its rate -- what every other voice in
/// the app answers with too, so nothing downstream learns that two engines are
/// behind it. `database` is what the stored options ride on.
pub async fn synthesize(
    selected: &Model,
    text: &str,
    speed: Option<f32>,
    app: &tauri::AppHandle,
    database: &Database,
) -> Result<(Vec<u8>, u32), String> {
    crate::audio::audiocpp::synthesize(selected, text, speed, app, database).await
}

/// The rate one voice's synthesis arrives at, or `0` while none is known.
pub async fn known_rate(voice_path: &str) -> u32 {
    crate::audio::audiocpp::known_rate(voice_path).await
}
