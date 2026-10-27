//! The commands the engines list calls, for every audio engine.
//!
//! audio.cpp installs from the catalog; whisper and Piper keep their pinned
//! first-party builds. All three uninstall the same way, and the catalog's
//! download progress arrives on the llama.cpp engines' own event, so a UI hook
//! written once shows all four.

use serde::Serialize;
use tauri::AppHandle;

use super::audiocpp;
use super::{piper, whisper};
use crate::audio::stt::models;
use crate::audio::tts::voices;

/// One engine as the Audio page draws it: install state and what depends on it.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EngineRow {
    pub id: String,
    pub name: String,
    pub version: String,
    pub installed: bool,
    /// How many voice or model files the engine serves, as the page's footnote.
    pub assets: usize,
    /// Who owns the engine's files, so the page names where to delete them.
    pub kind: String,
}

/// The three engines the app knows, in install order.
///
/// One command rather than three so the page's rows stay one list; each row
/// counts its own assets -- recognition models for whisper, Piper voices for
/// Piper, installed gguf packages for audio.cpp -- because "installed" without
/// "what can it run" is half an answer.
#[tauri::command]
pub async fn audio_engines_builtin() -> Vec<EngineRow> {
    let piper_voices = voices::list()
        .await
        .into_iter()
        .filter(|voice| voice.engine == "piper")
        .count();
    vec![
        EngineRow {
            id: "audiocpp".to_string(),
            name: "audio.cpp".to_string(),
            version: audiocpp::installed_version().unwrap_or_default(),
            installed: audiocpp::is_installed(),
            assets: crate::audio::audiocpp::families::installed().len(),
            kind: "catalog".to_string(),
        },
        EngineRow {
            id: "whisper".to_string(),
            name: "Whisper & Parakeet".to_string(),
            version: whisper::RELEASE_TAG.to_string(),
            installed: whisper::is_installed(),
            assets: models::list().len(),
            kind: "fixed".to_string(),
        },
        EngineRow {
            id: "piper".to_string(),
            name: "Piper".to_string(),
            version: piper::RELEASE_TAG.to_string(),
            installed: piper::is_installed(),
            assets: piper_voices,
            kind: "fixed".to_string(),
        },
        EngineRow {
            // eSpeak is not an engine, it is a component -- but its row draws
            // exactly like one, because "installed / not" is the same question
            // the page answers for engines. The note explains why it exists.
            id: "espeak".to_string(),
            name: "eSpeak-ng phonemizer".to_string(),
            version: super::espeak::COMPONENT
                .trim_start_matches("espeak-ng-")
                .to_string(),
            installed: super::espeak::is_installed(),
            assets: 0,
            kind: "fixed".to_string(),
        },
        EngineRow {
            // Optional, and last, because it is the one engine that is only
            // used if the user turns voice recognition on: it exists so a
            // wake word can check *who* said it.
            id: "sherpa".to_string(),
            name: "Sherpa-onnx speaker models".to_string(),
            version: super::sherpa::installed_version().unwrap_or_default(),
            installed: super::sherpa::is_installed(),
            assets: usize::from(super::sherpa::is_installed()),
            kind: "fixed".to_string(),
        },
        // The ONNX runtime and the two models every openWakeWord head shares.
        EngineRow {
            id: "openwakeword".to_string(),
            name: "openWakeWord".to_string(),
            version: super::openwakeword::installed_version().unwrap_or_default(),
            installed: super::openwakeword::is_installed(),
            assets: usize::from(super::openwakeword::is_installed()),
            kind: "fixed".to_string(),
        },
    ]
}

/// What the engines row shows about an audio.cpp release.
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CatalogRow {
    pub id: String,
    pub version: String,
    pub updated_at: String,
    pub asset_name: String,
    pub size_bytes: u64,
    pub installed: bool,
}

#[tauri::command]
pub async fn audio_engines(refresh: Option<bool>) -> Result<Vec<CatalogRow>, String> {
    let installed = audiocpp::installed_ids();
    Ok(audiocpp::list(refresh.unwrap_or(false))
        .await?
        .into_iter()
        .map(|entry| {
            let installed = installed.contains(&entry.id);
            CatalogRow {
                id: entry.id,
                version: entry.version,
                updated_at: entry.updated_at,
                asset_name: entry.asset_name,
                size_bytes: entry.size_bytes,
                installed,
            }
        })
        .collect())
}

/// Installs the audio.cpp engine from one catalog entry.
#[tauri::command]
pub async fn audio_engines_install(app: AppHandle, id: String) -> Result<(), String> {
    audiocpp::install(&app, &id).await
}

/// Installs one of the pinned first-party engines, or the eSpeak component.
#[tauri::command]
pub async fn audio_builtin_install(app: AppHandle, id: String) -> Result<(), String> {
    match id.as_str() {
        "whisper" => whisper::install(&app).await,
        "piper" => piper::install(&app).await,
        "espeak" => super::espeak::install(&app).await,
        "sherpa" => super::sherpa::install(&app).await,
        "openwakeword" => super::openwakeword::install(&app).await,
        other => Err(format!("Unknown engine: {other}")),
    }
}

/// Uninstalls one of the three engines: `whisper`, `piper` or an audio.cpp id.
///
/// For audio.cpp the server is stopped first: its files would otherwise be
/// deleted under a process still holding handles, leaving a half-removed
/// install that looks installed to the engine check and not to anything else.
#[tauri::command]
pub async fn audio_builtin_uninstall(app: AppHandle, id: String) -> Result<(), String> {
    match id.as_str() {
        "whisper" => whisper::uninstall(),
        // Piper carries voices; its own uninstall is what refuses while they
        // exist, the same rule whisper's applies to its models.
        "piper" => piper::uninstall(),
        // `audiocpp` is the page's way of saying "whichever is installed", so
        // the row does not have to know the version it shipped.
        "audiocpp" => {
            crate::audio::audiocpp::stop_all(&app).await;
            let Some(directory) = audiocpp::installed_dir() else {
                return Err("The audio.cpp engine is not installed.".to_string());
            };
            let Some(engine_id) = directory
                .file_name()
                .and_then(std::ffi::OsStr::to_str)
                .map(str::to_string)
            else {
                return Err("The audio.cpp engine is not installed.".to_string());
            };
            audiocpp::uninstall(&engine_id)
        }
        other if other.starts_with(audiocpp::ID_PREFIX) => {
            crate::audio::audiocpp::stop_all(&app).await;
            audiocpp::uninstall(other)
        }
        "espeak" => super::espeak::uninstall(&app).await,
        // The library is loaded into the running app, so it is unloaded first:
        // Windows will not delete a DLL a process still holds open.
        "sherpa" => {
            crate::audio::voice::unload();
            super::sherpa::uninstall()
        }
        // No DLL is loaded from this engine's folder by a running app yet, so
        // there is nothing to release before the directory goes.
        "openwakeword" => super::openwakeword::uninstall(),
        other => Err(format!("Unknown engine: {other}")),
    }
}
