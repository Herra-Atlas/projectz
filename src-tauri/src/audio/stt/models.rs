//! The recognition models on disk, and which one transcribes.
//!
//! Three engines, one registry. A whisper.cpp model is one `ggml-*.bin` file
//! under `models/stt`, audio.cpp's models are gguf packages under `models/audio`,
//! and a GGUF speech model is an ordinary local model in the chat library whose
//! projector declares an audio encoder. `resolve` walks all three and answers a
//! `Selected` that says which engine the file belongs to -- which is the only
//! caller that ever needs to know.
//!
//! Precedence is deliberate and documented on `resolve`: a model that can hear
//! is preferred, because one GGUF replacing a separate recognition engine is the
//! whole reason it is installed.
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

#[derive(Clone, Serialize, Deserialize)]
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
    /// Send a finished transcript as a message instead of leaving it in the
    /// draft. Off by default: a transcription is a draft until the user says
    /// otherwise, and a message sent by accident cannot be un-sent.
    #[serde(default)]
    pub auto_send: bool,
    /// How much quiet ends a recording once speech has been heard, in
    /// milliseconds. Clamped where it is read, so a hand-edited record cannot
    /// leave the microphone open for a minute.
    #[serde(default = "default_silence_ms")]
    pub silence_ms: u32,
}

/// 0.8 s: long enough not to cut a breath, short enough that a finished sentence
/// feels finished. It is also the first entry of the settings list, so the value
/// a fresh install shows is a value the picker can draw.
const DEFAULT_SILENCE_MS: u32 = 800;

/// What a record without one of these fields starts from.
///
/// Written out rather than derived: `#[derive(Default)]` gives `0` for both
/// numbers, and a silence of zero milliseconds is not a default -- it is a
/// microphone that closes the moment it opens. A derived default and the serde
/// defaults must agree, and the way to be sure is for one of them to stop
/// existing.
impl Default for SttSettings {
    fn default() -> Self {
        Self {
            model: None,
            languages: Default::default(),
            device: None,
            keep_loaded: false,
            auto_send: false,
            silence_ms: DEFAULT_SILENCE_MS,
        }
    }
}

fn default_silence_ms() -> u32 {
    DEFAULT_SILENCE_MS
}

/// The bound a stored silence length is held to.
const MIN_SILENCE_MS: u32 = 300;
const MAX_SILENCE_MS: u32 = 5_000;

impl SttSettings {
    pub fn language_for(&self, path: &str) -> Option<&str> {
        self.languages
            .get(path)
            .map(String::as_str)
            .filter(|code| !code.is_empty())
    }

    /// The silence that ends a recording, with the stored number bounded.
    ///
    /// Read through a method rather than the field so every caller agrees on the
    /// limits: the capture thread is not the place to discover that a record
    /// written by hand says 400 seconds.
    pub fn silence(&self) -> std::time::Duration {
        std::time::Duration::from_millis(
            self.silence_ms.clamp(MIN_SILENCE_MS, MAX_SILENCE_MS) as u64
        )
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
    /// Which engine serves this file: `whisper`, `audiocpp` or `llamacpp`.
    ///
    /// A string rather than a flag per engine, because the page groups on it and
    /// a third engine would otherwise need a second boolean that can contradict
    /// the first. The folder a file sits under is still the truth; this is that
    /// answer carried to the page.
    pub engine: String,
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
    let mut settings: SttSettings = database
        .setting(KEY)
        .and_then(|value| serde_json::from_value(value).ok())
        .unwrap_or_default();
    // A record that failed to parse, or one written before the field existed,
    // must not leave the microphone closing instantly. Zero is never a silence
    // anyone chose, so it is the one value repaired on the way in.
    if settings.silence_ms == 0 {
        settings.silence_ms = DEFAULT_SILENCE_MS;
    }
    settings
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
///
/// `app` is what makes the third engine's rows possible: a speech model is an
/// ordinary local model, registered in the chat library rather than in a folder
/// of its own, so it can only be listed by asking the app. Callers that have no
/// handle (an engine's own asset count) pass `None` and see the two folder
/// engines, which is the answer they were asking for.
pub fn list(app: Option<&tauri::AppHandle>) -> Vec<Model> {
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
                engine: "audiocpp".to_string(),
                family: model.family.clone(),
            }),
    );
    // The GGUF speech models, whose audio encoder is the only reason they are
    // here. One that cannot hear is a chat model with a projector and has no
    // business on this page.
    if let Some(app) = app {
        for model in crate::audio::speech::installed(app) {
            if !model.hears_audio {
                continue;
            }
            found.push(Model {
                path: model.path.clone(),
                name: model.name.clone(),
                repo: "llama.cpp".to_string(),
                size: std::fs::metadata(&model.path)
                    .map(|meta| meta.len())
                    .unwrap_or(0),
                parakeet: false,
                engine: "llamacpp".to_string(),
                family: String::new(),
            });
        }
    }
    found.sort_by(|left, right| {
        left.engine
            .cmp(&right.engine)
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
    /// A GGUF speech model, served by its own `llama-server`.
    Llamacpp(crate::audio::speech::SpeechModel),
}

impl Selected {
    pub fn path(&self) -> String {
        match self {
            Self::WhisperFile(model) => model.path.clone(),
            Self::Audiocpp(model) => model.path.display().to_string(),
            Self::Llamacpp(model) => model.path.clone(),
        }
    }
}

/// The model a recording should be transcribed with.
///
/// **A speech model wins when one is installed.** That is the whole point of
/// having it: one 2 GB GGUF replaces a separate recognition engine, so pressing
/// the microphone must not load whisper beside it. The stored selection still
/// answers for the two folder engines, and is what decides between them -- but a
/// model that can hear is preferred over both, which is why selecting a whisper
/// model does not bring it back while such a model exists.
pub fn resolve(app: Option<&tauri::AppHandle>, settings: &SttSettings) -> Result<Selected, String> {
    if let Some(app) = app {
        if let Some(model) = crate::audio::speech::hearing(app) {
            return Ok(Selected::Llamacpp(model));
        }
    }

    // `list` carries every engine's rows for the settings page, so each fallback
    // below names the engine instead of trusting the order: an audio.cpp gguf
    // handed to `whisper-cli` is a model that cannot load, however plausible the
    // list looked.
    let installed = list(app);
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
            .find(|model| model.engine != "audiocpp" && model.path == chosen)
        {
            return Ok(Selected::WhisperFile(found.clone()));
        }
    }
    // whisper.cpp is the engine this app came with, so it answers first.
    if let Some(first) = installed.iter().find(|model| model.engine == "whisper") {
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

    /// A record without a silence length gets one that works.
    ///
    /// `#[derive(Default)]` used to answer `0` here, and a zero-millisecond
    /// silence is a microphone that closes the instant it opens -- which is what
    /// the settings page showed as a blank picker.
    #[test]
    fn a_record_without_a_silence_gets_a_usable_default() {
        let settings: SttSettings = serde_json::from_str(r#"{"model":"/models/a.bin"}"#).unwrap();
        assert_eq!(settings.silence_ms, 800);
        assert_eq!(SttSettings::default().silence_ms, 800);
    }

    /// An unmatched parsing failure falls back to a real preference, not zeroes.
    #[test]
    fn the_fallback_record_carries_the_same_defaults_as_serde() {
        let fallback = SttSettings::default();
        assert_eq!(fallback.silence_ms, 800);
        assert!(!fallback.auto_send);
        assert!(!fallback.keep_loaded);
    }

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
        // Parakeet is a model family read by the same engine's second executable,
        // so the engine stays whisper.cpp and `parakeet` is the distinction.
        engine: "whisper".to_string(),
        family: String::new(),
    })
}
