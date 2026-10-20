//! What the taste learner reads from Settings.
//!
//! The record is deliberately small. Everything that changes per run lives in the
//! database (`taste_runs` and the watermark table); this is the user's decision
//! about whether the feature runs, which models may answer, and how often.
//!
//! # Why the record is normalized rather than trusted
//!
//! `next_run_at` is compared as a string, and `interval_days` is multiplied into a
//! timestamp. A zero interval would make the feature due every tick; a negative or
//! absurd interval would park it forever. A record written by an older build or
//! edited by hand lands on a working value instead of waking a background loop
//! that spends money.

use chrono::{Duration, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::database::Database;

/// Settings key holding the taste record.
pub const SETTINGS_KEY: &str = "app.taste";

/// Most models a run may try, in order, before giving up.
///
/// The same shape the title chain uses: the first model that answers wins, and a
/// later entry is a fallback for an engine that is not running or a provider that
/// stopped existing. More than three is not a fallback chain; it is a list nobody
/// reads.
pub const MAX_MODELS: usize = 3;

/// The default cadence. Four days is often enough to see a pattern and rare
/// enough that one unusual week cannot rewrite the profile.
pub const DEFAULT_INTERVAL_DAYS: u32 = 4;

const MIN_INTERVAL_DAYS: u32 = 1;
const MAX_INTERVAL_DAYS: u32 = 30;

/// The stored configuration.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TasteSettings {
    /// Whether learned context is sent with chats and the learner runs on its own.
    #[serde(default)]
    pub enabled: bool,
    /// Ordered model selections, in the shape the rest of the app uses:
    /// `{ endpointId, model }` or `{ localModelId }`.
    #[serde(default)]
    pub models: Vec<Value>,
    /// Days between successful runs.
    #[serde(default = "default_interval")]
    pub interval_days: u32,
    /// When the scheduler should next consider a run, as UTC ISO-8601.
    ///
    /// `None` on a fresh record means "due now": enabling the feature is a request
    /// to learn, not a request to wait four days for the first look.
    #[serde(default)]
    pub next_run_at: Option<String>,
}

fn default_interval() -> u32 {
    DEFAULT_INTERVAL_DAYS
}

impl Default for TasteSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            models: Vec::new(),
            interval_days: DEFAULT_INTERVAL_DAYS,
            next_run_at: None,
        }
    }
}

impl TasteSettings {
    /// Reads the record, falling back to defaults and clamping the values.
    pub fn load(database: &Database) -> Self {
        database
            .setting::<TasteSettings>(SETTINGS_KEY)
            .unwrap_or_default()
            .normalized()
    }

    /// Writes the record and returns what was stored.
    pub fn save(database: &Database, settings: &TasteSettings) -> Result<Self, String> {
        let normalized = settings.clone().normalized();
        database.set_setting(SETTINGS_KEY, &normalized)?;
        Ok(normalized)
    }

    /// Clamps the values a bad record could break.
    ///
    /// Enabling a record with no next run arms it for now. Disabling leaves the
    /// next run where it was: turning the feature off is not a request to forget
    /// when it was due, and turning it back on after a pause should catch up.
    pub fn normalized(mut self) -> Self {
        self.interval_days = self
            .interval_days
            .clamp(MIN_INTERVAL_DAYS, MAX_INTERVAL_DAYS);
        self.models.truncate(MAX_MODELS);
        self.models.retain(|selection| {
            selection
                .as_object()
                .is_some_and(|object| !object.is_empty())
        });
        if self.enabled && self.next_run_at.is_none() {
            self.next_run_at = Some(now_stamp());
        }
        self
    }

    /// Whether the learner should run now.
    pub fn is_due(&self, now: &str) -> bool {
        self.enabled && self.next_run_at.as_deref().map_or(true, |next| next <= now)
    }

    /// When the next automatic run falls due after a completed attempt.
    pub fn next_after(&self, now: chrono::DateTime<Utc>) -> String {
        (now + Duration::days(i64::from(self.interval_days)))
            .to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
    }

    /// A retry after a transient failure: sooner than the cadence, but not so
    /// often that an offline provider turns into a request every fifteen minutes.
    pub fn retry_after(now: chrono::DateTime<Utc>) -> String {
        (now + Duration::days(1)).to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
    }

    pub fn has_model(&self) -> bool {
        !self.models.is_empty()
    }
}

/// The current moment in the format every stored timestamp uses.
fn now_stamp() -> String {
    Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_fresh_record_is_off_and_four_days() {
        let settings = TasteSettings::default();
        assert!(!settings.enabled);
        assert_eq!(settings.interval_days, 4);
        assert!(!settings.is_due("2026-01-01T00:00:00.000Z"));
    }

    #[test]
    fn enabling_arms_the_first_run_for_now() {
        let settings = TasteSettings {
            enabled: true,
            ..TasteSettings::default()
        }
        .normalized();
        assert!(settings.next_run_at.is_some());
        assert!(settings.is_due("9999-01-01T00:00:00.000Z"));
    }

    #[test]
    fn an_interval_outside_the_range_is_clamped() {
        assert_eq!(
            TasteSettings {
                interval_days: 0,
                ..TasteSettings::default()
            }
            .normalized()
            .interval_days,
            1
        );
        assert_eq!(
            TasteSettings {
                interval_days: 365,
                ..TasteSettings::default()
            }
            .normalized()
            .interval_days,
            30
        );
    }

    #[test]
    fn only_three_models_are_kept() {
        let settings = TasteSettings {
            models: (0..5)
                .map(|index| serde_json::json!({"endpointId": index.to_string(), "model": "m"}))
                .collect(),
            ..TasteSettings::default()
        }
        .normalized();
        assert_eq!(settings.models.len(), MAX_MODELS);
    }

    #[test]
    fn a_record_is_due_only_at_its_moment_and_after() {
        let settings = TasteSettings {
            enabled: true,
            interval_days: 4,
            next_run_at: Some("2026-01-05T00:00:00.000Z".into()),
            ..TasteSettings::default()
        };
        assert!(!settings.is_due("2026-01-04T23:59:59.999Z"));
        assert!(settings.is_due("2026-01-05T00:00:00.000Z"));
        assert!(settings.is_due("2026-01-06T00:00:00.000Z"));
    }
}
