//! The commands the speech pages call.
//!
//! Four concerns, deliberately kept apart: what voices exist (`tts_voices`),
//! which one is chosen (`tts_select_voice`), how fast one speaks (`tts_speed`,
//! `tts_set_speed`), and speaking (`tts_voice`, `tts_speak`). The chat page only
//! ever needs the last two, which is why it does not have to know that a settings
//! record exists.

use std::sync::Arc;

use tauri::ipc::Response;
use tauri::State;

use super::piper;
use super::settings::{TtsSettings, TtsSettingsView, VoiceSpeedView};
use super::voice::{self, Voice, VoiceEntry, VoiceInfo};
use crate::database::Database;

/// What the chat can expect *before* it offers to read anything aloud.
///
/// `voice` names one explicitly, which is what a settings page's test button
/// needs; without it the stored selection is described, so the chat button always
/// talks about the voice that will actually speak.
#[tauri::command]
pub fn tts_voice(database: State<'_, Arc<Database>>, voice: Option<String>) -> VoiceInfo {
    let settings = TtsSettings::load(&database);
    let target = voice.or_else(|| settings.voice.clone());
    match voice::resolve(target.as_deref()) {
        Ok(voice) => voice.info(),
        Err(reason) => VoiceInfo::missing(reason),
    }
}

/// Every voice the app can speak with, the built-in one first.
#[tauri::command]
pub fn tts_voices() -> Vec<VoiceEntry> {
    voice::list()
}

/// The engine-wide record: which voice, and that voice's pace.
#[tauri::command]
pub fn tts_settings(database: State<'_, Arc<Database>>) -> TtsSettingsView {
    TtsSettingsView::of(&TtsSettings::load(&database))
}

/// Chooses the voice replies are read in.
///
/// Stored canonicalised, so the built-in voice is one value whether the caller
/// named it by its path or by "nothing".
#[tauri::command]
pub fn tts_select_voice(
    database: State<'_, Arc<Database>>,
    voice: Option<String>,
) -> Result<(), String> {
    let mut settings = TtsSettings::load(&database);
    settings.voice = voice::canonical(voice.as_deref());
    settings.save(&database)
}

/// One voice's pace. `voice` names it; without one, the built-in voice's.
#[tauri::command]
pub fn tts_speed(database: State<'_, Arc<Database>>, voice: Option<String>) -> VoiceSpeedView {
    let settings = TtsSettings::load(&database);
    let target = voice::canonical(voice.as_deref());
    VoiceSpeedView::of(&settings, target.as_deref())
}

/// Stores one voice's pace, leaving every other voice alone.
#[tauri::command]
pub fn tts_set_speed(
    database: State<'_, Arc<Database>>,
    voice: Option<String>,
    speed: f32,
) -> Result<(), String> {
    let mut settings = TtsSettings::load(&database);
    let target = voice::canonical(voice.as_deref());
    settings.set_speed(target.as_deref(), speed);
    settings.save(&database)
}

/// Whether code is left out of what is spoken. Applies to every voice.
#[tauri::command]
pub fn tts_set_skip_code(
    database: State<'_, Arc<Database>>,
    skip: bool,
) -> Result<(), String> {
    let mut settings = TtsSettings::load(&database);
    settings.skip_code = skip;
    settings.save(&database)
}

/// Deletes a voice the app installed, model and settings together.
///
/// A deleted voice also loses its stored pace and, if it was the selected one,
/// the selection clears -- which is the truth: there is nothing installed to read
/// with until another voice is downloaded.
#[tauri::command]
pub fn tts_remove_voice(
    database: State<'_, Arc<Database>>,
    voice: String,
) -> Result<(), String> {
    let path = std::path::PathBuf::from(&voice);
    let installed_root = crate::app_paths::models_root().join("tts");
    if !path.starts_with(&installed_root) {
        return Err("That voice is part of the app and cannot be deleted.".to_string());
    }
    if !path.is_file() {
        return Err("That voice is already gone.".to_string());
    }

    std::fs::remove_file(&path)
        .map_err(|error| format!("Could not delete the voice: {error}"))?;
    // The settings file is part of the voice; a copy left behind would be a file
    // nothing can load and nothing lists.
    let config = path.with_extension("onnx.json");
    if config.is_file() {
        std::fs::remove_file(&config)
            .map_err(|error| format!("The voice was deleted, but its settings were not: {error}"))?;
    }

    let mut settings = TtsSettings::load(&database);
    settings.forget(&voice);
    settings.save(&database)
}

/// Speaks `text` in the chosen voice and hands back raw PCM.
///
/// Raw samples rather than a WAV or an encoded file: the frontend has a player
/// that takes a buffer, and any container would be decoded straight back into
/// this. The sample rate it needs comes from `tts_voice`.
///
/// `voice` overrides the stored selection, which is what lets a settings page
/// read a voice it has not chosen yet.
#[tauri::command]
pub async fn tts_speak(
    database: State<'_, Arc<Database>>,
    text: String,
    voice: Option<String>,
) -> Result<Response, String> {
    let text = text.trim().to_string();
    if text.is_empty() {
        return Err("There is nothing to read.".to_string());
    }

    // Read before the await: the state guard is not held across a suspension
    // point, and the record is a few fields rather than a live object.
    let settings = TtsSettings::load(&database);
    // Code is removed here rather than in the player, so the test buttons and the
    // chat button all speak the same words.
    let text = super::text::for_speech(&text, settings.skip_code);
    if text.is_empty() {
        return Err("This reply is only code, and code is set to be skipped.".to_string());
    }
    // The same voice speaks and is looked up, so a test on a voice's own page
    // hears that voice at that voice's pace rather than at the selected one's.
    let speed_key = match voice.as_deref() {
        Some(explicit) => voice::canonical(Some(explicit)),
        None => settings.voice.clone(),
    };
    let speed = settings.speed_for(speed_key.as_deref());
    let target = voice.or_else(|| settings.voice.clone());
    let resolved: Voice = voice::resolve(target.as_deref())?;

    // Blocking, because piper is a process. Waiting on it here would hold a
    // runtime worker for the length of the synthesis.
    let pcm = tokio::task::spawn_blocking(move || piper::synthesize(&resolved, &text, speed))
        .await
        .map_err(|error| format!("Speech synthesis was interrupted: {error}"))??;
    Ok(Response::new(pcm))
}
