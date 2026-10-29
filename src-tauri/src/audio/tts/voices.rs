//! Which voices exist, which one is chosen, and where they live.
//!
//! # One place a voice comes from
//!
//! A voice is either a file the user installed -- `models/tts` (Piper, `.onnx`
//! plus its `.onnx.json`) or `models/audio` (audio.cpp, one gguf each) -- or a
//! GGUF speech model from the chat library, whose own template can emit audio.
//! Nothing is placed by the build.
//!
//! # Why a voice is identified by its path
//!
//! A name is not unique -- two engines can publish a voice with the same file
//! name -- and the path is what an engine is given anyway. The stored setting
//! therefore holds a path, and `resolve` refuses one that is not inside this
//! app's own folders. Which *engine* a stored path is for is told by the folder
//! it sits under: `models/audio` is audio.cpp's territory, `models/tts` is
//! Piper's, and a speech model is recognised by being one of the app's own
//! registered models. There is no second record that can disagree.

use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::app_paths::{models_root, voice_root};

/// The folder holding `piper.exe`, its DLLs and `espeak-ng-data`.
pub fn install_dir() -> PathBuf {
    voice_root().join("piper")
}

/// One voice, as the settings page lists it.
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VoiceEntry {
    /// The model file's path, which is also what a selection stores.
    pub id: String,
    pub name: String,
    pub sample_rate: u32,
    pub size: u64,
    /// `piper`, `audiocpp` or `llamacpp`, so the page can badge where a voice
    /// comes from and group it without a second list.
    pub engine: String,
    /// Whether this voice can speak right now without a cold start.
    ///
    /// Piper is always ready -- its process is per sentence -- and the two
    /// servers are ready when they are up. A speech model is only offered as a
    /// voice while it is loaded, which is why the page reads this instead of
    /// drawing a row that would start a 2 GB process when pressed.
    pub loaded: bool,
}

/// Every voice the app can speak with, Piper's first.
///
/// Async because the audio engine's rate is learned from a synthesis, not
/// stated in a config: an unread list costs nothing, and a guessed rate sounds
/// wrong rather than failing loudly.
///
/// `app` is what makes the GGUF speech models visible: they are registered in
/// the chat library rather than living in a speech folder, so only the app can
/// name them. A caller with no handle (an engine's asset count) passes `None`
/// and sees the two folder engines.
pub async fn list(app: Option<&tauri::AppHandle>) -> Vec<VoiceEntry> {
    let mut entries = Vec::new();
    collect(&models_root().join("tts"), &mut entries);
    for model in super::audiocpp::voices() {
        let sample_rate = super::audiocpp::known_rate(&model.path.display().to_string()).await;
        entries.push(VoiceEntry {
            id: model.path.display().to_string(),
            name: model.name.clone(),
            sample_rate,
            size: model.size,
            engine: "audiocpp".to_string(),
            loaded: crate::audio::audiocpp::running(crate::audio::audiocpp::Purpose::Speech).await,
        });
    }
    // One entry per speech model, not one per language or voice: the model is
    // what is installed, and the voice it uses is decided by the instruction it
    // is given rather than by a file. Listing eighteen languages as eighteen
    // voices would invent choices nothing stores.
    if let Some(app) = app {
        let loaded = crate::audio::speech::loaded_model().await;
        for model in crate::audio::speech::installed(app) {
            if !model.speaks {
                continue;
            }
            entries.push(VoiceEntry {
                id: model.path.clone(),
                name: model.name.clone(),
                sample_rate: crate::audio::speech::known_rate(&model.path),
                size: std::fs::metadata(&model.path)
                    .map(|meta| meta.len())
                    .unwrap_or(0),
                engine: "llamacpp".to_string(),
                loaded: loaded.as_deref() == Some(model.path.as_str()),
            });
        }
    }
    // Stable order, so a re-render does not reorder the list under the pointer.
    entries.sort_by(|left, right| {
        left.engine
            .cmp(&right.engine)
            .then_with(|| left.name.cmp(&right.name))
    });
    entries
}

/// The selection a voice path stands for.
///
/// Empty is the same choice as nothing, so the stored record cannot end up with
/// two spellings of "no voice".
pub fn canonical(id: Option<&str>) -> Option<String> {
    let id = id?;
    if id.trim().is_empty() {
        return None;
    }
    Some(id.to_string())
}

/// The chosen voice, resolved to whichever engine owns its folder.
///
/// **A speech model answers when one is installed.** It is a model the user
/// already has, so reading a reply with it must not load Piper or audio.cpp
/// beside it -- the same rule recognition follows. `id` still names a specific
/// voice; when it names a speech model's file that model is chosen, and when
/// nothing is stored the first such model answers.
///
/// A selection whose file has gone falls back to a speech model rather than
/// failing, for the same reason: there is a voice installed, and refusing to use
/// it would report no voice while one is plainly there.
pub async fn resolve(app: Option<&tauri::AppHandle>, id: Option<&str>) -> Result<Speaking, String> {
    if let Some(app) = app {
        if let Some(found) = id.and_then(|id| crate::audio::speech::find(app, id)) {
            if found.speaks {
                return Ok(Speaking::Llamacpp(found));
            }
        }
    }
    let Some(id) = id else {
        if let Some(app) = app {
            if let Some(model) = crate::audio::speech::speaking(app) {
                return Ok(Speaking::Llamacpp(model));
            }
        }
        return Err("No voice is installed.".to_string());
    };
    let path = PathBuf::from(id);
    // Only files this app put here. A stored path is data, and data that names
    // an executable's neighbour is not a voice.
    if !path.is_file() || !is_managed(&path) {
        if let Some(app) = app {
            if let Some(model) = crate::audio::speech::speaking(app) {
                return Ok(Speaking::Llamacpp(model));
            }
        }
        return Err(format!("The voice is missing. Expected {id}."));
    }
    if super::audiocpp::owns_path(id) {
        let model = super::audiocpp::voices()
            .into_iter()
            .find(|model| model.path == path)
            .ok_or_else(|| format!("The voice is missing. Expected {id}."))?;
        return Ok(Speaking::Audiocpp(model));
    }
    // A GGUF in the chat library is not a Piper voice and must never be handed to
    // `piper.exe`; the only engines it can belong to are the two servers.
    if let Some(app) = app {
        if let Some(model) = crate::audio::speech::find(app, id) {
            if model.speaks {
                return Ok(Speaking::Llamacpp(model));
            }
        }
    }
    Ok(Speaking::Piper(super::piper::resolve_voice(&path)?))
}

/// A resolved, usable voice, in the shape its engine expects.
pub enum Speaking {
    /// Piper's per-sentence process.
    Piper(super::piper::Voice),
    /// audio.cpp's server, one family and one file.
    Audiocpp(crate::audio::audiocpp::families::Model),
    /// A GGUF speech model, served by its own `llama-server`.
    Llamacpp(crate::audio::speech::SpeechModel),
}

impl Speaking {
    /// What the chat page is told before it offers to read anything aloud.
    ///
    /// One shape either way, so no caller learns that three engines are behind
    /// one ask.
    pub async fn info(&self) -> VoiceInfo {
        match self {
            Self::Piper(voice) => VoiceInfo {
                available: true,
                name: file_name(&voice.model),
                sample_rate: voice.sample_rate,
                engine: "piper".to_string(),
                reason: None,
            },
            Self::Audiocpp(model) => VoiceInfo {
                available: true,
                name: model.name.clone(),
                sample_rate: super::audiocpp::known_rate(&model.path.display().to_string()).await,
                engine: "audiocpp".to_string(),
                reason: None,
            },
            Self::Llamacpp(model) => VoiceInfo {
                available: true,
                name: model.name.clone(),
                // Zero until the model has spoken once: the rate is not stated
                // anywhere read before a synthesis, and the frontend treats zero
                // as unknown rather than playing at a guessed rate.
                sample_rate: crate::audio::speech::known_rate(&model.path),
                engine: "llamacpp".to_string(),
                reason: None,
            },
        }
    }
}

/// The chosen voice's rate and name, or why none could be.
///
/// `available` is what the Read button gate reads; `sample_rate` is what the
/// player builds its buffer against, and a wrong one is not an error there --
/// it is a chipmunk. audio.cpp voices answer `0` until their first synthesis,
/// which the frontend reads as "unknown" rather than as a fact.
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VoiceInfo {
    pub available: bool,
    pub name: String,
    pub sample_rate: u32,
    /// Which engine will speak: `piper`, `audiocpp` or `llamacpp`.
    ///
    /// Carried so the page can shape its work for the engine. A generated voice
    /// takes seconds per sentence where Piper takes milliseconds, and the chunk
    /// size that makes a reply start quickly is not the same for the two.
    pub engine: String,
    /// Why it is unavailable, when it is.
    pub reason: Option<String>,
}

impl VoiceInfo {
    /// The record for a voice that cannot be used, so the caller has one shape
    /// to handle rather than an `Err` it has to invent a sample rate for.
    pub fn missing(reason: String) -> Self {
        Self {
            available: false,
            name: String::new(),
            sample_rate: DEFAULT_SAMPLE_RATE,
            engine: String::new(),
            reason: Some(reason),
        }
    }
}

/// Assumed before a config has been read. Only describes a voice that is absent.
const DEFAULT_SAMPLE_RATE: u32 = 16_000;

/// Reads `audio.sample_rate` out of a Piper voice's config.
///
/// Taken from the file rather than assumed, because the rate is a property of
/// the voice: a "medium" voice of the same model is 22.05 kHz, and playing it
/// at 16 would not fail, it would just sound wrong.
pub fn piper_sample_rate(config: &Path) -> Result<u32, String> {
    let text = std::fs::read_to_string(config)
        .map_err(|error| format!("Could not read {}: {error}", config.display()))?;
    let json: serde_json::Value = serde_json::from_str(&text)
        .map_err(|error| format!("Could not read {}: {error}", config.display()))?;
    json.get("audio")
        .and_then(|audio| audio.get("sample_rate"))
        .and_then(|rate| rate.as_u64())
        .map(|rate| rate as u32)
        .filter(|rate| *rate > 0)
        .ok_or_else(|| format!("{} does not state its sample rate.", config.display()))
}

/// Whether `path` is inside a folder this app owns.
pub fn is_managed(path: &Path) -> bool {
    path.starts_with(voice_root()) || path.starts_with(models_root())
}

/// Whether any Piper voice is installed.
///
/// The engine uninstall consults this before it deletes `piper.exe`: an audio.cpp
/// voice does not need that folder, so only Piper's own voices count as a
/// dependency.
pub fn piper_voices_installed() -> bool {
    let root = models_root().join("tts");
    if !root.is_dir() {
        return false;
    }
    let mut stack = vec![root];
    while let Some(directory) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&directory) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if path
                .extension()
                .and_then(|extension| extension.to_str())
                .is_some_and(|extension| extension.eq_ignore_ascii_case("onnx"))
            {
                return true;
            }
        }
    }
    false
}

/// The row a page draws for one Piper model file, or nothing if it cannot be
/// read.
///
/// A voice whose config cannot be read is skipped rather than listed at a
/// guessed rate -- a list entry that would play wrong is worse than a short
/// list.
fn entry_for(path: &Path) -> Option<VoiceEntry> {
    let sample_rate = piper_sample_rate(&path.with_extension("onnx.json")).ok()?;
    Some(VoiceEntry {
        id: path.display().to_string(),
        name: file_name(path),
        sample_rate,
        size: std::fs::metadata(path).map(|meta| meta.len()).unwrap_or(0),
        engine: "piper".to_string(),
        // Piper holds no model between sentences, so it is ready by definition.
        loaded: true,
    })
}

/// Walks `directory` for Piper's `.onnx` models, skipping the `.onnx.json`
/// beside each.
fn collect(directory: &Path, found: &mut Vec<VoiceEntry>) {
    let Ok(entries) = std::fs::read_dir(directory) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect(&path, found);
            continue;
        }
        let is_model = path
            .extension()
            .and_then(|extension| extension.to_str())
            .is_some_and(|extension| extension.eq_ignore_ascii_case("onnx"));
        if !is_model {
            continue;
        }
        if let Some(entry) = entry_for(&path) {
            found.push(entry);
        }
    }
}

fn file_name(path: &Path) -> String {
    path.file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default()
        .to_string()
}
