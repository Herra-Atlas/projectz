//! Learning what the user is like, and remembering it compactly.
//!
//! # What this is
//!
//! A background learner with three parts: a stored profile of precise facts, a
//! run every few days over conversations it has not read, and a rendered block
//! that rides in the chat's leading system message. It is not a job: a job is an
//! agent run with tools, a transcript and a sidebar row, while this is one
//! non-streaming completion per batch with no tools at all. The title chain and
//! Assist are the closer relatives.
//!
//! # Why the profile is operation-edit rather than rewritten
//!
//! A model asked to rewrite the whole profile will silently drop a fact it was not
//! asked to touch. See [`merge`]: operations name rows, code applies them, and the
//! budget is enforced deterministically.
//!
//! # Why the chat block is cached in a process global
//!
//! The block is read while a request is assembled, where only the database is in
//! scope and no app state is reachable. It is also the one part of the prompt that
//! must be byte-identical between requests so a provider's prompt cache matches.
//! The global is refreshed at startup and after every run, save or reset -- never
//! on a timer, because the profile only changes at those moments.

pub mod corpus;
pub mod merge;
pub mod ops;
pub mod prompt;
pub mod runner;
pub mod section;
pub mod settings;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock, RwLock};
use std::time::Duration;

use chrono::Utc;
use serde::Serialize;
use tauri::{AppHandle, Emitter};

use crate::ai::runtime::AiRuntime;
use crate::database::taste::{TasteItemRow, TasteRunRow};
use crate::database::{utc_now, Database};

pub use settings::TasteSettings;

/// Event the settings page listens on.
pub const EVENT: &str = "taste-event";

/// How often the clock is consulted.
///
/// **This is not a request cadence.** Every tick reads one settings row and
/// compares a stored timestamp; only a due record reaches a model, and after a run
/// the next timestamp is four days away. The tick exists so enabling the feature
/// or pressing Run now does not have to wait for a launch.
const TICK: Duration = Duration::from_secs(15 * 60);

/// How long after launch the first check runs. The window is still opening and the
/// database is still being read; a run that started first would race both.
const STARTUP_DELAY: Duration = Duration::from_secs(15);

/// One learned fact, as the settings page draws it.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TasteItemView {
    pub id: i64,
    pub section: String,
    pub text: String,
    pub confidence: f32,
    pub last_seen_at: String,
    pub updated_at: String,
}

/// One run, as the settings page's history draws it.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TasteRunView {
    pub id: i64,
    pub started_at: String,
    pub finished_at: Option<String>,
    pub status: String,
    pub reason: Option<String>,
    pub model_id: Option<String>,
    pub provider_id: Option<String>,
    pub sessions_read: i64,
    pub messages_read: i64,
    pub prompt_tokens: Option<i64>,
    pub completion_tokens: Option<i64>,
    pub added: i64,
    pub updated: i64,
    pub removed: i64,
    pub report: Option<String>,
    pub document: Option<String>,
}

/// Everything the settings page draws, in one read.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TasteStatus {
    pub enabled: bool,
    pub settings: TasteSettings,
    pub items: Vec<TasteItemView>,
    /// The profile as the chat would receive it. Empty when nothing was learned.
    pub document: String,
    pub last_run: Option<TasteRunView>,
    pub running: bool,
    pub next_run_at: Option<String>,
}

struct Inner {
    database: Arc<Database>,
    running: AtomicBool,
    wake: tokio::sync::Notify,
}

/// The learner, as the app holds it.
#[derive(Clone)]
pub struct Taste {
    inner: Arc<Inner>,
}

impl Taste {
    /// Builds the handle and primes the prompt cache.
    ///
    /// Loading the cache here rather than on the first request is what keeps a
    /// chat from paying a database read for a value that changes twice a week.
    pub fn new(database: Arc<Database>) -> Self {
        refresh_prompt_cache(&database);
        Self {
            inner: Arc::new(Inner {
                database,
                running: AtomicBool::new(false),
                wake: tokio::sync::Notify::new(),
            }),
        }
    }

    /// Starts the background clock.
    pub fn start(&self, app: AppHandle, runtime: AiRuntime) {
        let taste = self.clone();
        tauri::async_runtime::spawn(async move {
            tokio::time::sleep(STARTUP_DELAY).await;
            loop {
                let settings = TasteSettings::load(&taste.inner.database);
                if settings.is_due(&utc_now()) {
                    if let Err(error) = taste.run(&app, &runtime, false).await {
                        tracing::warn!(%error, "a taste run could not start");
                    }
                }
                tokio::select! {
                    _ = tokio::time::sleep(TICK) => {}
                    _ = taste.inner.wake.notified() => {}
                }
            }
        });
    }

    /// The settings page's single read.
    pub fn status(&self) -> TasteStatus {
        let settings = TasteSettings::load(&self.inner.database);
        let items = merge::normalize(self.inner.database.taste_items().unwrap_or_default());
        TasteStatus {
            enabled: settings.enabled,
            document: merge::render(&items),
            items: items.into_iter().map(item_view).collect(),
            last_run: self
                .inner
                .database
                .latest_taste_run()
                .ok()
                .flatten()
                .map(run_view),
            running: self.inner.running.load(Ordering::Relaxed),
            next_run_at: settings.next_run_at.clone(),
            settings,
        }
    }

    /// Writes the record, preserving an already-scheduled run across a toggle.
    ///
    /// **Turning the switch off and on is not a request to learn now.** The next
    /// run is left where it was, so a user who briefly toggles the feature does
    /// not reset the four-day cadence -- which used to make every on/off pair look
    /// like a due run and produce a "skipped: nothing new" row on the same day.
    /// Only a record with no next time (the first enable) is armed for now, and a
    /// cadence change is measured from the last attempt rather than from the
    /// toggle.
    ///
    /// The cache is refreshed immediately so turning the feature off stops the
    /// next request from carrying a profile, and turning it on makes the next one
    /// carry what is already known.
    pub fn save(&self, app: &AppHandle, settings: &TasteSettings) -> Result<TasteSettings, String> {
        let previous = TasteSettings::load(&self.inner.database);
        let latest = self.inner.database.latest_taste_run().ok().flatten();
        let mut next = settings.clone();
        // The schedule is the backend's, not the frontend's. A page that was
        // open while a run finished still holds the old timestamp; trusting it
        // would make a toggle look like a due run and shorten the cadence. Keep
        // the stored time and let only an explicit cadence change move it.
        next.next_run_at = schedule_on_save(&previous, &next, latest.as_ref());
        let stored = TasteSettings::save(&self.inner.database, &next)?;
        refresh_prompt_cache(&self.inner.database);
        emit(app, "state", None);
        // Enabling or shortening the cadence should not wait out the remainder of a
        // fifteen-minute sleep to be noticed.
        self.inner.wake.notify_one();
        Ok(stored)
    }

    /// Runs now, whether or not the schedule says so.
    ///
    /// Allowed while the feature is switched off: the run is a preview of what it
    /// would learn, and the resulting profile is not sent with chats until the
    /// switch is on.
    pub async fn run_now(
        &self,
        app: &AppHandle,
        runtime: &AiRuntime,
    ) -> Result<TasteRunView, String> {
        self.run(app, runtime, true).await?;
        self.inner
            .database
            .latest_taste_run()
            .map_err(|error| error.to_string())?
            .map(run_view)
            .ok_or_else(|| "The run finished without writing a record".to_string())
    }

    /// One run, with the schedule advanced only when the record allows it.
    async fn run(
        &self,
        app: &AppHandle,
        runtime: &AiRuntime,
        manual: bool,
    ) -> Result<runner::RunOutcome, String> {
        if self.inner.running.swap(true, Ordering::SeqCst) {
            return Err("A taste run is already in progress.".to_string());
        }
        let result = self.run_inner(app, runtime, manual).await;
        self.inner.running.store(false, Ordering::Relaxed);
        result
    }

    async fn run_inner(
        &self,
        app: &AppHandle,
        runtime: &AiRuntime,
        manual: bool,
    ) -> Result<runner::RunOutcome, String> {
        let settings = TasteSettings::load(&self.inner.database);
        if !manual && !settings.enabled {
            return Err("Taste is switched off.".to_string());
        }
        emit(app, "started", None);

        let outcome = match runner::run(&self.inner.database, runtime, &settings).await {
            Ok(outcome) => outcome,
            Err(error) => {
                // A database failure is the one case the runner could not record.
                // Leave the next attempt soon rather than at the cadence, because
                // the reason is likely to be transient.
                if settings.enabled {
                    let mut retry = settings.clone();
                    retry.next_run_at = Some(TasteSettings::retry_after(Utc::now()));
                    let _ = TasteSettings::save(&self.inner.database, &retry);
                }
                emit(app, "failed", None);
                return Err(error);
            }
        };

        // Re-read before writing the schedule: the user may have switched the
        // feature off or changed its cadence while the run was in flight. Writing
        // the pre-run record back would undo that, and a disable that silently
        // re-enabled itself is the one bug this feature must not have.
        let mut next = TasteSettings::load(&self.inner.database);
        if next.enabled {
            next.next_run_at = Some(if outcome.status == "failed" && outcome.transient {
                TasteSettings::retry_after(Utc::now())
            } else {
                next.next_after(Utc::now())
            });
            TasteSettings::save(&self.inner.database, &next)?;
        }

        refresh_prompt_cache(&self.inner.database);
        let run = self.inner.database.latest_taste_run().ok().flatten();
        emit(app, outcome.status, run.map(run_view));
        Ok(outcome)
    }

    /// Run history, newest first.
    pub fn runs(&self, limit: i64) -> Vec<TasteRunView> {
        self.inner
            .database
            .taste_runs(limit.max(1).min(100))
            .unwrap_or_default()
            .into_iter()
            .map(run_view)
            .collect()
    }

    /// Rewrites one learned fact.
    ///
    /// The text goes through the same validation the model's additions do, so a
    /// manual edit cannot store a label or a paragraph. The cache is refreshed
    /// immediately: an edit is a change to what every later chat receives.
    pub fn update_item(&self, app: &AppHandle, id: i64, text: &str) -> Result<TasteStatus, String> {
        let text = merge::sanitize_user_text(text).ok_or_else(|| {
            "A fact needs at least two words; it cannot be a label or an empty line.".to_string()
        })?;
        self.inner.database.update_taste_item(id, &text)?;
        refresh_prompt_cache(&self.inner.database);
        emit(app, "state", None);
        Ok(self.status())
    }

    /// Deletes one learned fact.
    pub fn delete_item(&self, app: &AppHandle, id: i64) -> Result<TasteStatus, String> {
        self.inner.database.delete_taste_item(id)?;
        refresh_prompt_cache(&self.inner.database);
        emit(app, "state", None);
        Ok(self.status())
    }

    /// Forgets the profile and how far each conversation was read.
    pub fn reset(&self, app: &AppHandle) -> Result<TasteStatus, String> {
        self.inner.database.reset_taste()?;
        refresh_prompt_cache(&self.inner.database);
        emit(app, "state", None);
        Ok(self.status())
    }
}

/// The schedule a save should store.
///
/// Pure so the one behaviour that matters can be tested without an `AppHandle`:
/// **the stored next run survives a toggle.** The page sends back the record it
/// last read, which may be older than a run that finished in the meantime; the
/// backend keeps its own timestamp. Only a first enable (no time at all) arms the
/// feature for now, and only an explicit cadence change moves the clock.
fn schedule_on_save(
    previous: &TasteSettings,
    next: &TasteSettings,
    latest: Option<&TasteRunRow>,
) -> Option<String> {
    let stored = previous.next_run_at.clone();
    if !next.enabled {
        return stored;
    }
    match stored {
        None => Some(utc_now()),
        Some(_) if next.interval_days != previous.interval_days => {
            Some(next_run_after_last(latest, next.interval_days))
        }
        Some(time) => Some(time),
    }
}

/// The next time after the last attempt, under a new cadence.
///
/// The clock moves to "last run + new interval" rather than "toggle + new
/// interval", because the latter waits a full cadence after a settings change.
/// A result already in the past means the new cadence is due now.
fn next_run_after_last(latest: Option<&TasteRunRow>, interval_days: u32) -> String {
    let Some(run) = latest else {
        return utc_now();
    };
    let stamp = run.finished_at.as_deref().unwrap_or(&run.started_at);
    let Some(last) = chrono::DateTime::parse_from_rfc3339(stamp)
        .ok()
        .map(|at| at.with_timezone(&Utc))
    else {
        return utc_now();
    };
    let next = last + chrono::Duration::days(i64::from(interval_days));
    if next > Utc::now() {
        next.to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
    } else {
        utc_now()
    }
}

fn item_view(item: TasteItemRow) -> TasteItemView {
    TasteItemView {
        id: item.id,
        section: item.section,
        text: item.text,
        confidence: item.confidence,
        last_seen_at: item.last_seen_at,
        updated_at: item.updated_at,
    }
}

fn run_view(run: TasteRunRow) -> TasteRunView {
    TasteRunView {
        id: run.id,
        started_at: run.started_at,
        finished_at: run.finished_at,
        status: run.status,
        reason: run.reason,
        model_id: run.model_id,
        provider_id: run.provider_id,
        sessions_read: run.sessions_read,
        messages_read: run.messages_read,
        prompt_tokens: run.prompt_tokens,
        completion_tokens: run.completion_tokens,
        added: run.added,
        updated: run.updated,
        removed: run.removed,
        report: run.report,
        document: run.document,
    }
}

fn emit(app: &AppHandle, kind: &str, run: Option<TasteRunView>) {
    let _ = app.emit(
        EVENT,
        serde_json::json!({
            "kind": kind,
            "run": run,
        }),
    );
}

/// The process-wide rendered profile.
///
/// A `RwLock` rather than a `Mutex` because request assembly reads it and only a
/// run writes it: ten chats in flight should not queue behind each other for a
/// string that is not changing.
fn prompt_slot() -> &'static RwLock<Option<String>> {
    static SLOT: OnceLock<RwLock<Option<String>>> = OnceLock::new();
    SLOT.get_or_init(|| RwLock::new(None))
}

/// The block a chat request should carry, if taste is on and anything is known.
pub fn cached_prompt() -> Option<String> {
    prompt_slot().read().ok().and_then(|prompt| prompt.clone())
}

/// Rebuilds the cached block from storage.
///
/// Called at startup and after every event that can change the profile. A failure
/// to read leaves the previous block in place rather than clearing it: a transient
/// database error should not silently remove the context from every request.
pub fn refresh_prompt_cache(database: &Database) {
    let settings = TasteSettings::load(database);
    let prompt = if settings.enabled {
        match database.taste_items() {
            Ok(items) => merge::render_prompt_block(&merge::normalize(items)),
            // A read failure leaves the previous block in place rather than
            // silently removing the context from every request.
            Err(error) => {
                tracing::warn!(%error, "could not refresh the taste prompt");
                return;
            }
        }
    } else {
        None
    };
    if let Ok(mut slot) = prompt_slot().write() {
        *slot = prompt;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn settings(enabled: bool, interval_days: u32, next_run_at: Option<&str>) -> TasteSettings {
        TasteSettings {
            enabled,
            models: Vec::new(),
            interval_days,
            next_run_at: next_run_at.map(str::to_string),
        }
    }

    fn finished_at(stamp: String) -> TasteRunRow {
        TasteRunRow {
            id: 1,
            started_at: stamp.clone(),
            finished_at: Some(stamp),
            status: "completed".to_string(),
            reason: None,
            model_id: None,
            provider_id: None,
            sessions_read: 0,
            messages_read: 0,
            prompt_tokens: None,
            completion_tokens: None,
            added: 0,
            updated: 0,
            removed: 0,
            report: None,
            document: None,
        }
    }

    fn parse(stamp: &str) -> chrono::DateTime<Utc> {
        chrono::DateTime::parse_from_rfc3339(stamp)
            .expect("stamp")
            .with_timezone(&Utc)
    }

    /// The reported bug: turning the switch off and on again moved Next run to
    /// now. The stored time is the schedule, and a toggle is not a cadence change.
    #[test]
    fn a_toggle_keeps_the_stored_next_run() {
        let stored = "2099-01-01T00:00:00.000Z";
        let on = settings(true, 4, Some(stored));
        let off = settings(false, 4, Some(stored));
        assert_eq!(schedule_on_save(&on, &off, None).as_deref(), Some(stored));
        assert_eq!(schedule_on_save(&off, &on, None).as_deref(), Some(stored));
    }

    /// A first enable has no stored time, so it is the one case that arms for now.
    #[test]
    fn a_first_enable_arms_for_now() {
        let off = settings(false, 4, None);
        let on = settings(true, 4, None);
        assert!(schedule_on_save(&off, &on, None).is_some());
    }

    /// A cadence change is measured from the last attempt, not from the toggle.
    #[test]
    fn a_cadence_change_is_measured_from_the_last_run() {
        let last = (Utc::now() - chrono::Duration::days(2))
            .to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
        let previous = settings(true, 4, Some("2099-01-01T00:00:00.000Z"));
        let next = settings(true, 10, None);
        let scheduled =
            schedule_on_save(&previous, &next, Some(&finished_at(last))).expect("a scheduled time");
        let at = parse(&scheduled);
        assert!(at > Utc::now() + chrono::Duration::days(7), "{scheduled}");
        assert!(at < Utc::now() + chrono::Duration::days(9), "{scheduled}");
    }

    /// A shorter cadence that is already past leaves the feature due now rather
    /// than parking it in the past where the loop would never act.
    #[test]
    fn a_cadence_already_past_is_due_now() {
        let last = (Utc::now() - chrono::Duration::days(20))
            .to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
        let previous = settings(true, 4, Some("2099-01-01T00:00:00.000Z"));
        let next = settings(true, 1, None);
        let scheduled =
            schedule_on_save(&previous, &next, Some(&finished_at(last))).expect("a scheduled time");
        assert!(parse(&scheduled) <= Utc::now() + chrono::Duration::seconds(2));
    }
}
