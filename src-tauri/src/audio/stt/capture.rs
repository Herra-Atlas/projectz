//! The microphone.
//!
//! # Why a thread per recording
//!
//! `cpal`'s stream is a device handle whose lifetime is tied to the thread that
//! opened it on some backends, and its callbacks run on a thread the audio driver
//! owns. Holding the stream in shared state would mean the app's other threads
//! deciding when it is dropped. Instead one thread opens the device, waits for a
//! stop signal, drops the stream and hands back the samples -- so the device is
//! released exactly once, by the thread that took it.
//!
//! # Why 16 kHz mono
//!
//! That is what Whisper is trained on. Asking the device for it first avoids
//! resampling entirely on most microphones; when the device cannot do it, the
//! samples are converted here rather than by the model, which would be slower and
//! worse.
//!
//! Nothing here allocates per callback beyond the shared buffer's growth, which is
//! the one thing an audio callback must not do: a long `Vec` push is a pointer
//! bump, and the reserve is done before the stream starts.
//!
//! # Why a name and not an index
//!
//! The chosen endpoint is stored as its name because the list renumbers as
//! devices come and go: an index saved on Tuesday opens a different microphone
//! once a headset is unplugged. The name is resolved against the live endpoints
//! at every press, so a choice whose device is gone fails by name instead of
//! recording from whatever the system default has become.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::SampleFormat;
use tauri::{AppHandle, Emitter};

/// What the engine is told the audio is.
pub const SAMPLE_RATE: u32 = 16_000;

/// Seconds of audio at 16 kHz that count as "something was said". Below this a
/// recording is a stray click on the button, not speech.
pub const MIN_SAMPLES: usize = (SAMPLE_RATE as usize) / 4;

/// How often the recording looks at what it has heard.
const POLL: Duration = Duration::from_millis(100);

/// The level on the `-1..1` scale that counts as speech rather than room.
const SPEECH_LEVEL: f32 = 0.02;

/// How much quiet ends a recording, once speech has been heard.
///
/// A wake word starts the microphone without a second press, so the recording
/// has to be able to end itself; without this it waits for the end button
/// forever, which is exactly what it used to do.
const SILENCE_LIMIT: Duration = Duration::from_millis(1500);

/// A recording that hears nothing at all ends here, so a wake word that fired on
/// a noise cannot leave the microphone open.
const IDLE_LIMIT: Duration = Duration::from_secs(10);

/// The longest one recording may run, whatever it hears.
const MAX_RECORDING: Duration = Duration::from_secs(120);

/// A recording in progress.
pub struct Session {
    stop: mpsc::Sender<()>,
    audio: mpsc::Receiver<Result<Vec<i16>, String>>,
    ended: Arc<AtomicBool>,
}

impl Session {
    /// Whether the recording has already ended by itself (silence, or the cap).
    ///
    /// A caller that started a recording with no button behind it -- enrolling a
    /// voice is the one -- waits on this instead of guessing a duration, so a
    /// two-second sentence returns in two seconds rather than in the cap.
    pub fn ended(&self) -> bool {
        self.ended.load(Ordering::Relaxed)
    }

    /// Ends the recording and returns what was heard.
    pub fn finish(self) -> Result<Vec<i16>, String> {
        let _ = self.stop.send(());
        self.audio
            .recv_timeout(Duration::from_secs(10))
            .map_err(|_| "The microphone did not stop.".to_string())?
    }
}

/// Every capture endpoint this machine offers, and the one the system would use
/// with nothing chosen.
///
/// Sorted and deduplicated because a name is what gets stored: two rows carrying
/// one name would be two rows that choose the same device.
pub fn inputs() -> (Vec<String>, Option<String>) {
    let host = cpal::default_host();
    let default = host
        .default_input_device()
        .and_then(|device| device.name().ok());
    let mut names: Vec<String> = host
        .input_devices()
        .map(|devices| devices.filter_map(|device| device.name().ok()).collect())
        .unwrap_or_default();
    names.sort();
    names.dedup();
    (names, default)
}

/// Opens the microphone and starts recording.
///
/// `device` names the endpoint to open, or is `None` for the system default.
/// Returns once the stream is actually running, so a device that cannot be opened
/// fails here rather than on the user's first sentence.
/// `app` is how the recorder tells the composer it ended by itself. A caller
/// with no composer -- voice enrollment is the one -- passes `None`, and the
/// recording stays silent about itself rather than starting a dictation.
pub fn start(device: Option<&str>, app: Option<AppHandle>) -> Result<Session, String> {
    let device = device.map(str::to_string);
    let (stop_tx, stop_rx) = mpsc::channel();
    let (audio_tx, audio_rx) = mpsc::channel();
    let (ready_tx, ready_rx) = mpsc::channel();
    let ended = Arc::new(AtomicBool::new(false));
    let flag = Arc::clone(&ended);
    std::thread::spawn(move || {
        let _ = audio_tx.send(record(stop_rx, ready_tx, device, app, flag));
    });
    ready_rx
        .recv_timeout(Duration::from_secs(5))
        .map_err(|_| "The microphone did not start.".to_string())??;
    Ok(Session {
        stop: stop_tx,
        audio: audio_rx,
        ended,
    })
}

fn record(
    stop: mpsc::Receiver<()>,
    ready: mpsc::Sender<Result<(), String>>,
    name: Option<String>,
    app: Option<AppHandle>,
    ended: Arc<AtomicBool>,
) -> Result<Vec<i16>, String> {
    let host = cpal::default_host();
    let device = open_device(&host, name.as_deref())?;
    let (config, format) = pick_config(&device)?;
    let channels = config.channels as usize;
    let rate = config.sample_rate.0;

    let collected: Arc<Mutex<Vec<f32>>> =
        Arc::new(Mutex::new(Vec::with_capacity(rate as usize * 30)));
    let sink = Arc::clone(&collected);
    let failed = |error| eprintln!("microphone: {error}");

    let stream = match format {
        SampleFormat::F32 => device.build_input_stream(
            &config,
            move |data: &[f32], _| push(&sink, data, channels),
            failed,
            None,
        ),
        SampleFormat::I16 => device.build_input_stream(
            &config,
            move |data: &[i16], _| push_i16(&sink, data, channels),
            failed,
            None,
        ),
        SampleFormat::U16 => device.build_input_stream(
            &config,
            move |data: &[u16], _| push_u16(&sink, data, channels),
            failed,
            None,
        ),
        other => {
            return Err(format!(
                "This microphone reports {other}, which the app cannot read."
            ))
        }
    }
    .map_err(|error| format!("The microphone could not be opened: {error}"))?;

    stream
        .play()
        .map_err(|error| format!("The microphone could not start: {error}"))?;
    let _ = ready.send(Ok(()));

    // The stream keeps filling the buffer on the driver's own thread while this
    // one watches it: a recording ends on the stop command, or on its own when
    // the speech that started it has finished.
    let started = Instant::now();
    let window = (rate as usize / 10).max(1);
    let mut speech_heard = false;
    let mut quiet_for = Duration::ZERO;
    let mut ended_by_itself = false;
    loop {
        match stop.recv_timeout(POLL) {
            Ok(()) | Err(RecvTimeoutError::Disconnected) => break,
            Err(RecvTimeoutError::Timeout) => {}
        }
        let level = recent_level(&collected, window);
        if level >= SPEECH_LEVEL {
            speech_heard = true;
            quiet_for = Duration::ZERO;
        } else {
            quiet_for += POLL;
        }
        let elapsed = started.elapsed();
        // Before any speech, quiet means "not yet"; after it, quiet means done.
        let finished = if speech_heard {
            quiet_for >= SILENCE_LIMIT
        } else {
            elapsed >= IDLE_LIMIT
        };
        if finished || elapsed >= MAX_RECORDING {
            ended_by_itself = true;
            break;
        }
    }
    drop(stream);
    // Before the samples are moved, so a caller polling `ended` never waits on
    // a recording that is already over.
    ended.store(true, Ordering::Relaxed);
    if ended_by_itself {
        // The composer is holding a button that says "recording"; it has to be
        // told to finish, or it waits for a press that is never coming.
        if let Some(app) = &app {
            let _ = app.emit("stt-recording-auto-stopped", ());
        }
    }

    let recorded = collected
        .lock()
        .map(|values| values.clone())
        .unwrap_or_default();
    Ok(resample(&recorded, rate, SAMPLE_RATE))
}

/// The loudness of the newest slice of what has been recorded.
///
/// The root mean square over about a tenth of a second: speech clears
/// `SPEECH_LEVEL` easily and a room does not, which is all the decision needs.
/// Cheap on purpose -- it is read every 100 ms while a recording runs.
fn recent_level(collected: &Arc<Mutex<Vec<f32>>>, window: usize) -> f32 {
    let Ok(values) = collected.lock() else {
        return 0.0;
    };
    let from = values.len().saturating_sub(window);
    let slice = &values[from..];
    if slice.is_empty() {
        return 0.0;
    }
    (slice.iter().map(|value| value * value).sum::<f32>() / slice.len() as f32).sqrt()
}

/// The endpoint a recording should open.
///
/// A chosen name is looked up among the live endpoints rather than trusted: a
/// microphone unplugged since it was chosen must fail by name here, at the press,
/// instead of silently recording from whatever Windows made the default.
pub(crate) fn open_device(host: &cpal::Host, name: Option<&str>) -> Result<cpal::Device, String> {
    let Some(name) = name else {
        return host
            .default_input_device()
            .ok_or_else(|| "No microphone was found.".to_string());
    };
    let mut devices = host
        .input_devices()
        .map_err(|error| format!("The microphones could not be listed: {error}"))?;
    devices
        .find(|device| device.name().ok().as_deref() == Some(name))
        .ok_or_else(|| format!("The microphone \"{name}\" is not connected."))
}

/// Prefers a configuration at 16 kHz, and falls back to whatever the device
/// offers so a microphone that cannot do it still records.
///
/// The sample format comes back with the configuration because it is a property
/// of the range, not of the stream: the caller has to dispatch on it to build the
/// right callback.
pub(crate) fn pick_config(
    device: &cpal::Device,
) -> Result<(cpal::StreamConfig, SampleFormat), String> {
    if let Ok(ranges) = device.supported_input_configs() {
        for range in ranges {
            let (low, high) = (range.min_sample_rate().0, range.max_sample_rate().0);
            if low <= SAMPLE_RATE && SAMPLE_RATE <= high {
                let format = range.sample_format();
                let config = range.with_sample_rate(cpal::SampleRate(SAMPLE_RATE));
                return Ok((config.into(), format));
            }
        }
    }
    let default = device
        .default_input_config()
        .map_err(|error| format!("The microphone has no usable input format: {error}"))?;
    let format = default.sample_format();
    Ok((default.into(), format))
}

fn push(sink: &Arc<Mutex<Vec<f32>>>, data: &[f32], channels: usize) {
    if let Ok(mut values) = sink.lock() {
        values.extend(mix(data, channels, |value| value));
    }
}

fn push_i16(sink: &Arc<Mutex<Vec<f32>>>, data: &[i16], channels: usize) {
    if let Ok(mut values) = sink.lock() {
        values.extend(mix(data, channels, |value| value as f32 / 32768.0));
    }
}

fn push_u16(sink: &Arc<Mutex<Vec<f32>>>, data: &[u16], channels: usize) {
    if let Ok(mut values) = sink.lock() {
        values.extend(mix(data, channels, |value| {
            (value as f32 - 32768.0) / 32768.0
        }));
    }
}

/// Frames to one channel by averaging, so a stereo microphone does not halve the
/// length of everything downstream.
fn mix<T: Copy>(data: &[T], channels: usize, to_float: impl Fn(T) -> f32) -> Vec<f32> {
    if channels <= 1 {
        return data.iter().map(|value| to_float(*value)).collect();
    }
    data.chunks_exact(channels)
        .map(|frame| frame.iter().map(|value| to_float(*value)).sum::<f32>() / channels as f32)
        .collect()
}

/// Linear resampling to the engine's rate.
///
/// Speech does not need a windowed filter, and the alternative -- asking Whisper
/// to cope -- is not an option: it assumes 16 kHz and would read everything at the
/// wrong speed.
fn resample(samples: &[f32], from: u32, to: u32) -> Vec<i16> {
    if samples.is_empty() {
        return Vec::new();
    }
    let ratio = from as f64 / to as f64;
    let length = (samples.len() as f64 / ratio).floor() as usize;
    let mut out = Vec::with_capacity(length);
    for index in 0..length {
        let position = index as f64 * ratio;
        let left = position.floor() as usize;
        let right = (left + 1).min(samples.len() - 1);
        let weight = (position - left as f64) as f32;
        let value = samples[left] * (1.0 - weight) + samples[right] * weight;
        out.push((value.clamp(-1.0, 1.0) * 32767.0) as i16);
    }
    out
}
