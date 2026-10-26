//! The commands the recognition row and the composer's microphone call.
//!
//! The dispatch between the whisper.cpp CLI and the audio.cpp server happens in
//! `stt_recording_stop`, the one place that must know both; everything else
//! about a selection -- which path is stored, what its language pin is -- is
//! engine-independent, which is precisely why it lives in one settings record.

use std::sync::{Arc, Mutex};

use serde::Serialize;
use tauri::{AppHandle, State};

use crate::audio::engines::whisper as engine;
use crate::audio::stt::audiocpp;
use crate::database::Database;

use super::capture::{self, Session, MIN_SAMPLES};
use super::models::{self, SttSettings};

/// What the Local page draws before it offers to install anything.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SttStatus {
    /// The whisper.cpp build is unpacked and has its executable.
    pub engine_installed: bool,
    /// The release the app installs, shown so a build is identifiable.
    pub engine_version: String,
    /// How many models are on disk, across both engines.
    pub models: usize,
}

fn status() -> SttStatus {
    SttStatus {
        engine_installed: engine::is_installed(),
        engine_version: engine::RELEASE_TAG.to_string(),
        models: engine::installed_models() + audiocpp::models().len(),
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

/// One recognition model as the settings page lists it.
///
/// Both engines' lists become this row, with `audio` naming which engine the
/// file belongs to and `family` naming the audio.cpp family when it has one --
/// so the page draws one list and needs no idea that two engines produced it.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelRow {
    pub path: String,
    pub name: String,
    pub repo: String,
    pub size: u64,
    pub parakeet: bool,
    pub audio: bool,
    pub family: String,
}

#[tauri::command]
pub fn stt_models() -> Result<Vec<ModelRow>, String> {
    let mut rows: Vec<ModelRow> = models::list()
        .into_iter()
        .map(|model| ModelRow {
            path: model.path,
            name: model.name,
            repo: model.repo,
            size: model.size,
            parakeet: model.parakeet,
            audio: false,
            family: String::new(),
        })
        .collect();
    for model in audiocpp::models() {
        rows.push(ModelRow {
            path: model.path.display().to_string(),
            name: model.name,
            // The family is what a reader recognises about this model; the
            // repository folder is a packaging detail nobody types.
            repo: model.family.clone(),
            size: model.size,
            parakeet: false,
            audio: true,
            family: model.family,
        });
    }
    Ok(rows)
}

#[tauri::command]
pub fn stt_settings(database: State<'_, Arc<Database>>) -> SttSettings {
    models::settings(&database)
}

/// The capture endpoints the microphone picker offers.
///
/// `default` is what Windows would use with nothing chosen, so the row can say
/// which device "system default" currently means instead of leaving it a phrase.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Inputs {
    pub devices: Vec<String>,
    pub default: Option<String>,
}

#[tauri::command]
pub fn stt_inputs() -> Inputs {
    let (devices, default) = capture::inputs();
    Inputs { devices, default }
}

/// Chooses the microphone dictation records from, or none for the system default.
///
/// The name is checked against the endpoints that exist now: the picker only ever
/// offers those, so a name it did not offer is a stale record rather than a
/// choice, and storing it would fail on the user's next press instead of here.
#[tauri::command]
pub fn stt_select_input(
    database: State<'_, Arc<Database>>,
    device: Option<String>,
) -> Result<SttSettings, String> {
    let mut next = models::settings(&database);
    next.device = match device
        .map(|name| name.trim().to_string())
        .filter(|name| !name.is_empty())
    {
        Some(name) => {
            let (devices, _) = capture::inputs();
            if !devices.iter().any(|candidate| candidate == &name) {
                return Err(format!("The microphone \"{name}\" is not connected."));
            }
            Some(name)
        }
        None => None,
    };
    models::save(&database, &next)?;
    Ok(next)
}

#[tauri::command]
pub fn stt_select_model(
    database: State<'_, Arc<Database>>,
    model: String,
) -> Result<SttSettings, String> {
    if !std::path::Path::new(&model).is_file() {
        return Err("That model is no longer on disk.".to_string());
    }
    let mut next = models::settings(&database);
    next.model = Some(model);
    models::save(&database, &next)?;
    Ok(next)
}

/// Removes a model and forgets everything stored about it.
///
/// The folder is what decides which cleanup is due: `models/audio` is the
/// audio.cpp engine's territory and `models/stt` is whisper.cpp's. The audio
/// arm checks before it deletes for the same reason `models::remove` does --
/// a stored path is data, and data is not a licence to delete a file this app
/// did not install.
#[tauri::command]
pub fn stt_remove_model(database: State<'_, Arc<Database>>, model: String) -> Result<(), String> {
    if audiocpp::owns_path(&model) {
        let path = std::path::Path::new(&model);
        if !path.is_file() {
            return Err("That model is already gone.".to_string());
        }
        std::fs::remove_file(path)
            .map_err(|error| format!("The model could not be deleted: {error}"))?;
    } else {
        models::remove(&model)?;
    }
    let mut settings = models::settings(&database);
    if settings.model.as_deref() == Some(model.as_str()) {
        settings.model = None;
    }
    settings.languages.remove(&model);
    models::save(&database, &settings)
}

/// The language one model is told to expect, or none.
#[tauri::command]
pub fn stt_model_language(database: State<'_, Arc<Database>>, model: String) -> String {
    models::settings(&database)
        .language_for(&model)
        .unwrap_or_default()
        .to_string()
}

/// Pins the language one model is told to expect.
///
/// A pin on an audio.cpp model is stored but not sent for now: the languages
/// its families take are their own -- some are monolingual, some would be told
/// in a request field this app does not write yet -- and the choice is stored
/// under the same key so a later request can carry it without a migration.
#[tauri::command]
pub fn stt_set_language(
    database: State<'_, Arc<Database>>,
    model: String,
    language: String,
) -> Result<(), String> {
    let mut settings = models::settings(&database);
    let language = language.trim().to_string();
    if language.is_empty() {
        settings.languages.remove(&model);
    } else {
        settings.languages.insert(model, language);
    }
    models::save(&database, &settings)
}

/// Starts recording. Returns as soon as the device is actually open.
///
/// The device is read here rather than passed by the composer: which microphone
/// is in force is a stored setting, not a property of the press, so a change made
/// in Settings applies to the next recording without the chat page knowing the
/// setting exists.
#[tauri::command]
pub fn stt_recording_start(database: State<'_, Arc<Database>>) -> Result<(), String> {
    let mut slot = RECORDING
        .lock()
        .map_err(|_| "The microphone is in an unusable state.".to_string())?;
    if slot.is_some() {
        return Err("The microphone is already recording.".to_string());
    }
    let device = models::settings(&database).device;
    *slot = Some(capture::start(device.as_deref())?);
    Ok(())
}

/// Stops recording and transcribes what was heard.
///
/// The whisper path is blocking from end to end -- the device closes, then a
/// process runs -- so it is pushed off the async runtime's threads and only the
/// wait is awaited. The audio.cpp path is an HTTP call, so the same samples
/// travel there in an async context and no thread is parked for it.
#[tauri::command]
pub async fn stt_recording_stop(
    app: AppHandle,
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

    super::transcribe::samples(&app, &database, samples).await
}
