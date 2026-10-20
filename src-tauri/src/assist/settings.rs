//! What the assist watcher reads from Settings.
//!
//! # Why these are numbers with bounds rather than free fields
//!
//! Every one of them is a threshold the pipeline compares against on a path that
//! runs without the user watching: how long a block must sit still, how short a
//! block is too short to be a question, how many answers an hour is too many. A
//! value outside a sane range does not fail loudly -- it either silences the
//! feature or turns it into a machine that answers every line of every page -- so
//! [`AssistSettings::normalized`] clamps rather than trusting the stored record.
//! A record written by an older build, edited by hand, or truncated by a crash
//! lands on a working value instead of a broken feature.
//!
//! # Why `enabled` is stored rather than held in memory
//!
//! The tray's switch and the Settings row are two doors into one fact, and the
//! answer has to outlive the process: a user who turned this off because it was
//! annoying must not find it on again after a restart. Jobs hold their pause in
//! memory because a pause is about this session; being switched off is a
//! decision about the feature.
//!
//! # Why a level is a bundle of numbers rather than a second code path
//!
//! [`Level`] names how much this is allowed to cost, and every name resolves to
//! numbers the pipeline already reads. Nothing branches on the level below this
//! file: `Efficient` is not a cheaper pipeline, it is the same one given a
//! shorter prompt, a smaller reply budget and a lower cap. That is what keeps the
//! tests meaningful for every level at once, and it is why the record stores the
//! numbers *and* the name -- so the picker cannot show `Normal` while the pipeline
//! behaves like something else.
//!
//! `Custom` is the record as the user left it. Touching any number while a preset
//! is in force is a request to edit it, so the picker moves to `Custom` and the
//! stored numbers stay exactly as typed (clamped, never rewritten).
//!
//! # Why the gate is not one of the level's numbers
//!
//! `min_chars` is deliberately the same under every level. The shape check is what
//! decides whether something is a question, and a floor raised to save money would
//! silence the very pages the feature exists for -- `14 - 8 =` is seven characters,
//! and it is the canonical case in this module's own tests. What the level spends
//! is request size and request count, not what counts as a question.

use serde::{Deserialize, Serialize};

use super::offer::Kind;

/// Settings key holding the assist record.
pub const SETTINGS_KEY: &str = "app.assist";

/// One word describing what a candidate is about, when the model says so.
///
/// Free text on the wire -- the model names the subject in its own words -- but
/// the defaults below are what the settings page offers as weights, so a hint
/// and a stored weight use the same vocabulary.
pub const DEFAULT_SUBJECTS: [&str; 7] = [
    "math", "logic", "trivia", "language", "code", "science", "other",
];

/// Lower bound on `min_chars`.
///
/// Below this a "candidate" is a word, and a watcher that treats one word as a
/// question spends a request on every label on every page.
const MIN_CHARS_FLOOR: usize = 4;
/// Upper bound on `min_chars`. Above this only a long paragraph qualifies, which
/// is a watcher that never fires -- the user has effectively switched it off
/// without knowing it, so the scale stops short of that.
const MIN_CHARS_CEIL: usize = 400;

/// Shortest text worth sending when a block is capped.
const ASK_CHARS_FLOOR: usize = 120;
/// Longest text the model is ever shown for one candidate. A page's worth of
/// prose is not a question, and paying for it on every tick is the cost this
/// bound exists to stop. The cap is applied on a character boundary, so a
/// multi-byte character is never cut in half.
const ASK_CHARS_CEIL: usize = 4_000;

/// How many answers an hour is allowed, at least.
const PER_HOUR_FLOOR: u32 = 1;
/// How many answers an hour is allowed, at most.
///
/// Deliberately not unbounded: this feature spends money and a model's attention
/// on text nobody asked about, so the ceiling is what keeps a misread page from
/// turning into a hundred requests while the user is away from the desk.
const PER_HOUR_CEIL: u32 = 240;

/// The floor every level uses, and the one a fresh record starts on.
const MIN_CHARS_DEFAULT: usize = 6;

/// Most blocks one request may carry at the middle level.
///
/// Well past the number of questions a real page asks at once. It is a ceiling rather than a
/// target: it is applied *after* ordering by how strong the evidence is, so the eight that
/// survive are the eight likeliest questions on the page rather than the first eight.
pub const MAX_CANDIDATES: usize = 8;

/// Most blocks one reply may answer at the middle level.
///
/// Four cards already covers a quiz page, and a longer reply over a page someone is reading
/// is an intrusion rather than a help. `decide.rs` caps the reply by this as well as the
/// prompt asking for it, because the cap has to hold for a model that ignores instructions.
pub const MAX_ANSWERS: usize = 4;

/// The kinds of help each level may offer.
///
/// A level is not just a spend any more: it is also a *breadth*. The cheapest level answers
/// questions and nothing else, the middle one adds the short forms of help beside an answer,
/// and the top level offers everything -- including a summary of a section, which is the
/// noisiest offer and the one a small model is most likely to over-apply.
///
/// Never empty: a level that could offer nothing would be a level that switched the feature
/// off without saying so.
const EFFICIENT_KINDS: [Kind; 1] = [Kind::Answer];
const NORMAL_KINDS: [Kind; 4] = [Kind::Answer, Kind::Explain, Kind::Check, Kind::Translate];
const HIGH_KINDS: [Kind; 5] = Kind::ALL;

/// Most tokens one decision may produce at the middle level.
///
/// **Generous on purpose, and sized for a model that thinks out loud.** The ceiling is *not* the
/// answer's length: a model with a reasoning channel spends this budget on its thinking first and
/// writes its JSON afterwards, so a cap that fits the answer alone guarantees a reply that stops
/// half-way through a sentence -- which is what a local thinking model did at the 700 this used to
/// be, twice, on a page with two questions on it. A model that does not think spends a fraction of
/// this and stops when it is done, so the headroom costs nothing there.
///
/// It is still finite: the user waits for every token of it, and a model given no ceiling
/// deliberates for as long as it is allowed. `decide.rs` is what handles the reply that fills this
/// budget anyway -- its second attempt is made with thinking switched off.
const MAX_TOKENS: u32 = 1_500;

/// The top level's reply budget, which also has to carry a summary rather than one line.
const MAX_TOKENS_HIGH: u32 = 2_000;

/// Bounds on the numbers the Advanced panel now exposes.
///
/// They are clamped rather than trusted for the same reason as every other threshold: a
/// request carrying zero candidates, or a reply ceiling of ten million tokens, does not fail
/// loudly -- it either stalls the feature or turns one page into a bill. The candidate bounds
/// also carry two invariants that `normalized` enforces: a request cannot carry more candidates
/// than the look considers, and a reply cannot answer more than its request carried.
const CANDIDATES_FLOOR: usize = 1;
const CANDIDATES_CEIL: usize = 32;
const ANSWERS_FLOOR: usize = 1;
const ANSWERS_CEIL: usize = 16;
const TOKENS_FLOOR: u32 = 128;
const TOKENS_CEIL: u32 = 8_192;
const REQUESTS_FLOOR: usize = 1;
const REQUESTS_CEIL: usize = 18;

/// Most tokens one decision may produce at the cheapest level.
///
/// Lower, because that level sends a shorter prompt and fewer, shorter blocks: there is less to
/// deliberate over. Not *much* lower, for the reason above -- a thinking model still has to fit its
/// thinking and its answer in here, and a budget that only fits the answer is a budget that fails
/// every time on a model that thinks.
const MAX_TOKENS_EFFICIENT: u32 = 1_000;

/// How hard assist looks, and what one look is allowed to cost.
///
/// Every variant but [`Level::Custom`] is a bundle of numbers the pipeline already reads, so
/// nothing below this file branches on it -- see the module docs. The record stores the name
/// *and* the numbers, and [`AssistSettings::normalized`] settles a disagreement in favour of
/// the name, so a record claiming `Normal` cannot behave like something else.
///
/// The spelling on the wire is this enum's own, from serde: `efficient`, `normal`, `high`,
/// `custom`. A method returning the same strings would be a second definition of them, and the
/// settings page matches on what serde writes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Level {
    /// Fewer requests, smaller requests, and the shortest prompt that still works.
    Efficient,
    /// The default: the shapes the gate is tuned for, and the full prompt.
    Normal,
    /// Also offers blocks the shape check could not name, ranked behind the ones it could.
    High,
    /// The user's own numbers, exactly as they were left.
    Custom,
}

impl Level {
    /// What a record that names no level is taken to mean.
    ///
    /// `Custom` rather than `Normal`, and this is the one place it differs from
    /// [`AssistSettings::default`]: a record written before levels existed holds numbers this
    /// app never chose, and calling them `Normal` would put a name on them that a later click
    /// on `Normal` would appear not to change. A record that says nothing about a level is a
    /// record of the user's own numbers.
    fn unspecified() -> Self {
        Level::Custom
    }

    /// The numbers this level stands for. `None` for `Custom`, which stands for none.
    fn thresholds(self) -> Option<Thresholds> {
        match self {
            Level::Efficient => Some(Thresholds {
                max_ask_chars: 400,
                max_per_hour: 12,
                max_candidates: 4,
                // Four candidates fit one small request; the reply cap is the lower number,
                // because the cheap level is allowed to see more than it may say.
                candidates_per_request: 4,
                max_answers: 2,
                max_tokens: MAX_TOKENS_EFFICIENT,
                // One request a look: the cheap level is for a page with a question on it, not for
                // emptying a quiz.
                max_requests: 1,
                // A page's prose is never offered: the cheapest request is one whose blocks
                // are unambiguously questions.
                unshaped: false,
                brief: true,
                // Answers only. The cheapest level has no business explaining the page to a
                // reader who did not ask -- and a small model is least reliable exactly there.
                kinds: &EFFICIENT_KINDS,
                locate: false,
                min_confidence: 0.70,
            }),
            Level::Normal => Some(Thresholds {
                max_ask_chars: 1_200,
                max_per_hour: 30,
                max_candidates: MAX_CANDIDATES,
                candidates_per_request: MAX_ANSWERS,
                max_answers: MAX_ANSWERS,
                max_tokens: MAX_TOKENS,
                max_requests: 2,
                unshaped: false,
                brief: false,
                // The short forms of help: an answer, an explanation, a check, a translation.
                // Summarising a section stays at the top level, where "look at more of the page"
                // is what the reader asked for.
                kinds: &NORMAL_KINDS,
                locate: false,
                min_confidence: 0.55,
            }),
            Level::High => Some(Thresholds {
                max_ask_chars: 2_000,
                max_per_hour: 60,
                // The whole main content of a page, in one ranked list. This is the "send more
                // of the page" level, and it is the only one that admits prose.
                max_candidates: 16,
                // The detail pass sends a small slice at a time; the locate pass has its own
                // (smaller) chunk. Four is what a 2B reads well with a 2,000-character block.
                candidates_per_request: MAX_ANSWERS,
                // Not raised along with the candidate cap: how many answers may appear is a
                // judgement about intruding on someone's screen, not about cost.
                max_answers: MAX_ANSWERS,
                // Raised, because a summary is longer than an answer. It is still bounded: the
                // card is a glance, not an article.
                max_tokens: MAX_TOKENS_HIGH,
                // Two passes: a locate pass over the candidates, then a detail pass over what
                // it chose. Enough for sixteen candidates and four offers, plus a retry.
                max_requests: 6,
                unshaped: true,
                brief: false,
                // Everything, including a section summary -- the offer a small model over-applies,
                // which is why it is only here and only behind the strongest evidence first.
                kinds: &HIGH_KINDS,
                // The two-pass pipeline: the model points at what is worth helping with before
                // it is asked for the help. A locator is a far easier task for a 2B than
                // "read this page and decide", and the detail request that follows has a narrow
                // context instead of a whole page.
                locate: true,
                min_confidence: 0.40,
            }),
            Level::Custom => None,
        }
    }
}

/// What one level's name stands for.
///
/// The pipeline bounds live here rather than on [`AssistSettings`] on purpose: they are this
/// app's judgement about how to spend, not preferences, so they are not stored, not shown and
/// not clamped. The three user-visible numbers are copied onto the record, where the settings
/// page draws them as the fields they are.
#[derive(Debug, Clone, Copy)]
struct Thresholds {
    max_ask_chars: usize,
    max_per_hour: u32,
    max_candidates: usize,
    /// How many candidates one request may carry. See [`AssistSettings::candidates_per_request`].
    candidates_per_request: usize,
    max_answers: usize,
    max_tokens: u32,
    /// How many requests one look may make. See [`AssistSettings::max_requests`].
    max_requests: usize,
    /// Whether blocks the shape check could not name are offered at all.
    unshaped: bool,
    /// Whether the short prompt is used instead of the full one.
    brief: bool,
    /// The kinds of help this level may offer. See [`AssistSettings::allowed_kinds`].
    kinds: &'static [Kind],
    /// Whether the two-pass locate/detail pipeline is used.
    locate: bool,
    /// The lowest confidence an answer may have and still be drawn.
    min_confidence: f32,
}

impl Thresholds {
    /// The mid-level judgement, for `Custom`.
    ///
    /// A record that is not on a preset has no opinion about the bounds that are not
    /// settings, so it gets the ones this app would have chosen anyway.
    fn custom() -> Self {
        Level::Normal
            .thresholds()
            .expect("the middle level is a preset")
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct AssistSettings {
    /// Whether assist reads anything at all. Off until asked for.
    pub enabled: bool,
    /// The model that answers, in the same shape the other preferences use:
    /// `{endpointId, model}` for a provider, `{localModelId}` for a local model.
    ///
    /// `None` is a real state -- assist is on with nothing to answer with -- and
    /// the pipeline reports it by name rather than failing vaguely.
    pub model: Option<serde_json::Value>,
    /// Whether the model may think before it answers. Off by default, and measured.
    ///
    /// A model with a reasoning channel spends the reply budget on its thinking *before* it writes
    /// anything, and assist's task -- read a numbered list, answer in strict JSON, keep it short --
    /// is the one shape where that is usually a loss. Measured on a local 2B: thinking on cost
    /// 1500 tokens and 9.7 seconds and produced nothing, twice; the same page with thinking off
    /// cost 110 tokens and 0.63 seconds and answered correctly. A large model is a different
    /// story, which is why this is a setting rather than a rule.
    ///
    /// **It does not disable the retry.** A reply that is cut off at the ceiling is still asked
    /// again, and a reply that merely cannot be read is still asked again as it was; the one case
    /// this switches off is the *second* attempt of a truncation when thinking was already off,
    /// because there the retry would be the same request that just failed.
    pub thinking: bool,
    /// Free text appended to the prompt as a hint.
    ///
    /// A hint rather than a filter on purpose: a rule that only lets some
    /// subjects through is a feature that silently stops working, and the user
    /// cannot tell that from a page with nothing on it. Pointing the model at
    /// what the user cares about biases its judgement without ever being able to
    /// veto an answer.
    pub hint: String,
    /// Subjects the user is happy to be helped with, most wanted first.
    ///
    /// A weight, not a gate, for the same reason as [`Self::hint`]: an empty
    /// list means "no preference", not "nothing is allowed".
    pub subjects: Vec<String>,
    /// How much a look is allowed to cost.
    ///
    /// A bundle of the three numbers below, plus bounds that are not settings at all. Naming
    /// a preset here overwrites those three on the way in and on the way out, so this field
    /// and the numbers can never describe two different pipelines. A record that names no
    /// level is [`Level::Custom`] -- see [`Level::unspecified`].
    #[serde(default = "Level::unspecified")]
    pub level: Level,
    /// Shortest block that can be a candidate.
    ///
    /// Not part of any level: the floor decides what *counts* as a question, and every level
    /// leaves that judgement alone.
    pub min_chars: usize,
    /// Longest block that is sent, cut on a character boundary.
    pub max_ask_chars: usize,
    /// Most answers per hour.
    pub max_per_hour: u32,
    /// How many blocks one look considers at all.
    ///
    /// The short list's ceiling: a page can hold more, and the weakest are dropped by the cap.
    /// The Advanced panel exposes it, and a preset owns it.
    pub max_candidates: usize,
    /// How many candidates one request carries.
    ///
    /// **This is the number that decides how many requests a page costs.** Ten candidates at
    /// five per request is two requests, provided `max_requests` allows two; the reply cap
    /// (`max_answers`) is a separate bound and may be lower, letting the model see more than it
    /// may say. A value above `max_candidates` is clamped to it, and `max_answers` is clamped to
    /// this one, because a reply cannot answer more blocks than its request carried.
    pub candidates_per_request: usize,
    /// Most answers one reply may carry.
    ///
    /// A judgement about intruding on someone's screen as much as about cost: four cards
    /// already covers a quiz page. The parser enforces it, so a model that ignores the
    /// instruction cannot cover the page.
    pub max_answers: usize,
    /// The reply's token ceiling.
    ///
    /// Sized for a model that thinks out loud -- its thinking and its answer come out of the
    /// same budget. The Advanced panel changes it, and a preset owns it.
    pub max_tokens: u32,
    /// How many requests one look may make.
    ///
    /// The answer cap is per reply, so this is what decides how much of a long short list is
    /// ever asked about. Ten candidates at five per request needs two.
    pub max_requests: usize,
    /// The lowest confidence an answer may have and still be drawn.
    ///
    /// The model already reports how sure it is, and the prompt already tells it to stay
    /// quiet when unsure -- but a 2B's own judgement about when to decline is the least
    /// reliable thing it does, so the number is enforced rather than requested. A floor is
    /// the cheapest correctness control the feature has: a wrong card on someone's screen
    /// unasked costs more trust than ten useful ones earn.
    ///
    /// A preset owns this number, like the two above it. Editing it moves the record to
    /// `Custom`, so the picker cannot show one level over another level's floor.
    pub min_confidence: f32,
    /// The kinds of help this record may offer, in prompt order.
    ///
    /// Stored as well as derived because a record that moves to `Custom` must keep the breadth
    /// it had a moment ago: editing one number on `High` should not silently turn off summaries
    /// and the two-pass look. A preset still overwrites this on the way in and out, so the
    /// name and the behaviour cannot disagree.
    pub kinds: Vec<Kind>,
    /// Whether this record uses the two-pass locate/detail pipeline.
    ///
    /// Stored for the same reason as [`Self::kinds`]: `High` means "look at more of the page",
    /// and a custom floor must not quietly make that untrue.
    pub locate: bool,
    /// Run the whole pipeline but show nothing.
    ///
    /// The mode that answers "what would it have said?" without the feature
    /// interrupting anything, which is how the thresholds above get tuned from
    /// real pages instead of from guesses.
    pub dry_run: bool,
}

impl Default for AssistSettings {
    fn default() -> Self {
        // The middle level's own bundle rather than literals beside it, so a fresh record and
        // the `Normal` preset are the same numbers by construction rather than by agreement.
        let middle = Thresholds::custom();
        Self {
            // Off until the user asks. A feature that reads the screen has to be
            // switched on deliberately; defaulting it on would mean shipping a
            // watcher nobody agreed to.
            enabled: false,
            model: None,
            // Off by default: the measurement above is what a fresh install should land on, and a
            // switch that has to be turned *off* to make the feature fast is the wrong default.
            thinking: false,
            hint: String::new(),
            subjects: Vec::new(),
            level: Level::Normal,
            // Short, because the shortest thing worth answering is short: the
            // canonical case this feature exists for is `14 - 8 =`, and a
            // minimum above its length would mean a watcher that never fires on
            // the very pages it was built for. What a candidate must *look* like
            // is the job of the shape check, not of a length floor.
            min_chars: MIN_CHARS_DEFAULT,
            max_ask_chars: middle.max_ask_chars,
            // Timing -- when a page is still enough to read -- belongs to the extension,
            // which is the only thing that can see a page going quiet. These settings are the
            // app's judgement about what to *answer*, not about when to look.
            max_per_hour: middle.max_per_hour,
            max_candidates: middle.max_candidates,
            candidates_per_request: middle.candidates_per_request,
            max_answers: middle.max_answers,
            max_tokens: middle.max_tokens,
            max_requests: middle.max_requests,
            min_confidence: middle.min_confidence,
            kinds: middle.kinds.to_vec(),
            locate: middle.locate,
            dry_run: false,
        }
    }
}

impl AssistSettings {
    /// Reads the stored record, filling defaults for anything absent.
    ///
    /// A record that cannot be read is the defaults rather than an error: assist
    /// switched off is always a safe place to land, and a corrupt settings row
    /// must not stop the app opening.
    pub fn load(database: &crate::database::Database) -> Self {
        database
            .setting::<Self>(SETTINGS_KEY)
            .unwrap_or_default()
            .normalized()
    }

    /// Brings every threshold inside its range.
    ///
    /// Applied on the way in *and* on the way out, so a value seen by the
    /// pipeline is always one the pipeline can act on: clamping only at read time
    /// would leave a stored value that disagrees with the one on screen.
    pub fn normalized(mut self) -> Self {
        // The level is settled first, because a preset owns the numbers under it: the name is
        // the more recent statement -- only a deliberate choice can write `normal` -- so a
        // record that claims a preset and carries different numbers is a record whose numbers
        // are stale, and the preset wins. `Custom` overwrites nothing: it means exactly the
        // numbers stored beside it.
        if let Some(bundle) = self.level.thresholds() {
            self.max_ask_chars = bundle.max_ask_chars;
            self.max_per_hour = bundle.max_per_hour;
            self.max_candidates = bundle.max_candidates;
            self.candidates_per_request = bundle.candidates_per_request;
            self.max_answers = bundle.max_answers;
            self.max_tokens = bundle.max_tokens;
            self.max_requests = bundle.max_requests;
            self.min_confidence = bundle.min_confidence;
            // The breadth travels with the preset too, so a stored record always describes the
            // pipeline that will run and the settings page can show it without guessing.
            self.kinds = bundle.kinds.to_vec();
            self.locate = bundle.locate;
        } else if self.kinds.is_empty() {
            // A `Custom` record written before kinds existed: the middle level's breadth is the
            // honest default, rather than a record that could offer nothing.
            self.kinds = Thresholds::custom().kinds.to_vec();
        }
        // Deduplicated, keeping the order the record gave.
        let mut kinds: Vec<Kind> = Vec::new();
        for kind in &self.kinds {
            if !kinds.contains(kind) {
                kinds.push(*kind);
            }
        }
        self.kinds = kinds;
        self.min_chars = self.min_chars.clamp(MIN_CHARS_FLOOR, MIN_CHARS_CEIL);
        self.min_confidence = self.min_confidence.clamp(0.0, 1.0);
        self.max_ask_chars = self.max_ask_chars.clamp(ASK_CHARS_FLOOR, ASK_CHARS_CEIL);
        // A cap under the minimum would make every candidate both too short to
        // look at and too long to send, which is a feature that cannot fire.
        self.max_ask_chars = self.max_ask_chars.max(self.min_chars);
        self.max_per_hour = self.max_per_hour.clamp(PER_HOUR_FLOOR, PER_HOUR_CEIL);
        // **The three candidate numbers have an order, and it is enforced rather than assumed.**
        // A request cannot carry more than the look considers, and a reply cannot answer more
        // than its request carried -- so a hand-edited record or a step in the wrong direction
        // still describes a pipeline that can run.
        self.max_candidates = self.max_candidates.clamp(CANDIDATES_FLOOR, CANDIDATES_CEIL);
        self.candidates_per_request = self
            .candidates_per_request
            .clamp(CANDIDATES_FLOOR, CANDIDATES_CEIL)
            .min(self.max_candidates);
        self.max_answers = self
            .max_answers
            .clamp(ANSWERS_FLOOR, ANSWERS_CEIL)
            .min(self.candidates_per_request);
        self.max_tokens = self.max_tokens.clamp(TOKENS_FLOOR, TOKENS_CEIL);
        self.max_requests = self.max_requests.clamp(REQUESTS_FLOOR, REQUESTS_CEIL);
        // Trimmed and deduplicated, keeping the first spelling and the order the
        // user gave: the order is the preference, and a duplicate would weight a
        // subject twice.
        let mut subjects: Vec<String> = Vec::new();
        for subject in &self.subjects {
            let subject = subject.trim().to_lowercase();
            if !subject.is_empty() && !subjects.contains(&subject) {
                subjects.push(subject);
            }
        }
        subjects.truncate(24);
        self.subjects = subjects;
        self.hint = self.hint.trim().to_string();
        self
    }

    /// The bounds this record stands for, whichever way it names them.
    ///
    /// `Custom` falls back to the middle level's judgement, because a record that is not on a
    /// preset has said nothing about the bounds that are not settings.
    fn bounds(&self) -> Thresholds {
        self.level.thresholds().unwrap_or_else(Thresholds::custom)
    }

    /// How many blocks one look considers. Read by `candidate.rs`.
    ///
    /// Stored numbers are the record's only on `Custom`; a preset answers from its bundle even
    /// if the record has not been through [`Self::normalized`] yet, so the picker cannot show
    /// one level while another level's numbers run.
    pub fn max_candidates(&self) -> usize {
        match self.level {
            Level::Custom => self.max_candidates,
            _ => self.bounds().max_candidates,
        }
    }

    /// How many candidates one request carries. Read by `mod.rs` when it slices the list.
    pub fn candidates_per_request(&self) -> usize {
        match self.level {
            Level::Custom => self.candidates_per_request,
            _ => self.bounds().candidates_per_request,
        }
    }

    /// How many blocks one reply may answer. Read by `decide.rs`.
    pub fn max_answers(&self) -> usize {
        match self.level {
            Level::Custom => self.max_answers,
            _ => self.bounds().max_answers,
        }
    }

    /// How many tokens one decision may produce. Read by `decide.rs`.
    pub fn max_tokens(&self) -> u32 {
        match self.level {
            Level::Custom => self.max_tokens,
            _ => self.bounds().max_tokens,
        }
    }

    /// Whether a block the shape check could not name is offered as a candidate at all.
    pub fn offers_unshaped(&self) -> bool {
        self.bounds().unshaped
    }

    /// How many requests one look may make.
    ///
    /// **The answer cap is per request, not per page.** A page whose short list is longer than one
    /// reply can answer is a page with questions on it that would never be asked about: fifteen
    /// candidates against a cap of four is eleven questions silently dropped, however good the
    /// model is. So a look is a *sequence* of requests over the ranked list, each carrying the next
    /// slice, and this is how many slices may be spent. A page still costs one request when its
    /// strongest batch answers nothing -- the sequence stops there -- so the extra requests are
    /// spent only on pages that turn out to have questions.
    pub fn max_requests(&self) -> usize {
        match self.level {
            Level::Custom => self.max_requests,
            _ => self.bounds().max_requests,
        }
    }

    /// Whether the short prompt is used. Read by `decide.rs`.
    pub fn brief_prompt(&self) -> bool {
        self.bounds().brief
    }

    /// The kinds of help this record may offer. Read by the prompts and by `evaluate`.
    ///
    /// A preset answers from its bundle even if the record has not been through
    /// [`Self::normalized`] yet; `Custom` answers from the stored list, which `normalized`
    /// fills from the middle level when the record did not carry one.
    pub fn allowed_kinds(&self) -> &[Kind] {
        match self.level {
            Level::Custom => &self.kinds,
            _ => self.bounds().kinds,
        }
    }

    /// Whether a look uses the two-pass locate/detail pipeline.
    ///
    /// The top level does, and only it: pointing at what is worth helping with is a different
    /// request from asking for the help, and the second one benefits from the narrow context
    /// the first one leaves behind.
    pub fn locate(&self) -> bool {
        match self.level {
            Level::Custom => self.locate,
            _ => self.bounds().locate,
        }
    }

    /// The floor an answer's confidence has to clear before it is drawn.
    pub fn min_confidence(&self) -> f32 {
        self.min_confidence.clamp(0.0, 1.0)
    }

    /// The cap in characters, on a boundary a multi-byte character cannot be
    /// split by.
    pub fn cut(&self, text: &str) -> String {
        text.chars().take(self.max_ask_chars).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::assist::offer::Kind;
    use crate::database::Database;
    use std::sync::Arc;

    fn settings_from(json: serde_json::Value) -> AssistSettings {
        serde_json::from_value::<AssistSettings>(json)
            .expect("a partial record must deserialize")
            .normalized()
    }

    /// Assist starts switched off. The one default that matters most: a feature
    /// that reads the screen is not something a user should discover running.
    #[test]
    fn assist_starts_switched_off() {
        assert!(!AssistSettings::default().enabled);
    }

    /// **Thinking is off by default, and that is a measurement rather than a taste.** On a local 2B
    /// the thinking attempt spent the entire 1500-token budget and returned nothing, twice, while
    /// the same page answered in 110 tokens with thinking off -- so a fresh install should land on
    /// the fast one. A larger model is a different story, which is why the setting exists.
    #[test]
    fn assist_does_not_let_the_model_think_by_default() {
        assert!(!AssistSettings::default().thinking);
        // And it survives a round trip, like every other field.
        let stored = settings_from(serde_json::json!({ "thinking": true }));
        assert!(stored.thinking);
    }

    /// Every threshold is clamped on the way in, so a record edited by hand or
    /// written by an older build lands on a working value.
    #[test]
    fn out_of_range_thresholds_are_clamped_rather_than_trusted() {
        let settings = settings_from(serde_json::json!({
            "minChars": 0,
            "maxAskChars": 5,
            "maxPerHour": 100_000,
        }));
        assert_eq!(settings.min_chars, MIN_CHARS_FLOOR);
        assert_eq!(settings.max_ask_chars, MIN_CHARS_FLOOR.max(ASK_CHARS_FLOOR));
        assert_eq!(settings.max_per_hour, PER_HOUR_CEIL);

        let settings = settings_from(serde_json::json!({
            "minChars": 99_999,
            "maxAskChars": 99_999,
            "maxPerHour": 0,
        }));
        assert_eq!(settings.min_chars, MIN_CHARS_CEIL);
        assert_eq!(settings.max_ask_chars, ASK_CHARS_CEIL);
        assert_eq!(settings.max_per_hour, PER_HOUR_FLOOR);
    }

    /// A cap below the minimum would describe a candidate that is both too short
    /// to look at and too long to send, which is a watcher that can never fire.
    #[test]
    fn a_cap_under_the_minimum_is_raised_to_it() {
        let settings = settings_from(serde_json::json!({ "minChars": 400, "maxAskChars": 120 }));
        assert_eq!(settings.max_ask_chars, 400);
    }

    /// A partial record fills in defaults rather than failing, so a settings row
    /// written before a field existed still reads.
    #[test]
    fn a_partial_record_fills_in_the_defaults() {
        let settings = settings_from(serde_json::json!({ "enabled": true }));
        assert!(settings.enabled);
        assert_eq!(settings.min_chars, AssistSettings::default().min_chars);
        assert_eq!(settings.model, None);
    }

    /// Subjects are trimmed, lowercased and deduplicated in the order given: the
    /// order is the user's preference, and a duplicate would weight one twice.
    #[test]
    fn subjects_are_normalized_without_losing_their_order() {
        let settings = settings_from(serde_json::json!({
            "subjects": [" Math ", "math", "LOGIC", "", "  "],
        }));
        assert_eq!(settings.subjects, vec!["math", "logic"]);
    }

    /// Cutting a candidate counts characters, so a multi-byte character is never
    /// split into an invalid half.
    #[test]
    fn cutting_a_candidate_never_splits_a_character() {
        let settings = AssistSettings {
            max_ask_chars: 5,
            ..AssistSettings::default()
        };
        let cut = settings.cut("éééééééé");
        assert_eq!(cut.chars().count(), 5);
        assert_eq!(cut, "ééééé");
    }

    /// A record that is not there at all reads as the defaults.
    #[test]
    fn an_absent_record_reads_as_the_defaults() {
        let database = Database::open_in_memory();
        assert_eq!(AssistSettings::load(&database), AssistSettings::default());
        let _ = Arc::new(database);
    }

    /// Naming a level *is* choosing its numbers, whichever way the record arrived: the name is
    /// the more recent statement, and a stored number beside it is only ever the copy the
    /// settings page drew. Without this the picker could show `Efficient` over `Normal`'s
    /// spend, which is the one way a cost control can lie.
    #[test]
    fn a_level_is_a_bundle_and_the_name_wins() {
        let chosen = AssistSettings {
            level: Level::Efficient,
            max_ask_chars: ASK_CHARS_CEIL,
            max_per_hour: PER_HOUR_CEIL,
            ..AssistSettings::default()
        }
        .normalized();
        assert_eq!(chosen.max_ask_chars, 400);
        assert_eq!(chosen.max_per_hour, 12);

        // And over the wire, which is how the settings page writes it.
        let stored = settings_from(serde_json::json!({
            "level": "high",
            "maxAskChars": 120,
            "maxPerHour": 1,
        }));
        assert_eq!(stored.level, Level::High);
        assert_eq!(stored.max_ask_chars, 2_000);
        assert_eq!(stored.max_per_hour, 60);
    }

    /// **A record written before levels existed keeps the numbers it has.** Calling them
    /// `Normal` would put a name on a spend this app never chose, and the user's next click on
    /// `Normal` would then appear to change nothing.
    #[test]
    fn a_record_that_names_no_level_keeps_its_own_numbers() {
        let settings = settings_from(serde_json::json!({ "maxAskChars": 900, "maxPerHour": 7 }));
        assert_eq!(settings.level, Level::Custom);
        assert_eq!(settings.max_ask_chars, 900);
        assert_eq!(settings.max_per_hour, 7);
    }

    /// **No level may silence the case the feature exists for.** The floor is not a cost
    /// control: `14 - 8 =` is seven characters, and a level that raised the floor above it
    /// would be a level whose cheapest setting turns the feature off on real pages.
    #[test]
    fn no_level_raises_the_floor_above_a_sum() {
        let sum = "14 - 8 =";
        for level in [Level::Efficient, Level::Normal, Level::High] {
            let settings = AssistSettings {
                level,
                ..AssistSettings::default()
            }
            .normalized();
            assert_eq!(settings.min_chars, MIN_CHARS_DEFAULT);
            assert!(
                sum.chars().count() >= settings.min_chars,
                "{level:?} would ignore a sum"
            );
        }
    }

    /// `Custom` is the user's numbers for the three fields they can see, and the middle level's
    /// judgement for the bounds they cannot -- a record that is not on a preset has said
    /// nothing about those, and inventing a cheaper pipeline from them would be a silent
    /// behaviour change.
    #[test]
    fn custom_borrows_the_pipeline_bounds_from_the_middle_level() {
        let custom = AssistSettings {
            level: Level::Custom,
            max_ask_chars: 900,
            ..AssistSettings::default()
        }
        .normalized();
        let middle = AssistSettings::default();
        assert_eq!(custom.max_candidates(), middle.max_candidates());
        assert_eq!(custom.max_answers(), middle.max_answers());
        assert_eq!(custom.max_tokens(), middle.max_tokens());
        assert!(!custom.offers_unshaped());
        assert!(!custom.brief_prompt());
    }

    /// **The three candidate numbers are clamped into an order that can run.** A hand-edited
    /// Custom record is the case this exists for: a request cannot carry more than the look
    /// considers, and a reply cannot answer more than its request carried.
    #[test]
    fn custom_numbers_are_clamped_and_kept_in_order() {
        let settings = AssistSettings {
            level: Level::Custom,
            max_candidates: 10,
            candidates_per_request: 20,
            max_answers: 30,
            max_tokens: 99_999,
            max_requests: 99,
            ..AssistSettings::default()
        }
        .normalized();
        assert_eq!(settings.max_candidates(), 10);
        assert_eq!(settings.candidates_per_request(), 10);
        assert_eq!(settings.max_answers(), 10);
        assert_eq!(settings.max_tokens(), TOKENS_CEIL);
        assert_eq!(settings.max_requests(), REQUESTS_CEIL);

        // And the floors, so a cleared field cannot describe a request carrying nothing.
        let settings = AssistSettings {
            level: Level::Custom,
            max_candidates: 0,
            candidates_per_request: 0,
            max_answers: 0,
            max_tokens: 0,
            max_requests: 0,
            ..AssistSettings::default()
        }
        .normalized();
        assert_eq!(settings.max_candidates(), CANDIDATES_FLOOR);
        assert_eq!(settings.candidates_per_request(), CANDIDATES_FLOOR);
        assert_eq!(settings.max_answers(), ANSWERS_FLOOR);
        assert_eq!(settings.max_tokens(), TOKENS_FLOOR);
        assert_eq!(settings.max_requests(), REQUESTS_FLOOR);
    }

    /// **A level is a breadth as well as a spend.** The cheap level answers questions; the
    /// middle adds the short forms of help; the top adds a summary and the two-pass pipeline.
    /// The confidence floor moves the other way: the cheaper the level, the more sure it must
    /// be before it speaks.
    #[test]
    fn a_level_decides_the_kinds_and_the_confidence_floor() {
        let bundle = |level: Level| {
            AssistSettings {
                level,
                ..AssistSettings::default()
            }
            .normalized()
        };
        assert_eq!(
            bundle(Level::Efficient).allowed_kinds(),
            &[Kind::Answer][..]
        );
        assert!(bundle(Level::Normal)
            .allowed_kinds()
            .contains(&Kind::Explain));
        assert!(!bundle(Level::Normal)
            .allowed_kinds()
            .contains(&Kind::Summarize));
        assert!(bundle(Level::High)
            .allowed_kinds()
            .contains(&Kind::Summarize));
        assert!(!bundle(Level::Efficient).locate());
        assert!(!bundle(Level::Normal).locate());
        assert!(bundle(Level::High).locate());
        assert!(bundle(Level::Efficient).min_confidence() > bundle(Level::High).min_confidence());
    }

    /// The names mean what they say, and a later edit cannot quietly invert them: `Efficient`
    /// is the cheapest on every bound at once, and `High` is the only level that offers a block
    /// the shape check could not name.
    #[test]
    fn the_levels_are_ordered_from_cheapest_to_dearest() {
        let bundle = |level: Level| {
            AssistSettings {
                level,
                ..AssistSettings::default()
            }
            .normalized()
        };
        let (efficient, normal, high) = (
            bundle(Level::Efficient),
            bundle(Level::Normal),
            bundle(Level::High),
        );
        assert!(efficient.max_ask_chars <= normal.max_ask_chars);
        assert!(normal.max_ask_chars <= high.max_ask_chars);
        assert!(efficient.max_per_hour <= normal.max_per_hour);
        assert!(normal.max_per_hour <= high.max_per_hour);
        assert!(efficient.max_candidates() <= normal.max_candidates());
        assert!(normal.max_candidates() <= high.max_candidates());
        assert!(efficient.candidates_per_request() <= normal.candidates_per_request());
        assert!(normal.candidates_per_request() <= high.candidates_per_request());
        assert!(efficient.max_tokens() <= normal.max_tokens());
        assert!(normal.max_tokens() <= high.max_tokens());
        assert!(efficient.max_answers() <= normal.max_answers());
        assert!(efficient.max_requests() <= normal.max_requests());
        assert!(normal.max_requests() <= high.max_requests());
        assert!(efficient.brief_prompt() && !normal.brief_prompt() && !high.brief_prompt());
        assert!(!efficient.offers_unshaped() && !normal.offers_unshaped());
        assert!(high.offers_unshaped());
    }
}
