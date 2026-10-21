//! The commands the recognition row and the composer's microphone call.

use std::sync::{Arc, Mutex};

use serde::Serialize;
use tauri::{AppHandle, State};

use crate::database::Database;

use super::capture::{self, Session, MIN_SAMPLES};
use super::engine;
use super::model::{self, Model, SttSettings};
use super::transcribe;

/// What the Local page draws before it offers to install anything.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SttStatus {
    /// The whisper.cpp build is unpacked and has its executable.
    pub engine_installed: bool,
    /// The release the app installs, shown so a build is identifiable.
    pub engine_version: String,
    /// How many models are on disk.
    pub models: usize,
}

fn status() -> SttStatus {
    SttStatus {
        engine_installed: engine::is_installed(),
        engine_version: engine::RELEASE_TAG.to_string(),
        models: engine::installed_models(),
    }
}

/// The recording in progress, if there is one.
///
/// A `static` rather than Tauri state because there is exactly one microphone:
/// making it state would invite a second copy per handler. The mutex is held only
/// long enough to take or replace the session — never across a recording, which
/// would block every other command that touched it.
static RECORDING: Mutex<Option<Session>> = Mutex::new(None);

#[tauri::command]
pub fn stt_status() -> SttStatus {
    status()
}

/// Downloads and unpacks the engine, then answers with the new state.
///
/// Models are not fetched here: which one someone wants is their choice, and the
/// download page already lists them.
#[tauri::command]
pub async fn stt_install(app: AppHandle) -> Result<SttStatus, String> {
    engine::install(&app).await?;
    Ok(status())
}

#[tauri::command]
pub fn stt_models() -> Result<Vec<Model>, String> {
    Ok(model::list())
}

#[tauri::command]
pub fn stt_settings(database: State<'_, Arc<Database>>) -> SttSettings {
    model::settings(&database)
}

#[tauri::command]
pub fn stt_select_model(
    database: State<'_, Arc<Database>>,
    model: String,
) -> Result<SttSettings, String> {
    if !std::path::Path::new(&model).is_file() {
        return Err("That model is no longer on disk.".to_string());
    }
    let mut next = model::settings(&database);
    next.model = Some(model);
    model::save(&database, &next)?;
    Ok(next)
}

/// Removes a model and forgets everything stored about it.
#[tauri::command]
pub fn stt_remove_model(
    database: State<'_, Arc<Database>>,
    model: String,
) -> Result<(), String> {
    model::remove(&model)?;
    let mut settings = model::settings(&database);
    if settings.model.as_deref() == Some(model.as_str()) {
        settings.model = None;
    }
    settings.languages.remove(&model);
    model::save(&database, &settings)
}

#[tauri::command]
pub fn stt_model_language(database: State<'_, Arc<Database>>, model: String) -> String {
    model::settings(&database)
        .language_for(&model)
        .unwrap_or_default()
        .to_string()
}

#[tauri::command]
pub fn stt_set_language(
    database: State<'_, Arc<Database>>,
    model: String,
    language: String,
) -> Result<(), String> {
    let mut settings = model::settings(&database);
    let language = language.trim().to_string();
    if language.is_empty() {
        settings.languages.remove(&model);
    } else {
        settings.languages.insert(model, language);
    }
    model::save(&database, &settings)
}

/// Starts recording. Returns as soon as the device is actually open.
#[tauri::command]
pub fn stt_recording_start() -> Result<(), String> {
    let mut slot = RECORDING
        .lock()
        .map_err(|_| "The microphone is in an unusable state.".to_string())?;
    if slot.is_some() {
        return Err("The microphone is already recording.".to_string());
    }
    *slot = Some(capture::start()?);
    Ok(())
}

/// Stops recording and transcribes what was heard.
///
/// The work is blocking from end to end — the device closes, then a process runs —
/// so it is pushed off the async runtime's threads and only the wait is awaited.
#[tauri::command]
pub async fn stt_recording_stop(
    database: State<'_, Arc<Database>>,
) -> Result<String, String> {
    let session = RECORDING
        .lock()
        .map_err(|_| "The microphone is in an unusable state.".to_string())?
        .take();
    let Some(session) = session else {
        return Err("Nothing is recording.".to_string());
    };

    let samples = tokio::task::spawn_blocking(move || session.finish())
        .await
        .map_err(|error| format!("The recording was interrupted: {error}"))??;
    if samples.len() < MIN_SAMPLES {
        return Err("Nothing was heard. Hold the microphone a little longer.".to_string());
    }

    let settings = model::settings(&database);
    let selected = model::resolve(&settings)?;
    let language = settings.language_for(&selected.path).map(str::to_string);
    let text = tokio::task::spawn_blocking(move || {
        transcribe::transcribe(&selected, &samples, language.as_deref())
    })
    .await
    .map_err(|error| format!("Transcription was interrupted: {error}"))??;

    if text.is_empty() {
        return Err("Nothing was heard.".to_string());
    }
    Ok(text)
}
