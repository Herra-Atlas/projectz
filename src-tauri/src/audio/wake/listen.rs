//! The microphone, held open.
//!
//! The dictation microphone in `stt::capture` is opened for one recording and
//! closed the moment the user stops; a wake word needs the opposite -- a stream
//! that stays open for as long as the feature is switched on. So the device is
//! opened on a thread that outlives any one recording, and every buffer it
//! delivers is handed on as raw 16 kHz mono.
//!
//! # Why the callbacks never wait
//!
//! A callback that blocks is a callback that glitches the stream, so the queue
//! is `try_send` rather than `send`: when the reader falls behind, the window
//! that is dropped is one the model was told to ignore anyway. Nothing in the
//! callback allocates beyond the conversion itself.
//!
//! # Why the device rules are borrowed
//!
//! Which endpoint to open, and what sample rate to ask it for, are the same
//! questions the dictation microphone answers -- including the rule that a
//! chosen name which is no longer connected fails by name. They live in
//! `stt::capture` and are reused rather than copied, so two microphones cannot
//! drift apart in which device they open.

use std::sync::mpsc;
use std::time::Duration;

use cpal::traits::{DeviceTrait, StreamTrait};
use cpal::SampleFormat;

use crate::audio::stt::capture::{open_device, pick_config, SAMPLE_RATE};

/// A stream that is open until this value is dropped.
///
/// The stop signal is the channel itself: the listening thread blocks on a
/// receive that answers `Err` when the last sender goes, and that is when it
/// drops the stream on the thread that opened it.
pub struct Listening {
    /// Never sent on; dropping it closes the channel, which is the instruction
    /// the listening thread is waiting for.
    _stop: mpsc::Sender<()>,
}

/// How long a device is given to reach `play()`.
///
/// Generous on purpose. A virtual endpoint -- WO Mic with no client connected, a
/// cable with nothing feeding it, a Bluetooth headset that has gone to sleep --
/// does not refuse the open, it simply never finishes. Five seconds reported
/// that as "the microphone did not start", which is true and useless.
const OPEN_TIMEOUT: Duration = Duration::from_secs(15);

/// How far the open got, so a timeout can name the step that never returned.
///
/// "Did not start" is true of a device that refused, one that hung and one whose
/// thread died, and the three need different answers. This is written by the
/// audio thread and read by the caller when the budget runs out.
static STAGE: std::sync::atomic::AtomicU8 = std::sync::atomic::AtomicU8::new(0);

fn stage(step: u8) {
    STAGE.store(step, std::sync::atomic::Ordering::Relaxed);
}

/// The step the open was on when it stopped answering.
pub fn stage_name() -> &'static str {
    match STAGE.load(std::sync::atomic::Ordering::Relaxed) {
        0 => "looking for the audio host",
        1 => "opening the endpoint",
        2 => "reading the endpoint's formats",
        3 => "building the stream",
        4 => "starting the stream",
        _ => "waiting for the first buffer",
    }
}

/// Opens the microphone and starts delivering audio to `sink`.
///
/// Returns once the stream is actually running, so a device that cannot be
/// opened fails here -- at the switch the user just pressed -- rather than
/// leaving a wake word that silently never fires.
pub fn start(
    device: Option<String>,
    sink: tokio::sync::mpsc::Sender<Vec<i16>>,
) -> Result<Listening, String> {
    let label = device
        .clone()
        .unwrap_or_else(|| "The system default microphone".to_string());
    stage(0);
    let (stop_tx, stop_rx) = mpsc::channel();
    let (ready_tx, ready_rx) = mpsc::channel();
    // The signal has to come from *inside* the open, on the thread that is doing
    // it: `listen` blocks on the stop channel after the stream starts, so a
    // thread that only reported on return never reported at all -- and every
    // device, good or bad, timed out. The clone is what reports a failure.
    let failure_tx = ready_tx.clone();
    std::thread::spawn(move || {
        let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            listen(stop_rx, ready_tx, sink, device)
        }));
        let failure = match outcome {
            // Already reported by `listen`; the thread is ending normally.
            Ok(Ok(())) => return,
            Ok(Err(error)) => error,
            Err(_) => "The audio thread panicked while opening it.".to_string(),
        };
        let _ = failure_tx.send(Err(failure));
    });
    ready_rx
        .recv_timeout(OPEN_TIMEOUT)
        .map_err(|_| {
            format!(
                "{label} did not start within {} seconds — it was stuck at: {}. A virtual microphone needs its app running; a real one may be held exclusively by another program.",
                OPEN_TIMEOUT.as_secs(),
                stage_name()
            )
        })??;
    Ok(Listening { _stop: stop_tx })
}

fn listen(
    stop: mpsc::Receiver<()>,
    ready: mpsc::Sender<Result<(), String>>,
    sink: tokio::sync::mpsc::Sender<Vec<i16>>,
    name: Option<String>,
) -> Result<(), String> {
    let host = cpal::default_host();
    stage(1);
    let device = open_device(&host, name.as_deref())?;
    stage(2);
    let (config, format) = pick_config(&device)?;
    stage(3);
    let channels = config.channels as usize;
    let mut resampler = Resampler::new(config.sample_rate.0, SAMPLE_RATE);
    let failed = |error| eprintln!("wake microphone: {error}");

    // One `resampler` per arm: the arms are exclusive, so each may take it.
    let stream = match format {
        SampleFormat::F32 => device.build_input_stream(
            &config,
            move |data: &[f32], _| deliver(&sink, &mut resampler, mono_f32(data, channels)),
            failed,
            None,
        ),
        SampleFormat::I16 => device.build_input_stream(
            &config,
            move |data: &[i16], _| {
                deliver(
                    &sink,
                    &mut resampler,
                    mono(data, channels, |value| value as f32 / 32768.0),
                )
            },
            failed,
            None,
        ),
        SampleFormat::U16 => device.build_input_stream(
            &config,
            move |data: &[u16], _| {
                deliver(
                    &sink,
                    &mut resampler,
                    mono(data, channels, |value| (value as f32 - 32768.0) / 32768.0),
                )
            },
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

    stage(4);
    stream
        .play()
        .map_err(|error| format!("The microphone could not start: {error}"))?;
    stage(5);
    let _ = ready.send(Ok(()));

    // Blocks until the owner drops the listener; the stream fills the queue on
    // the driver's thread meanwhile.
    let _ = stop.recv();
    drop(stream);
    Ok(())
}

fn deliver(
    sink: &tokio::sync::mpsc::Sender<Vec<i16>>,
    resampler: &mut Resampler,
    values: Vec<f32>,
) {
    let samples = resampler.push(&values);
    if !samples.is_empty() {
        let _ = sink.try_send(samples);
    }
}

/// Frames to one channel by averaging, then to `f32`.
fn mono<T: Copy>(data: &[T], channels: usize, to_float: impl Fn(T) -> f32) -> Vec<f32> {
    if channels <= 1 {
        return data.iter().map(|value| to_float(*value)).collect();
    }
    data.chunks_exact(channels)
        .map(|frame| frame.iter().map(|value| to_float(*value)).sum::<f32>() / channels as f32)
        .collect()
}

fn mono_f32(data: &[f32], channels: usize) -> Vec<f32> {
    mono(data, channels, |value| value)
}

/// Linear resampling that survives chunk boundaries.
///
/// A wake window is cut from a continuous stream, so resampling each buffer on
/// its own would put a seam at every callback -- and the seam is exactly the
/// kind of click a detector reads as the start of a phrase. One sample and one
/// fractional position are carried between chunks, so the stream is resampled
/// as a stream.
struct Resampler {
    ratio: f64,
    previous: f32,
    position: f64,
}

impl Resampler {
    fn new(from: u32, to: u32) -> Self {
        Self {
            ratio: from as f64 / to as f64,
            previous: 0.0,
            position: 0.0,
        }
    }

    fn push(&mut self, values: &[f32]) -> Vec<i16> {
        if values.is_empty() {
            return Vec::new();
        }
        if (self.ratio - 1.0).abs() < f64::EPSILON {
            return values.iter().map(|value| to_i16(*value)).collect();
        }
        // Index 0 of this coordinate is the previous chunk's last sample, and
        // index i is `values[i - 1]`, so the two chunks meet without a gap.
        let length = values.len() as f64;
        let mut out = Vec::with_capacity((length / self.ratio) as usize + 1);
        while self.position < length {
            let left = self.position.floor() as usize;
            let fraction = (self.position - left as f64) as f32;
            let a = if left == 0 {
                self.previous
            } else {
                values[left - 1]
            };
            let b = values.get(left).copied().unwrap_or(a);
            out.push(to_i16(a + (b - a) * fraction));
            self.position += self.ratio;
        }
        self.position -= length;
        self.previous = values[values.len() - 1];
        out
    }
}

fn to_i16(value: f32) -> i16 {
    (value.clamp(-1.0, 1.0) * 32767.0) as i16
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Opens the default microphone for real. Ignored because it needs hardware
    /// and takes the device away from the app; run it by hand when a device
    /// refuses to open, and the failure names the step it stopped at.
    ///
    ///     cargo test --lib the_default_microphone_starts -- --ignored --nocapture
    #[test]
    #[ignore = "opens the default microphone"]
    fn the_default_microphone_starts() {
        let (sink, mut source) = tokio::sync::mpsc::channel(8);
        let listening =
            start(None, sink).unwrap_or_else(|error| panic!("the open failed: {error}"));
        // A stream that reports itself running but never delivers is the other
        // half of this bug, so a buffer is waited for rather than assumed.
        let deadline = std::time::Instant::now() + Duration::from_secs(3);
        let mut received = false;
        while std::time::Instant::now() < deadline && !received {
            received = source.blocking_recv().is_some();
        }
        drop(listening);
        assert!(received, "the stream started but delivered no audio");
    }

    #[test]
    fn a_resampler_keeps_its_phase_across_chunks() {
        // One second of a slow ramp, delivered in two chunks, must come out at
        // 16 kHz without a step at the join -- the seam a detector would read as
        // the start of a phrase.
        let mut resampler = Resampler::new(48_000, SAMPLE_RATE);
        let ramp: Vec<f32> = (0..48_000).map(|index| index as f32 / 48_000.0).collect();
        let mut out = resampler.push(&ramp[..24_000]);
        out.extend(resampler.push(&ramp[24_000..]));
        assert!(
            (out.len() as i64 - 16_000).abs() <= 2,
            "got {} samples",
            out.len()
        );
        let steps: Vec<i32> = out
            .windows(2)
            .map(|pair| (pair[1] as i32 - pair[0] as i32).abs())
            .collect();
        let largest = *steps.iter().max().unwrap_or(&0);
        // A 1/16000 step of a 1.0 ramp is about 2 in i16; a seam would be far more.
        assert!(largest <= 4, "a step of {largest} at a chunk boundary");
    }
}
