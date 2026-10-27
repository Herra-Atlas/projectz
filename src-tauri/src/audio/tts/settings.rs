//! The stored speech settings: which voice speaks, and how fast each one does.
//!
//! Pace is per voice because it is a property of the voice: a slow narrator and a
//! clipped assistant do not want the same multiplier, and one slider that moved
//! every voice would be a setting the user has to re-set on each change.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::database::Database;

/// The settings key, beside `app.taste` and `app.stt`.
pub const KEY: &str = "app.tts";

/// The pace range a voice may be asked for.
///
/// Outside this, Piper is either unintelligible or a chipmunk, and a number that
/// came from an older build or a hand-edited database should not be able to make
/// speech unusable.
pub const MIN_SPEED: f32 = 0.5;
pub const MAX_SPEED: f32 = 2.0;
const DEFAULT_SPEED: f32 = 1.0;

/// The key the built-in voice's pace is stored under.
///
/// The built-in voice is selected as `None` rather than by a path, so it needs a
/// name no file path can collide with.
const BUILT_IN_KEY: &str = "built-in";

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TtsSettings {
    /// The chosen voice's model path, or null for the one the build placed.
    pub voice: Option<String>,
    /// Pace per voice, keyed by the voice's id. An absent entry is the voice's own.
    #[serde(default)]
    pub speeds: BTreeMap<String, f32>,
    /// Leave code out of what is spoken. On by default: a reply full of source is
    /// mostly not worth hearing, and the one time it is, the switch is right here.
    #[serde(default = "yes")]
    pub skip_code: bool,
    /// Leave markdown markup out of what is spoken, so the voice reads what the
    /// reader sees -- the words, not the `**` and `#` that styled them.
    #[serde(default = "yes")]
    pub skip_markdown: bool,
    /// The selected voice reads replies as they finish, without being pressed.
    #[serde(default)]
    pub auto_speak: bool,
    /// The auto-speak caveat: keep a voice that would need a cold start waiting
    /// rather than starting one in the reader's back. Off, the voice may load.
    #[serde(default = "yes")]
    pub auto_speak_only_loaded: bool,
    /// Tidy the text before it is spoken: strip code, markup, and the separators
    /// left where either was removed. Off, the reply is read as written.
    #[serde(default = "yes")]
    pub smart_speak: bool,
    /// Synthesize sentence by sentence and start playing at once, instead of
    /// waiting for the whole reply to be ready.
    #[serde(default = "yes")]
    pub stream_speech: bool,
    /// Written by the first version of this record, when there was one pace for
    /// every voice. Folded into `speeds` on load and never written again.
    #[serde(default, rename = "speed", skip_serializing)]
    legacy_speed: Option<f32>,
}

impl Default for TtsSettings {
    fn default() -> Self {
        Self {
            voice: None,
            speeds: BTreeMap::new(),
            skip_code: true,
            skip_markdown: true,
            auto_speak: false,
            auto_speak_only_loaded: true,
            smart_speak: true,
            stream_speech: true,
            legacy_speed: None,
        }
    }
}

fn yes() -> bool {
    true
}

impl TtsSettings {
    /// The stored record, or the defaults for a fresh install.
    ///
    /// A record written by an older build -- or one that cannot be read at all --
    /// falls back rather than failing: speech should still work when a preference
    /// does not.
    pub fn load(database: &Database) -> Self {
        let mut settings = database
            .setting(KEY)
            .and_then(|value| serde_json::from_value::<Self>(value).ok())
            .unwrap_or_default();
        settings.adopt_legacy_speed();
        settings
    }

    pub fn save(&self, database: &Database) -> Result<(), String> {
        let value = serde_json::to_value(self)
            .map_err(|error| format!("Could not store the speech settings: {error}"))?;
        database.set_setting(KEY, &value)
    }

    /// The pace of one voice; `None` means the selected one.
    pub fn speed_for(&self, voice: Option<&str>) -> f32 {
        self.speeds
            .get(&key_for(voice))
            .copied()
            .map(clamp)
            .unwrap_or(DEFAULT_SPEED)
    }

    pub fn set_speed(&mut self, voice: Option<&str>, speed: f32) {
        self.speeds.insert(key_for(voice), clamp(speed));
    }

    /// Forgets everything stored about a voice that is being deleted.
    ///
    /// The selection falls back to the built-in voice rather than pointing at a
    /// file that no longer exists, and its pace goes with it -- a deleted voice
    /// that came back would otherwise return at a pace set months earlier.
    pub fn forget(&mut self, voice: &str) {
        self.speeds.remove(&key_for(Some(voice)));
        if self.voice.as_deref() == Some(voice) {
            self.voice = None;
        }
    }

    fn adopt_legacy_speed(&mut self) {
        let Some(speed) = self.legacy_speed.take() else {
            return;
        };
        let key = key_for(self.voice.as_deref());
        self.speeds.entry(key).or_insert_with(|| clamp(speed));
    }
}

fn key_for(voice: Option<&str>) -> String {
    match voice {
        Some(voice) if !voice.trim().is_empty() => voice.to_string(),
        _ => BUILT_IN_KEY.to_string(),
    }
}

fn clamp(speed: f32) -> f32 {
    if speed.is_finite() {
        speed.clamp(MIN_SPEED, MAX_SPEED)
    } else {
        DEFAULT_SPEED
    }
}

/// The record as the engine settings page draws it.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TtsSettingsView {
    pub voice: Option<String>,
    /// The selected voice's pace.
    pub speed: f32,
    pub min_speed: f32,
    pub max_speed: f32,
    /// Whether code is left out of what is spoken.
    pub skip_code: bool,
    /// Whether markdown markup is left out of what is spoken.
    pub skip_markdown: bool,
    /// Whether replies are read as they finish.
    pub auto_speak: bool,
    /// Whether the auto read waits for an already-warm voice.
    pub auto_speak_only_loaded: bool,
    /// Whether the text is tidied before it is spoken.
    pub smart_speak: bool,
    /// Whether synthesis is spoken sentence by sentence as it arrives.
    pub stream_speech: bool,
}

impl TtsSettingsView {
    pub fn of(settings: &TtsSettings) -> Self {
        Self {
            voice: settings.voice.clone(),
            speed: settings.speed_for(settings.voice.as_deref()),
            min_speed: MIN_SPEED,
            max_speed: MAX_SPEED,
            skip_code: settings.skip_code,
            skip_markdown: settings.skip_markdown,
            auto_speak: settings.auto_speak,
            auto_speak_only_loaded: settings.auto_speak_only_loaded,
            smart_speak: settings.smart_speak,
            stream_speech: settings.stream_speech,
        }
    }
}

/// One voice's pace, as its own settings page draws it.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VoiceSpeedView {
    pub speed: f32,
    pub min_speed: f32,
    pub max_speed: f32,
}

impl VoiceSpeedView {
    pub fn of(settings: &TtsSettings, voice: Option<&str>) -> Self {
        Self {
            speed: settings.speed_for(voice),
            min_speed: MIN_SPEED,
            max_speed: MAX_SPEED,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_pace_outside_the_range_is_pulled_back_into_it() {
        assert_eq!(clamp(0.0), MIN_SPEED);
        assert_eq!(clamp(9.0), MAX_SPEED);
        assert_eq!(clamp(1.25), 1.25);
    }

    #[test]
    fn a_pace_that_is_not_a_number_falls_back_to_the_voice() {
        // A hand-edited record must not be able to make speech unusable.
        assert_eq!(clamp(f32::NAN), DEFAULT_SPEED);
    }

    #[test]
    fn each_voice_keeps_its_own_pace() {
        let mut settings = TtsSettings::default();
        settings.set_speed(Some("C:/voices/slow.onnx"), 0.75);
        settings.set_speed(None, 1.5);

        assert_eq!(settings.speed_for(Some("C:/voices/slow.onnx")), 0.75);
        assert_eq!(settings.speed_for(None), 1.5);
        // A voice nobody has touched speaks at its own pace rather than borrowing
        // another's.
        assert_eq!(
            settings.speed_for(Some("C:/voices/new.onnx")),
            DEFAULT_SPEED
        );
    }

    #[test]
    fn an_empty_path_is_the_same_choice_as_no_path() {
        assert_eq!(key_for(Some("  ")), BUILT_IN_KEY);
        assert_eq!(key_for(None), BUILT_IN_KEY);
        assert_eq!(key_for(Some("C:/voices/one.onnx")), "C:/voices/one.onnx");
    }

    #[test]
    fn forgetting_a_voice_takes_its_pace_and_its_selection() {
        let mut settings = TtsSettings::default();
        settings.set_speed(Some("C:/voices/one.onnx"), 1.4);
        settings.set_speed(None, 1.1);
        settings.voice = Some("C:/voices/one.onnx".to_string());

        settings.forget("C:/voices/one.onnx");

        // Back to the built-in voice rather than pointing at a file that is gone.
        assert!(settings.voice.is_none());
        assert_eq!(
            settings.speed_for(Some("C:/voices/one.onnx")),
            DEFAULT_SPEED
        );
        // The other voice's pace is untouched.
        assert_eq!(settings.speed_for(None), 1.1);
    }

    #[test]
    fn the_old_single_pace_belongs_to_the_voice_that_was_selected() {
        let stored = serde_json::json!({ "voice": "C:/voices/one.onnx", "speed": 1.4 });
        let mut settings: TtsSettings = serde_json::from_value(stored).unwrap();
        settings.adopt_legacy_speed();

        assert_eq!(settings.speed_for(Some("C:/voices/one.onnx")), 1.4);
        assert_eq!(settings.speed_for(None), DEFAULT_SPEED);
    }
}
