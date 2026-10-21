//! The two commands the recognition row calls.

use serde::Serialize;
use tauri::AppHandle;

use super::engine;

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
