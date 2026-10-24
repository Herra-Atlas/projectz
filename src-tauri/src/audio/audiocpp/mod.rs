//! The audio.cpp engine: one `audiocpp_server` process for both speech tasks.
//!
//! # Why a server rather than a CLI
//!
//! The engine's CLI loads a model per invocation, which for the heavier TTS
//! families is seconds of graph build per sentence. The server keeps a model
//! resident, and this app asks it in HTTP the same way it asks `llama-server`.
//! One process to mind also means one place for the single-instance rule, the
//! port, and the restart-after-config-change story.
//!
//! # Why the child is kept and killed, never dropped
//!
//! `kill_on_drop` answers an app exit, but not a model-install change: the
//! config is rebuilt at every start, so a restart command kills the held child
//! deliberately. A server left from a previous app session cannot happen -- a
//! child belongs to this process.
//!
//! # Why `max_loaded_models: 1`
//!
//! A speech model resident on the GPU is in direct reach of the local LLM the
//! app may be running: loading one evicts nothing, it adds. Evicting at one
//! loaded model is what keeps a Read button from costing a chat its context.

pub mod client;
pub mod commands;
pub mod config;
pub mod families;
pub mod settings;
pub mod specs;

use tauri::Emitter;
use tokio::sync::Mutex;

use super::engines::audiocpp;
use crate::database::Database;

/// The held server process, or nothing while it is down.
///
/// A `static` rather than Tauri state, because the callers are the `tts` and
/// `stt` dispatch layers deep in their own call chains and no command ever
/// needs to see the handle: the surface is `running`, `stop` and
/// `ensure_started`.
static SERVER: Mutex<Option<tokio::process::Child>> = Mutex::const_new(None);

/// The server's output, as the Logs view draws it.
///
/// The same message stream the local model's panel shows: the frontend listens
/// to `local-model-log` for new lines and reads a full buffer on open, so the
/// two panels work exactly one way. `modelId` is `audiocpp` for every line
/// this server owns.
static LOG_BUFFER: std::sync::LazyLock<std::sync::Mutex<Vec<String>>> =
    std::sync::LazyLock::new(|| std::sync::Mutex::new(Vec::new()));

/// How many log lines are kept. `llama-server`'s buffer holds the same shape of
/// tail: a server that prints a wall of startup traces must not push out the
/// error the user is looking for, which is what a cap exists for.
const MAX_LOG_LINES: usize = 500;

/// Whether the server is answering right now.
pub async fn running() -> bool {
    client::health().await
}

/// The server's output so far, oldest first.
pub fn logs() -> Vec<String> {
    LOG_BUFFER.lock().map(|log| log.clone()).unwrap_or_default()
}

/// The current status, for the settings page and any caller that must not
/// start anything.
pub async fn status() -> Status {
    Status {
        engine_installed: audiocpp::is_installed(),
        engine_version: audiocpp::installed_version(),
        models: families::installed().len(),
        running: running().await,
        started_at: SPAWNED_AT.lock().ok().and_then(|slot| *slot),
    }
}

/// When this app last started the answering server: a unix-seconds figure the
/// header's "running for X" reads, cleared whenever the server stops. An
/// answering server this app did not spawn reads `None` -- an unknown start
/// time is exactly that, not a guess.
static SPAWNED_AT: std::sync::LazyLock<std::sync::Mutex<Option<i64>>> =
    std::sync::LazyLock::new(|| std::sync::Mutex::new(None));

fn stamp_spawned() {
    if let Ok(mut slot) = SPAWNED_AT.lock() {
        *slot = Some(
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|duration| duration.as_secs() as i64)
                .unwrap_or(0),
        );
    }
}

fn stamp_stopped() {
    if let Ok(mut slot) = SPAWNED_AT.lock() {
        *slot = None;
    }
}

/// What the Audio page draws.
#[derive(Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    /// The engine's files are on disk and its server is runnable.
    pub engine_installed: bool,
    /// The release tag the installed engine carries, when known.
    pub engine_version: Option<String>,
    /// How many gguf models sit under `models/audio`.
    pub models: usize,
    /// Whether the server is up and answering.
    pub running: bool,
    /// Unix seconds this app launched the server at, when it did.
    pub started_at: Option<i64>,
}

/// Starts the server if it is not already answering, and rebuilds the config
/// from what is installed first.
///
/// Returns at once when an answering server is found -- whether it is this
/// session's child or one started before is indistinguishable, and both are fit
/// for the requests about to come. Only when nothing answers is a config
/// written and a process started, so an install made while a server was
/// already running is *not* picked up here: restart is the door for that, on
/// purpose, so a voice never plans against a server whose shape it did not see.
pub async fn ensure_started(app: &tauri::AppHandle, database: &Database) -> Result<(), String> {
    if running().await {
        return Ok(());
    }
    let engine = audiocpp::server_executable().ok_or_else(|| {
        "The audio.cpp engine is not installed. Install it from the Voice settings first."
            .to_string()
    })?;
    let models = families::installed();
    if models.is_empty() {
        return Err(
            "No audio.cpp model is installed. Download one from the Voice settings.".to_string(),
        );
    }
    // An unknown-family file cannot be configured, so it is not in the config;
    // a start whose models are all unknown has nothing to serve.
    if !models.iter().any(|model| model.task.is_some()) {
        return Err("None of the installed models is a family this app knows about.".to_string());
    }
    config::write(&models, database)?;

    let mut command = tokio::process::Command::new(&engine);
    command
        .current_dir(config::working_dir())
        .args(config::args())
        // Output on a pipe, so the logs view can show what the engine says --
        // the same way `llama-server`'s output reaches it.
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true);
    // Families learned to read eSpeak from the environment (Kokoro's own G2P
    // does) get the two variables in the child, not in the config -- the same
    // figures every speaker of the paths would carry.
    let env_families = settings::env_families(database);
    if !env_families.is_empty() {
        if let Some((library, data)) = crate::audio::engines::espeak::paths() {
            command
                .env("AUDIOCPP_ESPEAK_LIBRARY", library.display().to_string())
                .env("AUDIOCPP_ESPEAK_DATA", data.display().to_string());
        }
    }
    crate::child::hide_console_async(&mut command);
    let mut child = command
        .spawn()
        .map_err(|error| format!("The audio engine could not be started: {error}"))?;

    // A fresh start begins with an empty panel, whatever the last one printed:
    // the log belongs to the process it came from.
    if let Ok(mut log) = LOG_BUFFER.lock() {
        log.clear();
    }
    if let Some(stdout) = child.stdout.take() {
        let app = app.clone();
        // The reader exists for as long as the process does; it ends on its own
        // when the pipe closes, and it is not awaited here because nothing
        // waits for a server to start printing.
        tauri::async_runtime::spawn(pump_logs(app, stdout));
    }

    let mut slot = SERVER.lock().await;
    *slot = Some(child);
    let _ = app.emit("audio-server-state", serde_json::json!({ "running": true }));
    drop(slot);
    stamp_spawned();

    wait_ready(&app).await
}

/// Reads the server's stdout into the log buffer, line by line.
///
/// Every line is deferred to the frontend's open terminal tab and held here,
/// so a panel opened later reads what it missed -- the same two halves the
/// local model's logs work in.
///
/// Two in-line filters keep the panel readable. Health checks are dropped in
/// full: the server prints two debug lines per probe, and whoever probes --
/// this app's own wait, its WebUI, anything else on the machine -- the
/// stream is noise a reader scrolls past, never a diagnosis. Identical
/// consecutive lines collapse to the first one plus a `×N` marker (the
/// contract_spec tracer prints the same file several times per request).
async fn pump_logs(app: tauri::AppHandle, stdout: tokio::process::ChildStdout) {
    use tokio::io::AsyncBufReadExt;
    async fn push(app: &tauri::AppHandle, line: &str) {
        if let Ok(mut log) = LOG_BUFFER.lock() {
            log.push(line.to_string());
            let excess = log.len().saturating_sub(MAX_LOG_LINES);
            log.drain(0..excess);
        }
        let _ = app.emit(
            "local-model-log",
            serde_json::json!({ "modelId": "audiocpp", "line": line }),
        );
    }
    let mut reader = tokio::io::BufReader::new(stdout);
    let mut line = String::new();
    let mut run: Option<String> = None;
    let mut repeats = 0u32;
    loop {
        line.clear();
        match reader.read_line(&mut line).await {
            Ok(0) | Err(_) => break,
            Ok(_) => {
                let shown = line.trim_end_matches(['\n', '\r']).to_string();
                if shown.is_empty() {
                    continue;
                }
                if shown.contains("path=/health") {
                    continue;
                }
                if run.as_ref() == Some(&shown) {
                    repeats += 1;
                    continue;
                }
                if let (Some(previous), true) = (run.as_ref(), repeats > 0) {
                    push(&app, &format!("{previous}  ⟨same line ×{}⟩", repeats + 1)).await;
                }
                push(&app, &shown).await;
                run = Some(shown);
                repeats = 0;
            }
        }
    }
}

/// Waits for the server to answer, bounded.
///
/// A model-free start is quick, but a start that has to reserve GPU memory can
/// take a while; the budget is generous compared with any user-visible wait
/// before it, and the failure is a plain message rather than an eternal poll.
async fn wait_ready(app: &tauri::AppHandle) -> Result<(), String> {
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(60);
    while tokio::time::Instant::now() < deadline {
        tokio::time::sleep(std::time::Duration::from_millis(250)).await;
        if running().await {
            return Ok(());
        }
    }
    stop(app).await;
    Err("The audio engine did not come up in time. Check the logs on the right.".to_string())
}

/// Stops the held server, if one is held.
///
/// Restarting is a start: the config is rebuilt before every start, so a change
/// to what is installed is the next start sees it, and nothing here needs to
/// know whether the config has changed. The event tells the panel to let its
/// session close, which is the same guard the uninstall applies first.
pub async fn stop(app: &tauri::AppHandle) {
    let mut slot = SERVER.lock().await;
    if let Some(mut child) = slot.take() {
        let _ = child.kill().await;
    }
    drop(slot);
    stamp_stopped();
    let _ = app.emit(
        "audio-server-state",
        serde_json::json!({ "running": false }),
    );
}

/// The voice list synthesizes with its model resolvable by the audio engine.
///
/// A `Model` is stored as a `path` string in settings, so the one that carries
/// it is looked up again by path rather than any slug -- the slug is what the
/// server config calls it, not what a selection records.
///
/// `speed` is the reader's pace; the spec's request options the user set for
/// this model ride alongside it, so a voice-design instruction or a temperature
/// reaches the engine the way its own docs name.
pub async fn synthesize(
    selected: &families::Model,
    text: &str,
    speed: Option<f32>,
    app: &tauri::AppHandle,
    database: &Database,
) -> Result<(Vec<u8>, u32), String> {
    let path = selected.path.display().to_string();
    ensure_started(app, database).await?;
    // The slug the config gave this file is what the server listens for.
    let id = model_for_request(&selected.slug());
    let request = settings::request_options(database, &path, &selected.family);
    match client::speech(&id, text, speed, request).await {
        Ok((pcm, rate)) => {
            remember_rate(path, rate).await;
            Ok((pcm, rate))
        }
        Err(message) => {
            negotiate(app, database, selected, message).await?;
            // One negotiation is the whole second chance: the server restarted
            // with the corrected shape, and the identical call goes out again.
            let request = settings::request_options(database, &path, &selected.family);
            let (pcm, rate) = client::speech(&id, text, speed, request).await?;
            remember_rate(path, rate).await;
            Ok((pcm, rate))
        }
    }
}

/// One refusal, one correct answer, one restart.
///
/// A family's real contract sometimes disagrees with its spec, and can refuse
/// two different ways: Kokoro's spec names no eSpeak options, its runtime
/// refuses to speak without them, and its frontend takes the paths from the
/// *process environment* rather than the config -- so a session-keyed answer
/// to its refusal is refused right back. Rather than a per-model list to
/// babysit, the runtime *negotiates* down a ladder -- session options, then
/// the spawn environment, then nothing -- each known refusal being the server
/// naming what it wants. The trait is recorded, so the next start is right
/// before the process even begins, and the identical request is retried once
/// against the corrected server. One negotiation per call; anything else in
/// the message is the real error.
async fn negotiate(
    app: &tauri::AppHandle,
    database: &Database,
    selected: &families::Model,
    message: String,
) -> Result<(), String> {
    let family = selected.family.as_str();
    if message.contains("Could not load eSpeak-ng")
        && settings::espeak_trait(database, family) != Some(settings::EspeakTrait::Session)
    {
        // The family wants eSpeak and did not get it. Without an install the
        // fix is the user's; with one, the session shape is the first guess.
        if crate::audio::engines::espeak::paths().is_none() {
            return Err(format!(
                "{message} — install the eSpeak-ng phonemizer on the Audio engine page."
            ));
        }
        settings::learn_session(database, family)?;
        stop(app).await;
        ensure_started(app, database).await?;
        return Ok(());
    }
    let unknown_session = message.contains("session option") && message.contains("unknown");
    if unknown_session
        && settings::espeak_trait(database, family) == Some(settings::EspeakTrait::Session)
    {
        // The namespaced keys were refused: the family takes its eSpeak some
        // other way. One step down the ladder is the environment -- the shape
        // Kokoro's own G2P reads.
        settings::learn_env(database, family)?;
        stop(app).await;
        ensure_started(app, database).await?;
        return Ok(());
    }
    if unknown_session
        && settings::espeak_trait(database, family) == Some(settings::EspeakTrait::Env)
    {
        // Even an env family should not be receiving keys; if the spec gave
        // them names the engine does not take, that wiring goes too.
        settings::learn_env_clear(database, family);
        stop(app).await;
        ensure_started(app, database).await?;
        return Ok(());
    }
    Err(message)
}

/// The id the running server's config gave a model.
///
/// The config is built once per start, and this is the same mapping
/// `config::build` applied -- slug of the stem, with a `-N` suffix for a
/// collision. Recomputed against the same list rather than cached, because the
/// config and the list cannot disagree while both are derived from one walk.
fn model_for_request(slug: &str) -> String {
    let models = families::installed();
    let mut used: Vec<String> = Vec::new();
    for model in models {
        if model.task.is_none() {
            continue;
        }
        let mut id = model.slug();
        if id.is_empty() {
            id = "model".to_string();
        }
        let mut index = 2;
        while used.contains(&id) {
            id = format!("{}-{index}", model.slug());
            index += 1;
        }
        used.push(id.clone());
        if id == slug {
            return id;
        }
    }
    slug.to_string()
}

/// The last rate a synthesis answers with, by voice path.
///
/// The frontend's player needs a rate before it builds a buffer, and audio.cpp
/// reports its rate in the WAV header, not in a catalogue. The first name a
/// page shows may not know it yet -- `0`, which the page renders as "unknown"
/// rather than guessing -- and after the first synthesis the true figure is
/// here for the rest of the session.
///
/// A `static` rather than Tauri state for the same reason the server child is
/// one: the askers are deep in `tts_speak`'s call chain, and the value is a
/// handful of entries nobody else touches.
static RATE_CACHE: std::sync::LazyLock<std::sync::Mutex<std::collections::HashMap<String, u32>>> =
    std::sync::LazyLock::new(|| std::sync::Mutex::new(std::collections::HashMap::new()));

/// The rate to advertise for one voice path.
pub async fn known_rate(voice_path: &str) -> u32 {
    RATE_CACHE
        .lock()
        .ok()
        .and_then(|cache| cache.get(voice_path).copied())
        .unwrap_or(0)
}

/// Records the rate one synthesis came back at.
async fn remember_rate(voice_path: String, rate: u32) {
    if rate > 0 {
        if let Ok(mut cache) = RATE_CACHE.lock() {
            cache.insert(voice_path, rate);
        }
    }
}

/// Reads one recording back through the engine, the way the microphone hands it.
///
/// The options a recognition model takes reach the call the same way a TTS
/// family's do: through the spec of its family and what the user stored.
pub async fn transcribe(
    selected: &families::Model,
    wav: &std::path::Path,
    app: &tauri::AppHandle,
    database: &Database,
) -> Result<String, String> {
    let path = selected.path.display().to_string();
    ensure_started(app, database).await?;
    let request = settings::request_options(database, &path, &selected.family);
    client::transcribe(&model_for_request(&selected.slug()), wav, request).await
}
