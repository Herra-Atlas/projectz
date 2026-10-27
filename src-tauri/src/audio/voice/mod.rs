//! Who is speaking: enrolled voiceprints, and the check that uses them.
//!
//! A wake word says "listen"; it does not say who spoke, and a television can
//! say it as well as a person. An enrolled voiceprint is the second gate: the
//! same two seconds that fired the wake word are embedded, compared with every
//! stored voice, and the detection is dropped unless the nearest voice clears
//! the threshold *and* stands clear of the runner-up.
//!
//! # What a voiceprint is
//!
//! One vector per person: the average of the clips they enrolled, normalised to
//! unit length so a comparison is a dot product. It is not a recording and it
//! cannot be turned back into audio, which is why it is stored as numbers rather
//! than as the clips it came from.
//!
//! # What it is not
//!
//! Identification, not authentication. Related voices collide, a cold can move
//! a score by more than a different microphone does, and two people who sound
//! alike will be one person here. Nothing should be gated on this that matters.

pub mod commands;
pub mod embed;

use std::sync::{LazyLock, Mutex};

use serde::{Deserialize, Serialize};

use crate::audio::engines;
use crate::database::Database;

/// The settings key holding the switch, the threshold and every voiceprint.
const KEY: &str = "app.voiceid";

/// How close to the nearest voice a clip has to be to count as that person.
///
/// Cosine on this model: different people sit around 0.1-0.3, one person on the
/// same microphone above 0.5. A middling default, meant to be moved once the
/// user has seen their own scores.
const DEFAULT_THRESHOLD: f32 = 0.45;

/// How far the best match has to beat the next one, so two similar voices do
/// not alternate on every phrase.
const MARGIN: f32 = 0.05;

/// One enrolled voice.
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Voice {
    pub name: String,
    /// How many clips made the average. Shown, not used.
    pub clips: usize,
    /// Unit-length embedding.
    pub model: Vec<f32>,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Settings {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default = "threshold_default")]
    pub threshold: f32,
    #[serde(default)]
    pub voices: Vec<Voice>,
}

fn threshold_default() -> f32 {
    DEFAULT_THRESHOLD
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            enabled: false,
            threshold: DEFAULT_THRESHOLD,
            voices: Vec::new(),
        }
    }
}

/// What the check concluded.
pub enum Heard {
    /// Nothing is enrolled, or the switch is off: the detection stands.
    Off,
    /// A voice was compared and none matched. The detection is dropped, because
    /// the user asked for this microphone to answer only to them.
    Unknown {
        best: f32,
    },
    Known {
        name: String,
        score: f32,
    },
}

/// What the page draws.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    pub installed: bool,
    pub enabled: bool,
    pub threshold: f32,
    pub voices: Vec<Summary>,
    /// Takes recorded for the voice being created, before it has a name.
    pub pending: usize,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Summary {
    pub name: String,
    pub clips: usize,
}

/// The extractor, opened on first use and kept: loading the model costs more
/// than every embedding after it, and a wake word verifies once per trigger.
static SPEAKER: LazyLock<Mutex<Option<embed::Speaker>>> = LazyLock::new(|| Mutex::new(None));

/// The takes recorded for the voice being created.
///
/// One draft at a time, and no name on it: the page records the takes first and
/// asks what to call them afterwards. Keying this by name made the name a
/// prerequisite for the first press, which is the wrong order to ask in.
static DRAFT: LazyLock<Mutex<Vec<Vec<f32>>>> = LazyLock::new(|| Mutex::new(Vec::new()));

pub fn settings(database: &Database) -> Settings {
    database
        .setting(KEY)
        .and_then(|stored| serde_json::from_value(stored).ok())
        .unwrap_or_default()
}

pub fn save(database: &Database, settings: &Settings) -> Result<(), String> {
    let value = serde_json::to_value(settings)
        .map_err(|error| format!("The voice settings could not be written: {error}"))?;
    database.set_setting(KEY, &value)
}

pub fn status(database: &Database) -> Status {
    let settings = settings(database);
    let pending = draft_clips();
    Status {
        installed: engines::sherpa::is_installed(),
        enabled: settings.enabled && !settings.voices.is_empty(),
        threshold: settings.threshold,
        voices: settings
            .voices
            .into_iter()
            .map(|voice| Summary {
                name: voice.name,
                clips: voice.clips,
            })
            .collect(),
        pending,
    }
}

/// Whether the check would run at all.
pub fn active(database: &Database) -> bool {
    let settings = settings(database);
    settings.enabled && !settings.voices.is_empty() && engines::sherpa::is_installed()
}

/// Compares one recording (16 kHz mono) with the enrolled voices.
///
/// A failure to load the engine is returned rather than swallowed: the caller
/// treats `Off` as "accept", and a broken install must not silently become an
/// open door.
pub fn identify(database: &Database, samples: &[i16]) -> Result<Heard, String> {
    if !active(database) {
        return Ok(Heard::Off);
    }
    let settings = settings(database);
    let audio: Vec<f32> = samples
        .iter()
        .map(|sample| *sample as f32 / 32_768.0)
        .collect();
    // Unit length before comparing, because the model's vectors are not: a dot
    // product of two raw vectors is a similarity scaled by their lengths, which
    // would rate a loud stranger above the enrolled voice.
    let embedding = normalise(with_speaker(|speaker| speaker.embed(&audio))?);

    let mut scored: Vec<(f32, &Voice)> = settings
        .voices
        .iter()
        .map(|voice| (dot(&embedding, &voice.model), voice))
        .collect();
    scored.sort_by(|left, right| right.0.total_cmp(&left.0));

    let Some((best, voice)) = scored.first() else {
        return Ok(Heard::Off);
    };
    let runner_up = scored.get(1).map(|(score, _)| *score).unwrap_or(0.0);
    if *best >= settings.threshold && best - runner_up >= MARGIN {
        Ok(Heard::Known {
            name: voice.name.clone(),
            score: *best,
        })
    } else {
        Ok(Heard::Unknown { best: *best })
    }
}

/// Embeds one enrollment take and holds it until the voice is named.
pub fn enroll_clip(samples: &[i16]) -> Result<usize, String> {
    if samples.len() < crate::audio::stt::capture::MIN_SAMPLES {
        return Err("That take was too short. Say the sentence, then stop.".to_string());
    }
    let audio: Vec<f32> = samples
        .iter()
        .map(|sample| *sample as f32 / 32_768.0)
        .collect();
    let vector = with_speaker(|speaker| speaker.embed(&audio))?;
    let mut draft = DRAFT
        .lock()
        .map_err(|_| "The enrollment list is unavailable.".to_string())?;
    draft.push(normalise(vector));
    Ok(draft.len())
}

/// How many takes are held for the voice being created.
pub fn draft_clips() -> usize {
    DRAFT.lock().map(|draft| draft.len()).unwrap_or(0)
}

/// Averages the held takes into the voiceprint that will be used.
pub fn finish_enrollment(database: &Database, name: &str) -> Result<Voice, String> {
    let name = name.trim().to_string();
    if name.is_empty() {
        return Err("Give the voice a name first.".to_string());
    }
    let clips = {
        let mut draft = DRAFT
            .lock()
            .map_err(|_| "The enrollment list is unavailable.".to_string())?;
        std::mem::take(&mut *draft)
    };
    if clips.is_empty() {
        return Err("No takes were recorded for that voice.".to_string());
    }
    let mut sum = vec![0.0f32; clips[0].len()];
    for clip in &clips {
        for (total, value) in sum.iter_mut().zip(clip) {
            *total += value;
        }
    }
    let voice = Voice {
        name: name.clone(),
        clips: clips.len(),
        model: normalise(sum),
    };
    let mut settings = settings(database);
    settings.voices.retain(|voice| voice.name != name);
    settings.voices.push(voice.clone());
    save(database, &settings)?;
    Ok(voice)
}

/// Throws away the takes held for the voice being created.
pub fn cancel_enrollment() {
    if let Ok(mut draft) = DRAFT.lock() {
        draft.clear();
    }
}

pub fn delete(database: &Database, name: &str) -> Result<(), String> {
    let mut settings = settings(database);
    let before = settings.voices.len();
    settings.voices.retain(|voice| voice.name != name);
    if settings.voices.len() == before {
        return Err("That voice is not enrolled.".to_string());
    }
    save(database, &settings)
}

/// Drops the loaded library, so uninstalling the engine can delete it.
pub fn unload() {
    if let Ok(mut slot) = SPEAKER.lock() {
        *slot = None;
    }
}

fn with_speaker<T>(run: impl FnOnce(&embed::Speaker) -> Result<T, String>) -> Result<T, String> {
    let mut slot = SPEAKER
        .lock()
        .map_err(|_| "The speaker model is unavailable.".to_string())?;
    if slot.is_none() {
        if !engines::sherpa::is_installed() {
            return Err(
                "Voice recognition is not installed. Install Sherpa-onnx on the Audio engine page."
                    .to_string(),
            );
        }
        *slot = Some(embed::Speaker::open(
            &engines::sherpa::install_dir(),
            &engines::sherpa::model(),
        )?);
    }
    match slot.as_ref() {
        Some(speaker) => run(speaker),
        None => Err("The speaker model could not be loaded.".to_string()),
    }
}

fn dot(left: &[f32], right: &[f32]) -> f32 {
    left.iter()
        .zip(right)
        .map(|(a, b)| a * b)
        .sum::<f32>()
        .clamp(-1.0, 1.0)
}

fn normalise(vector: Vec<f32>) -> Vec<f32> {
    let length = vector.iter().map(|value| value * value).sum::<f32>().sqrt();
    if length <= f32::EPSILON {
        return vector;
    }
    vector.into_iter().map(|value| value / length).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_voiceprint_is_unit_length_whatever_its_input() {
        let normalised = normalise(vec![3.0, 4.0]);
        assert!((normalised[0] - 0.6).abs() < 1e-6);
        assert!((normalised[1] - 0.8).abs() < 1e-6);
    }

    /// An old record holds no threshold, and must not become 0.0 -- a zero
    /// threshold matches everyone, which is the opposite of the intent.
    #[test]
    fn settings_without_a_threshold_take_the_default() {
        let settings: Settings = serde_json::from_str(r#"{"enabled":true}"#).unwrap();
        assert_eq!(settings.threshold, DEFAULT_THRESHOLD);
        assert!(settings.voices.is_empty());
    }
}
