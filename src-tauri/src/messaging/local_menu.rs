//! `/unload` as a menu of what is actually running.
//!
//! Three local things can be resident or startable: the chat model's
//! `llama-server`, the audio.cpp server that serves voices and some STT
//! families, and the per-recording Whisper/Parakeet CLI. Only the first two can
//! be running between turns; the recognition CLI exits after it transcribes, so
//! there is nothing for a button to stop.

use std::sync::Arc;

use crate::messaging::adapter::Channel;
use crate::messaging::types::Button;
use crate::messaging::Messaging;

pub async fn show(
    hub: &Arc<Messaging>,
    channel: &dyn Channel,
    chat: &str,
    user_id: &str,
) -> Result<(), String> {
    let user = hub.settings().user(user_id);
    let models = hub.runtime().local_models();
    let status = models.status();
    let chat_id = status
        .loaded_model_id
        .clone()
        .or_else(|| status.loading_model_id.clone());
    let chat_name = chat_id.map(|id| {
        models
            .stored()
            .into_iter()
            .find(|model| model.id == id)
            .map(|model| model.name)
            .unwrap_or(id)
    });
    let audio_running = crate::audio::audiocpp::running().await;

    let mut lines = vec!["Local engines".to_string()];
    lines.push(match &chat_name {
        Some(name) => format!("llama-server: {name}"),
        None => "llama-server: not running".to_string(),
    });
    lines.push(format!(
        "voice.cpp / audio.cpp: {}",
        if audio_running {
            "running"
        } else {
            "not running"
        }
    ));
    lines.push("Whisper & Parakeet: starts for one transcription, nothing persistent".to_string());
    if !user.allow_local_start && chat_name.is_some() {
        lines.push("You do not have access to stop the local chat model.".to_string());
    }
    if !user.allow_stt_start && audio_running {
        lines.push("You do not have access to stop the speech-to-text engine.".to_string());
    }

    let mut buttons = Vec::new();
    if chat_name.is_some() && user.allow_local_start && channel.capabilities().buttons {
        buttons.push(Button::new("Stop llama-server", "ul:llama"));
    }
    if audio_running && user.allow_stt_start && channel.capabilities().buttons {
        buttons.push(Button::new("Stop voice.cpp", "ul:audio"));
    }
    if buttons.is_empty() {
        lines.push("Nothing here is yours to stop.".to_string());
    }

    channel
        .send(chat, &lines.join("\n"), &buttons)
        .await
        .map(|_| ())
}

/// Stops one engine, after checking the per-id grant again.
pub async fn handle(
    hub: &Arc<Messaging>,
    channel: &dyn Channel,
    chat: &str,
    user_id: &str,
    data: &str,
) -> Result<String, String> {
    let user = hub.settings().user(user_id);
    match data {
        "ul:llama" => {
            if !user.allow_local_start {
                return Err("You do not have access to stop the local chat model.".to_string());
            }
            let models = hub.runtime().local_models();
            let status = models.status();
            let id = status
                .loaded_model_id
                .or(status.loading_model_id)
                .ok_or_else(|| "No local chat model is running.".to_string())?;
            let name = models
                .stored()
                .into_iter()
                .find(|model| model.id == id)
                .map(|model| model.name)
                .unwrap_or(id);
            models.unload();
            let message = format!("Stopped llama-server ({name}).");
            channel.send(chat, &message, &[]).await?;
            Ok("Stopped.".to_string())
        }
        "ul:audio" => {
            if !user.allow_stt_start {
                return Err("You do not have access to stop the speech-to-text engine.".to_string());
            }
            if !crate::audio::audiocpp::running().await {
                return Err("voice.cpp is not running.".to_string());
            }
            crate::audio::audiocpp::stop(hub.app()).await;
            channel
                .send(chat, "Stopped voice.cpp / audio.cpp.", &[])
                .await?;
            Ok("Stopped.".to_string())
        }
        _ => {
            // A stale button on an old message: redraw the current state.
            show(hub, channel, chat, user_id).await?;
            Ok("Local engines refreshed.".to_string())
        }
    }
}
