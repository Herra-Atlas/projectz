//! Which voices exist, which one is chosen, and where they live.
//!
//! # One place a voice comes from
//!
//! A voice is a file the user installed -- downloaded from the voice page or
//! copied in from disk -- so the list is whatever is under `models/tts`. Nothing
//! is placed by the build.
//!
//! # Why a voice is identified by its path
//!
//! A name is not unique -- two repositories can publish `en_US-amy-low.onnx` --
//! and the path is what Piper is given anyway. The stored setting therefore holds
//! a path, and `resolve` refuses one that is not inside this app's own folders.

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
}

/// Every voice the app can speak with.
pub fn list() -> Vec<VoiceEntry> {
    let mut entries = Vec::new();
    collect(&models_root().join("tts"), &mut entries);
    // Stable order, so a re-render does not reorder the list under the pointer.
    entries.sort_by(|left, right| left.name.cmp(&right.name));
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

/// The chosen voice.
///
/// A selection whose file has gone fails rather than falling back, because there
/// is nothing to fall back to: every voice is one the user installed.
pub fn resolve(id: Option<&str>) -> Result<Voice, String> {
    let id = id.ok_or_else(|| "No voice is installed.".to_string())?;
    let path = PathBuf::from(id);
    // Only files this app put here. A stored path is data, and data that names an
    // executable's neighbour is not a voice.
    if !path.is_file() || !is_managed(&path) {
        return Err(format!("The voice is missing. Expected {id}."));
    }
    from_model(&path)
}

/// A resolved, usable voice: every path `piper.exe` needs, already checked.
pub struct Voice {
    pub program: PathBuf,
    /// The child process' working directory, which is also where its data sits.
    pub home: PathBuf,
    pub model: PathBuf,
    /// The rate of the PCM Piper returns, read from the config beside the model.
    pub sample_rate: u32,
}

impl Voice {
    pub fn info(&self) -> VoiceInfo {
        VoiceInfo {
            available: true,
            name: file_name(&self.model),
            sample_rate: self.sample_rate,
            reason: None,
        }
    }
}

/// What the chat page is told before it offers to read anything aloud.
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VoiceInfo {
    pub available: bool,
    pub name: String,
    /// The rate of the PCM `tts_speak` returns. The frontend needs it to build a
    /// buffer, and a wrong one is not an error there -- it is a chipmunk.
    pub sample_rate: u32,
    /// Why it is unavailable, when it is.
    pub reason: Option<String>,
}

impl VoiceInfo {
    /// The record for a voice that cannot be used, so the caller has one shape to
    /// handle rather than an `Err` it has to invent a sample rate for.
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

/// Reads a model and its settings into a `Voice`, naming what is missing.
fn from_model(model: &Path) -> Result<Voice, String> {
    let home = install_dir();
    let program = home.join("piper.exe");
    if !program.is_file() {
        return Err(format!(
            "Piper is not installed. Expected {}.",
            program.display()
        ));
    }
    if !model.is_file() {
        return Err(format!("The voice is missing. Expected {}.", model.display()));
    }

    // `with_extension` replaces the last component, so `x.onnx` becomes
    // `x.onnx.json` -- the name Piper writes beside the model.
    let config = model.with_extension("onnx.json");
    if !config.is_file() {
        return Err(format!(
            "The voice's settings are missing. Expected {}.",
            config.display()
        ));
    }

    Ok(Voice {
        program,
        home,
        model: model.to_path_buf(),
        sample_rate: sample_rate_from(&config)?,
    })
}

/// The row a page draws for one model file, or nothing if it cannot be read.
///
/// The rules for reading a voice's rate belong here rather than in a command, so
/// the list and any future caller agree on what a voice is.
fn entry_for(model: &Path) -> Option<VoiceEntry> {
    let config = model.with_extension("onnx.json");
    let sample_rate = sample_rate_from(&config).ok()?;
    Some(VoiceEntry {
        id: model.display().to_string(),
        name: file_name(model),
        sample_rate,
        size: std::fs::metadata(model).map(|meta| meta.len()).unwrap_or(0),
    })
}

/// Walks `directory` for `.onnx` models, skipping the `.onnx.json` beside each.
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

/// Whether `path` is inside a folder this app owns.
fn is_managed(path: &Path) -> bool {
    path.starts_with(voice_root()) || path.starts_with(models_root())
}

fn file_name(path: &Path) -> String {
    path.file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default()
        .to_string()
}

/// Reads `audio.sample_rate` out of the voice's config.
///
/// Taken from the file rather than assumed, because the rate is a property of the
/// voice: a "medium" voice of the same model is 22.05 kHz, and playing it at 16
/// would not fail, it would just sound wrong.
fn sample_rate_from(config: &Path) -> Result<u32, String> {
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
