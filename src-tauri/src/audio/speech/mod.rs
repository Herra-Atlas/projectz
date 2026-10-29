//! A GGUF speech model, served by `llama-server` on a port of its own.
//!
//! The third engine behind both speech directions. It is not a separate kind of
//! thing from a chat model: it is an ordinary GGUF with an audio projector, run
//! by the same engine build, on `127.0.0.1:43124` instead of the chat port. One
//! process answers both asks -- recognition through `/v1/chat/completions` with
//! an `input_audio` part, synthesis through `/tts` -- which is why the lifecycle
//! lives here once rather than in `stt` and `tts` separately.
//!
//! # Why it is not always running
//!
//! The weights are about two gigabytes and the app can only afford one large
//! model resident next to a chat model. The server starts on the first ask and
//! is stopped by the same switches that release whisper's resident engine, so
//! "keep the model loaded" means the same thing for every engine.
//!
//! # What decides whether a model belongs here
//!
//! Its files, never its name. `hears_audio` comes from the projector's own
//! `clip.has_audio_encoder`, and `speaks` from an `<|audio_out|>` marker in the
//! model's chat template. A model that declares neither is a text model and is
//! never offered as a speech engine, whatever it is called.

use std::collections::HashMap;
use std::path::Path;
use std::sync::{LazyLock, Mutex};
use std::time::{Duration, Instant};

use serde::Serialize;
use tauri::{AppHandle, Manager};

use crate::ai::runtime::AiRuntime;

mod client;

/// The port this server listens on.
///
/// Taken below the others so the numbers stay readable: 43125 wake, 43126
/// audio.cpp speech, 43127 the chat model, 43128 the extension's setup page.
/// Where a speech model runs when nothing else is running.
pub const PORT: u16 = 43_124;

/// The chat model's server. It speaks `/tts` and takes `input_audio` too, so when
/// the chat model *is* the speech model there is no reason to load it twice.
const CHAT_PORT: u16 = 43_127;

/// The alias the server answers to, so a request never has to name a path.
const ALIAS: &str = "projectz-speech";

/// How long a 2 GB model may take to report healthy.
const READY: Duration = Duration::from_secs(120);

/// One installed model that can listen, speak, or both.
///
/// A copy of the facts rather than a `LocalModel` itself: the speech pages and
/// the two resolvers want to know a model's path, name and directions, and
/// nothing else. Keeping the list small means the settings UI never has to
/// understand the chat model's fields to draw a voice.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SpeechModel {
    /// The registered local model's id, which is what finds its engine.
    pub id: String,
    /// The GGUF's path. Also the identity a stored selection names.
    pub path: String,
    pub name: String,
    pub hears_audio: bool,
    pub speaks: bool,
    /// The audio projector, when the model has one configured.
    pub projector: Option<String>,
}

/// Every installed model whose own files say it can listen or speak.
pub fn installed(app: &AppHandle) -> Vec<SpeechModel> {
    let runtime = app.state::<AiRuntime>();
    let manager = runtime.local_models();
    manager
        .speech_models()
        .into_iter()
        .map(|model| {
            let projector = manager
                .runtime_settings(&model.id)
                .mmproj
                .filter(|path| !path.trim().is_empty());
            SpeechModel {
                id: model.id,
                path: model.path,
                name: model.name,
                hears_audio: model.hears_audio == Some(true),
                speaks: model.speaks == Some(true),
                projector,
            }
        })
        .collect()
}

/// The installed model a stored path names, if it is still a speech model.
pub fn find(app: &AppHandle, path: &str) -> Option<SpeechModel> {
    let wanted = crate::ai::local::memory::normalise_windows_path(path);
    installed(app)
        .into_iter()
        .find(|model| crate::ai::local::memory::normalise_windows_path(&model.path) == wanted)
}

/// The first installed model that can hear, for a recording with no selection.
pub fn hearing(app: &AppHandle) -> Option<SpeechModel> {
    installed(app).into_iter().find(|model| model.hears_audio)
}

/// The first installed model that can speak, for a reply with no selection.
pub fn speaking(app: &AppHandle) -> Option<SpeechModel> {
    installed(app).into_iter().find(|model| model.speaks)
}

/// The sample rate the server produced last time, by model path.
///
/// A speech model states no rate anywhere the app can read before it speaks, so
/// the first synthesis is what teaches it. `0` means "not asked yet", which the
/// frontend reads as unknown rather than as a fact -- the same contract
/// audio.cpp's voices use.
static RATES: LazyLock<Mutex<HashMap<String, u32>>> = LazyLock::new(|| Mutex::new(HashMap::new()));

pub fn known_rate(path: &str) -> u32 {
    RATES
        .lock()
        .ok()
        .and_then(|rates| rates.get(path).copied())
        .unwrap_or(0)
}

fn remember_rate(path: &str, rate: u32) {
    if let Ok(mut rates) = RATES.lock() {
        rates.insert(path.to_string(), rate);
    }
}

/// The running server, or nothing.
struct Running {
    child: tokio::process::Child,
    /// Which model, and which projector, the process was started for -- so a
    /// settings change restarts it instead of silently serving the old weights.
    model: String,
    projector: Option<String>,
}

static SERVER: LazyLock<tokio::sync::Mutex<Option<Running>>> =
    LazyLock::new(|| tokio::sync::Mutex::new(None));

/// Whether a speech server is up right now.
///
/// A chat server counts: when it is running the speech model, `/tts` and
/// `input_audio` are already being served by it.
pub async fn running() -> bool {
    client::health_at(PORT).await || client::health_at(CHAT_PORT).await
}

/// The model the running server is serving, if one is.
///
/// The settings page uses this to decide whether the `llama.cpp` section exists
/// at all: a speech model is offered as a voice only while it is loaded, which is
/// also what makes it safe to default to -- it cannot be picked while it is not
/// ready to speak.
pub async fn loaded_model() -> Option<String> {
    if let Some(current) = SERVER.lock().await.as_ref() {
        return Some(current.model.clone());
    }
    // No server of ours: the chat server may be the speech model, which is the
    // common case -- one model that both talks and listens.
    client::props_model(CHAT_PORT).await
}

/// Starts the server for `model` if it is not already serving those exact files,
/// and hands back the port to talk to.
///
/// The port is the result rather than a constant because the cheapest correct
/// answer is often "the chat server is already running this exact model". LFM3
/// Audio is both a chat model and a speech model; loading it a second time for
/// the microphone doubles the two gigabytes of weights *and* the KV cache, which
/// is the difference between roughly 3 GB and 6 GB of VRAM for one model.
pub async fn ensure_started(app: &AppHandle, model: &SpeechModel) -> Result<u16, String> {
    if client::serves(CHAT_PORT, &model.path).await {
        adopt_as_voice(app, &model.path);
        return Ok(CHAT_PORT);
    }
    {
        let guard = SERVER.lock().await;
        if let Some(current) = guard.as_ref() {
            if current.model == model.path && current.projector == model.projector {
                drop(guard);
                if client::health_at(PORT).await {
                    return Ok(PORT);
                }
            }
        }
    }
    stop().await;
    start(app, model).await
}

async fn start(app: &AppHandle, model: &SpeechModel) -> Result<u16, String> {
    let runtime = app.state::<AiRuntime>();
    let server = runtime.local_models().server_for(&model.id).ok_or_else(|| {
        "This speech model has no llama.cpp engine to run it. Pick one for it in Settings > Local."
            .to_string()
    })?;

    let mut command = tokio::process::Command::new(&server);
    command
        .arg("--model")
        .arg(&model.path)
        .arg("--alias")
        .arg(ALIAS)
        .arg("--host")
        .arg("127.0.0.1")
        .arg("--port")
        .arg(PORT.to_string())
        .arg("--no-webui")
        // A speech turn is seconds of audio, not a document. Left to itself the
        // server takes the context window the GGUF declares -- 128K on this
        // family -- and allocates a KV cache for all of it, which is most of the
        // 6 GB such a model appears to use. Four thousand tokens is a long
        // transcript and a short paragraph of speech, and it is the difference
        // between a 2 GB model and a 6 GB one.
        .arg("--ctx-size")
        .arg("8192")
        // The audio marker is rendered by the model's own template. Without the
        // Jinja engine the server would substitute a built-in template and the
        // `<|audio|>` placeholder a recording becomes would never be emitted.
        .arg("--jinja");
    if let Some(projector) = model.projector.as_deref() {
        command.arg("--mmproj").arg(projector);
    }
    command
        .stdin(std::process::Stdio::null())
        // Nothing is drawn from these: the speech server's log is not a surface
        // this app has, and a piped stdout nobody reads is a deadlock waiting for
        // the first full buffer.
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true);
    crate::child::hide_console_async(&mut command);

    let child = command
        .spawn()
        .map_err(|error| format!("The speech model could not be started: {error}"))?;
    *SERVER.lock().await = Some(Running {
        child,
        model: model.path.clone(),
        projector: model.projector.clone(),
    });

    let deadline = Instant::now() + READY;
    while Instant::now() < deadline {
        if client::health_at(PORT).await {
            // A load is the one moment a speech model may take the voice back.
            // The user's own choice stands until then, which is why this lives
            // here rather than in the settings read: loading is the event they
            // named, and the page asks about a running server.
            adopt_as_voice(app, &model.path);
            return Ok(PORT);
        }
        // A model that exited during load will never answer, and waiting the full
        // two minutes for it would look like the app had hung.
        {
            let mut guard = SERVER.lock().await;
            let exited = match guard.as_mut() {
                Some(current) => current.child.try_wait().ok().flatten(),
                None => return Err("The speech model was stopped while loading.".to_string()),
            };
            if let Some(status) = exited {
                *guard = None;
                return Err(format!(
                    "The speech model stopped while loading ({status}). Check that its engine supports audio models."
                ));
            }
        }
        tokio::time::sleep(Duration::from_millis(400)).await;
    }
    stop().await;
    Err("The speech model did not come up in time.".to_string())
}

/// Stops the server, if one is running. The cost of an idle 2 GB model is the
/// reason every release path calls this.
pub async fn stop() {
    let mut guard = SERVER.lock().await;
    if let Some(mut current) = guard.take() {
        let _ = current.child.kill().await;
    }
}

/// Transcribes one WAV recording.
pub async fn transcribe(
    app: &AppHandle,
    model: &SpeechModel,
    wav: &Path,
    language: Option<&str>,
) -> Result<String, String> {
    if !model.hears_audio {
        return Err("This model cannot take audio. Choose a speech model that can.".to_string());
    }
    let port = ensure_started(app, model).await?;
    let bytes = std::fs::read(wav)
        .map_err(|error| format!("The recording could not be read back: {error}"))?;
    client::transcribe(port, &bytes, language).await
}

/// Speaks `text` and hands back its sample rate with the samples as the raw
/// little-endian bytes the player reads.
///
/// Bytes rather than `i16`s at this boundary because that is what every other
/// engine already returns and what the frontend builds an `Int16Array` over: a
/// second shape here would be a conversion in TypeScript for no gain.
pub async fn speak(
    app: &AppHandle,
    model: &SpeechModel,
    text: &str,
) -> Result<(u32, Vec<u8>), String> {
    if !model.speaks {
        return Err("This model cannot produce speech. Choose one that can.".to_string());
    }
    let port = ensure_started(app, model).await?;
    let (rate, samples) = client::speak(port, text).await?;
    remember_rate(&model.path, rate);
    let mut bytes = Vec::with_capacity(samples.len() * 2);
    for sample in samples {
        bytes.extend_from_slice(&sample.to_le_bytes());
    }
    Ok((rate, bytes))
}

/// Makes the freshly loaded model the voice replies are read in.
///
/// The rule this exists for: a speech model becomes the default the moment it
/// loads, and a choice made afterwards sticks -- until the next load, which is
/// the only event that takes it back. Nothing here re-checks that the model can
/// speak: `installed` already refused the ones that cannot.
fn adopt_as_voice(app: &AppHandle, path: &str) {
    use tauri::Emitter;
    let database = app.state::<AiRuntime>().database();
    let mut settings = crate::audio::tts::settings::TtsSettings::load(&database);
    if settings.voice.as_deref() == Some(path) {
        return;
    }
    settings.voice = Some(path.to_string());
    if settings.save(&database).is_ok() {
        // The chat page and the voice page both hold the selection; without this
        // they would keep describing the voice that was chosen before the load.
        let _ = app.emit("tts-voice-changed", ());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A rate nobody has asked for yet is unknown, not zero hertz.
    #[test]
    fn an_unspoken_model_reports_no_rate() {
        assert_eq!(known_rate("/models/never-spoken.gguf"), 0);
    }
}
