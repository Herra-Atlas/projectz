//! The commands the Voice recognition page calls.

use std::sync::Arc;
use std::time::Duration;

use tauri::State;

use crate::database::Database;

use super::Status;

/// How long one enrollment clip may run. Long enough for a sentence, short
/// enough that a page waiting on it is not stuck: the recorder ends itself on
/// silence, so a short phrase finishes well before this.
const CLIP: Duration = Duration::from_secs(6);

#[tauri::command]
pub fn voice_status(database: State<'_, Arc<Database>>) -> Status {
    super::status(&database)
}

/// Turns the second gate on or off.
///
/// Enabled with nothing enrolled would silently match nobody, so the refusals
/// are named here rather than left to look like a broken microphone.
#[tauri::command]
pub fn voice_set_enabled(
    database: State<'_, Arc<Database>>,
    enabled: bool,
) -> Result<Status, String> {
    let mut settings = super::settings(&database);
    if enabled {
        if !crate::audio::engines::sherpa::is_installed() {
            return Err(
                "Install Sherpa-onnx on the Audio engine page before turning this on.".to_string(),
            );
        }
        if settings.voices.is_empty() {
            return Err("Enroll at least one voice before turning this on.".to_string());
        }
    }
    settings.enabled = enabled;
    super::save(&database, &settings)?;
    Ok(super::status(&database))
}

#[tauri::command]
pub fn voice_set_threshold(
    database: State<'_, Arc<Database>>,
    threshold: f32,
) -> Result<Status, String> {
    let mut settings = super::settings(&database);
    settings.threshold = threshold.clamp(0.1, 0.95);
    super::save(&database, &settings)?;
    Ok(super::status(&database))
}

/// Records one clip for a voice and embeds it immediately.
///
/// The recording runs on a blocking thread because the microphone and the model
/// are both blocking; the command returns once the clip has been embedded, so
/// the page can show how many takes are held without polling.
#[tauri::command]
pub async fn voice_enroll_clip(database: State<'_, Arc<Database>>) -> Result<Status, String> {
    let device = crate::audio::stt::models::settings(&database).device;
    let samples = tauri::async_runtime::spawn_blocking(move || {
        // No `AppHandle`, so this recording never announces itself to the
        // composer: enrolling a voice must not start or end a dictation.
        let session = crate::audio::stt::capture::start(
            device.as_deref(),
            None,
            crate::audio::stt::capture::DEFAULT_SILENCE,
        )?;
        // Waits on the recording's own ending rather than on a fixed duration:
        // a short sentence returns in seconds instead of in the cap.
        let deadline = std::time::Instant::now() + CLIP;
        while !session.ended() && std::time::Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(50));
        }
        session.finish()
    })
    .await
    .map_err(|error| format!("The recording task failed: {error}"))??;

    super::enroll_clip(&samples)?;
    Ok(super::status(&database))
}

/// Averages the held clips into the voiceprint and stores it.
#[tauri::command]
pub fn voice_enroll_finish(
    database: State<'_, Arc<Database>>,
    name: String,
) -> Result<Status, String> {
    super::finish_enrollment(&database, &name)?;
    Ok(super::status(&database))
}

#[tauri::command]
pub fn voice_enroll_cancel(database: State<'_, Arc<Database>>) -> Status {
    super::cancel_enrollment();
    super::status(&database)
}

#[tauri::command]
pub fn voice_delete(database: State<'_, Arc<Database>>, name: String) -> Result<Status, String> {
    super::delete(&database, &name)?;
    // The last voice going must not leave the switch on with nothing to match.
    let mut settings = super::settings(&database);
    if settings.voices.is_empty() && settings.enabled {
        settings.enabled = false;
        super::save(&database, &settings)?;
    }
    Ok(super::status(&database))
}
