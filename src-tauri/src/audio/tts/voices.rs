//! Which voices exist, which one is chosen, and where they live.
//!
//! # One place a voice comes from
//!
//! A voice is a file the user installed -- downloaded from the voice page or
//! copied in from disk -- so the list is whatever is under `models/tts` (Piper
//! voices, `.onnx` plus their `.onnx.json`) and `models/audio` (audio.cpp
//! voices, one gguf each). Nothing is placed by the build.
//!
//! # Why a voice is identified by its path
//!
//! A name is not unique -- two engines can publish a voice with the same file
//! name -- and the path is what an engine is given anyway. The stored setting
//! therefore holds a path, and `resolve` refuses one that is not inside this
//! app's own folders. Which *engine* a stored path is for is told by the folder
//! it sits under: `models/audio` is the audio.cpp engine's territory and
//! `models/tts` is Piper's. There is no second record that can disagree, and no
//! file that can be resolved under the wrong engine.

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
    /// `piper` or `audiocpp`, so the page can badge where a voice comes from.
    pub engine: String,
}

/// Every voice the app can speak with, Piper's first.
///
/// Async because the audio engine's rate is learned from a synthesis, not
/// stated in a config: an unread list costs nothing, and a guessed rate sounds
/// wrong rather than failing loudly.
pub async fn list() -> Vec<VoiceEntry> {
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
        });
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
/// A selection whose file has gone fails rather than falling back, because
/// there is nothing to fall back to: every voice is one the user installed, and
/// a fallback would pick a *different* user's voice.
pub async fn resolve(id: Option<&str>) -> Result<Speaking, String> {
    let id = id.ok_or_else(|| "No voice is installed.".to_string())?;
    let path = PathBuf::from(id);
    // Only files this app put here. A stored path is data, and data that names
    // an executable's neighbour is not a voice.
    if !path.is_file() || !is_managed(&path) {
        return Err(format!("The voice is missing. Expected {id}."));
    }
    if super::audiocpp::owns_path(id) {
        let model = super::audiocpp::voices()
            .into_iter()
            .find(|model| model.path == path)
            .ok_or_else(|| format!("The voice is missing. Expected {id}."))?;
        return Ok(Speaking::Audiocpp(model));
    }
    Ok(Speaking::Piper(super::piper::resolve_voice(&path)?))
}

/// A resolved, usable voice, in the shape its engine expects.
pub enum Speaking {
    /// Piper's per-sentence process.
    Piper(super::piper::Voice),
    /// audio.cpp's server, one family and one file.
    Audiocpp(crate::audio::audiocpp::families::Model),
}

impl Speaking {
    /// What the chat page is told before it offers to read anything aloud.
    ///
    /// One shape either way, so no caller learns that two engines are behind
    /// one ask.
    pub async fn info(&self) -> VoiceInfo {
        match self {
            Self::Piper(voice) => VoiceInfo {
                available: true,
                name: file_name(&voice.model),
                sample_rate: voice.sample_rate,
                reason: None,
            },
            Self::Audiocpp(model) => VoiceInfo {
                available: true,
                name: model.name.clone(),
                sample_rate: super::audiocpp::known_rate(&model.path.display().to_string()).await,
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
