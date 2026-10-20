//! Applying what the model proposed, deterministically.
//!
//! The model does not write the profile. It proposes operations against rows it
//! was shown, and this file decides what those operations mean:
//!
//! - an `add` that is close to an existing fact becomes a reinforcement of that
//!   fact rather than a second row saying the same thing;
//! - an `update` or `remove` that names an id not in the profile is dropped;
//! - text that is too short, too long or empty is dropped rather than stored;
//! - the result is trimmed to a hard character budget, lowest-confidence facts
//!   first.
//!
//! # Why the merge is code and not a second model call
//!
//! A model asked to rewrite a list will occasionally omit a fact it was not asked
//! to touch. That is invisible in the reply and expensive to discover later. An
//! operation either applies to a row this file can see or it does not, so a fact
//! can only leave the profile through an explicit `remove` or the budget.
//!
//! # Why the budget is characters, not tokens
//!
//! The app has no tokenizer for every provider's vocabulary, and a character cap
//! is deterministic. `MAX_DOCUMENT_CHARS` is sized so the rendered block is
//! comfortably under a thousand tokens for the English prose this profile holds.

use std::cmp::Ordering;
use std::collections::HashSet;

use chrono::{DateTime, Utc};

use crate::database::taste::TasteItemRow;

use super::ops::{RawOp, RawReply};
use super::section::Section;

/// The most text the rendered profile may hold. Roughly 900 English tokens.
pub const MAX_DOCUMENT_CHARS: usize = 3_600;

/// The longest one fact may be. Long enough for a precise sentence, short enough
/// that one fact cannot crowd out the section it belongs to.
pub const MAX_ITEM_CHARS: usize = 220;

/// The shortest text worth storing. Below this it is a label rather than a fact.
const MIN_ITEM_CHARS: usize = 8;

/// A ceiling on row count as well as characters, so a hundred tiny facts cannot
/// turn the profile into a list nobody reads.
const MAX_ITEMS: usize = 48;

/// How much two facts may overlap and still be considered the same fact.
///
/// Deliberately below 1.0: "Prefers Rust for backend services" and "Prefers Rust
/// for backend work" are one preference, and a profile that stored both would
/// spend its budget repeating itself. The containment score in [`similarity`] is
/// what lets a more specific version of an existing fact replace it instead of
/// becoming a second row.
const SIMILARITY: f32 = 0.6;

const DEFAULT_CONFIDENCE: f32 = 0.7;
const CONFIDENCE_FLOOR: f32 = 0.05;
const REINFORCE_STEP: f32 = 0.05;

/// Words that carry no identity on their own. Excluded from similarity so two
/// unrelated sentences cannot look alike because they both say "the user".
const STOPWORDS: [&str; 30] = [
    "a", "an", "and", "are", "as", "at", "be", "but", "by", "for", "from", "has", "have", "in",
    "is", "it", "its", "of", "on", "or", "that", "the", "their", "them", "they", "this", "to",
    "user", "was", "with",
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

    for operation in &reply.ops {
        match operation {
            RawOp::Add {
                section,
                text,
                confidence,
            } => {
                let Some(section) = Section::parse(section) else {
                    outcome.ignored += 1;
                    continue;
                };
                let text = clean_text(text);
                if !is_usable(&text) {
                    outcome.ignored += 1;
                    continue;
                }
                let confidence = confidence
                    .unwrap_or(DEFAULT_CONFIDENCE)
                    .clamp(CONFIDENCE_FLOOR, 1.0);

                // An addition that says what an existing row already says is an
                // edit to that row. The longer wording wins because a model that
                // restates a fact usually does so with more context; the
                // confidence is the higher of the two, because neither statement
                // weakens the other.
                if let Some(index) = similar_index(items, section, &text) {
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
            RawOp::Update {
                id,
                text,
                confidence,
            } => {
                let text = clean_text(text);
                if !is_usable(&text) {
                    outcome.ignored += 1;
                    continue;
                }
                let Some(item) = items.iter_mut().find(|item| item.id == *id) else {
                    outcome.ignored += 1;
                    continue;
                };
                item.text = text;
                if let Some(confidence) = confidence {
                    item.confidence = confidence.clamp(CONFIDENCE_FLOOR, 1.0);
                }
                item.updated_at = now.to_string();
                item.last_seen_at = now.to_string();
                outcome.updated += 1;
            }
            RawOp::Reinforce { id, confidence } => {
                let Some(item) = items.iter_mut().find(|item| item.id == *id) else {
                    outcome.ignored += 1;
                    continue;
                };
                // A reinforcement is agreement, so it can only raise confidence.
                // It never changes the wording: the fact already said what it says.
                let raised = confidence
                    .map(|value| value.clamp(CONFIDENCE_FLOOR, 1.0))
                    .unwrap_or(item.confidence + REINFORCE_STEP);
                item.confidence = item.confidence.max(raised).min(1.0);
                item.last_seen_at = now.to_string();
                outcome.updated += 1;
            }
            RawOp::Remove { id, .. } => {
                let before = items.len();
                items.retain(|item| item.id != *id);
                if items.len() == before {
                    outcome.ignored += 1;
                } else {
                    outcome.removed += 1;
                }
            }
        }
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
        .filter(|item| is_usable(&clean_text(&item.text)))
        .map(|mut item| {
            item.text = clean_text(&item.text);
            item.confidence = item.confidence.clamp(CONFIDENCE_FLOOR, 1.0);
            item
        })
        .collect();
    items.sort_by_key(|item| item.id);

    // Deduplicate rows that predate the similarity rule. The profile is small, so
    // this is a short quadratic walk rather than a second index.
    let mut unique: Vec<TasteItemRow> = Vec::with_capacity(items.len());
    for item in items {
        let section = Section::parse(&item.section).expect("filtered above");
        match unique.iter().position(|existing| {
            Section::parse(&existing.section) == Some(section)
                && similarity(&existing.text, &item.text) >= SIMILARITY
        }) {
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

/// The profile as it is sent to the model for editing.
///
/// Ids, confidence and age are included here and **not** in the chat block: the
/// model needs them to address a row, while the chat needs stable bytes and none
/// of the machinery.
pub fn render_for_model(items: &[TasteItemRow], now: &str) -> String {
    let now = parse_stamp(now);
    let mut out = String::new();
    for section in Section::ALL {
        let mut first = true;
        for item in items
            .iter()
            .filter(|item| Section::parse(&item.section) == Some(section))
        {
            if first {
                if !out.is_empty() {
                    out.push('\n');
                }
                out.push_str(&format!("[{}]\n", section.as_str()));
                first = false;
            }
            // Compact on purpose: `id|confidence|last-seen|text` is read by the
            // model the same way and costs a fraction of the prose form. The
            // system prompt explains the shape once.
            out.push_str(&format!(
                "{}|{:.2}|{}|{}\n",
                item.id,
                item.confidence,
                seen_label(now, &item.last_seen_at),
                item.text
            ));
        }
    }
    out.trim_end().to_string()
}

/// The profile as the chat receives it.
///
/// Only text, grouped by heading. No ids, confidence or age: those change when a
/// fact is reinforced, and a prefix that changes for a reason the conversation
/// cannot see would invalidate a provider's cache for nothing.
pub fn render(items: &[TasteItemRow]) -> String {
    let mut out = String::new();
    for section in Section::ALL {
        let mut first = true;
        for item in items
            .iter()
            .filter(|item| Section::parse(&item.section) == Some(section))
        {
            if first {
                if !out.is_empty() {
                    out.push('\n');
                }
                out.push_str(&format!("[{}]\n", section.as_str()));
                first = false;
            }
            out.push_str("- ");
            out.push_str(&item.text);
            out.push('\n');
        }
    }
    out.trim_end().to_string()
}

/// The complete block prepended to a chat request, or nothing when empty.
///
/// The header is deliberately short and makes two claims only: where this came
/// from, and that it is background rather than an instruction. An empty profile
/// returns `None` rather than a paragraph about having learned nothing.
pub fn render_prompt_block(items: &[TasteItemRow]) -> Option<String> {
    let body = render(items);
    if body.is_empty() {
        return None;
    }
    Some(format!("About the user (background context):\n\n{body}"))
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

/// An existing row in the same section that says what `text` says.
fn similar_index(items: &[TasteItemRow], section: Section, text: &str) -> Option<usize> {
    items.iter().position(|item| {
        Section::parse(&item.section) == Some(section) && similarity(&item.text, text) >= SIMILARITY
    })
}

/// How alike two facts are, by word overlap.
///
/// Two scores, whichever is higher: the Jaccard index for facts of similar length,
/// and containment for a short fact that is entirely present in a longer one
/// ("Prefers Rust" inside "Prefers Rust for backend services"). Containment is
/// what makes an `add` with more detail update the existing row rather than
/// duplicate it.
pub fn similarity(left: &str, right: &str) -> f32 {
    let left = tokens(left);
    let right = tokens(right);
    if left.is_empty() || right.is_empty() {
        return 0.0;
    }
    let intersection = left.intersection(&right).count() as f32;
    let union = left.union(&right).count() as f32;
    let jaccard = intersection / union;
    let containment = intersection / left.len().min(right.len()) as f32;
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

/// One fact with its decoration removed and its whitespace collapsed.
fn clean_text(raw: &str) -> String {
    let collapsed = raw.split_whitespace().collect::<Vec<_>>().join(" ");
    let trimmed = collapsed
        .trim()
        .trim_matches(|character: char| matches!(character, '-' | '*' | '`' | '"' | '\'' | '•'))
        .trim();
    clip(trimmed)
}

/// Validates text a person typed into a fact.
///
/// The same cleaning and floor the model's `add` goes through, so a manual edit
/// cannot store a label or a paragraph that the profile's budget would then have
/// to carry. `None` means the text is not usable; the caller reports that rather
/// than saving it.
pub fn sanitize_user_text(raw: &str) -> Option<String> {
    let cleaned = clean_text(raw);
    is_usable(&cleaned).then_some(cleaned)
}

/// A fact is worth storing when it says something, not just names something.
fn is_usable(text: &str) -> bool {
    let characters = text.chars().count();
    let words = text.split_whitespace().count();
    characters >= MIN_ITEM_CHARS && characters <= MAX_ITEM_CHARS && words >= 2
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

fn parse_stamp(text: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(text)
        .ok()
        .map(|at| at.with_timezone(&Utc))
}

/// How long ago a fact was last seen, in the model's own short form.
fn seen_label(now: Option<DateTime<Utc>>, last_seen: &str) -> String {
    let (Some(now), Some(last_seen)) = (now, parse_stamp(last_seen)) else {
        return "recently".to_string();
    };
    let days = now.signed_duration_since(last_seen).num_days();
    match days {
        days if days <= 0 => "today".to_string(),
        1 => "1d".to_string(),
        days => format!("{days}d"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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

    fn add(section: Section, text: &str, confidence: f32) -> RawOp {
        RawOp::Add {
            section: section.as_str().to_string(),
            text: text.to_string(),
            confidence: Some(confidence),
        }
    }

    fn reply(ops: Vec<RawOp>) -> RawReply {
        RawReply {
            ops,
            report: String::new(),
            ignored: 0,
        }
    }

    #[test]
    fn an_added_fact_is_precise_and_stored_verbatim() {
        let mut items = Vec::new();
        let outcome = merge(
            &mut items,
            &reply(vec![add(
                Section::Work,
                "Prefers Rust (programming language) for backend services.",
                0.9,
            )]),
            "2026-02-01T00:00:00.000Z",
        );
        assert_eq!(outcome.added, 1);
        assert_eq!(
            items[0].text,
            "Prefers Rust (programming language) for backend services."
        );
        assert_eq!(items[0].section, "work");
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
            &reply(vec![add(
                Section::Work,
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
            &reply(vec![add(
                Section::Work,
                "Prefers Python (programming language) for data analysis.",
                0.8,
            )]),
            "2026-02-01T00:00:00.000Z",
        );
        assert_eq!(items.len(), 2);
    }

    #[test]
    fn an_update_names_the_row_it_changes() {
        let mut items = vec![item(
            7,
            Section::Preferences,
            "Prefers light themes in every app.",
            0.6,
        )];
        let outcome = merge(
            &mut items,
            &reply(vec![RawOp::Update {
                id: 7,
                text: "Prefers dark themes in editors, light themes elsewhere.".into(),
                confidence: Some(0.95),
            }]),
            "2026-02-01T00:00:00.000Z",
        );
        assert_eq!(outcome.updated, 1);
        assert_eq!(
            items[0].text,
            "Prefers dark themes in editors, light themes elsewhere."
        );
        assert_eq!(items[0].confidence, 0.95);
    }

    #[test]
    fn reinforce_raises_confidence_without_changing_the_wording() {
        let mut items = vec![item(4, Section::Interests, "Draws regularly.", 0.7)];
        let outcome = merge(
            &mut items,
            &reply(vec![RawOp::Reinforce {
                id: 4,
                confidence: None,
            }]),
            "2026-02-01T00:00:00.000Z",
        );
        assert_eq!(outcome.updated, 1);
        assert_eq!(items[0].confidence, 0.75);
        assert_eq!(items[0].text, "Draws regularly.");
    }

    #[test]
    fn removing_and_unknown_ids_are_counted_correctly() {
        let mut items = vec![item(
            1,
            Section::Dislikes,
            "Dislikes loud notifications.",
            0.8,
        )];
        let outcome = merge(
            &mut items,
            &reply(vec![
                RawOp::Remove {
                    id: 1,
                    _reason: Some("contradicted".into()),
                },
                RawOp::Reinforce {
                    id: 99,
                    confidence: Some(0.9),
                },
            ]),
            "2026-02-01T00:00:00.000Z",
        );
        assert_eq!(outcome.removed, 1);
        assert_eq!(outcome.ignored, 1);
        assert!(items.is_empty());
    }

    #[test]
    fn an_unknown_section_or_a_label_is_ignored() {
        let mut items = Vec::new();
        let outcome = merge(
            &mut items,
            &reply(vec![
                RawOp::Add {
                    section: "misc".into(),
                    text: "Something true and long enough.".into(),
                    confidence: Some(0.8),
                },
                add(Section::Work, "Rust", 0.9),
            ]),
            "2026-02-01T00:00:00.000Z",
        );
        assert_eq!(outcome.ignored, 2);
        assert!(items.is_empty());
    }

    #[test]
    fn the_rendered_profile_is_stable_and_carries_no_machinery() {
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
            "[identity]\n- Uses Windows 11 with an RTX 5090 (GPU).\n\n[work]\n- Prefers Rust (programming language)."
        );
        assert!(!rendered.contains("id "));
        assert!(!rendered.contains("0.9"));
    }

    #[test]
    fn the_model_sees_ids_confidence_and_age() {
        let items = vec![item(
            4,
            Section::Work,
            "Prefers Rust (programming language).",
            0.9,
        )];
        let rendered = render_for_model(&items, "2026-01-03T00:00:00.000Z");
        assert!(rendered.contains("4|0.90|2d|"), "{rendered}");
    }

    #[test]
    fn the_budget_drops_the_weakest_fact_first() {
        let mut items: Vec<TasteItemRow> = (0..60)
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
        // The one strong fact must survive the trim.
        items.push(item(
            100,
            Section::Identity,
            "Uses Windows 11 on a desktop with an RTX 5090 (GPU).",
            0.99,
        ));
        let normalized = normalize(items);
        assert!(normalized.len() < 61);
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
    /// and ordinary extra whitespace is cleaned before it is stored.
    #[test]
    fn a_manually_typed_fact_is_cleaned_and_checked() {
        assert_eq!(
            sanitize_user_text("  Prefers   Rust (programming language) for backend work.  "),
            Some("Prefers Rust (programming language) for backend work.".to_string())
        );
        assert_eq!(sanitize_user_text("Rust"), None);
        assert_eq!(sanitize_user_text("   "), None);
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
