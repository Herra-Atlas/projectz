//! The commands the speech pages call.
//!
//! Four concerns, deliberately kept apart: what voices exist (`tts_voices`),
//! which one is chosen (`tts_select_voice`), how fast one speaks (`tts_speed`,
//! `tts_set_speed`), and speaking (`tts_voice`, `tts_speak`). The chat page only
//! ever needs the last two, which is why it does not have to know that a
//! settings record exists.
//!
//! The engine a voice belongs to is resolved once, from its folder
//! (`voices::resolve`), and the dispatch to Piper's process or audio.cpp's
//! server is the `match` in `tts_speak` -- which is the only place that knows
//! two engines are behind one ask.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use tauri::ipc::Response;
use tauri::{AppHandle, State};

use super::audiocpp;
use super::piper;
use super::settings::{TtsSettings, TtsSettingsView, VoiceSpeedView};
use super::voices::{self, Speaking, VoiceEntry, VoiceInfo};
use crate::database::Database;

/// What the chat can expect *before* it offers to read anything aloud.
///
/// `voice` names one explicitly, which is what a settings page's test button
/// needs; without it the stored selection is described, so the chat button always
/// talks about the voice that will actually speak.
#[tauri::command]
pub async fn tts_voice(
    database: State<'_, Arc<Database>>,
    voice: Option<String>,
) -> Result<VoiceInfo, String> {
    let settings = TtsSettings::load(&database);
    let target = voice.or_else(|| settings.voice.clone());
    let info = match voices::resolve(target.as_deref()).await {
        Ok(speaking) => speaking.info().await,
        Err(reason) => VoiceInfo::missing(reason),
    };
    Ok(info)
}

/// Every voice the app can speak with, Piper's first.
#[tauri::command]
pub async fn tts_voices() -> Vec<VoiceEntry> {
    voices::list().await
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
    settings.voice = voices::canonical(voice.as_deref());
    settings.save(&database)
}

/// One voice's pace. `voice` names it; without one, the built-in voice's.
#[tauri::command]
pub fn tts_speed(database: State<'_, Arc<Database>>, voice: Option<String>) -> VoiceSpeedView {
    let settings = TtsSettings::load(&database);
    let target = voices::canonical(voice.as_deref());
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
    let target = voices::canonical(voice.as_deref());
    settings.set_speed(target.as_deref(), speed);
    settings.save(&database)
}

/// Whether code is left out of what is spoken. Applies to every voice.
#[tauri::command]
pub fn tts_set_skip_code(database: State<'_, Arc<Database>>, skip: bool) -> Result<(), String> {
    let mut settings = TtsSettings::load(&database);
    settings.skip_code = skip;
    settings.save(&database)
}

/// Whether markdown markup is left out of what is spoken. Applies to every voice.
#[tauri::command]
pub fn tts_set_skip_markdown(database: State<'_, Arc<Database>>, skip: bool) -> Result<(), String> {
    let mut settings = TtsSettings::load(&database);
    settings.skip_markdown = skip;
    settings.save(&database)
}

/// Whether replies are read as they finish, without being pressed.
#[tauri::command]
pub fn tts_set_auto_speak(database: State<'_, Arc<Database>>, on: bool) -> Result<(), String> {
    let mut settings = TtsSettings::load(&database);
    settings.auto_speak = on;
    settings.save(&database)
}

/// Whether the auto read holds off on a voice that would need a cold start.
#[tauri::command]
pub fn tts_set_auto_speak_only_loaded(
    database: State<'_, Arc<Database>>,
    on: bool,
) -> Result<(), String> {
    let mut settings = TtsSettings::load(&database);
    settings.auto_speak_only_loaded = on;
    settings.save(&database)
}

/// The cleaned text a reply will be spoken from.
///
/// Split out of `tts_speak` because streaming needs the whole sentence plan
/// before it synthesizes the first one: the frontend asks once, splits the
/// answer into sentences, and then sends each sentence back with
/// `prepared: true`. Doing the cleaning there instead would run the fence rule
/// per fragment, where a fence has already been cut in half.
#[tauri::command]
pub fn tts_clean(database: State<'_, Arc<Database>>, text: String) -> String {
    let settings = TtsSettings::load(&database);
    super::text::for_speech(
        &text,
        settings.smart_speak,
        settings.skip_code,
        settings.skip_markdown,
    )
}

/// Whether the text is tidied before it is spoken.
#[tauri::command]
pub fn tts_set_smart_speak(database: State<'_, Arc<Database>>, on: bool) -> Result<(), String> {
    let mut settings = TtsSettings::load(&database);
    settings.smart_speak = on;
    settings.save(&database)
}

/// Whether synthesis is spoken sentence by sentence as it arrives.
#[tauri::command]
pub fn tts_set_stream_speech(database: State<'_, Arc<Database>>, on: bool) -> Result<(), String> {
    let mut settings = TtsSettings::load(&database);
    settings.stream_speech = on;
    settings.save(&database)
}

/// Whether speaking with the chosen voice would interrupt anyone waiting:
/// Piper synthesizes in a fresh, cheap process every time, audio.cpp wants
/// its server and its weights already resident.
///
/// `voice` names one explicitly, exactly as it does for `tts_voice`. Without it
/// the **stored selection** is resolved -- which is the only form the chat's
/// auto-read uses. Resolving the argument alone returned `false` for every call
/// the chat made, so "only if the voice is already ready" was on by default and
/// no finished reply was ever read aloud.
#[tauri::command]
pub async fn tts_voice_warm(
    database: State<'_, Arc<Database>>,
    voice: Option<String>,
) -> Result<bool, String> {
    let settings = TtsSettings::load(&database);
    let target = voice.or_else(|| settings.voice.clone());
    match voices::resolve(target.as_deref()).await {
        Ok(Speaking::Piper(_)) => Ok(true),
        Ok(Speaking::Audiocpp(_)) => Ok(crate::audio::audiocpp::running().await),
        Err(_) => Ok(false),
    }
}

/// Deletes a Piper voice the app installed, model and settings together.
///
/// A deleted voice also loses its stored pace and, if it was the selected one,
/// the selection clears -- which is the truth: there is nothing installed to read
/// with until another voice is downloaded.
#[tauri::command]
pub fn tts_remove_voice(database: State<'_, Arc<Database>>, voice: String) -> Result<(), String> {
    let path = PathBuf::from(&voice);
    let installed_root = crate::app_paths::models_root().join("tts");
    if !path.starts_with(&installed_root) {
        return Err("That voice is part of the app and cannot be deleted.".to_string());
    }
    if !path.is_file() {
        return Err("That voice is already gone.".to_string());
    }

    std::fs::remove_file(&path).map_err(|error| format!("Could not delete the voice: {error}"))?;
    // The settings file is part of the voice; a copy left behind would be a file
    // nothing can load and nothing lists.
    let config = path.with_extension("onnx.json");
    if config.is_file() {
        std::fs::remove_file(&config).map_err(|error| {
            format!("The voice was deleted, but its settings were not: {error}")
        })?;
    }

    let mut settings = TtsSettings::load(&database);
    settings.forget(&voice);
    settings.save(&database)
}

/// Removes one audio.cpp voice -- its model file, and the stored pace and
/// selection pointing at it.
///
/// A separate command rather than a branch in `tts_remove_voice`, because the
/// two engines have different neighbours to clean up (Piper carries a config
/// file beside the model, audio.cpp does not) and one `if` spanning two
/// engines' cleanup rules is how those rules start to disagree.
#[tauri::command]
pub fn tts_remove_audio_voice(
    database: State<'_, Arc<Database>>,
    voice: String,
) -> Result<(), String> {
    let path = Path::new(&voice);
    let installed_root = crate::app_paths::models_root().join("audio");
    if !path.starts_with(&installed_root) {
        return Err("That voice is part of the app and cannot be deleted.".to_string());
    }
    if !path.is_file() {
        return Err("That voice is already gone.".to_string());
    }
    std::fs::remove_file(path).map_err(|error| format!("Could not delete the voice: {error}"))?;

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
    app: AppHandle,
    database: State<'_, Arc<Database>>,
    text: String,
    voice: Option<String>,
    prepared: Option<bool>,
) -> Result<Response, String> {
    let text = text.trim().to_string();
    if text.is_empty() {
        return Err("There is nothing to read.".to_string());
    }

    // Read before the await: the state guard is not held across a suspension
    // point, and the record is a few fields rather than a live object.
    let settings = TtsSettings::load(&database);
    // Code is removed here rather than in the player, so the test buttons and the
    // chat button all speak the same words. Markdown markup leaves the same way,
    // when the switch asks for it: the voice reads what the reader sees.
    //
    // `prepared` is the streaming path: the frontend already asked `tts_clean`
    // for the whole reply, so re-running the fence and markdown rules per
    // sentence would be both wasted work and wrong at a boundary.
    let text = if prepared.unwrap_or(false) {
        text
    } else {
        super::text::for_speech(
            &text,
            settings.smart_speak,
            settings.skip_code,
            settings.skip_markdown,
        )
    };
    if text.is_empty() {
        return Err("There is nothing to read under the current skip settings.".to_string());
    }
    // The same voice speaks and is looked up, so a test on a voice's own page
    // hears that voice at that voice's pace rather than at the selected one's.
    let speed_key = match voice.as_deref() {
        Some(explicit) => voices::canonical(Some(explicit)),
        None => settings.voice.clone(),
    };
    let speed = settings.speed_for(speed_key.as_deref());
    let target = voice.or_else(|| settings.voice.clone());
    let resolved = voices::resolve(target.as_deref()).await?;

    match resolved {
        Speaking::Piper(p_voice) => {
            // Blocking, because piper is a process. Waiting on it here would
            // hold a runtime worker for the length of the synthesis.
            let pcm =
                tokio::task::spawn_blocking(move || piper::synthesize(&p_voice, &text, speed))
                    .await
                    .map_err(|error| format!("Speech synthesis was interrupted: {error}"))??;
            Ok(Response::new(pcm))
        }
        Speaking::Audiocpp(model) => {
            // Speed is the same reader's pace either way; audio.cpp takes it as
            // a plain multiplier in the request, so the stored value passes
            // through rather than being inverted as Piper's `length_scale` must
            // be.
            let speed = if speed > 0.0 { speed } else { 1.0 };
            let (pcm, _rate) =
                audiocpp::synthesize(&model, &text, Some(speed), &app, &database).await?;
            Ok(Response::new(pcm))
        }
    }
}
