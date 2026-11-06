//! Applying what the model proposed, deterministically.
//!
//! The model writes plain facts; it does not write the profile. This file decides
//! what a fact means:
//!
//! - a fact that does not read like a durable statement about the person is
//!   dropped (a question, a request, a sentence about the assistant, a secret);
//! - a first-person sentence is turned into the third person the profile uses;
//! - an addition that says what an existing fact already says reinforces that row
//!   rather than becoming a second one;
//! - the result is trimmed to a hard character budget, lowest-confidence facts
//!   first.
//!
//! # Why the filters are here and not only in the prompt
//!
//! A prompt is advice; a small model breaks it. The prompt keeps the common case
//! good, and this file keeps a bad case out of the profile. Both are cheap and
//! deterministic -- no second model call to judge the first.
//!
//! # Why the budget is characters, not tokens
//!
//! The app has no tokenizer for every provider's vocabulary, and a character cap
//! is deterministic. `MAX_DOCUMENT_CHARS` is sized so the rendered block is
//! comfortably under six hundred English tokens.

use std::cmp::Ordering;
use std::collections::HashSet;

use crate::database::taste::TasteItemRow;

use super::reply::RawReply;
use super::section::Section;

/// The most text the rendered profile may hold. Roughly 550 English tokens.
pub const MAX_DOCUMENT_CHARS: usize = 2_200;

/// The longest one fact may be. A fact is one sentence, not a paragraph.
pub const MAX_ITEM_CHARS: usize = 160;

/// The shortest text worth storing. Below this it is a label rather than a fact.
const MIN_ITEM_CHARS: usize = 10;

/// A ceiling on row count as well as characters, so a list of tiny facts cannot
/// turn the profile into something nobody reads.
const MAX_ITEMS: usize = 32;

/// How much two facts may overlap and still be considered the same fact.
///
/// Deliberately below 1.0: "Prefers Rust for backend services" and "Prefers Rust
/// for backend work" are one preference, and a profile that stored both would
/// spend its budget repeating itself.
const SIMILARITY: f32 = 0.6;

/// Below this the model is saying it is not sure, so the fact is not stored.
const CONFIDENCE_FLOOR: f32 = 0.5;

/// Words that carry no identity on their own. Excluded from similarity so two
/// unrelated sentences cannot look alike because they both say "the user".
const STOPWORDS: [&str; 30] = [
    "a", "an", "and", "are", "as", "at", "be", "but", "by", "for", "from", "has", "have", "in",
    "is", "it", "its", "of", "on", "or", "that", "the", "their", "them", "they", "this", "to",
    "user", "was", "with",
];

/// A line that opens with one of these is not a fact about the person.
///
/// Everything here is a request, a task, an act the assistant performed, or a
/// hedge that states nothing. Third-person fact verbs (`Uses`, `Prefers`, `Works`,
/// `Is`, `Lives`) are deliberately absent: "Is a backend developer" is a fact,
/// while "Is this a good idea?" is a question and already carries a `?`.
const NOT_A_FACT_WORDS: [&str; 40] = [
    "how",
    "what",
    "why",
    "when",
    "where",
    "who",
    "can",
    "could",
    "would",
    "should",
    "please",
    "write",
    "create",
    "build",
    "make",
    "add",
    "fix",
    "run",
    "show",
    "give",
    "help",
    "explain",
    "find",
    "search",
    "tell",
    "summarize",
    "summarise",
    "translate",
    "generate",
    "implement",
    "wants",
    "needs",
    "asked",
    "requested",
    "helped",
    "helping",
    "might",
    "maybe",
    "possibly",
    "perhaps",
];

/// What one merge changed.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct MergeOutcome {
    pub added: usize,
    pub updated: usize,
    pub removed: usize,
    pub ignored: usize,
}

/// Applies one reply to the profile in place.
///
/// `now` is the run's start stamp. Every item touched by this reply is marked as
/// seen then, which is what lets the next run tell a fact that keeps coming up
/// from one that has not been mentioned in months.
pub fn merge(items: &mut Vec<TasteItemRow>, reply: &RawReply, now: &str) -> MergeOutcome {
    let mut outcome = MergeOutcome {
        ignored: reply.ignored,
        ..MergeOutcome::default()
    };
    let mut next_id = items.iter().map(|item| item.id).max().unwrap_or(0) + 1;

    for fact in &reply.facts {
        let text = normalize_fact(&fact.text);
        if !is_usable(&text) || fact.confidence < CONFIDENCE_FLOOR {
            outcome.ignored += 1;
            continue;
        }
        let confidence = fact.confidence.clamp(CONFIDENCE_FLOOR, 1.0);
        let section = Section::classify(&text);

        // An addition that says what an existing row already says is an edit to
        // that row. The longer wording wins because a model restating a fact
        // usually does so with more context; confidence is the higher of the two,
        // because neither statement weakens the other.
        if let Some(index) = similar_index(items, &text) {
            let existing = &mut items[index];
            if text.chars().count() > existing.text.chars().count() {
                existing.text = text;
            }
            existing.confidence = existing.confidence.max(confidence);
            existing.updated_at = now.to_string();
            existing.last_seen_at = now.to_string();
            outcome.updated += 1;
            continue;
        }

        items.push(TasteItemRow {
            id: next_id,
            section: section.as_str().to_string(),
            text,
            confidence,
            created_at: now.to_string(),
            updated_at: now.to_string(),
            last_seen_at: now.to_string(),
        });
        next_id += 1;
        outcome.added += 1;
    }

    trim_to_budget(items);
    outcome
}

/// Reads stored rows into the shape the merge and renderer expect.
///
/// A row whose section is not one of the known names, or whose text is unusable,
/// is dropped rather than allowed to consume the budget invisibly. The profile is
/// local state that a crash, an older build or a hand edit can leave inconsistent;
/// this is the one place that normalizes it.
pub fn normalize(rows: Vec<TasteItemRow>) -> Vec<TasteItemRow> {
    let mut items: Vec<TasteItemRow> = rows
        .into_iter()
        .filter(|item| Section::parse(&item.section).is_some())
        // The same voice and filters a new line goes through. A profile written by
        // an older build can hold a first-person sentence or a request; rendering
        // it unchanged would keep the very facts these rules exist to keep out.
        .filter_map(|mut item| {
            let text = normalize_fact(&item.text);
            if !is_usable(&text) {
                return None;
            }
            item.text = text;
            item.confidence = item.confidence.clamp(CONFIDENCE_FLOOR, 1.0);
            Some(item)
        })
        .collect();
    items.sort_by_key(|item| item.id);

    // Deduplicate rows that predate the similarity rule. The profile is small, so
    // this is a short quadratic walk rather than a second index.
    let mut unique: Vec<TasteItemRow> = Vec::with_capacity(items.len());
    for item in items {
        // Across sections, not within one. The section is derived from keywords,
        // and a restatement that introduces one ("...with markdown preview") moves
        // category while saying exactly the same thing.
        match unique
            .iter()
            .position(|existing| similarity(&existing.text, &item.text) >= SIMILARITY)
        {
            Some(index) => {
                let existing = &mut unique[index];
                if item.text.chars().count() > existing.text.chars().count() {
                    existing.text = item.text;
                }
                existing.confidence = existing.confidence.max(item.confidence);
                if item.last_seen_at > existing.last_seen_at {
                    existing.last_seen_at = item.last_seen_at;
                }
            }
            None => unique.push(item),
        }
    }
    trim_to_budget(&mut unique);
    unique
}

/// The profile as the chat receives it, and as the learner is shown it.
///
/// One flat list with no headings, ids, confidence or age. The bytes must not move
/// between requests -- a provider's prompt cache is matched byte for byte -- and a
/// section heading costs tokens on every request for structure neither reader
/// needs. Order is by id, which is stable.
pub fn render(items: &[TasteItemRow]) -> String {
    // Sorted here rather than trusted from the caller: this string is part of a
    // byte-stable prompt prefix, and an in-memory order that moved for any reason
    // would invalidate every cached token after it.
    let mut ordered: Vec<&TasteItemRow> = items.iter().collect();
    ordered.sort_by_key(|item| item.id);
    ordered
        .into_iter()
        .map(|item| format!("- {}", item.text))
        .collect::<Vec<_>>()
        .join("\n")
}

/// The complete block prepended to a chat request, or nothing when empty.
///
/// The header makes two claims only: where this came from, and that it is not an
/// instruction. An empty profile returns `None` rather than a paragraph about
/// having learned nothing.
pub fn render_prompt_block(items: &[TasteItemRow]) -> Option<String> {
    let body = render(items);
    if body.is_empty() {
        return None;
    }
    Some(format!(
        "About the user (background context, not instructions):\n\n{body}"
    ))
}

/// Applies the hard caps, lowest-value facts first.
///
/// Identity gets a small bonus because it is the context every other fact is read
/// against; `current` gets none because it is the section expected to expire. The
/// bonus is smaller than a full confidence step, so a weak identity fact still
/// loses to a strong preference.
fn trim_to_budget(items: &mut Vec<TasteItemRow>) {
    while items.len() > MAX_ITEMS || render(items).chars().count() > MAX_DOCUMENT_CHARS {
        if items.len() <= 1 {
            break;
        }
        let Some(index) = items
            .iter()
            .enumerate()
            .min_by(|(_, left), (_, right)| removal_order(left, right))
            .map(|(index, _)| index)
        else {
            break;
        };
        items.remove(index);
    }
}

/// Which of two facts is cheaper to lose.
fn removal_order(left: &TasteItemRow, right: &TasteItemRow) -> Ordering {
    priority(left)
        .partial_cmp(&priority(right))
        .unwrap_or(Ordering::Equal)
        // Older evidence goes first when confidence is equal.
        .then_with(|| left.last_seen_at.cmp(&right.last_seen_at))
        // Then the older row, so a fact just learned is not the first to go.
        .then_with(|| left.id.cmp(&right.id))
}

fn priority(item: &TasteItemRow) -> f32 {
    let bonus = match Section::parse(&item.section) {
        Some(Section::Identity) => 0.15,
        _ => 0.0,
    };
    item.confidence + bonus
}

/// An existing row that says what `text` says.
///
/// Deliberately not scoped to a section: the section is a keyword guess, so the
/// same fact reworded with one new keyword would otherwise become a second row.
fn similar_index(items: &[TasteItemRow], text: &str) -> Option<usize> {
    items
        .iter()
        .position(|item| similarity(&item.text, text) >= SIMILARITY)
}

/// How alike two facts are, by word overlap.
///
/// The Jaccard index, plus containment **only when the shorter fact is wholly
/// inside the longer one** -- "Prefers Rust" inside "Prefers Rust for backend
/// services". Containment on mere overlap is wrong: "Uses Windows 11" and "Uses
/// Windows 10" share two of three words, and collapsing them replaces a true fact
/// with a different one. A full subset is a more specific version of the same
/// fact, which is what should replace a row rather than sit beside it.
pub fn similarity(left: &str, right: &str) -> f32 {
    let left = tokens(left);
    let right = tokens(right);
    if left.is_empty() || right.is_empty() {
        return 0.0;
    }
    let intersection = left.intersection(&right).count();
    let union = left.union(&right).count() as f32;
    let jaccard = intersection as f32 / union;
    let shorter = left.len().min(right.len());
    let containment = if intersection == shorter { 1.0 } else { 0.0 };
    jaccard.max(containment)
}

/// Meaningful words in a fact.
///
/// Lowercased, split on anything that is not a letter or digit, stopwords
/// removed, and a trailing plural `s` folded so "preference" and "preferences" are
/// the same word for this purpose. Deliberately not a real stemmer: the goal is to
/// catch restatements, not to conflate different words.
fn tokens(text: &str) -> HashSet<String> {
    text.split(|character: char| !character.is_alphanumeric())
        .map(str::to_lowercase)
        .filter(|word| word.len() >= 2 && !STOPWORDS.contains(&word.as_str()))
        .map(|word| {
            if word.len() > 4 && word.ends_with('s') {
                word[..word.len() - 1].to_string()
            } else {
                word
            }
        })
        .collect()
}

/// One fact with its decoration removed, its whitespace collapsed and its voice
/// made third person.
fn normalize_fact(raw: &str) -> String {
    let cleaned = clean_text(raw);
    let sentence = third_person(&cleaned);
    let sentence = ensure_period(sentence);
    clip(sentence.trim())
}

/// One fact with its decoration removed and its whitespace collapsed.
fn clean_text(raw: &str) -> String {
    let collapsed = raw.split_whitespace().collect::<Vec<_>>().join(" ");
    let trimmed = collapsed
        .trim()
        .trim_matches(|character: char| {
            matches!(character, '-' | '*' | '`' | '"' | '\'' | '•' | '–' | '—')
        })
        .trim();
    clip(trimmed)
}

/// Turns a first-person sentence into the third person the profile uses.
///
/// A small model ignores "no pronouns" more often than it gets anything else
/// wrong, and a profile line reading "I use Rust" is ambiguous to a chat model
/// reading it beside its own instructions. The mapping is a prefix table rather
/// than a rewrite: it changes only what it recognises and leaves the rest alone,
/// and a sentence it cannot convert is refused by [`is_usable`] rather than
/// stored in the wrong voice.
fn third_person(text: &str) -> String {
    const PREFIXES: [(&str, &str); 27] = [
        ("i am ", "Is "),
        ("i'm ", "Is "),
        ("i’m ", "Is "),
        ("i use ", "Uses "),
        ("i used ", "Used "),
        ("i have ", "Has "),
        ("i've ", "Has "),
        ("i’ve ", "Has "),
        ("i live ", "Lives "),
        ("i work ", "Works "),
        ("i prefer ", "Prefers "),
        ("i like ", "Likes "),
        ("i love ", "Loves "),
        ("i hate ", "Hates "),
        ("i dislike ", "Dislikes "),
        ("i don't ", "Does not "),
        ("i don’t ", "Does not "),
        ("i do not ", "Does not "),
        ("i want ", "Wants "),
        ("i need ", "Needs "),
        ("i know ", "Knows "),
        ("i speak ", "Speaks "),
        ("i play ", "Plays "),
        ("i read ", "Reads "),
        ("i build ", "Builds "),
        ("i write ", "Writes "),
        ("my name is ", "Name is "),
    ];
    let lower = text.to_lowercase();
    for (from, to) in PREFIXES {
        if lower.starts_with(from) {
            return format!("{to}{}", &text[from.len()..]);
        }
    }
    if lower.starts_with("my ") {
        return format!("Their {}", &text[3..]);
    }
    text.to_string()
}

/// A fact reads as a sentence, so it ends like one.
fn ensure_period(text: String) -> String {
    let trimmed = text.trim_end();
    if trimmed
        .chars()
        .last()
        .is_some_and(|last| last.is_alphanumeric())
    {
        format!("{trimmed}.")
    } else {
        trimmed.to_string()
    }
}

/// Validates text a person typed into a fact.
///
/// The same cleaning, voice and filters the model's line goes through, so a
/// manual edit cannot store a label, a question or a paragraph that the profile's
/// budget would then have to carry. `None` means the text is not usable; the
/// caller reports that rather than saving it.
pub fn sanitize_user_text(raw: &str) -> Option<String> {
    let cleaned = normalize_fact(raw);
    is_usable(&cleaned).then_some(cleaned)
}

/// A fact is worth storing when it says something durable, not just names
/// something or repeats a request.
fn is_usable(text: &str) -> bool {
    let characters = text.chars().count();
    let words = text.split_whitespace().count();
    if characters < MIN_ITEM_CHARS || characters > MAX_ITEM_CHARS {
        return false;
    }
    // Same rule as the corpus: the word floor applies to space-separated text.
    if words < 2 && text.is_ascii() {
        return false;
    }
    if text.contains('?') {
        return false;
    }
    let lower = text.to_lowercase();
    // A sentence left in the first person was not converted, which means it is
    // not the shape a profile line may take.
    if lower.starts_with("i ") || lower.starts_with("my ") {
        return false;
    }
    let first = lower
        .split(|character: char| !character.is_alphanumeric())
        .next()
        .unwrap_or("");
    if NOT_A_FACT_WORDS.contains(&first) {
        return false;
    }
    if lower.starts_with("the user") || lower.contains("this conversation") {
        return false;
    }
    if lower.contains("the assistant") || lower.contains("as an ai") {
        return false;
    }
    !has_secret(&lower)
}

/// Whether a sentence looks like a credential rather than a fact about a person.
///
/// Deliberately narrow: "Uses a password manager" is a preference, while
/// "Password: hunter2" is a leak. Matching the bare word would drop the first to
/// catch the second.
fn has_secret(lower: &str) -> bool {
    // A credential, not the word. "Uses the API key from the environment" is a
    // workflow fact, and dropping it to catch "API key: sk-..." loses more than it
    // protects.
    lower.contains("api key:")
        || lower.contains("api key is")
        || lower.contains("apikey=")
        || lower.contains("api_key=")
        || lower.contains("secret key:")
        || lower.contains("private key:")
        || lower.contains("access token:")
        || lower.contains("password:")
        || lower.contains("sk-proj-")
        || lower.contains("sk-ant-")
}

/// Cuts at a character boundary, never inside a multi-byte character.
fn clip(text: &str) -> String {
    if text.chars().count() <= MAX_ITEM_CHARS {
        return text.to_string();
    }
    let mut clipped: String = text.chars().take(MAX_ITEM_CHARS - 1).collect();
    clipped.push('…');
    clipped
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::taste::reply::RawFact;

    fn item(id: i64, section: Section, text: &str, confidence: f32) -> TasteItemRow {
        TasteItemRow {
            id,
            section: section.as_str().to_string(),
            text: text.to_string(),
            confidence,
            created_at: "2026-01-01T00:00:00.000Z".to_string(),
            updated_at: "2026-01-01T00:00:00.000Z".to_string(),
            last_seen_at: "2026-01-01T00:00:00.000Z".to_string(),
        }
    }

    fn fact(text: &str, confidence: f32) -> RawFact {
        RawFact {
            text: text.to_string(),
            confidence,
        }
    }

    fn reply(facts: Vec<RawFact>) -> RawReply {
        RawReply {
            facts,
            declined: false,
            ignored: 0,
        }
    }

    #[test]
    fn an_added_fact_is_precise_and_stored_verbatim() {
        let mut items = Vec::new();
        let outcome = merge(
            &mut items,
            &reply(vec![fact(
                "Uses Rust (programming language) for backend services.",
                0.9,
            )]),
            "2026-02-01T00:00:00.000Z",
        );
        assert_eq!(outcome.added, 1);
        assert!(items[0].text.starts_with("Uses Rust"));
        assert_eq!(items[0].section, "work");
        assert_eq!(items[0].confidence, 0.9);
    }

    #[test]
    fn a_first_person_sentence_becomes_third_person() {
        assert_eq!(
            normalize_fact("i use Rust (programming language) for backend work"),
            "Uses Rust (programming language) for backend work."
        );
        assert_eq!(
            normalize_fact("I'm a backend developer"),
            "Is a backend developer."
        );
        assert_eq!(
            normalize_fact("My laptop is a MacBook Pro"),
            "Their laptop is a MacBook Pro."
        );
    }

    #[test]
    fn a_restated_fact_updates_instead_of_duplicating() {
        let mut items = vec![item(
            1,
            Section::Work,
            "Prefers Rust (programming language) for backend services.",
            0.7,
        )];
        let outcome = merge(
            &mut items,
            &reply(vec![fact(
                "Prefers Rust (programming language) for backend services and CLI tools.",
                0.9,
            )]),
            "2026-02-01T00:00:00.000Z",
        );
        assert_eq!(outcome.added, 0);
        assert_eq!(outcome.updated, 1);
        assert_eq!(items.len(), 1);
        assert!(items[0].text.ends_with("CLI tools."));
        assert_eq!(items[0].confidence, 0.9);
    }

    /// Containment on any short side would collapse these into one row and replace
    /// a true fact with a different one.
    #[test]
    fn two_short_facts_with_one_different_word_are_not_restatements() {
        assert!(similarity("Uses Windows 11.", "Uses Windows 10.") < SIMILARITY);
        assert!(similarity("Uses a MacBook Pro.", "Uses a MacBook Air.") < SIMILARITY);
        assert!(similarity("Dislikes loud music.", "Dislikes loud notifications.") < SIMILARITY);
    }

    /// The pair this rule exists for: same verb, opposite object.
    #[test]
    fn a_different_language_is_not_a_restatement() {
        let mut items = vec![item(
            1,
            Section::Work,
            "Prefers Rust (programming language) for backend services.",
            0.8,
        )];
        merge(
            &mut items,
            &reply(vec![fact(
                "Prefers Python (programming language) for data analysis.",
                0.8,
            )]),
            "2026-02-01T00:00:00.000Z",
        );
        assert_eq!(items.len(), 2);
    }

    #[test]
    fn a_question_is_not_a_fact() {
        let mut items = Vec::new();
        let outcome = merge(
            &mut items,
            &reply(vec![fact("Is a quiz page something they want?", 0.9)]),
            "2026-02-01T00:00:00.000Z",
        );
        assert_eq!(outcome.ignored, 1);
        assert!(items.is_empty());
    }

    #[test]
    fn a_request_is_not_a_fact() {
        let mut items = Vec::new();
        let outcome = merge(
            &mut items,
            &reply(vec![
                fact("Build me a quiz page for the history class.", 0.9),
                fact("Explain how this app stores its settings.", 0.9),
                fact("Wants a quiz page built for the history class.", 0.9),
                fact("Asked for a quiz page about the history class.", 0.9),
            ]),
            "2026-02-01T00:00:00.000Z",
        );
        assert_eq!(outcome.ignored, 4);
        assert!(items.is_empty());
    }

    #[test]
    fn a_sentence_about_the_model_is_not_a_fact_about_the_user() {
        let mut items = Vec::new();
        merge(
            &mut items,
            &reply(vec![fact(
                "The assistant prefers short answers in this conversation.",
                0.9,
            )]),
            "2026-02-01T00:00:00.000Z",
        );
        assert!(items.is_empty());
    }

    #[test]
    fn a_credential_is_not_stored() {
        let mut items = Vec::new();
        merge(
            &mut items,
            &reply(vec![fact(
                "Uses the API key sk-proj-abcdef for every request.",
                0.9,
            )]),
            "2026-02-01T00:00:00.000Z",
        );
        assert!(items.is_empty());
    }

    #[test]
    fn a_fact_below_the_confidence_floor_is_dropped() {
        let mut items = Vec::new();
        let outcome = merge(
            &mut items,
            &reply(vec![fact("Might enjoy strategy games.", 0.3)]),
            "2026-02-01T00:00:00.000Z",
        );
        assert_eq!(outcome.ignored, 1);
        assert!(items.is_empty());
    }

    /// The section is chosen in code, so a model that only writes a line still
    /// produces a grouped profile.
    #[test]
    fn the_section_is_chosen_from_the_fact() {
        let mut items = Vec::new();
        merge(
            &mut items,
            &reply(vec![
                fact("Dislikes loud notifications.", 0.9),
                fact("Plays chess on weekends.", 0.8),
            ]),
            "2026-02-01T00:00:00.000Z",
        );
        assert!(items.iter().any(|item| item.section == "dislikes"));
        assert!(items.iter().any(|item| item.section == "interests"));
    }

    #[test]
    fn the_rendered_profile_is_flat_stable_text() {
        let items = vec![
            item(
                2,
                Section::Work,
                "Prefers Rust (programming language).",
                0.9,
            ),
            item(
                1,
                Section::Identity,
                "Uses Windows 11 with an RTX 5090 (GPU).",
                0.95,
            ),
        ];
        let rendered = render(&items);
        assert_eq!(
            rendered,
            "- Uses Windows 11 with an RTX 5090 (GPU).\n- Prefers Rust (programming language)."
        );
        assert!(!rendered.contains('['));
        assert!(!rendered.contains("0.9"));
    }

    #[test]
    fn the_budget_drops_the_weakest_fact_first() {
        let mut items: Vec<TasteItemRow> = (0..50)
            .map(|index| {
                item(
                    index as i64 + 1,
                    Section::Preferences,
                    &format!(
                        "Preference number {index} is a deliberately long fact that fills the budget."
                    ),
                    0.4,
                )
            })
            .collect();
        items.push(item(
            100,
            Section::Identity,
            "Uses Windows 11 on a desktop with an RTX 5090 (GPU).",
            0.99,
        ));
        let normalized = normalize(items);
        assert!(normalized.len() < 51);
        assert!(normalized.iter().any(|item| item.id == 100));
        assert!(render(&normalized).chars().count() <= MAX_DOCUMENT_CHARS);
    }

    /// A row from an older build with a section nobody knows is dropped rather
    /// than rendered as an invisible budget item.
    #[test]
    fn normalization_drops_unknown_sections_and_short_rows() {
        let mut unknown = item(1, Section::Work, "Uses Tauri (app framework).", 0.8);
        unknown.section = "misc".into();
        let short = item(2, Section::Work, "Rust", 0.8);
        let normalized = normalize(vec![unknown, short]);
        assert!(normalized.is_empty());
    }

    /// The same fact written twice in older stored data becomes one row.
    #[test]
    fn normalization_collapses_duplicate_rows() {
        let normalized = normalize(vec![
            item(1, Section::Work, "Prefers Rust for backend services.", 0.7),
            item(2, Section::Work, "Prefers Rust for backend work.", 0.9),
        ]);
        assert_eq!(normalized.len(), 1);
        assert_eq!(normalized[0].confidence, 0.9);
    }

    /// The manual editor uses the same floor as the model: a label is refused
    /// and a first-person sentence is normalized before it is stored.
    #[test]
    fn a_manually_typed_fact_is_cleaned_and_checked() {
        assert_eq!(
            sanitize_user_text("  I   use Rust (programming language) for backend work  "),
            Some("Uses Rust (programming language) for backend work.".to_string())
        );
        assert_eq!(sanitize_user_text("Rust"), None);
        assert_eq!(sanitize_user_text("   "), None);
        assert_eq!(sanitize_user_text("Why does this happen?"), None);
    }

    #[test]
    fn the_prompt_block_is_absent_when_nothing_was_learned() {
        assert!(render_prompt_block(&[]).is_none());
        assert!(render_prompt_block(&[item(
            1,
            Section::Work,
            "Prefers Rust (programming language).",
            0.9
        )])
        .is_some());
    }
}
