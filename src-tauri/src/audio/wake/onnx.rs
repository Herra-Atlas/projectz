//! openWakeWord's three graphs, run in-process.
//!
//! The math is not invented here: it is the published pipeline, checked against
//! the official Alexa model, which scores 1.000 on its own phrase and 0.000 on
//! silence.
//!
//! 1. **mel** — 16-bit audio in, a (frames, 32) spectrogram out, shifted by
//!    `x / 10 + 2` so it matches the range the embedding CNN was trained on.
//! 2. **embedding** — 76 mel frames in, 96 numbers out; one row per 8-frame step.
//! 3. **head** — 16 consecutive embeddings in, one score out. The heads are the
//!    205 KB files from the community collection, one per word.
//!
//! Two things are deliberate. It holds no microphone and no threshold:
//! `audio/wake` decides when to call this and what a score means. And the head's
//! input name is read from the session rather than assumed, because these heads
//! are converted TensorFlow graphs and each carries its own name.

use std::path::Path;
use std::sync::OnceLock;

use ort::environment::Environment;
use ort::session::Session;
use ort::value::Tensor;

/// The runtime environment, created once for the whole process.
///
/// ONNX Runtime permits exactly one per process, and it is created against the
/// DLL path on first use. Switching wake models opens new sessions, not a new
/// environment — which is what used to fail with "only one environment is
/// allowed per process" the first time the model was changed.
static ENVIRONMENT: OnceLock<Result<Environment, String>> = OnceLock::new();

fn environment(dll: &Path) -> Result<&'static Environment, String> {
    ENVIRONMENT
        .get_or_init(|| {
            let environment = ort::init_from(dll)
                .map_err(|error| format!("ONNX Runtime could not be loaded: {error}"))?
                .build()
                .map_err(|error| format!("ONNX Runtime could not be started: {error}"))?;
            // The runtime's own log is a transcript of graph optimisation per
            // session: useful when a model misbehaves, unreadable otherwise, and
            // it goes to the app's console rather than to a panel. Errors only.
            environment.set_log_level(ort::logging::LogLevel::Error);
            Ok(environment)
        })
        .as_ref()
        .map_err(|message| message.clone())
}

/// Mel frames one embedding covers, and how far the window moves.
const MEL_WINDOW: usize = 76;
const MEL_STEP: usize = 8;
/// One mel frame is 32 bins; one embedding is 96 numbers.
const MEL_BINS: usize = 32;
const FEATURE_DIM: usize = 96;
/// Embeddings one score covers: 16 × 8 mel frames, about 1.3 s.
const FEATURE_FRAMES: usize = 16;

pub struct Recognizer {
    mel: Session,
    embedding: Session,
    head: Session,
    head_input: String,
}

impl Recognizer {
    /// Opens the three graphs. Every file is read here, so a missing or
    /// mismatched model fails at the switch rather than mid-sentence.
    pub fn open(dll: &Path, mel: &Path, embedding: &Path, head: &Path) -> Result<Self, String> {
        if !dll.is_file() || !mel.is_file() || !embedding.is_file() || !head.is_file() {
            return Err("A model file for the wake word is missing.".to_string());
        }
        // `init_from` loads the DLL this engine installed rather than one found
        // on PATH, which is what lets the engine be installed and removed
        // without touching the machine. The environment is process-wide, so a
        // second model reuses it.
        let environment = environment(dll)?;
        let mel = open_session(environment, mel)?;
        let embedding = open_session(environment, embedding)?;
        let head = open_session(environment, head)?;
        let head_input = head
            .inputs()
            .first()
            .map(|input| input.name().to_string())
            .ok_or_else(|| "The wake word model takes no input.".to_string())?;
        Ok(Self {
            mel,
            embedding,
            head,
            head_input,
        })
    }

    /// The best score this clip reaches, 0..1.
    pub fn score(&mut self, samples: &[i16]) -> Result<f32, String> {
        let frames = self.mel_frames(samples)?;
        if frames.len() / MEL_BINS < MEL_WINDOW {
            return Ok(0.0);
        }
        let features = self.features(&frames)?;
        let rows = features.len() / FEATURE_DIM;
        if rows < FEATURE_FRAMES {
            return Ok(0.0);
        }
        // Every 16-row window inside what was heard, so a phrase spoken early in
        // the clip is not lost to a window that only looks at the end.
        let mut best = 0.0f32;
        let mut end = FEATURE_FRAMES;
        while end <= rows {
            let window = features[(end - FEATURE_FRAMES) * FEATURE_DIM..end * FEATURE_DIM].to_vec();
            let batch =
                Tensor::from_array(([1i64, FEATURE_FRAMES as i64, FEATURE_DIM as i64], window))
                    .map_err(|error| format!("The wake word input could not be built: {error}"))?;
            let outputs = self
                .head
                .run(ort::inputs![self.head_input.as_str() => batch])
                .map_err(|error| format!("The wake word model failed: {error}"))?;
            let value = outputs[0]
                .try_extract_tensor::<f32>()
                .map_err(|error| format!("The wake word result could not be read: {error}"))?
                .1
                .first()
                .copied()
                .unwrap_or(0.0);
            best = best.max(value);
            end += 1;
        }
        Ok(best)
    }

    /// `(frames, 32)` laid out flat, already shifted into the embedding's range.
    fn mel_frames(&mut self, samples: &[i16]) -> Result<Vec<f32>, String> {
        let audio: Vec<f32> = samples.iter().map(|sample| *sample as f32).collect();
        let input = Tensor::from_array(([1i64, audio.len() as i64], audio))
            .map_err(|error| format!("The audio could not be shaped for the mel model: {error}"))?;
        let outputs = self
            .mel
            .run(ort::inputs!["input" => input])
            .map_err(|error| format!("The mel model failed: {error}"))?;
        let (shape, values) = outputs[0]
            .try_extract_tensor::<f32>()
            .map_err(|error| format!("The mel result could not be read: {error}"))?;
        // The model keeps a leading frame axis of one; the frame count is
        // whatever is left once the bins and that axis are accounted for.
        let frames = if shape.len() >= 3 {
            shape[shape.len() - 2].max(1) as usize
        } else {
            values.len() / MEL_BINS
        };
        let mut shifted = Vec::with_capacity(frames * MEL_BINS);
        for value in values.iter().take(frames * MEL_BINS) {
            shifted.push(value / 10.0 + 2.0);
        }
        Ok(shifted)
    }

    /// Flat `(rows, 96)` embeddings, one row per 8 mel frames.
    fn features(&mut self, frames: &[f32]) -> Result<Vec<f32>, String> {
        let count = frames.len() / MEL_BINS;
        let mut windows: Vec<f32> = Vec::new();
        let mut rows = 0usize;
        let mut start = 0usize;
        while start + MEL_WINDOW <= count {
            for row in 0..MEL_WINDOW {
                let offset = (start + row) * MEL_BINS;
                windows.extend_from_slice(&frames[offset..offset + MEL_BINS]);
            }
            rows += 1;
            start += MEL_STEP;
        }
        if rows == 0 {
            return Ok(Vec::new());
        }
        let batch = Tensor::from_array((
            [rows as i64, MEL_WINDOW as i64, MEL_BINS as i64, 1i64],
            windows,
        ))
        .map_err(|error| format!("The mel windows could not be shaped: {error}"))?;
        let outputs = self
            .embedding
            .run(ort::inputs!["input_1" => batch])
            .map_err(|error| format!("The embedding model failed: {error}"))?;
        let (_, values) = outputs[0]
            .try_extract_tensor::<f32>()
            .map_err(|error| format!("The embedding result could not be read: {error}"))?;
        Ok(values.to_vec())
    }
}

/// One message for every step of opening a graph, whichever error type it hands
/// back: `ort` carries the half-built session inside its errors, so the chain
/// cannot be a single `and_then`.
fn failed<T>(path: &Path) -> impl Fn(ort::Error<T>) -> String + '_ {
    move |error| format!("{} could not be opened: {error}", path.display())
}

fn open_session(environment: &Environment, path: &Path) -> Result<Session, String> {
    // One thread and no spinning. ONNX Runtime's default is a per-session pool
    // that spins for work that may never come; with three sessions and a wake
    // word listening, that was the app idling at ~40% CPU with no webview
    // involved. These graphs are milliseconds wide and do not need a pool.
    let builder = Session::builder(environment).map_err(failed(path))?;
    let builder = builder.with_intra_threads(1).map_err(failed(path))?;
    let builder = builder.with_inter_threads(1).map_err(failed(path))?;
    let mut builder = builder
        .with_intra_op_spinning(false)
        .map_err(failed(path))?;
    builder.commit_from_file(path).map_err(failed(path))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Runs the whole chain against real files. Ignored because it needs the
    /// runtime DLL, the two shared models, a head and a clip:
    ///
    /// ```text
    /// OWW_DLL=… OWW_MEL=… OWW_EMB=… OWW_HEAD=… OWW_WAV=… \
    ///   cargo test --lib the_chain_scores_its_own_phrase -- --ignored --nocapture
    /// ```
    ///
    /// The clip has to be 16 kHz mono and say the head's phrase; the assertion is
    /// what proves the preprocessing matches openWakeWord rather than merely
    /// running.
    #[test]
    #[ignore = "needs the runtime and a clip"]
    fn the_chain_scores_its_own_phrase() {
        let dll = std::env::var("OWW_DLL").expect("OWW_DLL");
        let mel = std::env::var("OWW_MEL").expect("OWW_MEL");
        let embedding = std::env::var("OWW_EMB").expect("OWW_EMB");
        let head = std::env::var("OWW_HEAD").expect("OWW_HEAD");
        let clip = std::env::var("OWW_WAV").expect("OWW_WAV");
        let samples = read_wav(&clip);
        let mut recognizer = Recognizer::open(
            Path::new(&dll),
            Path::new(&mel),
            Path::new(&embedding),
            Path::new(&head),
        )
        .expect("the models should open");
        let score = recognizer.score(&samples).expect("the clip should score");
        eprintln!("score = {score}");
        assert!(
            score > 0.5,
            "the phrase should clear its own floor, got {score}"
        );
    }

    /// Just enough WAV reading for the probe: a 16 kHz mono PCM16 file.
    fn read_wav(path: &str) -> Vec<i16> {
        let bytes = std::fs::read(path).expect("the clip should exist");
        let start = bytes
            .windows(4)
            .position(|window| window == b"data")
            .expect("a data chunk")
            + 8;
        bytes[start..]
            .chunks_exact(2)
            .map(|pair| i16::from_le_bytes([pair[0], pair[1]]))
            .collect()
    }
}
