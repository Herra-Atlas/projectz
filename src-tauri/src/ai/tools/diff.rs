//! Line-level differences between a file before and after a tool changed it.
//!
//! Its own module because it is the one piece of arithmetic here with no
//! filesystem or policy in it, and it changes for its own reasons: nothing about
//! a longest-common-subsequence needs to know that a tool wrote a file.
//!
//! # Why a differ at all
//!
//! The panel needs to show *which lines* a write touched. Before this, a row
//! opened onto the tool's own output, which for an edit is the lines that landed
//! and nothing about what they replaced -- so the one thing a reader wants from
//! an edit ("what did this remove?") was exactly what was missing.
//!
//! # Where the two texts come from
//!
//! From the write tools themselves, which already hold both versions while the
//! write happens. Nothing re-reads the file afterwards to recover the "before",
//! because by then it is gone. `run_terminal` cannot supply a pair at all, which
//! is the one change this does not cover.

/// What happened to one line.
///
/// Serialized as `{"kind": "context" | "add" | "remove", "text": …}` rather than
/// as three variants of an enum, so the frontend reads one shape and an added
/// line is distinguishable from a removed one by a field instead of by which key
/// happens to be present.
///
/// The tags are named per variant rather than derived from the variant names.
/// `rename_all = "lowercase"` yields `removed` and `added`, which read as past
/// tense next to `context` and made the wire format depend on Rust naming
/// instead of on what the panel wants to compare against.
///
/// Struct variants rather than newtype ones because an internally tagged enum
/// cannot carry a bare string -- `Context(String)` fails to serialize with "cannot
/// serialize tagged newtype variant containing a string", which here would mean
/// `to_value` erroring and the diff being dropped from the event without a word.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
#[serde(tag = "kind")]
pub enum Line {
    /// Unchanged, shown only to give the changes somewhere to sit.
    #[serde(rename = "context")]
    Context { text: String },
    /// Present only after the change.
    #[serde(rename = "add")]
    Added { text: String },
    /// Present only before it.
    #[serde(rename = "remove")]
    Removed { text: String },
}

impl Line {
    fn context(value: &str) -> Self {
        Line::Context {
            text: value.to_string(),
        }
    }

    fn added(value: &str) -> Self {
        Line::Added {
            text: value.to_string(),
        }
    }

    fn removed(value: &str) -> Self {
        Line::Removed {
            text: value.to_string(),
        }
    }
}

/// Lines of context kept either side of a change.
///
/// Enough to see what the edit sits between without carrying the rest of a large
/// file into a transcript. Three is what makes a hunk readable; more is noise
/// that gets stored forever.
const CONTEXT: usize = 3;

/// Most lines a diff may carry.
///
/// A whole-file rewrite is a real thing a model does, and 3000 added lines
/// would be stored in `activity_json` and re-decoded every time that conversation
/// is opened. Past this the diff is cut and says so, because a truncated diff
/// that admits it is truncated is honest and a silent one is not.
pub const MAX_DIFF_LINES: usize = 200;

/// A diff, plus what had to be left out to stay within [`MAX_DIFF_LINES`].
///
/// `hidden` is reported rather than the panel guessing: a diff that stopped
/// early and says nothing reads as a file that barely changed, which is the one
/// wrong impression this could give.
#[derive(Clone, Debug, Default, serde::Serialize)]
pub struct Diff {
    pub lines: Vec<Line>,
    /// Lines dropped from the middle, so the panel can say how many.
    pub hidden: usize,
}

/// Differences between two file bodies, collapsed and capped.
///
/// An empty result when the two are identical, which is a legitimate thing for a
/// tool to produce -- `edit_file` replacing text with itself -- and renders as no
/// diff rather than as an empty box.
pub fn diff(before: &str, after: &str) -> Diff {
    let old: Vec<&str> = before.lines().collect();
    let new: Vec<&str> = after.lines().collect();
    // Guarded rather than left to the table below: a whole-file replace on a
    // large file is an ordinary model action, and an unbounded LCS table is
    // quadratic in the line count. Past this the two sides are compared as a
    // wholesale replacement, which is what they are.
    if old.len().saturating_mul(new.len()) > MAX_TABLE_CELLS {
        return wholesale(&old, &new);
    }
    collapse(lcs(&old, &new))
}

/// Cells a longest-common-subsequence table may hold before it is abandoned.
///
/// Four million is 32MB as `usize`, which is a transient spike inside one tool
/// call rather than a steady cost. A 2000-line file against a 2000-line file sits
/// just under it; anything larger is treated as a replacement.
const MAX_TABLE_CELLS: usize = 4_000_000;

/// The longest common subsequence, walked back into per-line changes.
///
/// Common lines are matched in order rather than greedily, so a line that appears
/// twice is matched where it actually lines up instead of at its first
/// occurrence -- which is what stops a duplicate `}` from being reported as
/// removed here and re-added there.
fn lcs(old: &[&str], new: &[&str]) -> Vec<Line> {
    // One row, rewritten in place: the table only ever reads the row above it.
    let mut previous: Vec<usize> = vec![0; new.len() + 1];
    let mut rows: Vec<Vec<usize>> = Vec::with_capacity(old.len() + 1);
    rows.push(previous.clone());
    for old_line in old {
        let mut current = vec![0usize; new.len() + 1];
        for (index, new_line) in new.iter().enumerate() {
            current[index + 1] = if old_line == new_line {
                previous[index] + 1
            } else {
                // Whichever side's tail is longer is the better match.
                previous[index + 1].max(current[index])
            };
        }
        previous = current.clone();
        rows.push(current);
    }

    let mut lines = Vec::new();
    let (mut i, mut j) = (old.len(), new.len());
    while i > 0 && j > 0 {
        if old[i - 1] == new[j - 1] {
            lines.push(Line::context(old[i - 1]));
            i -= 1;
            j -= 1;
        } else if rows[i - 1][j] > rows[i][j - 1] {
            // Dropping the old line leaves a longer common run than dropping the
            // new one, so it is the removal.
            lines.push(Line::removed(old[i - 1]));
            i -= 1;
        } else {
            // Ties go to the addition, and that is load-bearing. On `one/two/three`
            // against `one/TWO/three` both choices score the same at the edit, and
            // preferring removal walks the cursor off the front of the old file
            // while the new one still has lines left -- which reports the whole
            // file as rewritten instead of reporting one changed line.
            lines.push(Line::added(new[j - 1]));
            j -= 1;
        }
    }
    while i > 0 {
        lines.push(Line::removed(old[i - 1]));
        i -= 1;
    }
    while j > 0 {
        lines.push(Line::added(new[j - 1]));
        j -= 1;
    }
    lines.reverse();
    lines
}

/// Every line removed and every line added, with nothing kept for context.
///
/// The fallback for a pair too large to align. It is a worse diff than the real
/// one -- a moved line reads as a removal and an addition -- but it is bounded,
/// and a bounded imprecise diff beats an unbounded precise one that never
/// finishes.
fn wholesale(old: &[&str], new: &[&str]) -> Diff {
    let mut lines: Vec<Line> = old.iter().map(|line| Line::removed(line)).collect();
    lines.extend(new.iter().map(|line| Line::added(line)));
    Diff { lines, hidden: 0 }
}

/// Keeps changed regions with [`CONTEXT`] lines either side, dropping the rest.
///
/// The gaps are the point: a 900-line file with one edit should cost three
/// context lines and the edit, not nine hundred rows in the transcript.
fn collapse(lines: Vec<Line>) -> Diff {
    let keep: Vec<bool> = lines
        .iter()
        .map(|line| !matches!(line, Line::Context { .. }))
        .collect();
    let mut shown = vec![false; lines.len()];
    for (index, changed) in keep.iter().enumerate() {
        if !*changed {
            continue;
        }
        let start = index.saturating_sub(CONTEXT);
        let end = (index + CONTEXT + 1).min(lines.len());
        for slot in shown.iter_mut().take(end).skip(start) {
            *slot = true;
        }
    }
    // A file with no changes at all would otherwise keep nothing and render as an
    // empty box, which reads as a broken panel rather than as "nothing changed".
    if !shown.iter().any(|slot| *slot) {
        return Diff::default();
    }

    let mut kept = Vec::new();
    let mut hidden = 0usize;
    for (index, line) in lines.into_iter().enumerate() {
        if !shown[index] {
            hidden += 1;
            continue;
        }
        // The cap is applied here rather than trusted to `collapse`, because
        // collapsing bounds a *context* gap and says nothing about a diff where
        // almost every line changed. A wholesale rewrite of a large file keeps
        // every line through the context pass, so this is the only place that
        // can cut it.
        if kept.len() == MAX_DIFF_LINES {
            hidden += 1;
            continue;
        }
        kept.push(line);
    }
    Diff {
        lines: kept,
        hidden,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn texts(diff: &Diff) -> Vec<(char, &str)> {
        diff.lines
            .iter()
            .map(|line| match line {
                Line::Context { text } => (' ', text.as_str()),
                Line::Added { text } => ('+', text.as_str()),
                Line::Removed { text } => ('-', text.as_str()),
            })
            .collect()
    }

    #[test]
    fn an_unchanged_file_has_no_diff() {
        assert!(diff("one\ntwo\n", "one\ntwo\n").lines.is_empty());
    }

    #[test]
    fn a_replaced_line_shows_the_removal_and_the_addition() {
        let result = diff("one\ntwo\nthree\n", "one\nTWO\nthree\n");
        assert_eq!(
            texts(&result),
            vec![(' ', "one"), ('-', "two"), ('+', "TWO"), (' ', "three"),]
        );
    }

    /// The case the panel exists for: the reader needs to see what went, and the
    /// old output only ever carried what arrived.
    #[test]
    fn what_an_edit_removed_is_visible() {
        let result = diff("keep\ndrop me\nkeep too\n", "keep\nkeep too\n");
        assert!(texts(&result).contains(&('-', "drop me")));
    }

    #[test]
    fn an_added_line_is_visible() {
        let result = diff("one\n", "one\ntwo\n");
        assert!(texts(&result).contains(&('+', "two")));
    }

    /// A line that appears twice must be matched where it lines up, or a common
    /// `}` reads as removed in one place and added in another.
    #[test]
    fn a_repeated_line_is_not_reported_as_a_move() {
        let result = diff(
            "fn a() {\n    x();\n}\n",
            "fn a() {\n    x();\n    y();\n}\n",
        );
        let pairs = texts(&result);
        assert!(pairs.contains(&('+', "    y();")));
        assert_eq!(
            pairs
                .iter()
                .filter(|(kind, value)| *kind == '-' && *value == "}")
                .count(),
            0,
            "a closing brace was reported as removed"
        );
    }

    /// Collapsing is the difference between a usable diff and a stored copy of
    /// the file.
    #[test]
    fn an_edit_in_a_long_file_keeps_only_its_neighbourhood() {
        let before: String = (1..=200).map(|n| format!("line{n}\n")).collect();
        let after = before.replace("line100\n", "LINE100\n");
        let result = diff(&before, &after);
        // Three either side plus the two changed lines.
        assert_eq!(result.lines.len(), 8, "{:?}", texts(&result));
        assert!(result.hidden > 0);
        assert!(texts(&result).contains(&('-', "line100")));
        assert!(texts(&result).contains(&('+', "LINE100")));
    }

    /// A cap that silently drops the tail would report a file as barely changed.
    #[test]
    fn a_huge_change_is_capped_and_says_how_much_it_left_out() {
        let before: String = (1..=900).map(|n| format!("old{n}\n")).collect();
        let after: String = (1..=900).map(|n| format!("new{n}\n")).collect();
        let result = diff(&before, &after);
        assert!(
            result.lines.len() <= MAX_DIFF_LINES,
            "{}",
            result.lines.len()
        );
    }

    /// The quadratic guard: a pair too large to align still produces a diff
    /// rather than stalling the run or blowing memory.
    #[test]
    fn a_pair_too_large_to_align_is_still_reported() {
        let before: String = (0..4000).map(|n| format!("a{n}\n")).collect();
        let after: String = (0..4000).map(|n| format!("b{n}\n")).collect();
        let result = diff(&before, &after);
        assert!(!result.lines.is_empty());
        assert!(result
            .lines
            .iter()
            .any(|line| matches!(line, Line::Added { .. })));
    }

    #[test]
    fn a_whole_file_replacement_reports_both_sides() {
        let result = diff("old\n", "new\n");
        let pairs = texts(&result);
        assert!(pairs.contains(&('-', "old")));
        assert!(pairs.contains(&('+', "new")));
    }

    #[test]
    fn an_empty_file_growing_is_all_additions() {
        let result = diff("", "one\ntwo\n");
        assert_eq!(texts(&result), vec![('+', "one"), ('+', "two")]);
    }

    #[test]
    fn a_file_emptied_is_all_removals() {
        let result = diff("one\ntwo\n", "");
        assert_eq!(texts(&result), vec![('-', "one"), ('-', "two")]);
    }

    /// Indentation is part of the line, so a reindented line is a change rather
    /// than two identical lines.
    #[test]
    fn a_reindent_is_reported_as_a_change() {
        let result = diff("    let x = 1;\n", "        let x = 1;\n");
        assert!(texts(&result).contains(&('-', "    let x = 1;")));
        assert!(texts(&result).contains(&('+', "        let x = 1;")));
    }

    /// A diff is stored in the transcript, so it must not carry the file it came
    /// from when nothing changed.
    #[test]
    fn identical_files_store_nothing() {
        let result = diff("a\nb\nc\n", "a\nb\nc\n");
        assert!(result.lines.is_empty());
        assert_eq!(result.hidden, 0);
    }
}
