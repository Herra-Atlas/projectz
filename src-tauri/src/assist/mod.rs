//! Helping without being asked.
//!
//! # What this is, in one paragraph
//!
//! Something on the user's screen is a question -- a sum, a riddle, a fact -- and they would
//! like the answer without having to ask for it. This module reads a *page* from a source
//! (today: the browser, through an extension), decides which of its blocks are worth
//! answering, asks a model once about all of them, and hands back one card per answer for the
//! source to draw beside the block it came from.
//!
//! # The rules everything here is built around
//!
//! **Cheap first, and nothing cheap is skipped.** Length and shape are checked before the
//! model is contacted, in `candidate.rs`, and they are checked per block: a page of forty
//! paragraphs usually costs no request at all.
//!
//! **One look, one request.** A page's candidate blocks go in together, numbered, and the
//! answers come back numbered. A quiz page with five questions costs what one answer costs,
//! and each card lands beside its own question rather than in a corner.
//!
//! **Nothing is remembered between looks.** The same page read twice is answered twice, on
//! purpose. A watcher that refuses to answer a question it has already answered is a watcher
//! whose behaviour cannot be reasoned about while it is being tuned -- "why did it say nothing
//! this time?" has no answer the user can check -- and a question written again is a question
//! someone is looking at again. The only thing counted across looks is the hourly budget,
//! which is a cost rather than a judgement about the text.
//!
//! **It never loads or evicts a local model.** Loading one would kill the model the user is
//! chatting with -- the app runs one at a time, and `parallel` is 1 -- so assist uses a model
//! that is *already up*, and says which one is missing when none is. That is why the
//! resolution below is stricter than the title chain's: a feature that reads the screen is not
//! worth losing a conversation over.
//!
//! **It draws, it never acts.** An answer is text. Nothing in this module can click, type,
//! write a file or call a tool, so there is no permission to grant and nothing an unattended
//! screen could be talked into doing.
//!
//! **What it spends is written down.** Every request that reaches a model is recorded in
//! `assist_usage`, so the statistics page can report what this feature costs beside everything
//! else -- and by model, which is the only way to answer "on what".

pub mod candidate;
/// A throwaway dump of a reply that could not be read. **Temporary** -- see the file itself for
/// what it writes, and delete the module with its call site in `decide.rs` when the failure it
/// was written for is understood.
pub mod debug;
pub mod decide;
pub mod offer;
pub mod settings;
pub mod source;
/// Arithmetic the app can answer itself. See the module for why that is not a model's job.
pub mod verify;

use std::sync::{Arc, Mutex};
use std::time::Instant;

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter};

use crate::ai::remote::client::ReportedUsage;
use crate::database::statistics::AssistUsage;

pub use candidate::{Shape, Stage};
pub use offer::Kind;
pub use settings::{AssistSettings, SETTINGS_KEY};

/// How many candidates one locate request may carry.
///
/// Smaller than the candidate cap on purpose: this is the pass that reads *every* block, and
/// a small model labels a short list better than a long one. A page of sixteen becomes two
/// locate requests, which is still cheaper than one request that gets the labels wrong.
const LOCATE_CHUNK: usize = 8;

/// One look at a source, to consider.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AssistRequest {
    /// The page as the source sees it: every block it could read, numbered by the source.
    pub frame: source::Frame,
    /// Which source it came from: `browser`, or `preview` for the app's own tests. Recorded
    /// rather than used: it is the first thing anyone asks when the feature misbehaves.
    #[serde(default)]
    pub source: String,
}

/// One card, for one block.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AssistCard {
    /// The block's page index, as the source numbered it -- **not** the number the model answered
    /// with, which is the block's place in the request and is mapped back here. The source uses it
    /// as a fallback only, when a card arrives without a key.
    pub index: usize,
    /// The block's identity, as the page named it. A number is a position, and a page that changes
    /// while a request is in flight can move it; the source places the card by this where it can,
    /// and drops a card it cannot place rather than drawing it over whatever the number became.
    #[serde(default)]
    pub key: String,
    pub text: String,
    pub subject: String,
    pub confidence: f32,
    /// What kind of help this is, which is what the card is titled as.
    pub kind: Kind,
    /// How the gate classified the block, for the card's own subtitle.
    pub shape: Shape,
    /// The source's name for the element, for the debug view.
    pub tag: String,
}

/// What came of one look at a page.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AssistAnswer {
    Answered {
        cards: Vec<AssistCard>,
        /// The model that answered, as the user chose it. Named because "which model said
        /// that" is the first question about a wrong answer.
        model: String,
        /// How many blocks the source offered, so the app can say "3 of 24".
        screened: usize,
    },
    Skipped {
        stage: Stage,
        reason: String,
        /// How many blocks the source offered. Carried on a skip as well, because a page where
        /// nothing was even a candidate and a page that was never read are different problems.
        screened: usize,
    },
    Failed {
        reason: String,
    },
}

impl AssistAnswer {
    /// The stage that produced it, for the socket's verdict frame.
    pub fn stage(&self) -> Stage {
        match self {
            AssistAnswer::Answered { .. } => Stage::Answered,
            AssistAnswer::Skipped { stage, .. } => *stage,
            AssistAnswer::Failed { .. } => Stage::Failed,
        }
    }

    /// A skip, with how much was looked at.
    fn skipped(stage: Stage, reason: impl Into<String>, screened: usize) -> Self {
        AssistAnswer::Skipped {
            stage,
            reason: reason.into(),
            screened,
        }
    }
}

/// One card, from one answered candidate.
///
/// Shared by the one-pass and detail paths so the page index, the block key and the shape
/// travel the same way whichever request produced the answer -- the difference between the two
/// paths is how the content was decided, never how it is addressed.
fn card_for(candidate: &candidate::Candidate, kind: Kind, answer: &decide::Answer) -> AssistCard {
    AssistCard {
        index: candidate.index,
        key: candidate.key.clone(),
        text: answer.text.clone(),
        subject: answer.subject.clone(),
        confidence: answer.confidence,
        kind,
        shape: candidate.shape,
        tag: candidate.tag.clone(),
    }
}

/// What the bridge knows about itself.
#[derive(Default)]
struct BridgeState {
    port: Option<u16>,
    error: Option<String>,
    /// The profile labels of the connections that have paired. A label rather than an id,
    /// because the user is being told *which browser* is attached and a UUID answers a
    /// question they did not ask.
    clients: Vec<String>,
}

struct AssistInner {
    database: Arc<crate::database::Database>,
    /// The extension folder, when this build can hand it over.
    ///
    /// `None` is a normal state -- an installed build that did not ship the folder -- and the
    /// status page says so rather than offering a download that would 404.
    extension: Option<std::path::PathBuf>,
    /// How many requests this session has sent, for the hourly limit.
    budget: Mutex<candidate::Budget>,
    bridge: Mutex<BridgeState>,
    /// Pushes the switch to every connected extension at once.
    updates: tokio::sync::broadcast::Sender<String>,
}

/// The assist watcher, as the app holds it.
#[derive(Clone)]
pub struct Assist {
    inner: Arc<AssistInner>,
}

impl Assist {
    /// `extension` is the folder the setup page hands out, when there is one.
    pub fn new(
        database: Arc<crate::database::Database>,
        extension: Option<std::path::PathBuf>,
    ) -> Self {
        Self {
            inner: Arc::new(AssistInner {
                database,
                extension,
                budget: Mutex::new(candidate::Budget::default()),
                bridge: Mutex::new(BridgeState::default()),
                // Small: the only thing sent on it is the switch, and a burst of state changes
                // is a burst of the same fact.
                updates: tokio::sync::broadcast::channel(8).0,
            }),
        }
    }

    /// Starts the socket the extension connects to.
    ///
    /// Spawned rather than awaited: a listener is a task for the life of the app, and the setup
    /// path must not wait on a port binding to open the window.
    pub fn start(&self, app: AppHandle, runtime: crate::ai::runtime::AiRuntime) {
        // A clone of the handle, not of the state: `Assist` is an `Arc` inside, so the task
        // shares the budget and the switch with every other holder.
        let assist = self.clone();
        tauri::async_runtime::spawn(async move {
            source::browser::bridge::serve(assist, runtime, app).await;
        });
    }

    /// The settings record as stored, normalized.
    pub fn settings(&self) -> AssistSettings {
        AssistSettings::load(&self.inner.database)
    }

    /// Writes the settings record, clamped.
    pub fn save(&self, settings: &AssistSettings) -> Result<AssistSettings, String> {
        let normalized = settings.clone().normalized();
        self.inner.database.set_setting(SETTINGS_KEY, &normalized)?;
        Ok(normalized)
    }

    /// Moves the one switch the tray, Settings and the extension all read.
    ///
    /// Everything else about assist is a threshold; this is the decision about whether it runs
    /// at all, so it is written down and then announced on both paths -- a window event for the
    /// app's own views, a socket frame for the browser -- from one place, so the three cannot
    /// drift apart.
    pub fn set_enabled(&self, app: &AppHandle, enabled: bool) -> Result<AssistSettings, String> {
        let mut settings = self.settings();
        settings.enabled = enabled;
        let settings = self.save(&settings)?;
        crate::sync_assist_toggle(app, settings.enabled);
        let _ = app.emit(
            "assist-event",
            serde_json::json!({ "kind": "state", "enabled": settings.enabled }),
        );
        if let Ok(text) = serde_json::to_string(&self.state_frame()) {
            // An error means nobody is connected, which is the normal state of a machine with
            // no extension paired.
            let _ = self.inner.updates.send(text);
        }
        Ok(settings)
    }

    /// What this sitting has spent against the hour's ceiling.
    ///
    /// Read by every frame the extension gets, because the browser is where the user is when
    /// the question "why has it gone quiet?" comes up, and "12 of 30 this hour" is the answer.
    /// Locked for the length of one `usize` count, so a caller may hold this while asking for
    /// nothing else.
    pub fn usage(&self) -> source::browser::Usage {
        usage_of(&self.settings(), self.calls_in_window())
    }

    /// Requests sent inside the current hour, as the budget counts them.
    ///
    /// The count on its own, so a caller that already holds the record -- the status page, a
    /// frame built from settings it has just read -- does not pay for a second read to get it.
    fn calls_in_window(&self) -> usize {
        self.inner
            .budget
            .lock()
            .map(|budget| budget.calls_in_window(Instant::now()))
            .unwrap_or(0)
    }

    /// The floor and the cut the browser has to apply before it sends anything.
    ///
    /// Taken from the record rather than kept as constants on the other side of the wire: the
    /// extension reads a page many times more often than this app sees one, so a floor of its
    /// own would be a second answer to a question the user can only change here.
    pub fn limits(&self) -> source::browser::Limits {
        limits_of(&self.settings())
    }

    /// The switch, the limits and the hour's spending as one frame.
    ///
    /// Built in one place because three doors read the same three facts -- the tray, the
    /// extension's popup and a ping -- and a second builder would be a second description of
    /// them, drifting one field at a time. The record is read once for both of its contributions
    /// to the frame; the spending is a lock and a count, not a read.
    pub fn state_frame(&self) -> source::browser::ServerFrame {
        let settings = self.settings();
        source::browser::ServerFrame::State {
            enabled: settings.enabled,
            limits: limits_of(&settings),
            usage: self.usage(),
        }
    }

    /// Everything the settings page and the tray draw.
    pub fn status(&self, runtime: &crate::ai::runtime::AiRuntime) -> AssistStatus {
        let settings = self.settings();
        // Read through the one accessor rather than building it here as well: the status page
        // and the settings page describe the same socket, and two builders would be two
        // descriptions of it.
        let bridge = self.bridge_status();
        let calls_past_hour = self.calls_in_window();
        AssistStatus {
            enabled: settings.enabled,
            dry_run: settings.dry_run,
            // The editable record travels with the derived one, so the settings page draws its
            // controls and its status from a single read and cannot show a threshold that has
            // moved since it was loaded.
            settings: settings.clone(),
            model: settings
                .model
                .as_ref()
                .map(|selection| model_label(selection, runtime)),
            // Whether the chosen model can be used *right now*, which is a different fact from
            // whether it is chosen -- and the one the user has to act on, because assist never
            // loads a model itself.
            model_ready: model_readiness(settings.model.as_ref(), runtime),
            suggested_subjects: settings::DEFAULT_SUBJECTS.to_vec(),
            bridge,
            calls_past_hour,
            level_bounds: bounds_of(&settings),
        }
    }

    /// The part of the status that needs no live state.
    ///
    /// **This is the page's fast path.** `status` also names the chosen local
    /// model -- which may mean walking the model list and statting files -- and
    /// reads the bridge and budget locks. None of that is needed to draw the
    /// controls, so the settings page gets the record and its derived bounds
    /// first and fills the live facts in when the full status arrives, instead of
    /// showing "Reading settings…" behind a file walk.
    pub fn settings_view(&self) -> AssistSettingsView {
        let settings = self.settings();
        AssistSettingsView {
            level_bounds: bounds_of(&settings),
            suggested_subjects: settings::DEFAULT_SUBJECTS.to_vec(),
            settings,
        }
    }

    /// What the socket is doing, for the settings page and for the setup page it serves.
    ///
    /// Locked and released in one place, because the status page reads it on every poll: a
    /// caller that held this lock while asking for anything else would be a stall in the one
    /// path that has to stay cheap.
    pub fn bridge_status(&self) -> BridgeStatus {
        let bridge = self.inner.bridge.lock().unwrap();
        BridgeStatus {
            running: bridge.port.is_some(),
            port: bridge.port,
            error: bridge.error.clone(),
            clients: bridge.clients.clone(),
        }
    }

    /// The extension folder this build can hand over, when it has one.
    pub fn extension_dir(&self) -> Option<std::path::PathBuf> {
        self.inner.extension.clone()
    }

    /// Runs the pipeline for one page and reports it to the app's own views.
    pub async fn ask(
        &self,
        app: &AppHandle,
        runtime: &crate::ai::runtime::AiRuntime,
        request: AssistRequest,
    ) -> AssistAnswer {
        let answer = self.evaluate(runtime, request).await;
        let _ = app.emit("assist-event", &answer);
        answer
    }

    /// The pipeline itself: gate, budget, resolve, ask, record.
    ///
    /// Separate from [`Self::ask`] so it can be exercised without a window -- these stages are
    /// the whole behaviour of the feature and every one of them is worth a test, which is not
    /// true of an emitted event.
    pub async fn evaluate(
        &self,
        runtime: &crate::ai::runtime::AiRuntime,
        request: AssistRequest,
    ) -> AssistAnswer {
        let settings = self.settings();
        if !settings.enabled {
            return AssistAnswer::skipped(Stage::Off, "assist is switched off", 0);
        }

        // The cheap gate first, in full, over the whole page. Nothing below this line runs for
        // the overwhelming majority of what a watcher looks at.
        let screen = candidate::screen_page(&request.frame, &settings);
        let screened = screen.considered;
        if screen.candidates.is_empty() {
            // Which of the two empty outcomes it was matters to the user: a page of labels that
            // were all too short needs a lower length floor, while a page of prose needs
            // nothing changed at all.
            let stage = if screened > 0 && screen.too_short == screened {
                Stage::TooShort
            } else {
                Stage::NoShape
            };
            return AssistAnswer::skipped(stage, screen.reason(), screened);
        }
        // What the gate let through, with where it came from. Logged rather than reported
        // because a candidate is not yet an outcome, and this is the line that answers "it says
        // it read something -- what?".
        tracing::debug!(
            source = %request.source,
            url = %request.frame.url,
            considered = screened,
            candidates = screen.candidates.len(),
            "assist page"
        );

        // **Arithmetic before a model is resolved.** A sum is a computation, not a judgement:
        // solving it here is exact, instant and free. It also means a page of sums is answered
        // with no model chosen, no request spent and no wait -- which is the one class of
        // question a small model is least needed for.
        let mut cards: Vec<AssistCard> = Vec::new();
        let mut pending: Vec<candidate::Candidate> = Vec::new();
        for candidate in screen.candidates {
            // A code sample at the top level is a `Passage`, and `2 * 3` on its own is not a
            // question even though it is arithmetic -- so a code block is left to the model,
            // which can say what the snippet is *about* rather than printing its value.
            let arithmetic =
                candidate.shape != Shape::Passage && !candidate.hint.eq_ignore_ascii_case("code");
            if arithmetic {
                if let Some(answer) = verify::solve(&candidate.text) {
                    cards.push(AssistCard {
                        index: candidate.index,
                        key: candidate.key.clone(),
                        text: answer,
                        subject: "math".to_string(),
                        confidence: 1.0,
                        kind: Kind::Answer,
                        shape: candidate.shape,
                        tag: candidate.tag.clone(),
                    });
                    continue;
                }
            }
            pending.push(candidate);
        }
        if pending.is_empty() {
            return AssistAnswer::Answered {
                cards,
                // No model was involved at all, and saying so is more useful than naming the
                // one that happens to be configured.
                model: "local arithmetic".to_string(),
                screened,
            };
        }

        let Some(selection) = settings.model.as_ref() else {
            return if cards.is_empty() {
                AssistAnswer::skipped(Stage::NoModel, "no assist model is chosen", screened)
            } else {
                // The sums were answerable without one. Draw those rather than losing them
                // because the page also held something the model would have had to read.
                AssistAnswer::Answered {
                    cards,
                    model: "local arithmetic".to_string(),
                    screened,
                }
            };
        };
        let (endpoint, model) = match self.resolve(runtime, selection) {
            Ok(resolved) => resolved,
            Err(reason) => {
                return if cards.is_empty() {
                    AssistAnswer::skipped(Stage::NoModel, reason, screened)
                } else {
                    AssistAnswer::Answered {
                        cards,
                        model: "local arithmetic".to_string(),
                        screened,
                    }
                };
            }
        };

        let mut spent = ReportedUsage::default();
        let mut failure: Option<String> = None;
        let mut reasoned = String::new();
        let mut requests = 0usize;
        let mut over_budget = false;
        let kinds = settings.allowed_kinds();
        let title = request.frame.title.as_str();

        if settings.locate() && kinds.len() > 1 {
            // **Pass A: locate.** Label only, over short slices. A 2B is good at this and bad
            // at "read the whole page and decide".
            let mut selected: Vec<(candidate::Candidate, Kind)> = Vec::new();
            // Whether the locator *failed* rather than answered. The two empty outcomes are
            // different: "none of this is worth helping with" is the model's decision and is
            // respected, while a reply that could not be read is not a decision at all.
            let mut locate_failed = false;
            for chunk in pending.chunks(LOCATE_CHUNK) {
                if requests >= settings.max_requests() {
                    break;
                }
                if !self.take_request(&settings) {
                    over_budget = true;
                    break;
                }
                requests += 1;
                match decide::locate(&endpoint, &model, chunk, &settings, title).await {
                    Ok((choices, usage)) => {
                        spent = spent.plus(usage);
                        for choice in choices {
                            if let Some(found) =
                                choice.index.checked_sub(1).and_then(|at| chunk.get(at))
                            {
                                selected.push((found.clone(), choice.kind));
                            }
                        }
                    }
                    Err(reason) => {
                        // A locator that failed is not a page with nothing on it. The
                        // single-pass path is the fallback, so a bad sample costs the extra
                        // request rather than the whole look.
                        tracing::info!(%reason, "the assist locate pass failed; asking in one pass");
                        locate_failed = true;
                        selected.extend(chunk.iter().cloned().map(|candidate| {
                            let kind = Kind::from_shape(candidate.shape);
                            (candidate, kind)
                        }));
                    }
                }
            }
            // **Only a failed locator falls back.** A locator that answered with no labels has
            // said that nothing on the page is worth helping with, which is the judgement this
            // feature depends on it being allowed to make -- re-asking all of it would spend
            // requests to overrule the very answer it was asked for.
            if selected.is_empty() && locate_failed {
                selected.extend(pending.iter().cloned().map(|candidate| {
                    let kind = Kind::from_shape(candidate.shape);
                    (candidate, kind)
                }));
            }
            // No page may produce more offers than the level allows, however many labels came
            // back. The list is already ranked strongest first.
            let offer_cap = settings
                .max_answers()
                .saturating_mul(settings.max_requests().max(1));
            selected.truncate(offer_cap);

            // **Pass B: detail.** One small slice at a time, with narrow context. The slice
            // size is `candidates_per_request`, which is not the reply cap: seeing more than
            // you may say is allowed, and is what lets one request answer the best of a batch.
            for chunk in selected.chunks(settings.candidates_per_request()) {
                if requests >= settings.max_requests() {
                    break;
                }
                if !self.take_request(&settings) {
                    over_budget = true;
                    break;
                }
                requests += 1;
                let batch: Vec<candidate::Candidate> = chunk
                    .iter()
                    .map(|(candidate, _)| candidate.clone())
                    .collect();
                let batch_kinds: Vec<Kind> = chunk.iter().map(|(_, kind)| *kind).collect();
                match decide::detail(&endpoint, &model, &batch, &batch_kinds, &settings, title)
                    .await
                {
                    Ok((verdict, usage)) => {
                        spent = spent.plus(usage);
                        // Each slice is a different kind, so an empty one says nothing about
                        // the next: unlike the one-pass path, the sequence continues.
                        for answer in &verdict.answers {
                            let Some((candidate, kind)) =
                                answer.index.checked_sub(1).and_then(|at| chunk.get(at))
                            else {
                                continue;
                            };
                            cards.push(card_for(candidate, *kind, answer));
                        }
                    }
                    Err(reason) => {
                        tracing::info!(%reason, "an assist detail request failed");
                        if cards.is_empty() {
                            failure = Some(reason);
                        }
                        break;
                    }
                }
            }
        } else {
            // **The one-pass levels.** The list is ranked strongest first, so an empty reply is
            // a judgement about the page rather than about one slice, and the sequence stops.
            for batch in pending.chunks(settings.candidates_per_request()) {
                if requests >= settings.max_requests() {
                    break;
                }
                if !self.take_request(&settings) {
                    over_budget = true;
                    break;
                }
                requests += 1;
                match decide::ask(&endpoint, &model, batch, &settings, title).await {
                    Ok((verdict, usage)) => {
                        spent = spent.plus(usage);
                        if verdict.answers.is_empty() {
                            reasoned = verdict.reason;
                            break;
                        }
                        // **The number in the reply is the block's place in this request**, not
                        // the page's index: the list is numbered 1..n because the page's own
                        // numbers are sparse and a small model copied the reply example's number.
                        // The page index and the block's key travel on the candidate.
                        for answer in &verdict.answers {
                            let Some(candidate) =
                                answer.index.checked_sub(1).and_then(|at| batch.get(at))
                            else {
                                continue;
                            };
                            cards.push(card_for(candidate, answer.kind, answer));
                        }
                    }
                    Err(reason) => {
                        tracing::info!(%reason, "an assist request failed");
                        // A failure with nothing to show is the look's outcome; a failure after a
                        // card was already drawn is not, because throwing those away to report
                        // the second request would take away an answer the user can see.
                        if cards.is_empty() {
                            failure = Some(reason);
                        }
                        break;
                    }
                }
            }
        }

        // **Written down whatever the outcome.** The requests reached a model and were paid for
        // whether or not they produced a card, so a decline is as much a row as an answer is.
        // One row per *look*, with the figures the whole sequence cost.
        if requests > 0 {
            self.record_usage(&endpoint, &model, &spent, screened, cards.len());
        }

        if cards.is_empty() {
            if let Some(reason) = failure {
                return AssistAnswer::Failed { reason };
            }
            if over_budget {
                // Whatever was already answered stays answered; this is reached only when the
                // look produced nothing at all.
                return AssistAnswer::skipped(
                    Stage::OverBudget,
                    format!(
                        "{} already sent in the last hour, which is this hour's limit",
                        self.calls_in_window()
                    ),
                    screened,
                );
            }
            return AssistAnswer::skipped(
                Stage::Declined,
                if reasoned.is_empty() {
                    "the model judged none of it worth helping with".to_string()
                } else {
                    reasoned
                },
                screened,
            );
        }
        AssistAnswer::Answered {
            cards,
            model,
            screened,
        }
    }

    /// Counts one request against the hour, or says the hour is spent.
    ///
    /// Counted *before* the request rather than after it, because what the budget protects is
    /// the spending: a failure costs the same request as an answer, and a run of failures must
    /// use up the hour like anything else.
    fn take_request(&self, settings: &AssistSettings) -> bool {
        let mut budget = self.inner.budget.lock().unwrap();
        let now = Instant::now();
        if budget.calls_in_window(now) >= settings.max_per_hour as usize {
            return false;
        }
        budget.note_call(now);
        true
    }

    /// Writes what one request cost.
    ///
    /// A failure to record is logged and dropped: the answer is the thing the user is waiting
    /// for, and a statistics row is not worth failing it over.
    fn record_usage(
        &self,
        endpoint: &crate::ai::remote::types::Endpoint,
        model: &str,
        usage: &ReportedUsage,
        blocks: usize,
        cards: usize,
    ) {
        let row = AssistUsage {
            created_at: crate::database::utc_now(),
            model_id: model.to_string(),
            // The endpoint is the provider for a remote selection and the local server for a
            // local one, which is the same key the per-model breakdown uses elsewhere.
            provider_id: endpoint.id.clone(),
            prompt_tokens: usage.prompt_tokens,
            completion_tokens: usage.completion_tokens,
            cached_tokens: usage.cached_tokens,
            blocks: blocks as i64,
            cards: cards as i64,
        };
        if let Err(error) = self.inner.database.record_assist_usage(&row) {
            tracing::warn!(%error, "could not record what assist spent");
        }
    }

    /// Turns the stored choice into a live endpoint, refusing to disturb a model.
    ///
    /// Stricter than `AiRuntime::resolve_selection`, which a local selection satisfies by
    /// loading the model. Here a local model must be **the** loaded one: the app runs a single
    /// `llama-server`, so loading assist's model would evict whichever model the user is talking
    /// to. A message naming what to load is a smaller failure than a conversation that loses its
    /// model mid-reply.
    fn resolve(
        &self,
        runtime: &crate::ai::runtime::AiRuntime,
        selection: &serde_json::Value,
    ) -> Result<(crate::ai::remote::types::Endpoint, String), String> {
        let wanted = selection
            .get("localModelId")
            .and_then(serde_json::Value::as_str)
            .filter(|id| !id.trim().is_empty());
        let Some(wanted) = wanted else {
            return runtime.resolve_selection(selection);
        };
        let loaded = runtime.local_models().status().loaded_model_id;
        match loaded {
            Some(loaded) if loaded == wanted => runtime
                .local_models()
                .loaded_endpoint()
                .ok_or_else(|| "the local model is still starting".to_string()),
            Some(_) => Err(
                "a different local model is loaded. Assist only uses a model that is already \
                 running, because loading one would unload the model your chat is using."
                    .to_string(),
            ),
            None => Err(
                "the local model assist is set to is not loaded. Load it in Settings > Local \
                 first; assist never loads or unloads a model itself."
                    .to_string(),
            ),
        }
    }

    /// The frame protocol's push channel, for anything that wants state changes.
    pub fn subscribe(&self) -> tokio::sync::broadcast::Receiver<String> {
        self.inner.updates.subscribe()
    }

    /// Whether a token is the one this machine issued.
    pub fn token_is_valid(&self, offered: &str) -> bool {
        let expected = source::browser::token(&self.inner.database);
        source::browser::token_matches(&expected, offered)
    }

    /// The pairing token itself, for the page the app serves to pair with.
    pub fn token(&self) -> String {
        source::browser::token(&self.inner.database)
    }

    /// Whether assist is switched on, for the page the app serves.
    pub fn is_enabled(&self) -> bool {
        self.settings().enabled
    }

    /// Records a paired connection.
    pub fn client_joined(&self, profile: Option<String>) {
        if let Ok(mut bridge) = self.inner.bridge.lock() {
            bridge.clients.push(profile.unwrap_or_default());
        }
    }

    /// Records a connection ending, whatever ended it.
    pub fn client_left(&self) {
        if let Ok(mut bridge) = self.inner.bridge.lock() {
            bridge.clients.pop();
        }
    }

    /// Records the port the bridge actually got.
    pub fn bridge_ready(&self, port: u16) {
        if let Ok(mut bridge) = self.inner.bridge.lock() {
            bridge.port = Some(port);
            bridge.error = None;
        }
    }

    /// Records a bridge that could not start, so the settings page can say why rather than
    /// showing a listener that never came up.
    pub fn bridge_failed(&self, error: &str) {
        if let Ok(mut bridge) = self.inner.bridge.lock() {
            bridge.port = None;
            bridge.error = Some(error.to_string());
        }
    }
}

/// The floor and the cut one record stands for, in the shape the extension reads them.
///
/// A free function over the record rather than a method on the handle, so a caller that has just
/// read the settings -- a frame that carries several of their facts -- builds this from what it
/// already holds instead of reading them again.
fn limits_of(settings: &AssistSettings) -> source::browser::Limits {
    source::browser::Limits {
        min_chars: settings.min_chars,
        max_ask_chars: settings.max_ask_chars,
    }
}

/// What one record's spending looks like, given the count for the last hour.
fn usage_of(settings: &AssistSettings, used: usize) -> source::browser::Usage {
    source::browser::Usage {
        used,
        limit: settings.max_per_hour,
    }
}

/// How a stored selection is named for the user.
fn model_label(selection: &serde_json::Value, runtime: &crate::ai::runtime::AiRuntime) -> String {
    if let Some(id) = selection
        .get("localModelId")
        .and_then(serde_json::Value::as_str)
    {
        return runtime
            .local_models()
            .stored()
            .into_iter()
            .find(|model| model.id == id)
            .map(|model| format!("{} (local)", model.name))
            .unwrap_or_else(|| "a local model that no longer exists".to_string());
    }
    let provider = selection
        .get("model")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("a model");
    match selection
        .get("endpointId")
        .and_then(serde_json::Value::as_str)
        .and_then(|id| runtime.endpoints().get(id))
    {
        Some(endpoint) => format!("{provider} via {}", endpoint.name),
        None => provider.to_string(),
    }
}

/// Whether the chosen model can be used right now, in the user's words.
///
/// `None` when no model is chosen at all, which the row states as its own thing rather than as
/// a failure: nothing is wrong, nothing has been set up yet.
fn model_readiness(
    selection: Option<&serde_json::Value>,
    runtime: &crate::ai::runtime::AiRuntime,
) -> Option<String> {
    let selection = selection?;
    let is_local = selection
        .get("localModelId")
        .and_then(serde_json::Value::as_str)
        .is_some_and(|id| !id.trim().is_empty());
    if !is_local {
        return Some("ready".to_string());
    }
    let wanted = selection
        .get("localModelId")
        .and_then(serde_json::Value::as_str)?;
    match runtime.local_models().status().loaded_model_id {
        Some(loaded) if loaded == wanted => Some("ready — the loaded model".to_string()),
        Some(_) => Some("waiting — another local model is loaded".to_string()),
        None => Some("not loaded".to_string()),
    }
}

/// The settings record plus everything derived from it without live state.
///
/// Separate from [`AssistStatus`] so the page can render before the bridge, the
/// budget and the local-model list have been read.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AssistSettingsView {
    pub settings: AssistSettings,
    pub level_bounds: LevelBounds,
    pub suggested_subjects: Vec<&'static str>,
}

/// What the settings page draws, and what the tray's item reflects.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AssistStatus {
    pub enabled: bool,
    pub dry_run: bool,
    /// The editable record, so the settings page reads its controls from the same call that
    /// reports their effect.
    pub settings: AssistSettings,
    /// The chosen model, named.
    pub model: Option<String>,
    /// Whether it can be used now, in the user's words.
    pub model_ready: Option<String>,
    /// The subject words the settings page offers as weights.
    pub suggested_subjects: Vec<&'static str>,
    pub bridge: BridgeStatus,
    /// How many requests were sent in the last hour, against the limit.
    pub calls_past_hour: usize,
    /// What the chosen level is actually made of, so the picker can describe a click instead of
    /// naming it and hoping. The bounds that are not editable are only visible here.
    pub level_bounds: LevelBounds,
}

/// The bounds one record stands for.
///
/// A free function over the record because both the full status and the fast
/// settings view need it, and two inline copies would be two descriptions of a
/// level that could drift from what the pipeline actually reads.
fn bounds_of(settings: &AssistSettings) -> LevelBounds {
    LevelBounds {
        max_candidates: settings.max_candidates(),
        candidates_per_request: settings.candidates_per_request(),
        max_answers: settings.max_answers(),
        max_ask_chars: settings.max_ask_chars,
        max_per_hour: settings.max_per_hour,
        max_tokens: settings.max_tokens(),
        max_requests: settings.max_requests(),
        unshaped: settings.offers_unshaped(),
        brief_prompt: settings.brief_prompt(),
        kinds: settings
            .allowed_kinds()
            .iter()
            .map(|kind| kind.as_str())
            .collect(),
        locate: settings.locate(),
        min_confidence: settings.min_confidence(),
    }
}

/// The bounds one record stands for, in the words the settings page needs.
///
/// Read from the same accessors the pipeline reads, so a description of a level
/// cannot disagree with what that level does.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LevelBounds {
    pub max_candidates: usize,
    /// How many candidates one request carries -- the number that decides how many requests a
    /// page costs at `max_answers` cards per reply.
    pub candidates_per_request: usize,
    pub max_answers: usize,
    pub max_ask_chars: usize,
    pub max_per_hour: u32,
    /// The reply's own ceiling -- and, for a model that thinks out loud, the budget for its
    /// thinking *and* its answer. The number to look at when a reply stops half-way.
    pub max_tokens: u32,
    /// How many requests one look may make: the answer cap is per reply, so this is what decides
    /// how much of a long short list is ever asked about.
    pub max_requests: usize,
    /// Whether a block the shape check could not name is offered at all.
    pub unshaped: bool,
    /// Whether the short prompt is used.
    pub brief_prompt: bool,
    /// The kinds of help this level may offer, in the prompts' own words.
    pub kinds: Vec<&'static str>,
    /// Whether a look runs the locate/detail pipeline.
    pub locate: bool,
    /// The floor an answer's confidence has to clear before it is drawn.
    pub min_confidence: f32,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BridgeStatus {
    /// Whether the socket came up. False means `error` says why.
    pub running: bool,
    pub port: Option<u16>,
    pub error: Option<String>,
    /// The profile labels of the paired browsers.
    pub clients: Vec<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::database::Database;

    /// A runtime backed by a throwaway database, with no local model loaded -- the same shape
    /// `ai::runtime`'s own tests use, so assist's tests exercise the real resolver rather than a
    /// stand-in.
    fn runtime(label: &str) -> crate::ai::runtime::AiRuntime {
        let database = Arc::new(
            Database::open(std::env::temp_dir().join(format!(
                "projectz-assist-test-{}-{label}.sqlite3",
                std::process::id()
            )))
            .expect("test database"),
        );
        database.initialize().expect("migrate test database");
        crate::ai::runtime::AiRuntime::new_with_database(std::env::temp_dir(), database)
    }

    fn assist(runtime: &crate::ai::runtime::AiRuntime) -> Assist {
        Assist::new(runtime.database(), None)
    }

    fn request(text: &str) -> AssistRequest {
        AssistRequest {
            frame: source::Frame::single(text),
            source: "preview".to_string(),
        }
    }

    fn page(texts: &[&str]) -> AssistRequest {
        AssistRequest {
            frame: source::Frame {
                id: "test".to_string(),
                url: "https://example.com/quiz".to_string(),
                title: "Quiz".to_string(),
                blocks: texts
                    .iter()
                    .enumerate()
                    .map(|(position, text)| source::Block {
                        index: position + 1,
                        key: format!("k{position}"),
                        text: (*text).to_string(),
                        tag: "p".to_string(),
                        hint: String::new(),
                        rect: None,
                    })
                    .collect(),
            },
            source: "browser".to_string(),
        }
    }

    fn stage_of(answer: &AssistAnswer) -> Stage {
        answer.stage()
    }

    /// Switched off is the first thing checked, and it is checked before the page is even looked
    /// at.
    #[tokio::test]
    async fn assist_that_is_off_answers_nothing() {
        let runtime = runtime("off");
        let answer = assist(&runtime)
            .evaluate(&runtime, request("The value of PI?"))
            .await;
        assert_eq!(stage_of(&answer), Stage::Off);
    }

    /// A page with no question on it stops at the gate, before a model is resolved or a request
    /// is spent -- and says how much was looked at.
    #[tokio::test]
    async fn a_page_of_prose_stops_at_the_gate() {
        let runtime = runtime("prose");
        let assist = assist(&runtime);
        let mut settings = assist.settings();
        settings.enabled = true;
        assist.save(&settings).expect("save");
        let answer = assist
            .evaluate(
                &runtime,
                page(&[
                    "The report is due on Friday and the numbers have been checked twice.",
                    "Note: the third paragraph needs rewriting before it is published.",
                ]),
            )
            .await;
        match answer {
            AssistAnswer::Skipped {
                stage,
                reason,
                screened,
            } => {
                assert_eq!(stage, Stage::NoShape);
                assert_eq!(screened, 2);
                assert!(reason.contains("2 blocks looked at"), "{reason}");
            }
            other => panic!("expected a skip, got {other:?}"),
        }
    }

    /// A page of labels that were all too short is reported as that, not as "not a question":
    /// the two need different settings changed.
    #[tokio::test]
    async fn a_page_of_labels_names_the_length_floor() {
        let runtime = runtime("labels");
        let assist = assist(&runtime);
        let mut settings = assist.settings();
        settings.enabled = true;
        assist.save(&settings).expect("save");
        let answer = assist
            .evaluate(&runtime, page(&["Home", "About", "Menu"]))
            .await;
        assert_eq!(stage_of(&answer), Stage::TooShort);
    }

    /// Assist on with nothing chosen says so by name rather than failing vaguely.
    #[tokio::test]
    async fn a_question_with_no_model_chosen_says_which_stage_stopped_it() {
        let runtime = runtime("no-model");
        let assist = assist(&runtime);
        let mut settings = assist.settings();
        settings.enabled = true;
        assist.save(&settings).expect("save");
        let answer = assist.evaluate(&runtime, request("The value of PI?")).await;
        match answer {
            AssistAnswer::Skipped {
                stage,
                reason,
                screened,
            } => {
                assert_eq!(stage, Stage::NoModel);
                assert_eq!(screened, 1);
                assert!(reason.contains("no assist model"));
            }
            other => panic!("expected a skip, got {other:?}"),
        }
    }

    /// **A page of sums is answered with no model and no request.** Arithmetic is a
    /// computation rather than a judgement, so the local solver answers it exactly and
    /// instantly; the prose question beside them still needs a model, and with none chosen it
    /// is simply left unanswered rather than losing the sums.
    #[tokio::test]
    async fn arithmetic_is_answered_locally_without_a_model() {
        let runtime = runtime("arithmetic");
        let assist = assist(&runtime);
        let mut settings = assist.settings();
        settings.enabled = true;
        settings.model = None;
        assist.save(&settings).expect("save");

        let answer = assist
            .evaluate(
                &runtime,
                page(&["14 - 8 =", "3x + 7 = 22", "The value of PI?"]),
            )
            .await;
        match answer {
            AssistAnswer::Answered {
                cards,
                model,
                screened,
            } => {
                assert_eq!(screened, 3);
                assert_eq!(model, "local arithmetic");
                let texts: Vec<&str> = cards.iter().map(|card| card.text.as_str()).collect();
                assert!(texts.contains(&"6"), "{texts:?}");
                assert!(texts.contains(&"x = 5"), "{texts:?}");
                assert_eq!(cards.len(), 2, "the prose block still needs a model");
                assert_eq!(cards[0].kind, Kind::Answer);
                assert_eq!(cards[0].confidence, 1.0);
                // Nothing was sent, so nothing is spent.
                assert_eq!(assist.status(&runtime).calls_past_hour, 0);
            }
            other => panic!("expected local answers, got {other:?}"),
        }
    }

    /// **The rule this feature is built around.** A local model that is not the loaded one is
    /// refused rather than loaded, because loading it would unload the model the user's chat is
    /// using -- and the refusal says what to do.
    #[tokio::test]
    async fn assist_never_loads_a_local_model_over_the_one_in_use() {
        let runtime = runtime("local");
        let assist = assist(&runtime);
        let mut settings = assist.settings();
        settings.enabled = true;
        settings.model = Some(serde_json::json!({ "localModelId": "some-model" }));
        assist.save(&settings).expect("save");

        let answer = assist.evaluate(&runtime, request("The value of PI?")).await;
        match answer {
            AssistAnswer::Skipped { stage, reason, .. } => {
                assert_eq!(stage, Stage::NoModel);
                assert!(
                    reason.contains("not loaded"),
                    "the reason should name the state: {reason}"
                );
                assert!(
                    reason.contains("never loads"),
                    "the reason should say what assist will not do: {reason}"
                );
            }
            other => panic!("expected a skip, got {other:?}"),
        }
    }

    /// A block under the length floor is named as such, so a user whose puzzles are all short
    /// knows exactly which setting to lower.
    #[tokio::test]
    async fn a_block_under_the_floor_names_the_floor() {
        let runtime = runtime("short");
        let assist = assist(&runtime);
        let mut settings = assist.settings();
        settings.enabled = true;
        assist.save(&settings).expect("save");
        let answer = assist.evaluate(&runtime, request("hi?")).await;
        assert_eq!(stage_of(&answer), Stage::TooShort);
    }

    /// **The page is never remembered.** Reading it twice answers it twice, which is the whole
    /// point of not carrying a memory: a question on screen is a question worth answering, every
    /// time it is on screen.
    ///
    /// The second look stops at the model for the same reason the first one did, which is what
    /// this pins: nothing about the first look changes the second one's outcome.
    #[tokio::test]
    async fn the_same_page_is_looked_at_the_same_way_every_time() {
        let runtime = runtime("no-memory");
        let assist = assist(&runtime);
        let mut settings = assist.settings();
        settings.enabled = true;
        assist.save(&settings).expect("save");
        let first = assist.evaluate(&runtime, request("The value of PI?")).await;
        let second = assist.evaluate(&runtime, request("The value of PI?")).await;
        assert_eq!(stage_of(&first), Stage::NoModel);
        assert_eq!(stage_of(&second), Stage::NoModel);
    }

    /// **The hour is spent by requests that were made, not by requests that could not be.** A model
    /// that cannot be resolved sends nothing, so nothing is counted against the hour -- which is
    /// the difference the budget is about: it protects the spending, and there is none here.
    #[tokio::test]
    async fn a_look_that_never_reaches_a_model_spends_nothing() {
        let runtime = runtime("no-model-budget");
        let assist = assist(&runtime);
        let mut settings = assist.settings();
        settings.enabled = true;
        settings.model = Some(serde_json::json!({ "endpointId": "nowhere", "model": "m" }));
        assist.save(&settings).expect("save");

        let answer = assist.evaluate(&runtime, request("The value of PI?")).await;
        assert_eq!(stage_of(&answer), Stage::NoModel);
        assert_eq!(assist.status(&runtime).calls_past_hour, 0);
    }

    /// The hourly limit is the one thing that stops a repeat, and it says so by name. The request
    /// here is *made* -- the endpoint exists and refuses the connection -- because that is what the
    /// budget counts, and a request that fails costs the same hour as one that answers.
    #[tokio::test]
    async fn the_hourly_limit_stops_repeats() {
        let runtime = runtime("budget");
        runtime
            .database()
            .save_endpoint(&crate::ai::remote::types::Endpoint {
                id: "unreachable".to_string(),
                name: "Unreachable".to_string(),
                // The discard port: refused at once, so the request is made and cannot answer.
                base_url: "http://127.0.0.1:9/v1".to_string(),
                api_key: String::new(),
                models: Vec::new(),
                disabled_models: Vec::new(),
                enabled: true,
            })
            .expect("save endpoint");

        let assist = assist(&runtime);
        let mut settings = assist.settings();
        settings.enabled = true;
        // The limit is only the record's while the record is the user's own: on a preset, the
        // preset owns that number and would put the middle level's thirty back. A test that
        // wants a limit of one has to say that it means `Custom`, which is exactly what the
        // settings page does the moment someone edits the field.
        settings.level = settings::Level::Custom;
        settings.max_per_hour = 1;
        settings.model = Some(serde_json::json!({ "endpointId": "unreachable", "model": "m" }));
        assist.save(&settings).expect("save");

        let first = assist.evaluate(&runtime, request("The value of PI?")).await;
        let second = assist.evaluate(&runtime, request("The value of PI?")).await;
        // The first request was made and failed; the second is not made at all.
        assert_eq!(stage_of(&first), Stage::Failed);
        assert_eq!(stage_of(&second), Stage::OverBudget);
        assert_eq!(assist.status(&runtime).calls_past_hour, 1);
    }

    /// The switch is stored, so a user who turned assist off because it was annoying does not
    /// find it on again after a restart.
    #[test]
    fn the_switch_is_remembered_between_launches() {
        let runtime = runtime("switch");
        let assist = assist(&runtime);
        assert!(!assist.is_enabled());
        let mut settings = assist.settings();
        settings.enabled = true;
        assist.save(&settings).expect("save");
        assert!(assist.is_enabled());
    }
}
