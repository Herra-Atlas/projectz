//! The commands the Wake word page calls.
//!
//! The record is written after the microphone is, not before: a switch that says
//! on while nothing listens is worse than a switch that says off and a message
//! saying why.

use std::sync::Arc;

use tauri::{AppHandle, State};

use crate::audio::audiocpp::families;
use crate::database::Database;

use super::WakeStatus;

#[tauri::command]
pub fn wake_status(database: State<'_, Arc<Database>>) -> WakeStatus {
    super::status(&database)
}

/// Turns the microphone on or off.
#[tauri::command]
pub async fn wake_set_enabled(
    app: AppHandle,
    database: State<'_, Arc<Database>>,
    enabled: bool,
) -> Result<WakeStatus, String> {
    if enabled {
        super::start(&app, database.inner().clone()).await?;
    } else {
        super::stop();
        // The engine is only up because this was listening; leaving it running
        // with nothing to ask it is what kept the header claiming audio.cpp was
        // resident after the switch said off.
        crate::audio::audiocpp::stop(crate::audio::audiocpp::Purpose::Wake, &app).await;
    }
    let mut settings = super::settings(&database);
    settings.enabled = enabled;
    super::save(&database, &settings)?;
    Ok(super::status(&database))
}

/// Chooses which installed wake model listens.
///
/// A change while the switch is on restarts the microphone, because the model is
/// captured by the runner that is already asking: the alternative is a page that
/// claims a choice the next window will not use.
#[tauri::command]
pub async fn wake_select_model(
    app: AppHandle,
    database: State<'_, Arc<Database>>,
    model: String,
) -> Result<WakeStatus, String> {
    // Both kinds of model are choosable: a pack the engine runs, or a head the
    // ONNX path runs. `status` is the one list that knows about both.
    let installed = super::status(&database).models;
    if !installed.iter().any(|candidate| candidate.path == model) {
        return Err("That wake word model is not on disk.".to_string());
    }
    let mut settings = super::settings(&database);
    settings.model = Some(model);
    super::save(&database, &settings)?;
    super::restart(&app, database.inner().clone()).await?;
    Ok(super::status(&database))
}

/// The wake engine's own lines, for its terminal row.
///
/// The ONNX path has no process to read, so the engine keeps its own account:
/// what started, what it heard, what it refused.
#[tauri::command]
pub fn wake_logs() -> Vec<String> {
    super::logs()
}

/// Deletes one wake-word model: a pack from the audio.cpp folders, or a head
/// from the ONNX engine's.
///
/// The path arrives from the page, so it is checked rather than trusted: a file
/// is only removable when it is a head in that folder or its name is one of the
/// wake families. A stored selection pointing at it is cleared, because a
/// setting naming a file that no longer exists would fall back silently and look
/// like the switch ignored the choice.
#[tauri::command]
pub fn wake_remove_model(
    database: State<'_, Arc<Database>>,
    model: String,
) -> Result<super::WakeStatus, String> {
    let path = std::path::PathBuf::from(&model);
    let heads = crate::audio::engines::openwakeword::heads_dir();
    let named = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default();
    let is_head = path.parent().is_some_and(|parent| parent == heads);
    let is_pack = matches!(families::classify(named), Some((_, families::Task::Wake)));
    if !is_head && !is_pack {
        return Err("That file is not a wake word model.".to_string());
    }
    if path.is_file() {
        std::fs::remove_file(&path)
            .map_err(|error| format!("That model could not be deleted: {error}"))?;
    }
    let mut settings = super::settings(&database);
    if settings.model.as_deref() == Some(model.as_str()) {
        settings.model = None;
        super::save(&database, &settings)?;
    }
    Ok(super::status(&database))
}

/// Copies a model from anywhere on the disk: a `.gguf` pack into `models/audio`,
/// or an `.onnx` head into the ONNX engine's folder.
///
/// The folder is the truth about what is installed, so "add a model" is a copy
/// into it -- the same door a hand-copied file comes through. The **name** of a
/// pack is what tells the app which family it is, and one whose name says nothing
/// would be loaded as the wrong architecture, so it is refused here with the rule
/// stated rather than at the first phrase.
#[tauri::command]
pub fn wake_add_model(
    database: State<'_, Arc<Database>>,
    source: String,
) -> Result<WakeStatus, String> {
    let origin = std::path::Path::new(&source);
    if !origin.is_file() {
        return Err("That file is not there.".to_string());
    }
    let extension = origin
        .extension()
        .and_then(|extension| extension.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    if extension == "onnx" {
        // A word head for the ONNX engine. It needs no family name: the file is
        // the phrase, and the engine that runs it is already installed.
        let folder = crate::audio::engines::openwakeword::heads_dir();
        std::fs::create_dir_all(&folder)
            .map_err(|error| format!("The wake word folder could not be made: {error}"))?;
        let name = origin
            .file_name()
            .ok_or_else(|| "That file has no name this app can read.".to_string())?;
        std::fs::copy(origin, folder.join(name))
            .map_err(|error| format!("That head could not be copied in: {error}"))?;
        return Ok(super::status(&database));
    }
    if extension != "gguf" {
        return Err("A wake word model is a .gguf package or an .onnx head.".to_string());
    }
    let name = origin
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| "That file has no name this app can read.".to_string())?
        .to_string();
    match families::classify(&name) {
        Some((_, families::Task::Wake)) => {}
        Some(_) => {
            return Err(format!(
                "\"{name}\" names a speech or recognition model, not a wake word one."
            ))
        }
        None => {
            return Err(format!(
                "\"{name}\" does not name a wake word family. Name it the way the engine's own packages are: open-wake-word-<your word>.gguf, micro-wake-word-<your word>.gguf, or sherpa-kws-<model>.gguf."
            ))
        }
    }
    let folder = crate::app_paths::models_root().join("audio");
    std::fs::create_dir_all(&folder)
        .map_err(|error| format!("Could not create {}: {error}", folder.display()))?;
    let target = folder.join(&name);
    if target.exists() {
        return Err(format!("A model called \"{name}\" is already installed."));
    }
    std::fs::copy(origin, &target)
        .map_err(|error| format!("The model could not be copied: {error}"))?;
    Ok(super::status(&database))
}

/// Sets how sure the model has to be. Clamped rather than refused: the page's
/// control cannot express a value outside the range, so one that arrives is a
/// rounding, not an instruction to break the floor.
#[tauri::command]
pub fn wake_set_threshold(
    database: State<'_, Arc<Database>>,
    threshold: f32,
) -> Result<WakeStatus, String> {
    let mut settings = super::settings(&database);
    settings.threshold = threshold.clamp(0.0, 1.0);
    super::save(&database, &settings)?;
    // Handed to the running listener rather than restarting it: this is called
    // on every pointer move of a slider.
    super::set_threshold(settings.threshold);
    Ok(super::status(&database))
}
