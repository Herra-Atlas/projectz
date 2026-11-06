//! What the panel asks about the GGUF speech server.
//!
//! One command per question, and none of them starts the server: it starts
//! itself when a voice or a recording first needs it, which is the only moment
//! the caller knows which model is wanted. These exist so the process it started
//! is something the user can see and stop, like `llama-server` and the audio.cpp
//! engine already are.

use super::Status;

/// Whether the server is up, which model it is holding, and since when.
#[tauri::command]
pub async fn speech_model_status() -> Status {
    super::status().await
}

/// What the server has printed this run, oldest first.
#[tauri::command]
pub fn speech_model_logs() -> Vec<String> {
    super::logs()
}

/// Stops the server and the model it holds.
///
/// The panel's row × calls this, and so does anything else that needs the two
/// gigabytes back. A stop with nothing running is not an error: the state the
/// caller wants is the state it leaves.
#[tauri::command]
pub async fn speech_model_stop() -> Result<(), String> {
    super::stop().await;
    Ok(())
}
