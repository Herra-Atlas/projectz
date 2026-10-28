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

use super::engines::audiocpp;
use crate::database::Database;

/// The held server processes, one per purpose.
///
/// A `static` rather than Tauri state, because the callers are the `tts` and
/// `stt` dispatch layers deep in their own call chains and no command ever
/// needs to see the handle: the surface is `running`, `stop` and
/// `ensure_started_for`.
///
/// **One process per purpose, not one process for both.** With a single server
/// and `max_loaded_models: 1`, a Read pressed while the wake word listened
/// evicted the wake model, and the next wake window evicted the voice: two
/// features taking turns loading the same slot. Two processes also give each
/// one its own terminal and its own resident model, which is what a reader
/// expects to see when both are working.
static SERVERS: std::sync::LazyLock<
    tokio::sync::Mutex<std::collections::HashMap<Purpose, tokio::process::Child>>,
> = std::sync::LazyLock::new(Default::default);

/// The servers' output, per purpose, as the Logs view draws it.
///
/// The same message stream the local model's panel shows: the frontend listens
/// to `local-model-log` for new lines and reads a full buffer on open, so the
/// panels work exactly one way. Each line carries the purpose's own model id.
static LOG_BUFFER: std::sync::LazyLock<
    std::sync::Mutex<std::collections::HashMap<Purpose, Vec<String>>>,
> = std::sync::LazyLock::new(Default::default);

/// How many log lines are kept. `llama-server`'s buffer holds the same shape of
/// tail: a server that prints a wall of startup traces must not push out the
/// error the user is looking for, which is what a cap exists for.
const MAX_LOG_LINES: usize = 500;

/// Whether one purpose's server is answering right now.
pub async fn running(purpose: Purpose) -> bool {
    client::health(purpose).await
}

/// One server's output so far, oldest first.
pub fn logs(purpose: Purpose) -> Vec<String> {
    LOG_BUFFER
        .lock()
        .ok()
        .and_then(|buffers| buffers.get(&purpose).cloned())
        .unwrap_or_default()
}

/// The current status, for the settings page and any caller that must not
/// start anything.
///
/// One request answers both questions: a server that lists its models is
/// running, and the entry marked loaded is the one a panel should name. Asking
/// health and then the list would be two round trips for one fact.
pub async fn status(purpose: Purpose) -> Status {
    let known = client::models(purpose).await.ok();
    let loaded = known
        .as_deref()
        .and_then(|list| list.iter().find(|model| model.loaded))
        .map(label);
    Status {
        engine_installed: audiocpp::is_installed(),
        engine_version: audiocpp::installed_version(),
        models: families::installed()
            .into_iter()
            .filter(|model| serves(purpose, model.task))
            .count(),
        running: known.is_some(),
        started_at: SPAWNED_AT
            .lock()
            .ok()
            .and_then(|slots| slots.get(&purpose).copied()),
        loaded,
    }
}

/// Whether a model belongs to one purpose's server.
///
/// The split is the reason there are two processes: a wake model in the speech
/// server would be loaded by the same slot a voice needs.
fn serves(purpose: Purpose, task: Option<families::Task>) -> bool {
    match (purpose, task) {
        (Purpose::Speech, Some(families::Task::Tts | families::Task::Asr)) => true,
        (Purpose::Wake, Some(families::Task::Wake)) => true,
        _ => false,
    }
}

/// What a panel should call the model the server is holding.
///
/// The family, because it is the name a person recognises -- `open wake word`
/// rather than `wake` (the config id) or a path. A family the table does not
/// know falls back to the id, which is still better than repeating "audio.cpp".
fn label(model: &client::KnownModel) -> String {
    let family = model.family.replace('_', " ");
    if family.trim().is_empty() {
        model.id.clone()
    } else {
        family
    }
}

/// When this app last started the answering server: a unix-seconds figure the
/// header's "running for X" reads, cleared whenever the server stops. An
/// answering server this app did not spawn reads `None` -- an unknown start
/// time is exactly that, not a guess.
static SPAWNED_AT: std::sync::LazyLock<std::sync::Mutex<std::collections::HashMap<Purpose, i64>>> =
    std::sync::LazyLock::new(Default::default);

fn stamp_spawned(purpose: Purpose) {
    if let Ok(mut slots) = SPAWNED_AT.lock() {
        slots.insert(
            purpose,
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|duration| duration.as_secs() as i64)
                .unwrap_or(0),
        );
    }
}

fn stamp_stopped(purpose: Purpose) {
    if let Ok(mut slots) = SPAWNED_AT.lock() {
        slots.remove(&purpose);
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
    /// The model resident right now, as its family reads: `open wake word`.
    /// A panel names this instead of the engine, because the engine is not what
    /// the user started.
    pub loaded: Option<String>,
}

/// Which of the two engines a call is for.
///
/// A purpose is a process: its own config, its own port, its own resident
/// model, its own terminal. Speech covers every voice and transcription;
/// wake is the listener, which asks every second and must not compete for the
/// one model slot a voice needs.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Purpose {
    Speech,
    Wake,
}

impl Purpose {
    /// The settings/command name, and the key of its config and log buffer.
    pub fn key(self) -> &'static str {
        match self {
            Purpose::Speech => "speech",
            Purpose::Wake => "wake",
        }
    }

    pub fn from_key(key: &str) -> Option<Self> {
        match key {
            "speech" => Some(Purpose::Speech),
            "wake" => Some(Purpose::Wake),
            _ => None,
        }
    }

    /// The loopback port its server listens on.
    pub fn port(self) -> u16 {
        match self {
            Purpose::Speech => 43_126,
            Purpose::Wake => 43_125,
        }
    }

    /// The id the panel's log stream and its session carry.
    pub fn session(self) -> &'static str {
        match self {
            Purpose::Speech => "audiocpp",
            Purpose::Wake => "audiocpp-wake",
        }
    }
}

/// Starts the server for one caller, recording why it is up.
pub async fn ensure_started_for(
    purpose: Purpose,
    app: &tauri::AppHandle,
    database: &Database,
) -> Result<(), String> {
    if running(purpose).await {
        return Ok(());
    }
    let engine = audiocpp::server_executable().ok_or_else(|| {
        "The audio.cpp engine is not installed. Install it from the Voice settings first."
            .to_string()
    })?;
    // Only this purpose's models are configured: a wake file in the speech
    // server would be a second loadable model competing for its one slot.
    let models: Vec<families::Model> = families::installed()
        .into_iter()
        .filter(|model| serves(purpose, model.task))
        .collect();
    if models.is_empty() {
        return Err(match purpose {
            Purpose::Speech => {
                "No audio.cpp model is installed. Download one from the Voice settings.".to_string()
            }
            Purpose::Wake => {
                "No wake word model is installed. Add one from the Wake word page.".to_string()
            }
        });
    }
    config::write(&models, database, purpose)?;

    let mut command = tokio::process::Command::new(&engine);
    command
        .current_dir(config::working_dir())
        .args(config::args(purpose))
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
    if let Ok(mut buffers) = LOG_BUFFER.lock() {
        buffers.remove(&purpose);
    }
    if let Some(stdout) = child.stdout.take() {
        let app = app.clone();
        // The reader exists for as long as the process does; it ends on its own
        // when the pipe closes, and it is not awaited here because nothing
        // waits for a server to start printing.
        tauri::async_runtime::spawn(pump_logs(app, purpose, stdout));
    }

    let mut servers = SERVERS.lock().await;
    servers.insert(purpose, child);
    drop(servers);
    let _ = app.emit(
        "audio-server-state",
        serde_json::json!({ "running": true, "purpose": purpose.key() }),
    );
    stamp_spawned(purpose);

    wait_ready(purpose, &app).await
}

/// Reads the server's stdout into the log buffer, line by line.
///
/// Every line is deferred to the frontend's open terminal tab and held here,
/// so a panel opened later reads what it missed -- the same two halves the
/// local model's logs work in.
///
/// Two in-line filters keep the panel readable. **Request traffic is dropped in
/// full**: the server logs two debug lines per HTTP call, and this app probes
/// `/health` while starting and `/v1/models` on every status poll -- without
/// the filter the panel was a wall of `http.body_ready path=/v1/models` and the
/// one line worth reading was buried. Identical consecutive lines collapse to
/// the first one plus a `×N` marker (the contract_spec tracer prints the same
/// file several times per request).
async fn pump_logs(app: tauri::AppHandle, purpose: Purpose, stdout: tokio::process::ChildStdout) {
    use tokio::io::AsyncBufReadExt;
    let push = |line: &str| {
        if let Ok(mut buffers) = LOG_BUFFER.lock() {
            let buffer = buffers.entry(purpose).or_default();
            buffer.push(line.to_string());
            let excess = buffer.len().saturating_sub(MAX_LOG_LINES);
            buffer.drain(0..excess);
        }
        let _ = app.emit(
            "local-model-log",
            serde_json::json!({ "modelId": purpose.session(), "line": line }),
        );
    };
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
                if shown.contains("path=/health") || shown.contains("path=/v1/models") {
                    continue;
                }
                if run.as_ref() == Some(&shown) {
                    repeats += 1;
                    continue;
                }
                if let (Some(previous), true) = (run.as_ref(), repeats > 0) {
                    push(&format!("{previous}  ⟨same line ×{}⟩", repeats + 1));
                }
                push(&shown);
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
async fn wait_ready(purpose: Purpose, app: &tauri::AppHandle) -> Result<(), String> {
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(60);
    while tokio::time::Instant::now() < deadline {
        tokio::time::sleep(std::time::Duration::from_millis(250)).await;
        if running(purpose).await {
            return Ok(());
        }
    }
    stop(purpose, app).await;
    Err("The audio engine did not come up in time. Check the logs on the right.".to_string())
}

/// Stops both engines.
///
/// Uninstalling the engine, or replacing the eSpeak library it may hold open,
/// is about the files on disk rather than one feature, so nothing may survive
/// it.
pub async fn stop_all(app: &tauri::AppHandle) {
    stop(Purpose::Speech, app).await;
    stop(Purpose::Wake, app).await;
}

/// Stops one engine, if one is held for it.
///
/// Restarting is a start: the config is rebuilt before every start, so a change
/// to what is installed is the next start sees it, and nothing here needs to
/// know whether the config has changed. The event tells the panel to let its
/// session close, which is the same guard the uninstall applies first.
pub async fn stop(purpose: Purpose, app: &tauri::AppHandle) {
    let mut servers = SERVERS.lock().await;
    if let Some(mut child) = servers.remove(&purpose) {
        let _ = child.kill().await;
    }
    drop(servers);
    stamp_stopped(purpose);
    let _ = app.emit(
        "audio-server-state",
        serde_json::json!({ "running": false, "purpose": purpose.key() }),
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
    ensure_started_for(Purpose::Speech, app, database).await?;
    // The slug the config gave this file is what the server listens for.
    let id = model_for_request(&selected.slug(), Purpose::Speech);
    let request = settings::request_options(database, &path, &selected.family);
    match client::speech(Purpose::Speech, &id, text, speed, request).await {
        Ok((pcm, rate)) => {
            remember_rate(path, rate).await;
            Ok((pcm, rate))
        }
        Err(message) => {
            negotiate(app, database, selected, message).await?;
            // One negotiation is the whole second chance: the server restarted
            // with the corrected shape, and the identical call goes out again.
            let request = settings::request_options(database, &path, &selected.family);
            let (pcm, rate) = client::speech(Purpose::Speech, &id, text, speed, request).await?;
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
        stop(Purpose::Speech, app).await;
        ensure_started_for(Purpose::Speech, app, database).await?;
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
        stop(Purpose::Speech, app).await;
        ensure_started_for(Purpose::Speech, app, database).await?;
        return Ok(());
    }
    if unknown_session
        && settings::espeak_trait(database, family) == Some(settings::EspeakTrait::Env)
    {
        // Even an env family should not be receiving keys; if the spec gave
        // them names the engine does not take, that wiring goes too.
        settings::learn_env_clear(database, family);
        stop(Purpose::Speech, app).await;
        ensure_started_for(Purpose::Speech, app, database).await?;
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
fn model_for_request(slug: &str, purpose: Purpose) -> String {
    let models = families::installed();
    let mut used: Vec<String> = Vec::new();
    // The same purpose filter the config applied: two servers number their own
    // models from their own list, so a collision suffix must be counted per
    // server or the id sent would name a different file.
    for model in models
        .into_iter()
        .filter(|model| serves(purpose, model.task))
    {
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
    ensure_started_for(Purpose::Speech, app, database).await?;
    let request = settings::request_options(database, &path, &selected.family);
    client::transcribe(
        Purpose::Speech,
        &model_for_request(&selected.slug(), Purpose::Speech),
        wav,
        request,
    )
    .await
}

/// Asks one wake model whether a window of audio holds its phrase.
///
/// The same three steps the two speech calls take -- start the server if it is
/// not up, name the model the way the config names it, ask -- because a wake
/// model is configured and addressed exactly like a voice. The model's stored
/// options ride the request, so a `sherpa_kws` model's keyword rows and score
/// names reach the engine from its settings page.
pub async fn wake(
    selected: &families::Model,
    wav: &std::path::Path,
    threshold: f32,
    app: &tauri::AppHandle,
    database: &Database,
) -> Result<Vec<client::Detection>, String> {
    ensure_started_for(Purpose::Wake, app, database).await?;
    let path = selected.path.display().to_string();
    let options = options_with_threshold(
        settings::request_options(database, &path, &selected.family)
            .and_then(|value| value.as_object().cloned()),
        declares_threshold(&selected.family),
        threshold,
    );
    client::wake(
        Purpose::Wake,
        &model_for_request(&selected.slug(), Purpose::Wake),
        wav,
        options,
    )
    .await
}

/// Whether a family's spec takes a plain `threshold` at request time.
///
/// `sherpa_kws` does not: its scores are `keywords_threshold` and
/// `keywords_score`, set per model on its settings page. A name the family does
/// not declare is a request the server is entitled to refuse, so the page's
/// slider is only sent to the families that name it. No spec -- an engine that
/// is not installed -- keeps the old behaviour rather than silently dropping
/// the one option every shipped family understands.
fn declares_threshold(family: &str) -> bool {
    specs::for_family(family)
        .map(|spec| spec.request.iter().any(|option| option.name == "threshold"))
        .unwrap_or(true)
}

/// The request options one wake call carries.
///
/// Pure so the rule above is testable without a database or an installed
/// engine: a family that names `threshold` gets the page's value, one that
/// does not gets only what its settings page stored, and nothing at all is
/// `None` rather than an empty object.
fn options_with_threshold(
    stored: Option<serde_json::Map<String, serde_json::Value>>,
    takes_threshold: bool,
    threshold: f32,
) -> Option<serde_json::Value> {
    let mut options = stored.unwrap_or_default();
    if takes_threshold {
        options.insert("threshold".to_string(), serde_json::Value::from(threshold));
    }
    (!options.is_empty()).then(|| serde_json::Value::Object(options))
}

#[cfg(test)]
mod tests {
    use super::options_with_threshold;

    /// `sherpa_kws` names no `threshold`; its settings page carries the score
    /// names instead. Sending one it does not declare is the request the
    /// server would refuse.
    #[test]
    fn a_family_without_a_threshold_keeps_only_its_own_options() {
        let stored = serde_json::json!({ "keywords": "▁HE Y ▁J A R V I S" });
        let options = options_with_threshold(stored.as_object().cloned(), false, 0.5).unwrap();
        assert_eq!(options["keywords"], stored["keywords"]);
        assert!(options.get("threshold").is_none());
    }

    #[test]
    fn a_family_that_names_a_threshold_gets_the_page_value() {
        // The slider is an f32, so compare in the float it widens to rather
        // than against a literal that has a different binary spelling.
        let options = options_with_threshold(None, true, 0.4).unwrap();
        assert_eq!(options["threshold"].as_f64(), Some(0.4f32 as f64));
    }

    /// An empty object is not the same request as no options at all: the
    /// engine's own defaults answer when nothing is sent.
    #[test]
    fn nothing_stored_and_no_threshold_sends_nothing() {
        assert!(options_with_threshold(None, false, 0.4).is_none());
    }
}
