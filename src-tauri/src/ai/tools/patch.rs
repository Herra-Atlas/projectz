//! The one text-matching engine behind every edit the model makes.
//!
//! `edit_file` changes a workspace file and `skill_manage` changes a skill's
//! `SKILL.md`; both need the same rules, and a second copy of them would be a
//! second thing to keep correct. The rules are:
//!
//! - **Exact first, then lenient.** An exact match is tried before anything is
//!   folded, so the lenient pass can never override a precise one. It exists for
//!   drift that is not a mistake: a curly quote where the model typed a straight
//!   one, a tab where it typed spaces, a block reproduced with the wrong
//!   indentation. Every such failure used to cost a round trip.
//! - **Ambiguity is refused, never guessed.** Two matches is an error, in both
//!   passes. Picking the first is how a model silently edits the wrong function
//!   and reports success.
//! - **A batch is all-or-nothing.** Changes are applied in memory against the
//!   running text, so a later change sees the result of an earlier one, and the
//!   caller writes once and only if every change landed.
//!
//! # Why the messages are generic
//!
//! The not-found message says what to do rather than naming a sibling tool: this
//! file serves two tools, and the alternative one differs between them. Each
//! tool's own description carries its alternative (`edit_file` points at
//! `edit_lines`, `skill_manage` at `skill_read`).

use serde_json::Value;

/// One replacement, from either the single-change or the batch form.
pub(crate) struct Change {
    pub(crate) old_text: String,
    pub(crate) new_text: String,
    pub(crate) replace_all: bool,
}

/// The result of applying one change.
struct Applied {
    body: String,
    /// The text was located by the lenient pass rather than exactly.
    lenient: bool,
}

/// Reads the changes from whichever shape the caller used.
///
/// Both are accepted because the single-change shape is the one every model was
/// trained on, and the batch is the improvement. Taking only the batch would make
/// a one-line edit carry an array it does not need; taking both costs one branch
/// and keeps the familiar call working exactly as before.
pub(crate) fn changes_of(arguments: &Value) -> Result<Vec<Change>, String> {
    let batch = arguments.get("edits");
    let single = arguments.get("old_text").is_some() || arguments.get("new_text").is_some();
    if batch.is_some() && single {
        return Err(
            "Pass either `edits` (a list of changes) or `old_text`/`new_text` (one change), not \
             both."
                .to_string(),
        );
    }
    if let Some(batch) = batch {
        let list = batch
            .as_array()
            .ok_or("`edits` must be an array of changes")?;
        if list.is_empty() {
            return Err("`edits` is empty; there is nothing to change.".to_string());
        }
        return list.iter().map(change_from).collect();
    }
    Ok(vec![change_from(arguments)?])
}

/// Whether the caller offered any change at all, in either shape.
///
/// Split out because `skill_manage edit` may carry a description and no text
/// change, which is a legitimate call rather than a missing argument.
pub(crate) fn has_changes(arguments: &Value) -> bool {
    arguments.get("edits").is_some()
        || arguments.get("old_text").is_some()
        || arguments.get("new_text").is_some()
}

/// Validates one change object, wherever it came from.
fn change_from(value: &Value) -> Result<Change, String> {
    let old_text = value
        .get("old_text")
        .and_then(Value::as_str)
        .ok_or("a change needs a string `old_text`")?
        .to_string();
    // An empty needle would match at every position and the count would be the
    // file's length in bytes. Reported plainly rather than as a match count.
    if old_text.is_empty() {
        return Err("`old_text` cannot be empty; there would be nothing to identify".to_string());
    }
    let new_text = value
        .get("new_text")
        .and_then(Value::as_str)
        .ok_or("a change needs a string `new_text`")?
        .to_string();
    let replace_all = value
        .get("replace_all")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    Ok(Change {
        old_text,
        new_text,
        replace_all,
    })
}

/// Applies every change in order, all or nothing.
///
/// Answers with the text after the last change, and how many of them were located
/// by the lenient pass -- the one case worth reporting back to the model, because
/// the text it gave did not match byte for byte.
pub(crate) fn apply_all(body: &str, edits: &[Change]) -> Result<(String, usize), String> {
    let mut updated = body.to_string();
    let mut lenient = 0usize;
    for (index, change) in edits.iter().enumerate() {
        let applied = apply(&updated, change)
            .map_err(|error| format!("Change {} of {}: {error}", index + 1, edits.len()))?;
        if applied.lenient {
            lenient += 1;
        }
        updated = applied.body;
    }
    Ok((updated, lenient))
}

/// Applies one change: exactly first, then leniently.
fn apply(body: &str, change: &Change) -> Result<Applied, String> {
    let occurrences = body.matches(&change.old_text).count();
    if occurrences > 1 && !change.replace_all {
        // Ambiguity is refused rather than guessed. Picking the first of two
        // matches is how a model silently edits the wrong function, and it is
        // invisible in the diff it reports back.
        return Err(format!(
            "`old_text` appears {occurrences} times. Include more surrounding text to identify \
             one, or set replace_all to change all of them."
        ));
    }
    if occurrences > 0 {
        let updated = if change.replace_all {
            body.replace(&change.old_text, &change.new_text)
        } else {
            body.replacen(&change.old_text, &change.new_text, 1)
        };
        return Ok(Applied {
            body: updated,
            lenient: false,
        });
    }
    // Nothing matched exactly. Look again with indentation, whitespace runs and
    // look-alike punctuation folded away.
    let candidates = lenient_ranges(body, &change.old_text);
    match candidates.len() {
        0 => Err(
            "`old_text` was not found. Read the file and copy the text exactly as it appears, \
             including its indentation, or copy a shorter run that you can see."
                .to_string(),
        ),
        1 => {
            let (start, end) = candidates[0];
            let mut updated = String::with_capacity(body.len() + change.new_text.len());
            updated.push_str(&body[..start]);
            updated.push_str(&change.new_text);
            updated.push_str(&body[end..]);
            Ok(Applied {
                body: updated,
                lenient: true,
            })
        }
        // The exact path refuses ambiguity rather than guessing, and this one has
        // to as well. Two blocks that differ only in indentation is exactly the
        // case the lenient pass exists for, so replacing the first of them edits a
        // place the model never named -- and reports success.
        count if !change.replace_all => Err(format!(
            "`old_text` appears {count} times once whitespace and punctuation differences are \
             ignored. Include more surrounding text to identify one, or set replace_all to change \
             all of them."
        )),
        _ => {
            // Applied from the end backwards, so an earlier range's offsets are
            // still valid when its turn comes. A window that overlaps one already
            // replaced is skipped: applying it would rewrite text that is no
            // longer the text it was matched against.
            let mut updated = body.to_string();
            let mut next_start = usize::MAX;
            for (start, end) in candidates.into_iter().rev() {
                if end > next_start {
                    continue;
                }
                next_start = start;
                updated.replace_range(start..end, &change.new_text);
            }
            Ok(Applied {
                body: updated,
                lenient: true,
            })
        }
    }
}

/// Finds every byte range where `needle` occurs, ignoring per-line whitespace and
/// look-alike punctuation.
///
/// Line-based rather than character-based on purpose: the drift this exists to
/// absorb -- a re-indented block, tabs where spaces were typed, a curly apostrophe
/// -- is per-line, and comparing whole lines cannot slide past a boundary and
/// match the wrong function the way a character window could.
///
/// All of them, not the first: the count is what lets [`apply`] refuse an
/// ambiguous lenient match the same way the exact pass does.
fn lenient_ranges(body: &str, needle: &str) -> Vec<(usize, usize)> {
    let wanted: Vec<String> = needle.lines().map(normalise).collect();
    if wanted.is_empty() {
        return Vec::new();
    }
    let spans = line_spans(body);
    if spans.len() < wanted.len() {
        return Vec::new();
    }
    // Folded once for the whole body rather than once per candidate window. The
    // previous shape normalised every line again at every offset, which is
    // O(lines x needle lines) allocations on the path taken exactly when matching
    // is already hard.
    let folded: Vec<String> = spans.iter().map(|(_, _, text)| normalise(text)).collect();
    let mut found = Vec::new();
    for start in 0..=spans.len() - wanted.len() {
        let matched = wanted
            .iter()
            .enumerate()
            .all(|(offset, expected)| folded[start + offset] == *expected);
        if matched {
            found.push((spans[start].0, spans[start + wanted.len() - 1].1));
        }
    }
    found
}

/// Each line of `body` as `(start byte, end byte, text)`, the range excluding the
/// line's newline.
fn line_spans(body: &str) -> Vec<(usize, usize, &str)> {
    let mut spans = Vec::new();
    let mut offset = 0;
    for chunk in body.split_inclusive('\n') {
        let text = chunk.strip_suffix('\n').unwrap_or(chunk);
        let text = text.strip_suffix('\r').unwrap_or(text);
        spans.push((offset, offset + text.len(), text));
        offset += chunk.len();
    }
    spans
}

/// Folds whitespace and look-alike punctuation so two lines that read the same
/// compare equal.
///
/// Only the look-alikes a model actually produces are folded -- curly quotes and
/// dashes -- rather than a broad Unicode normalisation, because a fold that
/// rewrote more could make two genuinely different lines compare equal and the
/// lenient pass would then change the wrong one.
fn normalise(line: &str) -> String {
    let folded: String = line
        .chars()
        .map(|character| match character {
            '\u{2018}' | '\u{2019}' | '\u{201b}' => '\'',
            '\u{201c}' | '\u{201d}' | '\u{201f}' => '"',
            '\u{2012}' | '\u{2013}' | '\u{2014}' | '\u{2015}' => '-',
            other => other,
        })
        .collect();
    folded.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn change(old_text: &str, new_text: &str) -> Change {
        Change {
            old_text: old_text.to_string(),
            new_text: new_text.to_string(),
            replace_all: false,
        }
    }

    #[test]
    fn one_exact_occurrence_is_replaced() {
        let (body, lenient) = apply_all("let a = 1;\n", &[change("a = 1", "a = 2")]).expect("edit");
        assert_eq!(body, "let a = 2;\n");
        assert_eq!(lenient, 0, "an exact match must not report lenient");
    }

    /// The rule that makes a wrong edit loud instead of silent.
    #[test]
    fn an_ambiguous_match_is_refused() {
        let error = apply_all("x\nx\n", &[change("x", "y")]).expect_err("ambiguous");
        assert!(error.contains("2 times"), "{error}");
    }

    /// Indentation drift is the case the lenient pass exists for, and it is
    /// reported so the model knows the text it sent did not match byte for byte.
    ///
    /// A tab against spaces rather than two spaces against four: the exact pass
    /// would find the latter as a substring of the former, so it never reaches the
    /// folded one.
    #[test]
    fn indentation_drift_is_found_and_reported() {
        let (body, lenient) = apply_all(
            "fn a() {\n\tlet x = 1;\n}\n",
            &[change("  let x = 1;", "  let x = 2;")],
        )
        .expect("lenient");
        assert_eq!(body, "fn a() {\n  let x = 2;\n}\n");
        assert_eq!(lenient, 1);
    }

    #[test]
    fn text_that_is_not_there_is_refused() {
        let error = apply_all("one\n", &[change("two", "three")]).expect_err("missing");
        assert!(error.contains("was not found"), "{error}");
    }

    /// A batch is all-or-nothing: the second change failing must not leave the
    /// first one applied, and the message names which change broke.
    #[test]
    fn a_failed_change_names_itself_and_nothing_is_returned() {
        let error = apply_all("one\n", &[change("one", "two"), change("absent", "x")])
            .expect_err("second change fails");
        assert!(error.contains("Change 2 of 2"), "{error}");
    }

    /// A later change sees the result of an earlier one, which is why the batch
    /// is applied against the running text rather than the original.
    #[test]
    fn a_batch_applies_against_the_running_text() {
        let edits = [
            Change {
                old_text: "middle".into(),
                new_text: "center".into(),
                replace_all: false,
            },
            Change {
                old_text: "a center b".into(),
                new_text: "done".into(),
                replace_all: true,
            },
        ];
        let (body, _) = apply_all("a middle b", &edits).expect("batch");
        assert_eq!(body, "done");
    }

    /// Both argument shapes are read, and mixing them is a mistake rather than a
    /// precedence question.
    #[test]
    fn the_two_argument_shapes_do_not_mix() {
        let one = changes_of(&json!({ "old_text": "a", "new_text": "b" })).expect("single");
        assert_eq!(one.len(), 1);
        let many =
            changes_of(&json!({ "edits": [{ "old_text": "a", "new_text": "b" }] })).expect("batch");
        assert_eq!(many.len(), 1);
        assert!(changes_of(&json!({
            "old_text": "a",
            "new_text": "b",
            "edits": [{ "old_text": "c", "new_text": "d" }]
        }))
        .is_err());
        assert!(changes_of(&json!({ "edits": [] })).is_err());
        assert!(changes_of(&json!({ "old_text": "", "new_text": "b" })).is_err());
    }
}
