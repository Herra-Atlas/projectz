//! The audio.cpp side of speech recognition.
//!
//! The dispatch mirror of `tts/audiocpp.rs`: the server, config and HTTP client
//! live in `audiocpp/`, and a recognition model under `models/audio` resolves
//! here -- its family and path named once, then handed across.
//!
//! What a model belongs to is decided by its folder, exactly as the voices are:
//! `models/audio` is this engine's territory, and `models/stt` is whisper.cpp's.
//! One `Path::starts_with` is the whole definition, so no second record can
//! disagree about which engine a stored model belongs to.

use super::super::audiocpp::families::{self, Model};
use crate::database::Database;

/// Whether a stored model path is one of this engine's models.
pub fn owns_path(path: &str) -> bool {
    let root = crate::app_paths::models_root().join("audio");
    std::path::Path::new(path).starts_with(root)
}

/// The installed recognition models, which are the engine's ASR families.
pub fn models() -> Vec<Model> {
    families::installed_for(families::Task::Asr)
}

/// Reads one recording back through the engine.
///
/// The WAV is the recording the microphone produced -- the same 16 kHz mono file
/// whisper.cpp takes -- so both engines are given one identical input and
/// whichever handles it, the capture side never knows. `database` is what the
/// stored options ride on.
pub async fn transcribe(
    selected: &Model,
    wav: &std::path::Path,
    app: &tauri::AppHandle,
    database: &Database,
) -> Result<String, String> {
    crate::audio::audiocpp::transcribe(selected, wav, app, database).await
}
