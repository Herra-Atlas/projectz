//! The recognition models on disk, and which one transcribes.
//!
//! A whisper.cpp model is one `ggml-*.bin` file, and audio.cpp's models are
//! gguf packages under `models/audio`: two folders, one registry. `resolve`
//! walks both, prefers the whisper.cpp folder because that is the engine this
//! app shipped with, and answers a `Selected` that says which engine the file
//! belongs to -- which is the only caller that ever needs to know.
//!
//! Whisper models come from `ggerganov/whisper.cpp` and Parakeet ones from
//! `ggml-org/parakeet-GGUF`; the two are read by different executables in the
//! same engine install, which is why the model -- not the engine -- is what the
//! user chooses.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::app_paths::models_root;
use crate::audio::audiocpp::families;
use crate::database::Database;

use crate::audio::engines::whisper::{PARAKEET_EXE, WHISPER_EXE};

/// The settings key, beside `app.tts`.
const KEY: &str = "app.stt";

#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SttSettings {
    /// The chosen model's path, or null for the first one found.
    pub model: Option<String>,
    /// The language each model is told to expect, by path. Absent means the
    /// engine decides, which is right for a first recording and wrong for a
    /// microphone that only ever hears one language.
    #[serde(default)]
    pub languages: std::collections::BTreeMap<String, String>,
    /// The capture endpoint to record from, by name, or none for the system
    /// default. A name rather than an index because the list renumbers as
    /// devices come and go -- see `capture`.
    #[serde(default)]
    pub device: Option<String>,
    /// Keep the recognition model in memory between recordings.
    ///
    /// The CLI loads a model, transcribes one file and exits, so without this the
    /// weights are read from disk for every press. On means a resident engine
    /// (`whisper-server`) that holds them; the cost is memory and one process.
    #[serde(default)]
    pub keep_loaded: bool,
}

impl SttSettings {
    pub fn language_for(&self, path: &str) -> Option<&str> {
        self.languages
            .get(path)
            .map(String::as_str)
            .filter(|code| !code.is_empty())
    }
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Model {
    /// Absolute path. It is both the id and what the engine is handed.
    pub path: String,
    pub name: String,
    /// The repository folder it was installed under.
    pub repo: String,
    pub size: u64,
    /// Parakeet is a different model family, read by a different executable.
    pub parakeet: bool,
    /// Served by audio.cpp rather than read by the whisper.cpp CLI. The page
    /// groups on this, because the two engines keep their files in two folders.
    pub audio: bool,
    /// The audio.cpp family, for the rows that engine serves. Empty otherwise.
    pub family: String,
}

impl Model {
    pub fn executable(&self) -> &'static str {
        if self.parakeet {
            PARAKEET_EXE
        } else {
            WHISPER_EXE
        }
    }
}

pub fn settings(database: &Database) -> SttSettings {
    database
        .setting(KEY)
        .and_then(|value| serde_json::from_value(value).ok())
        .unwrap_or_default()
}

pub fn save(database: &Database, settings: &SttSettings) -> Result<(), String> {
    let value = serde_json::to_value(settings)
        .map_err(|error| format!("The recognition settings could not be stored: {error}"))?;
    database.set_setting(KEY, &value)
}

/// Deletes a model this app installed.
///
/// Refuses anything outside `models/stt`: a path from the frontend is not a
/// reason to delete an arbitrary file, and the model list is the only thing that
/// should ever reach this.
pub fn remove(path: &str) -> Result<(), String> {
    let file = PathBuf::from(path);
    if !file.starts_with(models_root().join("stt")) {
        return Err("That model was not installed by this app.".to_string());
    }
    if !file.is_file() {
        return Err("That model is already gone.".to_string());
    }
    std::fs::remove_file(&file).map_err(|error| format!("The model could not be deleted: {error}"))
}

/// Every model installed, Whisper first because that is what most machines run.
pub fn list() -> Vec<Model> {
    let mut found = Vec::new();
    walk(&models_root().join("stt"), &mut found);
    // audio.cpp's recognisers live in the other engine's folder, and this page
    // chooses a *model*, not an engine, so both are listed. `repo` carries the
    // grouping label the page paints.
    found.extend(
        families::installed_for(families::Task::Asr)
            .into_iter()
            .map(|model| Model {
                path: model.path.display().to_string(),
                name: model.name.clone(),
                repo: "audio.cpp".to_string(),
                size: model.size,
                parakeet: false,
                audio: true,
                family: model.family.clone(),
            }),
    );
    found.sort_by(|left, right| {
        left.audio
            .cmp(&right.audio)
            .then_with(|| left.parakeet.cmp(&right.parakeet))
            .then_with(|| left.name.cmp(&right.name))
    });
    found
}

/// The model a recording should be transcribed with, whichever engine owns it.
///
/// The stored path is the primary answer; the whisper.cpp folder is the
/// fallback, then the audio.cpp folder -- so a user who has only ever installed
/// a whisper model resolves exactly as before, and the audio engine only ever
/// answers when there is nothing else installed from the one this app shipped
/// with.
pub enum Selected {
    /// whisper.cpp (or its Parakeet port), via the pinned build's CLI.
    WhisperFile(Model),
    /// audio.cpp's server, one family and one file.
    Audiocpp(families::Model),
}

impl Selected {
    pub fn path(&self) -> String {
        match self {
            Self::WhisperFile(model) => model.path.clone(),
            Self::Audiocpp(model) => model.path.display().to_string(),
        }
    }
}

/// The model a recording should be transcribed with.
///
/// Falls back to the first installed model when the stored choice is gone -- a
/// deleted model should not leave the microphone pointing at nothing.
pub fn resolve(settings: &SttSettings) -> Result<Selected, String> {
    // `list` carries both engines' rows for the settings page, so every
    // whisper fallback below names the engine (`!model.audio`) instead of
    // trusting the order: an audio.cpp gguf handed to `whisper-cli` is a model
    // that cannot load, however plausible the list looked.
    let installed = list();
    let audio_models = super::audiocpp::models();
    if let Some(chosen) = settings.model.as_deref() {
        if super::audiocpp::owns_path(chosen) {
            if let Some(found) = audio_models
                .iter()
                .find(|model| model.path.display().to_string() == chosen)
            {
                return Ok(Selected::Audiocpp(found.clone()));
            }
        } else if let Some(found) = installed
            .iter()
            .find(|model| !model.audio && model.path == chosen)
        {
            return Ok(Selected::WhisperFile(found.clone()));
        }
    }
    // whisper.cpp is the engine this app came with, so it answers first.
    if let Some(first) = installed.iter().find(|model| !model.audio) {
        return Ok(Selected::WhisperFile(first.clone()));
    }
    // Only the audio engine's models are installed: that is the honest next
    // answer rather than a second error about an engine nobody asked for.
    if let Some(first) = audio_models.into_iter().next() {
        return Ok(Selected::Audiocpp(first));
    }
    Err("No recognition model is installed. Download one from Settings > Voice.".to_string())
}

fn walk(directory: &Path, found: &mut Vec<Model>) {
    let Ok(entries) = std::fs::read_dir(directory) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            walk(&path, found);
            continue;
        }
        let is_model = path
            .extension()
            .and_then(|extension| extension.to_str())
            .is_some_and(|extension| extension.eq_ignore_ascii_case("bin"));
        if !is_model {
            continue;
        }
        if let Some(model) = describe(&path) {
            found.push(model);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::SttSettings;

    /// A record written before the microphone could be chosen has no `device`
    /// key. It must load with the system default rather than fail into
    /// `unwrap_or_default`, which would silently forget the chosen model too.
    #[test]
    fn a_record_without_a_device_still_loads() {
        let settings: SttSettings =
            serde_json::from_str(r#"{"model":"/models/ggml-small.bin"}"#).unwrap();
        assert_eq!(settings.device, None);
        assert_eq!(settings.model.as_deref(), Some("/models/ggml-small.bin"));
    }
}

fn describe(path: &PathBuf) -> Option<Model> {
    let name = path.file_name()?.to_str()?.to_string();
    let repo = path
        .parent()
        .and_then(|parent| parent.file_name())
        .and_then(|name| name.to_str())
        .unwrap_or("local")
        .to_string();
    let size = std::fs::metadata(path).map(|meta| meta.len()).unwrap_or(0);
    let parakeet = name.to_ascii_lowercase().contains("parakeet");
    Some(Model {
        path: path.display().to_string(),
        name,
        repo,
        size,
        parakeet,
        audio: false,
        family: String::new(),
    })
}
