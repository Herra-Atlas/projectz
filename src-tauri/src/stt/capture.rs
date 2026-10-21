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

use std::sync::{mpsc, Arc, Mutex};
use std::time::Duration;

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::SampleFormat;

/// What the engine is told the audio is.
pub const SAMPLE_RATE: u32 = 16_000;

/// Seconds of audio at 16 kHz that count as "something was said". Below this a
/// recording is a stray click on the button, not speech.
pub const MIN_SAMPLES: usize = (SAMPLE_RATE as usize) / 4;

/// A recording in progress.
pub struct Session {
    stop: mpsc::Sender<()>,
    audio: mpsc::Receiver<Result<Vec<i16>, String>>,
}

impl Session {
    /// Ends the recording and returns what was heard.
    pub fn finish(self) -> Result<Vec<i16>, String> {
        let _ = self.stop.send(());
        self.audio
            .recv_timeout(Duration::from_secs(10))
            .map_err(|_| "The microphone did not stop.".to_string())?
    }
}

/// Opens the microphone and starts recording.
///
/// Returns once the stream is actually running, so a device that cannot be opened
/// fails here rather than on the user's first sentence.
pub fn start() -> Result<Session, String> {
    let (stop_tx, stop_rx) = mpsc::channel();
    let (audio_tx, audio_rx) = mpsc::channel();
    let (ready_tx, ready_rx) = mpsc::channel();
    std::thread::spawn(move || {
        let _ = audio_tx.send(record(stop_rx, ready_tx));
    });
    ready_rx
        .recv_timeout(Duration::from_secs(5))
        .map_err(|_| "The microphone did not start.".to_string())??;
    Ok(Session {
        stop: stop_tx,
        audio: audio_rx,
    })
}

fn record(stop: mpsc::Receiver<()>, ready: mpsc::Sender<Result<(), String>>) -> Result<Vec<i16>, String> {
    let host = cpal::default_host();
    let device = host
        .default_input_device()
        .ok_or_else(|| "No microphone was found.".to_string())?;
    let (config, format) = pick_config(&device)?;
    let channels = config.channels as usize;
    let rate = config.sample_rate.0;

    let collected: Arc<Mutex<Vec<f32>>> = Arc::new(Mutex::new(Vec::with_capacity(rate as usize * 30)));
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
        other => return Err(format!("This microphone reports {other}, which the app cannot read.")),
    }
    .map_err(|error| format!("The microphone could not be opened: {error}"))?;

    stream
        .play()
        .map_err(|error| format!("The microphone could not start: {error}"))?;
    let _ = ready.send(Ok(()));

    // Blocks until the command asks to stop; the stream keeps filling the buffer
    // on the driver's own thread meanwhile.
    let _ = stop.recv();
    drop(stream);

    let recorded = collected
        .lock()
        .map(|values| values.clone())
        .unwrap_or_default();
    Ok(resample(&recorded, rate, SAMPLE_RATE))
}

/// Prefers a configuration at 16 kHz, and falls back to whatever the device
/// offers so a microphone that cannot do it still records.
///
/// The sample format comes back with the configuration because it is a property
/// of the range, not of the stream: the caller has to dispatch on it to build the
/// right callback.
fn pick_config(device: &cpal::Device) -> Result<(cpal::StreamConfig, SampleFormat), String> {
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
        values.extend(mix(data, channels, |value| (value as f32 - 32768.0) / 32768.0));
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
