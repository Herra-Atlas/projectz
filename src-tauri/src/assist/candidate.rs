//! Deciding what is worth sending, before anything is sent.
//!
//! # Why this file exists at all
//!
//! Everything downstream of it costs money, latency and the user's patience, and a
//! watcher reads a screen many times more often than it should ever speak. So the
//! cheap questions are asked first, in one place, as pure functions over a string:
//! *is this long enough to mean anything*, *does it look like a question*, *have I
//! seen it already*, *have I answered too much this hour*. A block that fails any of
//! them never reaches a model, and a page whose blocks all fail never costs a
//! request at all.
//!
//! # It screens a page, not a paragraph
//!
//! The unit is a [`Frame`]: everything a source can see, one entry per block. The
//! gate's job is to turn that into the **short list** -- the handful of blocks that
//! could plausibly be a question -- and the request carries the short list, numbered
//! as the page numbered it. A page of forty paragraphs usually becomes a short list
//! of none, which is one comparison per paragraph and no request.
//!
//! # Two things a level moves, and nothing else
//!
//! The cap on how many blocks survive, and whether a block the shape check could not name
//! is admitted at all ([`Shape::Passage`], top level only). The floor is not one of them:
//! what *counts* as a question is not a cost setting, and raising it to spend less would
//! silence the pages the feature exists for. See [`super::settings::Level`].
//!
//! # Why the shape check is a heuristic and not a classifier
//!
//! A second model to decide whether to ask the first would cost a request per tick
//! -- exactly what this stage exists to avoid -- and it would still be guessing. A
//! handful of structural cues (a digit beside an operator, a question mark, a few
//! short lines of clues) is free, runs in microseconds, and is wrong in the
//! direction that costs nothing: a missed puzzle is a puzzle the user can still ask
//! about, while a false positive is the app interrupting them. The model gets the
//! last word -- [`Shape`] decides what is worth *asking* about, not what is worth
//! answering.
//!
//! # Why nothing is remembered between looks
//!
//! There is deliberately no memory of what has already been answered. A watcher that refuses
//! to answer the same question twice is one whose behaviour is hard to reason about while it
//! is being tuned -- "why did it say nothing this time?" has no answer the user can check --
//! and a question written again is a question someone is looking at again. The only thing
//! carried across looks is the hourly budget, which is a cost rather than a judgement about
//! the text.
//!
//! Within one request, the same words twice are one block: that is a rule about the page in
//! front of the reader, not a memory of the past.

use std::collections::VecDeque;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use super::settings::AssistSettings;
#[cfg(test)]
use super::source::Block;
use super::source::{Frame, Rect};

/// Most blocks from one page that may go into one request.
///
/// The middle level's cap, defined once in [`super::settings`] because a level is what decides
/// it, and re-exported here because this is the file that applies it.
// Only the tests below read the constant; the pipeline reads the level's number through
// `AssistSettings::max_candidates`, so this re-export does not need to exist in a release.
#[cfg(test)]
pub use super::settings::MAX_CANDIDATES;

/// What a candidate looked like structurally, for the settings page's debug view.
///
/// Kept as part of the answer rather than thrown away: when the feature does not
/// fire, "the shape was `question` and the model declined" and "the shape was
/// nothing at all" are different problems with different fixes, and the user can
/// only tell them apart if the stage that decided is named.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Shape {
    /// Digits beside an operator: `14 - 8 =`, `7 multiplied by 87`.
    Arithmetic,
    /// A question mark, or an interrogative opener.
    Question,
    /// A few short lines of clues that add up to one answer.
    Riddle,
    /// Long enough to be worth reading, and shaped like nothing in particular.
    ///
    /// **Only ever offered at the top level**, where "look at more of the page" is the point:
    /// the gate cannot name what this is, so it goes in behind everything it can name and is
    /// the first thing a cap drops. It is a candidate at all because a question can be dressed
    /// as prose -- a paragraph ending in a colon, a "how many ..." written out as a sentence --
    /// and the alternative at that level is not sending the page at all.
    Passage,
}

impl Shape {
    /// The wire name, the same one serde writes.
    ///
    /// A method as well as a serialization because a card frame is built by hand from a
    /// value this app already holds, and the extension's own labels are keyed by this
    /// spelling -- so the one word is defined once and cannot drift from the payload.
    pub fn as_str(self) -> &'static str {
        match self {
            Shape::Arithmetic => "arithmetic",
            Shape::Question => "question",
            Shape::Riddle => "riddle",
            Shape::Passage => "passage",
        }
    }

    /// How much this shape is evidence of a question, for the short list's order.
    ///
    /// A sum and a riddle are almost always someone testing themselves or being
    /// tested; a question mark is commoner and weaker on its own, and a passage is
    /// not evidence at all -- it is what the page is made of, admitted only so the
    /// model can look at it. Used only to choose which candidates survive the cap.
    fn weight(self) -> u8 {
        match self {
            Shape::Arithmetic | Shape::Riddle => 2,
            Shape::Question => 1,
            Shape::Passage => 0,
        }
    }
}

/// The stage that stopped a frame, so the user can see where it stopped.
///
/// One name per reason rather than a sentence, because these are counted and
/// compared: the settings page reports "rejected: not a question (412)" and that
/// number is what tells the user which threshold to move.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Stage {
    /// Assist is switched off.
    Off,
    /// Every block was shorter than the minimum, so none was a candidate at all.
    TooShort,
    /// Nothing on the page looked like a question.
    NoShape,
    /// The hourly budget is spent.
    OverBudget,
    /// Assist is on but no model resolves.
    NoModel,
    /// The model judged none of the blocks worth answering.
    Declined,
    /// The request to the model failed.
    Failed,
    /// The model answered at least one block.
    Answered,
}

impl Stage {
    pub fn as_str(self) -> &'static str {
        match self {
            Stage::Off => "off",
            Stage::TooShort => "too_short",
            Stage::NoShape => "no_shape",
            Stage::OverBudget => "over_budget",
            Stage::NoModel => "no_model",
            Stage::Declined => "declined",
            Stage::Failed => "failed",
            Stage::Answered => "answered",
        }
    }
}

/// Why a block was not sent.
#[derive(Debug, Clone, Serialize)]
pub struct Rejected {
    pub stage: Stage,
    pub reason: String,
}

impl Rejected {
    pub fn new(stage: Stage, reason: impl Into<String>) -> Self {
        Self {
            stage,
            reason: reason.into(),
        }
    }
}

/// A block that passed the cheap checks and is worth one line in the request.
#[derive(Debug, Clone)]
pub struct Candidate {
    /// The block's own number, carried through untouched so the answer can be
    /// matched back to the element it came from.
    pub index: usize,
    /// The block's identity, carried through untouched for the same reason -- and because a number
    /// is only a position, which the page can move out from under an answer that is in flight.
    pub key: String,
    /// The source's name for the element, for the debug view.
    pub tag: String,
    /// What the source already knows about the block's structure in the page.
    ///
    /// `code`, `table`, `image`, `heading`, or empty. Carried through to the prompt so a
    /// small model does not have to infer from the words whether a line is a snippet or a
    /// sentence -- which is the distinction `check` versus `explain` turns on.
    pub hint: String,
    /// The normalized text, already cut to the configured cap.
    pub text: String,
    pub shape: Shape,
    /// How far from the reader's screen the block sat when the page was read, in CSS pixels.
    ///
    /// Used only to break ties between candidates of the same shape. A page can hold twenty
    /// questions -- a chat, a quiz, a long article -- and the cap has to keep the ones the reader is
    /// looking at rather than the oldest ones in the document. `i64::MAX` is "the source has no
    /// geometry", which sorts such candidates last among their shape, exactly where they were.
    pub distance: i64,
}

/// What one look at a page came to.
///
/// Every block is accounted for, because "nothing happened" is the commonest result
/// and the only useful answer to it says *how much* was looked at: a page where
/// nothing was even a candidate is a different thing from a page whose one candidate
/// the model declined, and the user can only act on the difference if it is
/// reported.
#[derive(Debug, Default, Clone)]
pub struct Screen {
    /// How many blocks the source offered.
    pub considered: usize,
    /// How many were under the length floor.
    pub too_short: usize,
    /// How many were long enough and looked like nothing in particular.
    pub not_a_question: usize,
    /// How many passed the gate and were dropped by the cap.
    pub over_cap: usize,
    pub candidates: Vec<Candidate>,
}

impl Screen {
    /// The short list as a sentence, for the app's own report of a skip.
    pub fn reason(&self) -> String {
        if self.considered == 0 {
            return "nothing on the page could be read".to_string();
        }
        let mut reason = format!(
            "{} looked at: {} too short, {} not a question",
            count(self.considered, "block"),
            self.too_short,
            self.not_a_question
        );
        if self.over_cap > 0 {
            reason.push_str(&format!(", {} over the per-page limit", self.over_cap));
        }
        reason
    }
}

fn count(count: usize, noun: &str) -> String {
    if count == 1 {
        format!("1 {noun}")
    } else {
        format!("{count} {noun}s")
    }
}

/// The window the hourly budget counts over.
pub const BUDGET_WINDOW: Duration = Duration::from_secs(60 * 60);

/// How many requests have been sent recently.
///
/// **The only thing assist remembers, and it remembers it for one hour.** Nothing here is
/// keyed by what was asked or answered: there is deliberately no list of puzzles already seen,
/// because a watcher that refuses to answer the same question twice is a watcher whose
/// behaviour is hard to reason about while it is being tuned -- and the same question written
/// again is a question someone is looking at again.
///
/// In memory rather than in the database for the same reason: this is about the sitting, and
/// a restart is a new one.
#[derive(Default)]
pub struct Budget {
    calls: VecDeque<Instant>,
}

impl Budget {
    /// How many requests have been sent inside the window.
    pub fn calls_in_window(&self, now: Instant) -> usize {
        self.calls
            .iter()
            .filter(|at| now.saturating_duration_since(**at) < BUDGET_WINDOW)
            .count()
    }

    /// Counts one sent request.
    ///
    /// Counted when the request is *made* rather than when it succeeds, because what the
    /// budget protects is the spending: a failure costs the same request as an answer.
    pub fn note_call(&mut self, now: Instant) {
        self.calls.push_back(now);
        while self
            .calls
            .front()
            .is_some_and(|at| now.saturating_duration_since(*at) >= BUDGET_WINDOW)
        {
            self.calls.pop_front();
        }
    }
}

/// The first gate: the short list from a whole page.
///
/// Pure over a frame and the settings, so every rule below is testable without a
/// browser, a model or a clock.
pub fn screen_page(frame: &Frame, settings: &AssistSettings) -> Screen {
    let mut screen = Screen {
        considered: frame.blocks.len(),
        ..Screen::default()
    };
    for block in &frame.blocks {
        match screen_text(&block.text, settings) {
            Err(rejected) => match rejected.stage {
                Stage::TooShort => screen.too_short += 1,
                _ => screen.not_a_question += 1,
            },
            Ok((text, shape)) => {
                let text = settings.cut(&text);
                // The same paragraph twice on one page -- a heading repeated in a card, a
                // quote echoed in a comment -- is one candidate. This is a rule about *one
                // request*, not a memory: the same words would otherwise be sent twice and
                // answered twice, for two cards over one question. Nothing is carried between
                // one look at a page and the next.
                if screen
                    .candidates
                    .iter()
                    .any(|candidate| candidate.text == text)
                {
                    continue;
                }
                screen.candidates.push(Candidate {
                    index: block.index,
                    key: block.key.clone(),
                    tag: block.tag.clone(),
                    hint: block.hint.clone(),
                    text,
                    shape,
                    distance: distance_of(block.rect),
                });
            }
        }
    }
    // Strongest evidence first, and within one shape the blocks nearest the reader's screen. The
    // cap is a budget, so it should buy what the person is looking at rather than the top of the
    // document: on a chat, twenty questions above the one being read would otherwise fill it. A
    // level may raise the cap; the order is what keeps that safe, because the blocks a raised cap
    // admits are the weakest ones.
    screen.candidates.sort_by_key(|candidate| {
        (
            std::cmp::Reverse(candidate.shape.weight()),
            candidate.distance,
            candidate.index,
        )
    });
    let cap = settings.max_candidates();
    if screen.candidates.len() > cap {
        screen.over_cap = screen.candidates.len() - cap;
        screen.candidates.truncate(cap);
    }
    screen
}

/// How far outside the reader's screen a block sat, in pixels. `i64::MAX` when the source sent no
/// geometry, which puts a candidate last among its shape rather than first.
fn distance_of(rect: Option<Rect>) -> i64 {
    rect.map(|rect| rect.y.abs() as i64).unwrap_or(i64::MAX)
}

/// The first gate for one block: whether it is worth one line of a request.
pub fn screen_text(text: &str, settings: &AssistSettings) -> Result<(String, Shape), Rejected> {
    let normalized = normalize(text);
    let length = normalized.chars().count();
    if length < settings.min_chars {
        return Err(Rejected::new(
            Stage::TooShort,
            format!(
                "{} characters, under the {} this needs",
                length, settings.min_chars
            ),
        ));
    }
    let Some(shape) = shape_of(&normalized) else {
        // The top level admits a block the gate could not name, as [`Shape::Passage`]: at that
        // level "send more of the page" is the instruction, and a block that is long enough to
        // read is what a page is made of. Every other level refuses it by name, which is the
        // direction this gate is tuned in -- a missed puzzle is still there to ask about, while
        // a card on someone's screen unasked is the app interrupting them.
        if settings.offers_unshaped() {
            return Ok((normalized, Shape::Passage));
        }
        return Err(Rejected::new(
            Stage::NoShape,
            "nothing about it reads as a question".to_string(),
        ));
    };
    Ok((normalized, shape))
}

/// Screens one block on its own, as a page of one.
///
/// The tests' door into the gate, and only theirs: everything that reads a real screen
/// goes through [`screen_page`], and a second public way in would be a second thing to
/// keep in step with it.
#[cfg(test)]
pub fn screen(text: &str, settings: &AssistSettings) -> Result<Candidate, Rejected> {
    let (normalized, shape) = screen_text(text, settings)?;
    Ok(Candidate {
        index: 1,
        key: String::new(),
        tag: "preview".to_string(),
        hint: String::new(),
        text: settings.cut(&normalized),
        shape,
        distance: 0,
    })
}

/// Lays text out as the block it is: one line per line, no layout noise.
///
/// Whitespace inside a line is collapsed and blank lines are dropped, because the
/// text arrives from a rendered page where indentation and blank space carry the
/// DOM's structure rather than the writer's. Newlines are *kept*: the shape of a
/// riddle is its lines, and flattening them would leave a run of clauses that reads
/// as neither a riddle nor a question.
pub fn normalize(text: &str) -> String {
    text.lines()
        .map(|line| line.split_whitespace().collect::<Vec<_>>().join(" "))
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

/// What kind of question this is, if it is one.
pub fn shape_of(text: &str) -> Option<Shape> {
    if looks_arithmetic(text) {
        return Some(Shape::Arithmetic);
    }
    if looks_like_a_riddle(text) {
        return Some(Shape::Riddle);
    }
    if looks_like_a_question(text) {
        return Some(Shape::Question);
    }
    None
}

/// Digits beside an operator, written either way.
///
/// The spelled forms are here because people write the question the way they would
/// say it -- "7 multiplied by 87" has no symbol in it at all -- and a symbol-only
/// check silently misses the whole natural-language half of the arithmetic it exists
/// to catch.
///
/// **The symbol half needs both a sum and a reason.** A digit next to an operator is not enough on
/// its own, and a real page proves it: `SCORE 0 / 4` puts a digit beside a `/` and is a counter,
/// while `s = "abc"` in a code sample has an `=` and no arithmetic anywhere in it. Both were being
/// sent to the model as candidates, ranked *ahead* of the actual questions, on one signal apiece.
/// Requiring a digit-operator-digit run **and** an `=` or `?` keeps every sum the feature is for --
/// `14 - 8 =`, `3x + 7 = 22`, `1/2 + 1/3 = ?` -- and drops both of those. A bare `8 + 5` with
/// neither is missed, which is the direction this gate is tuned in: a missed sum is still there to
/// ask about, while a card over somebody's score line is the app interrupting them.
fn looks_arithmetic(text: &str) -> bool {
    if !text.chars().any(|character| character.is_ascii_digit()) {
        return false;
    }
    let lowered = text.to_lowercase();
    const SPELLED: [&str; 12] = [
        "multiplied by",
        "divided by",
        "added to",
        "plus",
        "minus",
        "times",
        "square root",
        "percent of",
        "sum of",
        "product of",
        "difference between",
        "squared",
    ];
    if SPELLED.iter().any(|phrase| lowered.contains(phrase)) {
        return true;
    }
    // A result, or a question. This is what tells a sum from a label that merely contains numbers:
    // a score, a price, a page count, a date range.
    if !text.contains('=') && !text.contains('?') {
        return false;
    }
    // A digit on either side of an operator, with the whitespace between them
    // removed first so `14 - 8 =` matches the same rule as `14-8=`: a person writing
    // a sum spaces it out, and the spacing is not part of the sum.
    const SYMBOLS: [char; 10] = ['+', '-', '*', '/', '×', '÷', '−', '=', '^', '%'];
    let tight: Vec<char> = text
        .chars()
        .filter(|character| !character.is_whitespace())
        .collect();
    tight
        .windows(3)
        .any(|run| run[0].is_ascii_digit() && SYMBOLS.contains(&run[1]) && run[2].is_ascii_digit())
}

/// Several short lines of clues that add up to one answer.
///
/// The bounds are what keep this from matching a navigation menu: a riddle is a
/// handful of lines, none of them long, and at least one of them carries a clue a
/// menu would not (a number, or a word that makes a claim about the thing being
/// described). A wall of thirty short lines is a sidebar, and it is rejected here
/// rather than sent to a model to be puzzled over.
fn looks_like_a_riddle(text: &str) -> bool {
    let lines: Vec<&str> = text.lines().collect();
    if !(3..=8).contains(&lines.len()) {
        return false;
    }
    // Every line short, not merely under a generous ceiling: four long sentences
    // stacked on separate lines are a paragraph a page wrapped, and the whole
    // distinction between a riddle and prose is that its lines are clauses.
    if !lines.iter().all(|line| line.chars().count() < 60) {
        return false;
    }
    let lowered = text.to_lowercase();
    const CLUES: [&str; 12] = [
        "bigger than",
        "smaller than",
        "less than",
        "greater than",
        "add up to",
        "digits",
        "letters",
        "first letter",
        "last letter",
        "i am",
        "i have",
        "always",
    ];
    lines
        .iter()
        .any(|line| line.chars().any(|c| c.is_ascii_digit()))
        || CLUES.iter().any(|clue| lowered.contains(clue))
}

/// The words a question opens with.
const OPENERS: [&str; 16] = [
    "who", "what", "when", "where", "which", "why", "how", "whose", "whom", "is", "are", "was",
    "were", "does", "do", "can",
];

/// A question mark, or an opener that asks something.
///
/// Both are checked because each catches what the other misses: "The value of PI?"
/// has no interrogative opener, and a question split across lines by the page's own
/// markup can lose its question mark to whichever line it landed on.
///
/// **Only the first word, or the word after a numbering prefix.** Scanning the
/// opening words for an interrogative reads ordinary prose as a question, because
/// the commonest of them -- "is" -- is also one of the commonest words in any
/// sentence: "The report is due on Friday" would be sent to a model as a question. A
/// page's own numbering is the one thing that legitimately pushes the interrogative
/// off the front, so it is the one thing allowed to.
fn looks_like_a_question(text: &str) -> bool {
    if text.trim_end().ends_with('?') {
        return true;
    }
    let mut words = text.split_whitespace().map(cleaned_word);
    let Some(first) = words.next() else {
        return false;
    };
    if OPENERS.contains(&first.as_str()) {
        return true;
    }
    if is_numbering(&first) {
        return words
            .next()
            .is_some_and(|word| OPENERS.contains(&word.as_str()));
    }
    false
}

/// A word reduced to what it means, so "1." and "Who?" compare as "1" and "who".
fn cleaned_word(word: &str) -> String {
    word.trim_matches(|character: char| !character.is_alphanumeric())
        .to_lowercase()
}

/// Whether a word is a question's number rather than a question's word.
fn is_numbering(word: &str) -> bool {
    let digits = word.strip_prefix('q').unwrap_or(word);
    !digits.is_empty() && digits.chars().all(|character| character.is_ascii_digit())
}

#[cfg(test)]
fn frame_of(texts: &[&str]) -> Frame {
    Frame {
        id: "test".to_string(),
        url: String::new(),
        title: String::new(),
        blocks: texts
            .iter()
            .enumerate()
            .map(|(position, text)| Block {
                index: position + 1,
                key: format!("k{position}"),
                text: (*text).to_string(),
                tag: "p".to_string(),
                hint: String::new(),
                rect: None,
            })
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::assist::settings::Level;

    fn settings() -> AssistSettings {
        AssistSettings::default()
    }

    /// The examples the feature exists for, shape by shape. Each of these is a block
    /// a user really had on screen, so a rule that stops matching one is a feature
    /// that stopped working on a real page.
    #[test]
    fn every_kind_of_question_the_feature_is_for_is_recognized() {
        let cases = [
            ("14 - 8 =", Shape::Arithmetic),
            ("7 multiplied by 87", Shape::Arithmetic),
            ("1. Who is the father of Mathematics?", Shape::Question),
            ("The value of PI?", Shape::Question),
            (
                "I am bigger than 20.\nI am smaller than 30.\nMy digits add up to 8.\nI am an even number.",
                Shape::Riddle,
            ),
        ];
        for (text, expected) in cases {
            let candidate = screen(text, &settings())
                .unwrap_or_else(|rejected| panic!("{text:?} was rejected: {rejected:?}"));
            assert_eq!(candidate.shape, expected, "wrong shape for {text:?}");
        }
    }

    /// A statement is not a question. This is the direction the heuristic is tuned
    /// to be wrong in: missing a puzzle costs the user nothing, while answering a
    /// sentence they wrote or read costs their attention.
    #[test]
    fn ordinary_prose_is_not_a_question() {
        for text in [
            "The report is due on Friday and the numbers have been checked twice already.",
            "Note: the third paragraph of the document needs rewriting before it is published.",
            "Die Temperatur beträgt heute etwa zwanzig Grad und es ist sehr windig draußen.",
        ] {
            assert!(
                screen(text, &settings()).is_err(),
                "{text:?} should not be treated as a question"
            );
        }
    }

    /// A whole page: the questions on it become the short list, in the page's own
    /// numbering, and everything else is counted as looked at and dropped.
    #[test]
    fn a_page_becomes_a_short_list_in_the_pages_own_numbering() {
        let frame = frame_of(&[
            "Welcome to the quiz. Answer every question below.",
            "1. Who is the father of Mathematics?",
            "The value of PI?",
            "This paragraph is here to be looked at and rejected, as most of a page is.",
            "",
            "I am bigger than 20.\nMy digits add up to 8.\nI am an even number.",
        ]);
        let screen = screen_page(&frame, &settings());
        assert_eq!(screen.considered, 6);
        let indices: Vec<usize> = screen.candidates.iter().map(|c| c.index).collect();
        assert!(indices.contains(&2), "the numbered question should be kept");
        assert!(indices.contains(&3), "the question mark should be kept");
        assert!(indices.contains(&6), "the riddle should be kept");
        assert!(!indices.contains(&4), "prose should not be kept");
        assert!(screen.not_a_question >= 1);
    }

    /// The same paragraph twice on one page is one candidate. A query page echoes
    /// its heading in a card, and two identical candidates would buy one answer
    /// twice and draw two cards over the same words.
    #[test]
    fn the_same_text_twice_on_a_page_is_one_candidate() {
        let frame = frame_of(&["The value of PI?", "The value of PI?"]);
        let screen = screen_page(&frame, &settings());
        assert_eq!(screen.candidates.len(), 1);
    }

    /// The cap keeps the likeliest questions rather than the first ones, and says how
    /// many it dropped -- a silent truncation would look like a page with less on it.
    #[test]
    fn the_cap_keeps_the_strongest_and_reports_the_rest() {
        let mut blocks: Vec<Block> = Vec::new();
        // Strong evidence first, then weak, so a cap that took the first eight would
        // keep the wrong ones.
        for position in 0..3 {
            blocks.push(Block {
                index: position + 1,
                key: format!("sum{position}"),
                text: format!("{} + {} = ?", position, position + 2),
                tag: "li".to_string(),
                hint: String::new(),
                rect: None,
            });
        }
        for position in 0..MAX_CANDIDATES {
            blocks.push(Block {
                index: position + 4,
                key: format!("ask{position}"),
                text: format!("What is number {position} on this list?"),
                tag: "li".to_string(),
                hint: String::new(),
                rect: None,
            });
        }
        let frame = Frame {
            id: "cap".to_string(),
            url: String::new(),
            title: String::new(),
            blocks,
        };
        let screen = screen_page(&frame, &settings());
        assert_eq!(screen.candidates.len(), MAX_CANDIDATES);
        assert_eq!(screen.over_cap, 3);
        // The three sums are what survived, not the first eight blocks.
        let kept: Vec<usize> = screen.candidates.iter().map(|c| c.index).collect();
        for index in 1..=3 {
            assert!(kept.contains(&index), "the sums should survive the cap");
        }
        assert!(screen.reason().contains("over the per-page limit"));
    }

    /// **Among questions, the ones the reader is looking at come first.** A chat or a long article
    /// holds more questions than a cap admits, and buying them in document order buys the oldest
    /// ones -- on a conversation that is the top, which nobody is reading any more. The block at the
    /// reader's place on the page has to win that tie, or the feature works only on short pages.
    #[test]
    fn the_cap_keeps_the_questions_nearest_the_reader() {
        let blocks = (0..10)
            .map(|position| Block {
                index: position + 1,
                key: format!("q{position}"),
                text: format!("What is number {position} on this page?"),
                tag: "p".to_string(),
                hint: String::new(),
                rect: Some(Rect {
                    x: 0.0,
                    // The first five were read far above the screen; the last five were on it.
                    y: if position < 5 {
                        -4_000.0 - position as f64
                    } else {
                        position as f64 * 10.0
                    },
                    width: 400.0,
                    height: 20.0,
                }),
            })
            .collect();
        let screen = screen_page(
            &Frame {
                id: "near".to_string(),
                url: String::new(),
                title: String::new(),
                blocks,
            },
            &settings(),
        );
        let kept: Vec<usize> = screen
            .candidates
            .iter()
            .map(|candidate| candidate.index)
            .collect();
        assert_eq!(kept.len(), MAX_CANDIDATES);
        assert_eq!(screen.over_cap, 2);
        // The two dropped are the farthest above the screen (the oldest), not the last two in the
        // document -- and the five that were on the screen survived.
        assert!(!kept.contains(&4) && !kept.contains(&5), "{kept:?}");
        assert!(kept.contains(&10), "{kept:?}");
    }

    /// A page with nothing on it that reads as a question is a frame that never
    /// reaches a model, and the reason says how much was looked at.
    #[test]
    fn a_page_with_no_question_never_reaches_a_model() {
        let frame = frame_of(&[
            "The report is due on Friday and the numbers have been checked twice already.",
            "Note: the third paragraph needs rewriting before it is published.",
        ]);
        let screen = screen_page(&frame, &settings());
        assert!(screen.candidates.is_empty());
        assert!(screen.reason().contains("2 blocks looked at"));
    }

    /// A page of nothing but short labels is reported as short rather than as
    /// something mysterious.
    #[test]
    fn a_page_of_labels_is_reported_as_too_short() {
        let frame = frame_of(&["Home", "About", "Menu", "More"]);
        let screen = screen_page(&frame, &settings());
        assert!(screen.candidates.is_empty());
        assert_eq!(screen.too_short, 4);
        assert_eq!(screen.not_a_question, 0);
    }

    /// A menu of short lines is not a riddle: the line bounds are what tell a riddle
    /// from a sidebar.
    #[test]
    fn a_menu_of_short_lines_is_not_a_riddle() {
        let menu = (0..14)
            .map(|index| format!("Item {index}"))
            .collect::<Vec<_>>()
            .join("\n");
        assert_eq!(shape_of(&menu), None);
    }

    /// A riddle is not a list of long paragraphs, either.
    #[test]
    fn long_lines_do_not_make_a_riddle() {
        let paragraph = "I am a thing that is bigger than the other thing and it has digits in it.";
        let text = std::iter::repeat(paragraph)
            .take(4)
            .collect::<Vec<_>>()
            .join("\n");
        assert_ne!(shape_of(&text), Some(Shape::Riddle));
    }

    /// **A label with numbers in it is not a sum, and a code sample is not arithmetic.**
    /// `SCORE 0 / 4` matched the old rule on a digit beside a `/` and was sent to the model as a
    /// candidate -- ranked *ahead* of the real questions, because strong shapes sort first -- while
    /// `s = "abc"` in a code block matched on the `=` alone. Both are shapes a page is full of, and
    /// between them they spend the cap the questions needed.
    #[test]
    fn a_counter_and_a_code_sample_are_not_arithmetic() {
        assert_eq!(shape_of("SCORE 0 / 4"), None);
        assert_eq!(shape_of("s = \"abc\"\nprint(s * 2)"), None);
        // And every sum the feature exists for is still a sum.
        assert_eq!(shape_of("14 - 8 ="), Some(Shape::Arithmetic));
        assert_eq!(shape_of("3x + 7 = 22"), Some(Shape::Arithmetic));
        assert_eq!(shape_of("1/2 + 1/3 = ?"), Some(Shape::Arithmetic));
        assert_eq!(shape_of("7 multiplied by 87"), Some(Shape::Arithmetic));
        // A percentage question is a question, not a sum: no operator between two digits.
        assert_eq!(shape_of("What is 15% of 240?"), Some(Shape::Question));
    }

    /// Whitespace is the DOM's, not the writer's, so it is collapsed -- but line
    /// breaks are the writer's, and they are kept.
    #[test]
    fn normalization_collapses_layout_but_keeps_lines() {
        assert_eq!(
            normalize("  The value of PI?  \n\n   \n  and more text  "),
            "The value of PI?\nand more text"
        );
    }

    /// Whitespace is the page's, not the writer's, so the same paragraph read twice as the
    /// DOM rendered it is one block -- which is what the per-request duplicate rule compares.
    #[test]
    fn the_same_text_with_different_whitespace_is_one_block() {
        let a = screen(
            "I am bigger than 20.\nMy digits add up to 8.\nI am an even number.",
            &settings(),
        )
        .expect("candidate");
        // The same riddle as a page would render it: indentation inside the lines, blank
        // lines between them, runs of spaces where the markup had them.
        let b = screen(
            "  I am bigger than 20.  \n\n   My   digits  add up to 8.\n I am an even number. ",
            &settings(),
        )
        .expect("candidate");
        assert_eq!(a.text, b.text);
    }

    /// A different question is a different question.
    #[test]
    fn different_text_is_different_text() {
        let a = screen("What is the value of PI?", &settings()).expect("candidate");
        let b = screen("What is the value of E?", &settings()).expect("candidate");
        assert_ne!(a.text, b.text);
    }

    /// Text under the minimum never reaches a model, and says by how much.
    #[test]
    fn text_under_the_minimum_is_rejected_by_name() {
        let rejected = screen("hi?", &settings()).expect_err("too short");
        assert_eq!(rejected.stage, Stage::TooShort);
        assert!(
            rejected.reason.contains("3 characters"),
            "the length should be named: {}",
            rejected.reason
        );
    }

    /// A candidate is cut to the cap on a character boundary, so a long page of prose
    /// costs a bounded request and never an invalid one.
    #[test]
    fn a_long_candidate_is_cut_to_the_cap_without_breaking_a_character() {
        let settings = AssistSettings {
            max_ask_chars: 20,
            ..settings()
        };
        let text = format!("{}{}", "What is é".repeat(20), "?");
        let candidate = screen(&text, &settings).expect("candidate");
        assert_eq!(candidate.text.chars().count(), 20);
        assert!(candidate.text.is_char_boundary(candidate.text.len()));
    }

    /// The budget counts the last hour only, so a busy morning does not silence the
    /// afternoon.
    #[test]
    fn the_budget_counts_only_the_last_hour() {
        let now = Instant::now();
        let mut budget = Budget::default();
        for _ in 0..5 {
            budget.note_call(now - BUDGET_WINDOW - Duration::from_secs(60));
        }
        assert_eq!(budget.calls_in_window(now), 0);
        for _ in 0..3 {
            budget.note_call(now);
        }
        assert_eq!(budget.calls_in_window(now), 3);
    }

    /// The calls are dropped once they age out, so a session that runs all day does not
    /// accumulate a queue of timestamps nobody will count again.
    #[test]
    fn the_budget_forgets_the_calls_that_have_aged_out() {
        let now = Instant::now();
        let mut budget = Budget::default();
        for _ in 0..10 {
            budget.note_call(now - BUDGET_WINDOW - Duration::from_secs(1));
        }
        // One call now prunes everything older than the window on the way in.
        budget.note_call(now);
        assert_eq!(budget.calls_in_window(now), 1);
    }

    /// **The top level looks at prose; every other level refuses it by name.** This is what
    /// "send more of the page" means here, and it is the direction the gate is tuned in: the
    /// default misses a puzzle rather than interrupting someone with a card on a paragraph.
    #[test]
    fn only_the_top_level_admits_a_block_the_gate_could_not_name() {
        let prose = "The report is due on Friday and the numbers have been checked twice already.";
        assert_eq!(
            screen(prose, &settings())
                .expect_err("refused by default")
                .stage,
            Stage::NoShape
        );

        let high = AssistSettings {
            level: Level::High,
            ..settings()
        };
        let candidate = screen(prose, &high).expect("the top level admits it");
        assert_eq!(candidate.shape, Shape::Passage);
    }

    /// A block the gate could not name goes in **behind** everything it could, so a cap -- or
    /// the model's own attention -- reaches the questions first.
    #[test]
    fn a_passage_sorts_behind_every_shape_that_was_named() {
        let frame = frame_of(&[
            "The report is due on Friday and the numbers have been checked twice already.",
            "The value of PI?",
        ]);
        let high = AssistSettings {
            level: Level::High,
            ..settings()
        };
        let screen = screen_page(&frame, &high);
        let shapes: Vec<Shape> = screen.candidates.iter().map(|c| c.shape).collect();
        assert_eq!(shapes, vec![Shape::Question, Shape::Passage]);
    }

    /// A raised cap admits the weakest blocks last, which is the whole reason a level may raise
    /// it at all: the extra room is spent on the page's prose, never on its questions.
    #[test]
    fn a_raised_cap_spends_the_extra_room_on_the_weakest_blocks() {
        let mut blocks: Vec<Block> = Vec::new();
        for position in 0..MAX_CANDIDATES {
            blocks.push(Block {
                index: position + 1,
                key: format!("item{position}"),
                text: format!("What is item {position}?"),
                tag: "li".to_string(),
                hint: String::new(),
                rect: None,
            });
        }
        // As much prose as questions, all of it below the last named shape on the page.
        for position in 0..MAX_CANDIDATES {
            blocks.push(Block {
                index: MAX_CANDIDATES + position + 1,
                key: format!("prose{position}"),
                text: format!(
                    "Paragraph {position} of the page, long enough to be read and shaped like nothing in particular."
                ),
                tag: "p".to_string(),
                hint: String::new(),
                rect: None,
            });
        }
        let screen = screen_page(
            &Frame {
                id: "cap".to_string(),
                url: String::new(),
                title: String::new(),
                blocks,
            },
            &AssistSettings {
                level: Level::High,
                ..settings()
            },
        );
        // The top level's cap is past the middle level's, and what it admits is the prose.
        assert!(screen.candidates.len() > MAX_CANDIDATES);
        let named = screen
            .candidates
            .iter()
            .filter(|candidate| candidate.shape != Shape::Passage)
            .count();
        assert_eq!(named, MAX_CANDIDATES, "every question survives");
        assert!(screen
            .candidates
            .last()
            .is_some_and(|c| c.shape == Shape::Passage));
    }
}
