//! The wake word: a microphone held open, and a model asked about what it hears.
//!
//! # What this is not
//!
//! It is not a job, a run, or a conversation. Nothing here reaches the agent:
//! the only effect of a detection is an event the frontend can act on, because a
//! phrase someone said in the room is not an instruction until the app decides
//! what to do with it.
//!
//! # Why windows rather than a stream
//!
//! The engine's wake families report detections for a piece of audio they were
//! given (`GET`-free `POST /v1/tasks/run`, a WAV path, `segments` back), so the
//! microphone is cut into overlapping windows and each is asked about. Overlap
//! is half the window on purpose: a phrase spoken across a boundary is whole in
//! the window after it. Silence costs one request per second while the feature
//! is on, and the model answers it with an empty list.
//!
//! # Why the model is asked only while someone is listening
//!
//! The switch is the whole privacy story. Off means the microphone is closed,
//! not filtered, and the thread that held it is gone.

pub mod commands;
pub mod listen;
pub mod onnx;

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter};

use crate::audio::audiocpp::{self, families};
use crate::audio::stt::capture::SAMPLE_RATE;
use crate::audio::stt::whisper;
use crate::database::Database;

/// The settings key, beside `app.stt` and `app.tts`.
const KEY: &str = "app.wake";

/// How much audio one question carries. A wake phrase is under a second, so two
/// seconds is a window with room on both sides of it.
const WINDOW: Duration = Duration::from_secs(2);

/// How often the window is asked about: half its length, so the windows overlap.
const HOP: Duration = Duration::from_secs(1);

/// Silence after a detection. One utterance is one trigger, not the four or five
/// windows that contain it.
const COOLDOWN: Duration = Duration::from_secs(2);

/// The switch, the model, and how sure the model has to be.
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WakeSettings {
    #[serde(default)]
    pub enabled: bool,
    /// The chosen model's path, or none for the first wake model installed.
    #[serde(default)]
    pub model: Option<String>,
    /// The probability a detection has to clear. A model reports its own
    /// confidence, and this is the floor the app holds it to.
    #[serde(default = "default_threshold")]
    pub threshold: f32,
}

fn default_threshold() -> f32 {
    0.5
}

impl Default for WakeSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            model: None,
            threshold: default_threshold(),
        }
    }
}

pub fn settings(database: &Database) -> WakeSettings {
    database
        .setting(KEY)
        .and_then(|value| serde_json::from_value(value).ok())
        .unwrap_or_default()
}

pub fn save(database: &Database, settings: &WakeSettings) -> Result<(), String> {
    let value = serde_json::to_value(settings)
        .map_err(|error| format!("The wake word settings could not be stored: {error}"))?;
    database.set_setting(KEY, &value)
}

/// One installed wake model, as the page lists it.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelRow {
    pub path: String,
    pub name: String,
    /// The family id the server config names, so a reader can tell OpenWakeWord
    /// from sherpa's keyword spotting.
    pub family: String,
    /// The engine that runs this file. The page groups on it, because a pack
    /// and a head are two different engines behind one question.
    pub engine: &'static str,
    pub size: u64,
    /// Whether the family ships a spec, which is what its settings page draws.
    /// A head is one phrase with no options, so it has no edit door.
    pub editable: bool,
}

/// What the Wake word page is built from.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WakeStatus {
    pub enabled: bool,
    /// The microphone is open right now.
    pub running: bool,
    /// The model in force, or none when nothing is installed.
    pub model: Option<String>,
    pub threshold: f32,
    /// The wake models on disk, whoever put them there.
    pub models: Vec<ModelRow>,
    /// The audio.cpp engine, which every wake pack runs on.
    pub audio_engine_installed: bool,
    pub audio_engine_version: Option<String>,
    /// The ONNX engine, which runs the word heads.
    pub open_wake_word_installed: bool,
    pub open_wake_word_version: Option<String>,
}

/// The engine's own account of itself, for its terminal row.
///
/// The ONNX path is not a process, so there is no stdout to read: these lines are
/// what the panel shows instead. Capped, because a session that listens for hours
/// must not grow a buffer for hours.
static LOG: Mutex<Vec<String>> = Mutex::new(Vec::new());
const MAX_LOG_LINES: usize = 200;

fn log(app: &AppHandle, line: impl Into<String>) {
    let line = line.into();
    if let Ok(mut buffer) = LOG.lock() {
        buffer.push(line.clone());
        if buffer.len() > MAX_LOG_LINES {
            buffer.remove(0);
        }
    }
    let _ = app.emit(
        "local-model-log",
        serde_json::json!({ "modelId": "openwakeword", "line": line }),
    );
}

/// What the wake engine's terminal row starts from.
pub fn logs() -> Vec<String> {
    LOG.lock().map(|buffer| buffer.clone()).unwrap_or_default()
}

pub fn status(database: &Database) -> WakeStatus {
    let settings = settings(database);
    let mut models: Vec<ModelRow> = families::installed_for(families::Task::Wake)
        .into_iter()
        .map(|model| ModelRow {
            path: model.path.display().to_string(),
            name: model.name,
            // Asked before the family is moved into the row, because the spec
            // lookup is what decides whether the edit door exists.
            editable: audiocpp::specs::for_family(&model.family).is_some(),
            family: model.family,
            engine: "audio.cpp",
            size: model.size,
        })
        .collect();
    // Word heads are models too, and the page lists one kind of thing: the
    // engine name is the only thing that says which runtime runs it.
    models.extend(heads().into_iter().map(|(path, name)| ModelRow {
        size: std::fs::metadata(&path).map(|meta| meta.len()).unwrap_or(0),
        path: path.display().to_string(),
        name,
        family: "openWakeWord".to_string(),
        engine: "openWakeWord",
        editable: false,
    }));
    let model = match resolve(&settings) {
        Ok(Pack(model)) => Some(model.path.display().to_string()),
        Ok(Onnx { head, .. }) => Some(head.display().to_string()),
        Err(_) => None,
    };
    WakeStatus {
        enabled: settings.enabled,
        running: running(),
        model,
        threshold: settings.threshold,
        models,
        audio_engine_installed: crate::audio::engines::audiocpp::is_installed(),
        audio_engine_version: crate::audio::engines::audiocpp::installed_version(),
        open_wake_word_installed: crate::audio::engines::openwakeword::is_installed(),
        open_wake_word_version: crate::audio::engines::openwakeword::installed_version(),
    }
}

/// A word head installed beside the ONNX engine, as `(file, spoken name)`.
///
/// The name is the file stem with its separators opened out: the collection
/// names files for machines (`hey_jarvis`), and a reader is choosing a phrase.
fn heads() -> Vec<(std::path::PathBuf, String)> {
    let directory = crate::audio::engines::openwakeword::heads_dir();
    let Ok(entries) = std::fs::read_dir(&directory) else {
        return Vec::new();
    };
    let mut found: Vec<(std::path::PathBuf, String)> = entries
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| {
            path.extension()
                .and_then(|extension| extension.to_str())
                .is_some_and(|extension| extension.eq_ignore_ascii_case("onnx"))
        })
        .map(|path| {
            let name = path
                .file_stem()
                .and_then(|stem| stem.to_str())
                .unwrap_or("wake word")
                .replace(['_', '-'], " ");
            (path, name)
        })
        .collect();
    found.sort_by(|left, right| left.1.cmp(&right.1));
    found
}

/// The microphone, while one is open. There is exactly one, like the dictation
/// microphone, and the mutex is held only long enough to take or replace it.
static LISTENING: Mutex<Option<listen::Listening>> = Mutex::new(None);

pub fn running() -> bool {
    LISTENING.lock().map(|slot| slot.is_some()).unwrap_or(false)
}

/// The floor a detection has to clear, as a percentage, shared with the runner.
///
/// A slider produces a value on every pointer move, so a threshold that could
/// only take effect by restarting would close and reopen the microphone dozens
/// of times while someone drags it. The runner reads this before each window
/// instead, and a change lands on the next second of audio.
static THRESHOLD: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(50);

/// Stores a threshold for the running listener, and for the next one.
pub fn set_threshold(threshold: f32) {
    let percent = (threshold.clamp(0.0, 1.0) * 100.0).round() as u32;
    THRESHOLD.store(percent, std::sync::atomic::Ordering::Relaxed);
}

fn threshold_now() -> f32 {
    THRESHOLD.load(std::sync::atomic::Ordering::Relaxed) as f32 / 100.0
}

/// What a detection comes from.
///
/// Two engines answer the same question with the same interface, so the loop
/// below does not care which one it is holding: audio.cpp's own `wake` families,
/// or an openWakeWord head run through ONNX Runtime.
#[derive(Clone)]
enum Source {
    Pack(families::Model),
    Onnx {
        head: std::path::PathBuf,
        name: String,
    },
}

use Source::{Onnx, Pack};

/// The model a detection should come from.
///
/// The stored path is the answer while it is still on disk; a deleted model is
/// not worth an error when another one is installed, so the first falls back —
/// the same rule `stt::models::resolve` uses for recognition. A word head wins
/// that fallback, because a head is what the wake page offers first now.
fn resolve(settings: &WakeSettings) -> Result<Source, String> {
    let installed = heads();
    if let Some(chosen) = settings.model.as_deref() {
        if let Some((head, name)) = installed
            .iter()
            .find(|(path, _)| path.display().to_string() == chosen)
        {
            return Ok(Onnx {
                head: head.clone(),
                name: name.clone(),
            });
        }
    }
    if let Some((head, name)) = installed.into_iter().next() {
        if settings.model.is_none() {
            return Ok(Onnx { head, name });
        }
    }
    let packs = families::installed_for(families::Task::Wake);
    if let Some(chosen) = settings.model.as_deref() {
        if let Some(found) = packs
            .iter()
            .find(|model| model.path.display().to_string() == chosen)
        {
            return Ok(Pack(found.clone()));
        }
    }
    packs.into_iter().next().map(Pack).ok_or_else(|| {
        "No wake word model is installed. Add one from the Wake word page.".to_string()
    })
}

/// Opens the microphone and starts asking the model about it.
///
/// The server is started first, so a failure -- no engine, an unknown family --
/// is reported at the switch rather than silently at the first phrase.
pub async fn start(app: &AppHandle, database: Arc<Database>) -> Result<(), String> {
    if running() {
        return Ok(());
    }
    let settings = settings(&database);
    let engine = resolve(&settings)?;
    // The ONNX path runs the graphs in this process, so there is no server to
    // start; the pack path asks audio.cpp, and a failure there is reported at the
    // switch rather than at the first phrase.
    let recognizer = match &engine {
        Onnx { head, .. } => Some((
            onnx::Recognizer::open(
                &crate::audio::engines::openwakeword::runtime(),
                &crate::audio::engines::openwakeword::mel(),
                &crate::audio::engines::openwakeword::embedding(),
                head,
            )?,
            match &engine {
                Onnx { name, .. } => name.clone(),
                Pack(_) => String::new(),
            },
        )),
        Pack(_) => {
            // Asked for as the wake word, so turning the switch off can stop the
            // server again without disturbing a voice using it for another reason.
            audiocpp::ensure_started_for(audiocpp::Purpose::Wake, app, &database).await?;
            None
        }
    };

    // Which endpoint to listen on is the same choice dictation records from:
    // one microphone for the app, made once on the System page.
    let device = crate::audio::stt::models::settings(&database).device;
    let (sink, source) = tokio::sync::mpsc::channel(64);
    let listening = listen::start(device, sink)?;
    match LISTENING.lock() {
        Ok(mut slot) => *slot = Some(listening),
        Err(_) => return Err("The microphone is in an unusable state.".to_string()),
    }

    set_threshold(settings.threshold);
    // The label is what the row is called: the word itself for a head, the
    // family for a pack.
    let label = match &recognizer {
        Some((_, name)) => name.clone(),
        None => match &engine {
            Pack(model) => model.name.clone(),
            Onnx { name, .. } => name.clone(),
        },
    };
    // The panel needs to know which engine answered: an audio.cpp pack is a
    // process the audio server already reports, so only the ONNX path has no
    // other row. Without this the same pack appeared twice -- once as
    // `audio.cpp · <family>` and once as `wake · <family>`.
    let engine_name = match &engine {
        Onnx { .. } => "openwakeword",
        Pack(_) => "audiocpp",
    };
    log(app, format!("{label} · listening"));
    // The panel opens a row when an engine starts. The ONNX path has no process,
    // so this event is the only thing that can tell it one started.
    let _ = app.emit(
        "wake-state",
        serde_json::json!({ "running": true, "model": label, "engine": engine_name }),
    );
    let app = app.clone();
    tauri::async_runtime::spawn(run(app, database, source, engine, recognizer));
    Ok(())
}

/// Closes the microphone. The runner ends on its own when the audio stops.
pub fn stop() {
    if let Ok(mut slot) = LISTENING.lock() {
        slot.take();
    }
}

/// Restarts only if it is already listening -- what a change of model means
/// while the switch is on. A change of threshold does not come through here: the
/// runner reads that value per window.
pub async fn restart(app: &AppHandle, database: Arc<Database>) -> Result<(), String> {
    if !running() {
        return Ok(());
    }
    stop();
    start(app, database).await
}

/// Asks the model about one window after another, for as long as audio arrives.
async fn run(
    app: AppHandle,
    database: Arc<Database>,
    mut source: tokio::sync::mpsc::Receiver<Vec<i16>>,
    engine: Source,
    mut recognizer: Option<(onnx::Recognizer, String)>,
) {
    let window = SAMPLE_RATE as usize * WINDOW.as_secs() as usize;
    let hop = SAMPLE_RATE as usize * HOP.as_secs() as usize;
    let mut buffer: Vec<i16> = Vec::with_capacity(window * 2);
    let mut heard = 0usize;
    let mut quiet_until = Instant::now();

    while let Some(chunk) = source.recv().await {
        // The switch can be thrown while a window is in flight; once the
        // listener is gone there is nobody to answer, and asking again would
        // bring back the server that was just stopped.
        if !running() {
            break;
        }
        buffer.extend_from_slice(&chunk);
        // Keep one window plus one hop: the newest audio is the only audio the
        // next question needs, and an unbounded buffer would grow all session.
        if buffer.len() > window * 2 {
            let excess = buffer.len() - window;
            buffer.drain(0..excess);
        }
        heard += chunk.len();
        if heard < hop || buffer.len() < window {
            continue;
        }
        heard = 0;
        if Instant::now() < quiet_until {
            continue;
        }

        let slice = buffer[buffer.len() - window..].to_vec();
        // Kept for the voice check as well as the ask: the second gate reads
        // exactly the audio the first gate fired on.
        let audio = slice.clone();

        // Both engines answer the same shape -- a phrase and its score, or
        // nothing. The ONNX path runs here; the pack path is a request to a
        // server that may not even be up.
        let fired: Option<(String, f32)> = match recognizer.as_mut() {
            Some((model, name)) => {
                // Silence scores near zero anyway, but running three networks over
                // a silent room every second is what kept a listening app warm.
                // This is a floor, not a voice detector: it only skips the ask.
                if !audible(&audio) {
                    None
                } else {
                    match model.score(&audio) {
                        Ok(score) if score >= threshold_now() => Some((name.clone(), score)),
                        Ok(_) => None,
                        Err(message) => {
                            eprintln!("wake: {message}");
                            None
                        }
                    }
                }
            }
            None => match &engine {
                Pack(model) => {
                    let Ok(Ok(wav)) =
                        tokio::task::spawn_blocking(move || whisper::write_wav(&slice)).await
                    else {
                        continue;
                    };
                    let outcome =
                        audiocpp::wake(model, &wav, threshold_now(), &app, &database).await;
                    let _ = std::fs::remove_file(&wav);
                    match outcome {
                        Ok(detections) => detections
                            .into_iter()
                            .max_by(|left, right| left.confidence.total_cmp(&right.confidence))
                            .map(|best| (best.text, best.confidence)),
                        Err(message) => {
                            // A refused window is not worth stopping the
                            // microphone over: the next one may be fine.
                            eprintln!("wake: {message}");
                            None
                        }
                    }
                }
                Onnx { .. } => None,
            },
        };
        let Some((text, confidence)) = fired else {
            continue;
        };

        // The second gate: who said it. Off when nothing is enrolled or the
        // switch is off, in which case the detection stands. A voice that is not
        // recognised is dropped rather than announced -- the switch exists so
        // this microphone answers to one person.
        let speaker = match crate::audio::voice::identify(&database, &audio) {
            Ok(crate::audio::voice::Heard::Off) => None,
            Ok(crate::audio::voice::Heard::Known { name, score }) => {
                eprintln!("voice: matched {name} at {score:.2}");
                Some(name)
            }
            Ok(crate::audio::voice::Heard::Unknown { best }) => {
                eprintln!("voice: ignored, best score {best:.2}");
                quiet_until = Instant::now() + COOLDOWN;
                buffer.clear();
                continue;
            }
            Err(message) => {
                // A broken engine is not an open door: say why, and do not fire.
                eprintln!("voice: {message}");
                quiet_until = Instant::now() + COOLDOWN;
                buffer.clear();
                continue;
            }
        };

        quiet_until = Instant::now() + COOLDOWN;
        buffer.clear();
        log(&app, format!("heard \"{text}\" at {confidence:.2}"));
        let _ = app.emit(
            "wake-detected",
            serde_json::json!({
                "text": text,
                "confidence": confidence,
                "speaker": speaker,
            }),
        );
    }
}

/// Whether a window holds anything worth asking a model about.
///
/// About -44 dBFS RMS: below any speech, above a quiet room's noise floor. It
/// exists so an idle microphone costs nothing, and it is a floor rather than a
/// detector, because the model is the authority on what it heard.
fn audible(samples: &[i16]) -> bool {
    if samples.is_empty() {
        return false;
    }
    let energy: f64 = samples
        .iter()
        .map(|sample| (*sample as f64) * (*sample as f64))
        .sum();
    (energy / samples.len() as f64).sqrt() > 200.0
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A record written before the threshold existed still loads, and a record
    /// with none set is the default rather than zero -- zero would fire on
    /// everything the model so much as suspected.
    #[test]
    fn an_older_record_loads_with_the_default_threshold() {
        let settings: WakeSettings = serde_json::from_str(r#"{"enabled":true}"#).unwrap();
        assert!(settings.enabled);
        assert_eq!(settings.threshold, default_threshold());
        assert_eq!(settings.model, None);
    }
}
