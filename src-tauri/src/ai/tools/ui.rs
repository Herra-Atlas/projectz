//! How one tool call reads in the transcript.
//!
//! The panel above an assistant reply needs a human label and the thing it was
//! pointed at: "Read file" beside `src/App.tsx`, not `read_file` beside a JSON
//! blob. That description belongs here rather than in the frontend for two
//! reasons.
//!
//! **The frontend must not re-derive it.** A label computed from the arguments
//! in TypeScript is a second implementation of the same fact, and the two drift
//! the first time a tool is changed. Worse, the arguments of `write_file` carry
//! the entire file body, so anything computed there would have to handle a
//! megabyte to render one line.
//!
//! **The outcome comes from the tool's own text.** "100 results", "12 lines" and
//! "exit code 101" are read out of what the tool reported, so the number in the
//! panel and the number the model was given cannot disagree.
//!
//! Grouping is deliberately *not* here. Folding three consecutive reads into one
//! row needs the whole round, which only the chat loop and the frontend hold, so
//! [`label`] is singular and the plural is applied by whoever is laying out
//! consecutive calls.

use serde_json::Value;

/// One call, described for the user rather than for the model.
#[derive(Clone, Debug)]
pub struct ToolSummary {
    /// Verb phrase naming what the tool did, singular: "Read file".
    pub label: &'static str,
    /// What it was pointed at: a path, a query, a command line.
    pub detail: String,
    /// What came of it, or why it failed.
    ///
    /// `Err` does not mean the run ended: a tool that fails is reported to the
    /// model so it can correct itself. It means *this call did not do what was
    /// asked*, which is what the panel marks rather than hides.
    pub outcome: Result<String, String>,
    /// What the call changed, line by line.
    ///
    /// Set by the chat loop rather than derived here: this module sees one call's
    /// result text, and the "before" a diff needs has already been overwritten by
    /// the time a write tool reports. The loop drains it off the sink, where both
    /// versions still existed.
    pub diff: Option<super::diff::Diff>,
    /// True when this call brought the file into existence rather than editing one.
    ///
    /// Carried as its own flag rather than left to the frontend, which would
    /// otherwise have to recognise "Created " in the result prose to tell a new
    /// file from a rewritten one. Rewording that message would then silently turn
    /// every new file into an edit, which is exactly the distinction a reader is
    /// scanning the summary for. Set from the tool's own knowledge of whether the
    /// path existed.
    pub created: bool,
}

impl ToolSummary {
    /// The event payload for this call.
    ///
    /// `failed` is flattened out of the `Result` rather than serialized as one,
    /// because `serde` would write `{"Err": "…"}` and the frontend would then
    /// have to know that a tool's outcome arrives tagged. Timing is added by the
    /// caller, which is the only place that knows it.
    pub fn event_fields(&self) -> serde_json::Value {
        let mut fields = serde_json::json!({
            "label": self.label,
            "detail": self.detail,
            "outcome": self.outcome.as_ref().ok().or_else(|| self.outcome.as_ref().err()),
            "failed": self.outcome.is_err(),
        });
        // Absent for anything that did not change a file. A `read_file` carrying
        // an empty diff would be a field every non-write row has to check for, and
        // one that means the same as not having it.
        //
        // Inserted into the object rather than assigned through `fields["diff"]`:
        // indexing a `Value` that is `Null` at that key with `IndexMut` *replaces*
        // the null, so the assignment compiles and silently drops the diff.
        if let (Some(diff), Some(object)) = (&self.diff, fields.as_object_mut()) {
            if let Ok(value) = serde_json::to_value(diff) {
                object.insert("diff".into(), value);
            }
        }
        // Only where it means something, for the same reason as the diff: absent
        // on every read rather than a `false` each of them has to ignore.
        if self.created {
            if let Some(object) = fields.as_object_mut() {
                object.insert("created".into(), serde_json::Value::Bool(true));
            }
        }
        fields
    }

    /// Records what this call changed.
    ///
    /// A setter rather than an argument to [`summarize`] because the diff is not
    /// knowable when the summary is built: it is drained off the sink only after
    /// the tool has returned, by which point the summary already exists.
    pub fn attach_diff(&mut self, diff: super::diff::Diff) {
        self.diff = Some(diff);
    }
}

/// Describes one call.
///
/// A tool with no entry here falls back to its own name and its arguments, which
/// is deliberately a fallback rather than a gap: an undescribed tool still gets
/// a row, and a tool row that says less than it should is better than one that
/// is missing.
pub fn summarize(tool: &str, arguments: &Value, result: &str, succeeded: bool) -> ToolSummary {
    ToolSummary {
        label: label_for(tool),
        detail: detail_for(tool, arguments),
        outcome: if succeeded {
            outcome_for(tool, result)
        } else {
            // The tool's own text names the reason, and the model has just been
            // told the same thing, so the panel is not inventing a second
            // wording for one failure.
            Err(first_line(result).to_string())
        },
        diff: None,
        // Only `write_file` can bring a file into existence; the two edit tools
        // are refused rather than creating one. Read here rather than plumbed
        // through from `write.rs` so the one place that recognises a created file
        // is this one, next to the wording it recognises.
        created: tool == "write_file" && succeeded && created_wording(result),
    }
}

/// Whether a `write_file` result reports a file that did not exist before.
///
/// Matched against the wording `write.rs` produces, which is the only place that
/// knows the answer. The alternative was carrying the fact on the summary from
/// the tool, which means changing the sink's shape and every test that builds
/// one by hand; this is a phrase the tool already chose, read once, in the module
/// whose whole job is describing calls to the user.
///
/// Narrow on purpose: the other branch is "Wrote N bytes to …, replacing its
/// contents", which is an edit and must not match. Only a result that actually
/// names a creation counts, so a reworded message degrades to "every file reads
/// as an edit" rather than to "every file reads as new".
fn created_wording(result: &str) -> bool {
    first_line(result).starts_with("Created ")
}

/// Largest tool output kept for the panel, in characters.
///
/// Separate from [`super::output::MAX_TOOL_OUTPUT`] on purpose. That one bounds
/// what a *model* has to read, and is deliberately generous because a truncated
/// source file is a file the model cannot work with. This one bounds what is
/// stored in the transcript, where a `read_file` of a large file or a verbose
/// build log would be re-loaded and re-decoded every time that conversation is
/// opened. The panel is for reading a result at a glance, not for reproducing one.
pub const MAX_PANEL_OUTPUT: usize = 4_000;

/// The tool's own result text, capped for the panel, or `None` when there is
/// nothing worth showing.
///
/// **Nothing for a failure.** `outcome` already carries the reason on the row
/// itself, and printing it a second time below the row says the same thing twice
/// about the same event. The panel earns its space by showing what a *successful*
/// call produced.
///
/// **Nothing for a denial or a malformed call.** Both are one short sentence that
/// the row already reports, and a refusal message repeated beneath itself reads as
/// an error rather than as a declined permission.
///
/// **Head and tail, like every other cap here.** A compiler diagnostic and a
/// failing assertion are both at the end, so keeping only the head would discard
/// the part that explains the failure.
pub fn panel_snippet(result: &str, failed: bool) -> Option<String> {
    let trimmed = result.trim();
    if failed || trimmed.is_empty() {
        return None;
    }
    let capped = super::output::truncate_within(trimmed, MAX_PANEL_OUTPUT);
    // A one-line result is already on the row as `outcome` or `detail`, so
    // repeating it underneath adds nothing but a disclosure to click through.
    // Multi-line is the only case worth opening: a diff, a stack trace, a listing.
    capped.contains('\n').then_some(capped)
}

/// What a tool was pointed at.
///
/// Per tool rather than one chain of fallbacks, because the field that identifies
/// the work differs between them: a read is identified by its path, a search by
/// what it was looking for and where. Taking the first field present would report
/// a search as "src" — the directory — and say nothing about the pattern, which
/// is the actual question.
fn detail_for(tool: &str, arguments: &Value) -> String {
    if tool == "search_files" {
        let pattern = string_at(arguments, "pattern").unwrap_or_default();
        // A search without a path covered the whole workspace, and that is worth
        // saying: it is the difference between "needle" and "needle in src".
        return match string_at(arguments, "path") {
            Some(path) => format!("{pattern} in {path}"),
            None => pattern,
        };
    }
    // A sub-agent is identified by the label the caller gave it, and only failing
    // that by the task itself. The instructions are the fallback because they are
    // what the row is *about* when nothing shorter was provided, and they are
    // trimmed to one line so a paragraph-long brief does not push the row's
    // outcome off screen.
    if tool == "sub_agent" {
        return string_at(arguments, "label")
            .or_else(|| {
                string_at(arguments, "instructions")
                    .map(|instructions| first_line(&instructions).to_string())
            })
            .unwrap_or_default();
    }
    let fields: &[&str] = if matches!(tool, "search_web" | "web_fetch" | "run_terminal") {
        &["query", "url", "command", "path", "pattern"]
    } else {
        &["path", "pattern", "command", "query", "url"]
    };
    fields
        .iter()
        .find_map(|field| string_at(arguments, field))
        .unwrap_or_default()
}

/// The singular verb phrase for a tool.
///
/// Matched by name rather than carried on [`super::ToolSpec`] because
/// `ToolSpec` feeds the cached `tools` array: adding a field to it would change
/// the bytes a provider caches on, which is exactly what the sorted-tools
/// invariant in `registry.rs` exists to protect.
fn label_for(tool: &str) -> &'static str {
    match tool {
        "read_file" => "Read file",
        "list_dir" => "List folder",
        "search_files" => "Find files",
        "write_file" => "Write file",
        "edit_file" => "Edit file",
        // Its own label rather than sharing `Edit file`: the two take different
        // arguments, and a panel where two rows say the same thing for edits by
        // text and edits by line is a row that cannot be told apart at a glance.
        "edit_lines" => "Edit lines",
        "run_terminal" => "Run command",
        "search_web" => "Search the web",
        "web_fetch" => "Open page",
        "skill_read" => "Read skill",
        "skill_manage" => "Save skill",
        "sub_agent" => "Run agent",
        _ => "Tool call",
    }
}

/// The short figure shown beside a successful call.
///
/// Only tools that report something countable have one. The rest return an
/// empty string rather than a placeholder, so the row shows the label and the
/// path and nothing that reads as a missing value.
fn outcome_for(tool: &str, result: &str) -> Result<String, String> {
    match tool {
        // `read_file` numbers every line it returns, so the count is exact and a
        // sliced read reports what was shown rather than the file's size.
        "read_file" => Ok(plural(count_numbered_lines(result), "line")),
        // Read from the shape each search tool writes, so the panel and the model
        // are looking at the same figure rather than two independent counts.
        "search_files" | "search_web" => Ok(search_outcome(tool, result)),
        // A workspace listing ends with how many entries it showed.
        "list_dir" => Ok(listing_outcome(result)),
        "run_terminal" => exit_outcome(result),
        // A write says what it did in words that read better than a figure, and
        // so does a line edit -- its own result opens with exactly that, so
        // there is nothing to count here.
        "write_file" | "edit_file" | "edit_lines" => Ok(first_line(result).to_string()),
        _ => Ok(String::new()),
    }
}

fn plural(count: usize, noun: &str) -> String {
    match count {
        0 => String::new(),
        1 => format!("1 {noun}"),
        other => format!("{other} {noun}s"),
    }
}

fn string_at(arguments: &Value, field: &str) -> Option<String> {
    arguments
        .get(field)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn first_line(text: &str) -> &str {
    text.lines().next().unwrap_or_default().trim()
}

/// Lines of a `read_file` result, recognised by their line numbers.
///
/// Splitting on the tab rather than counting every line is what keeps the tool's
/// own "[showing lines 5-6 of 40]" trailer out of the figure, and a blank line
/// that a file legitimately contains.
///
/// The tab is load-bearing here: this was a ` | ` split, which matched the old
/// pipe format and matched nothing at all once the numbers moved to a tab. The
/// count silently became zero for every read, so every row said no line count
/// rather than failing -- which is the failure mode a parser has when it is
/// handed a shape it does not recognise.
fn count_numbered_lines(result: &str) -> usize {
    result
        .lines()
        .filter(|line| {
            line.split_once('\t')
                .is_some_and(|(head, _)| head.trim().parse::<usize>().is_ok())
        })
        .count()
}

/// How many results a search found, from the shape its own tool writes.
///
/// Two formats, because there are two search tools with different output and
/// neither has a count line: `search_files` appends a trailer saying how many
/// lines matched, and `search_web` numbers its entries `[1]`, `[2]`. Reading the
/// first is the number the model was given; counting the second is the only way
/// to know it, since the context it returns is prose plus numbered entries.
///
/// Anything unrecognised reports nothing rather than a guess. A wrong count is
/// worse than none: it would be read as a fact about the search.
fn search_outcome(tool: &str, result: &str) -> String {
    if result.starts_with("No matches") {
        return "no matches".to_string();
    }
    if tool == "search_files" {
        // The trailer reads "N matching lines in M files". The phrase "matching
        // lines" is the anchor and the count is the word before it, rather than
        // after it: the number and the noun it counts are separated by the
        // adjective, so neither `strip_prefix("N ")` nor counting backwards from
        // "lines" lands on it without this.
        for line in result.lines().rev().take(3) {
            let words = line.split_whitespace().collect::<Vec<_>>();
            let Some(index) = words.iter().position(|word| *word == "matching") else {
                continue;
            };
            if let Some(count) = index
                .checked_sub(1)
                .and_then(|before| words.get(before))
                .and_then(|word| word.parse::<usize>().ok())
            {
                return plural(count, "result");
            }
        }
        return String::new();
    }
    // A numbered entry is the highest `[n]` in the text, which is how many were
    // written rather than how many survived the context budget.
    let highest = result
        .lines()
        .filter_map(|line| line.trim().strip_prefix('['))
        .filter_map(|rest| rest.split_once(']'))
        .filter_map(|(number, _)| number.parse::<usize>().ok())
        .max()
        .unwrap_or(0);
    plural(highest, "result")
}

/// "12 of 30 entries", or nothing when the listing was empty.
fn listing_outcome(result: &str) -> String {
    for line in result.lines().rev().take(2) {
        if line.contains(" entries") {
            return line.trim().to_string();
        }
        if line.starts_with("The directory is empty") {
            return "empty".to_string();
        }
    }
    String::new()
}

/// A command's exit code, which is the fact a build or a test turns on.
///
/// Exit code 0 is reported as success and anything else as a failure carrying
/// the code, so the panel cannot show a red row for a command that worked.
fn exit_outcome(result: &str) -> Result<String, String> {
    let Some(code) = first_line(result).strip_prefix("Exit code: ") else {
        return Ok(String::new());
    };
    let code = code.trim();
    if code == "0" {
        Ok("succeeded".to_string())
    } else {
        Err(format!("exit code {code}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// The panel is for reading a result, so multi-line output is exactly what it
    /// should be opening.
    #[test]
    fn multi_line_output_is_shown_in_the_panel() {
        let out = panel_snippet("1\tone\n2\ttwo\n3\tthree", false).expect("shown");
        assert!(out.contains("3\tthree"), "{out}");
    }

    /// A one-line result is already on the row as `outcome` or `detail`, so
    /// repeating it under the row would add a disclosure that opens onto the text
    /// already visible.
    #[test]
    fn a_one_line_result_is_not_repeated_under_the_row() {
        assert_eq!(panel_snippet("Exited with code 0", false), None);
    }

    /// The reason is already on the row, and a failure printed twice reads as two
    /// separate problems.
    #[test]
    fn a_failure_carries_no_output() {
        assert_eq!(panel_snippet("1\tone\n2\ttwo", true), None);
    }

    /// A refusal or an empty result is one sentence the row already reports.
    #[test]
    fn an_empty_or_whitespace_result_is_nothing() {
        assert_eq!(panel_snippet("", false), None);
        assert_eq!(panel_snippet("   \n  ", false), None);
    }

    /// The panel's copy lives in the transcript, so it gets its own smaller cap
    /// than the model's -- and the head-only mistake is the one that loses the
    /// part worth reading.
    #[test]
    fn the_panel_copy_is_capped_and_keeps_the_tail() {
        let result = format!("{}\nFATAL: build failed", "h".repeat(MAX_PANEL_OUTPUT * 2));
        let capped = panel_snippet(&result, false).expect("shown");
        assert!(
            capped.chars().count() < MAX_PANEL_OUTPUT + 300,
            "not capped"
        );
        assert!(capped.contains("FATAL: build failed"), "tail was lost");
    }

    #[test]
    fn a_read_is_named_for_its_path_and_line_count() {
        let summary = summarize(
            "read_file",
            &json!({ "path": "src/App.tsx" }),
            "1\timport a\n2\timport b\n",
            true,
        );
        assert_eq!(summary.label, "Read file");
        assert_eq!(summary.detail, "src/App.tsx");
        assert_eq!(summary.outcome, Ok("2 lines".to_string()));
    }

    /// The line counter reads the tab format the tool actually writes.
    ///
    /// This parser was written against the old `1 | one` shape, so when the
    /// numbers moved to a tab it matched nothing and every read reported no
    /// count -- silently, because a parser handed a shape it does not recognise
    /// returns zero rather than failing. A test that feeds the real shape is
    /// the only thing that catches it.
    #[test]
    fn the_line_count_follows_the_numbered_format_the_tool_writes() {
        let rendered = "1\tfn main() {\n2\t    println!(\"hi\");\n3\t}\n";
        assert_eq!(
            summarize("read_file", &json!({ "path": "a.rs" }), rendered, true).outcome,
            Ok("3 lines".to_string())
        );
    }

    /// A blank line inside a file is still a line, and a read containing one
    /// must not have its count thrown off by it.
    #[test]
    fn a_blank_line_in_a_file_is_still_counted() {
        let rendered = "1\tone\n2\t\n3\tthree\n";
        assert_eq!(
            summarize("read_file", &json!({ "path": "a.txt" }), rendered, true).outcome,
            Ok("3 lines".to_string())
        );
    }

    /// A source line containing the old pipe separator must not be mistaken for
    /// a numbered row, which is exactly what a ` | ` split would have done.
    #[test]
    fn a_line_containing_a_pipe_is_still_one_numbered_line() {
        let rendered = "1\tlet x = a | b;\n2\tlet y = c | d;\n";
        assert_eq!(
            summarize("read_file", &json!({ "path": "a.rs" }), rendered, true).outcome,
            Ok("2 lines".to_string())
        );
    }

    /// Two reads that both say "Read file" would be indistinguishable without
    /// the path, so the detail is not optional decoration.
    #[test]
    fn reads_of_different_files_carry_different_details() {
        let one = summarize("read_file", &json!({ "path": "a.ts" }), "1 | x\n", true);
        let two = summarize("read_file", &json!({ "path": "b.ts" }), "1 | x\n", true);
        assert_ne!(one.detail, two.detail);
    }

    /// A sliced read reports the lines it returned, not the file's size. Those
    /// are different facts and conflating them would overstate every read.
    #[test]
    fn a_sliced_read_reports_the_lines_it_showed() {
        let summary = summarize(
            "read_file",
            &json!({ "path": "a.ts", "offset": 5, "limit": 2 }),
            "5\tfive\n6\tsix\n\n[showing lines 5-6 of 40]\n",
            true,
        );
        assert_eq!(summary.outcome, Ok("2 lines".to_string()));
    }

    #[test]
    fn an_empty_read_reports_no_count_rather_than_zero_lines() {
        // "0 lines" reads as a failure to read anything at all.
        let summary = summarize(
            "read_file",
            &json!({ "path": "empty" }),
            "The file is empty.",
            true,
        );
        assert_eq!(summary.outcome, Ok(String::new()));
    }

    /// The exit code is the fact a build turns on, and a red row for a command
    /// that worked would be worse than no row at all.
    #[test]
    fn a_command_reports_its_exit_code() {
        let good = summarize(
            "run_terminal",
            &json!({ "command": "cargo build" }),
            "Exit code: 0\n\nok",
            true,
        );
        assert_eq!(good.label, "Run command");
        assert_eq!(good.detail, "cargo build");
        assert_eq!(good.outcome, Ok("succeeded".to_string()));

        let bad = summarize(
            "run_terminal",
            &json!({ "command": "cargo test" }),
            "Exit code: 101\n\ntest failed",
            true,
        );
        assert_eq!(bad.outcome, Err("exit code 101".to_string()));
    }

    /// A failing command is a failure of the *run*, not of the panel's parsing,
    /// so the reason comes from the tool's own text.
    #[test]
    fn a_failed_call_reports_why_it_failed() {
        let summary = summarize(
            "run_terminal",
            &json!({ "command": "nope" }),
            "run_terminal failed: Could not start the command",
            false,
        );
        assert!(summary.outcome.is_err());
        assert!(summary.outcome.unwrap_err().contains("Could not start"));
    }

    #[test]
    fn a_search_names_its_pattern_and_where_it_looked() {
        let root = summarize("search_files", &json!({ "pattern": "needle" }), "", true);
        assert_eq!(root.label, "Find files");
        assert_eq!(root.detail, "needle");

        let scoped = summarize(
            "search_files",
            &json!({ "pattern": "needle", "path": "src" }),
            "",
            true,
        );
        assert_eq!(scoped.detail, "needle in src");
    }

    /// The count is read from the trailer the search tool itself appended, so
    /// the panel and the model are looking at the same number.
    #[test]
    fn a_search_reports_the_match_count_it_appended() {
        let summary = summarize(
            "search_files",
            &json!({ "pattern": "x" }),
            "a.rs:1: x\n\n100 matching lines in 40 files",
            true,
        );
        assert_eq!(summary.outcome, Ok("100 results".to_string()));

        // The singular case, because the tool writes it that way and a row
        // reading "1 results" would look like a counting bug.
        let one = summarize(
            "search_files",
            &json!({ "pattern": "x" }),
            "a.rs:1: x\n\n1 matching line in 1 files",
            true,
        );
        assert_eq!(one.outcome, Ok("1 result".to_string()));

        let none = summarize(
            "search_files",
            &json!({ "pattern": "x" }),
            "No matches for `x` in 40 files.",
            true,
        );
        assert_eq!(none.outcome, Ok("no matches".to_string()));
    }

    /// A web search numbers its entries rather than appending a count, so the
    /// only way to know how many it found is to read the numbering.
    #[test]
    fn a_web_search_counts_its_numbered_entries() {
        let summary = summarize(
            "search_web",
            &json!({ "query": "tauri 2" }),
            "A web search has already been performed.\n\n[1] First\nURL: a\n\n[2] Second\nURL: b\n\n[3] Third\nURL: c\n",
            true,
        );
        assert_eq!(summary.label, "Search the web");
        assert_eq!(summary.detail, "tauri 2");
        assert_eq!(summary.outcome, Ok("3 results".to_string()));
    }

    #[test]
    fn a_write_and_an_edit_are_labelled_apart() {
        assert_eq!(
            summarize(
                "write_file",
                &json!({ "path": "a.ts" }),
                "Created a.ts.",
                true
            )
            .label,
            "Write file"
        );
        assert_eq!(
            summarize(
                "edit_file",
                &json!({ "path": "a.ts" }),
                "Replaced 1 occurrence in a.ts.",
                true
            )
            .label,
            "Edit file"
        );
    }

    /// A line edit is labelled apart from a text edit and names the range, so
    /// two different edits of the same file are distinguishable at a glance.
    #[test]
    fn a_line_edit_names_its_range_and_is_labelled_apart() {
        let summary = summarize(
            "edit_lines",
            &json!({ "path": "a.ts", "start_line": 2, "end_line": 3, "new_text": "x" }),
            "Replaced lines 2-3 in a.ts with 1 line.\n2\tx\n",
            true,
        );
        assert_eq!(summary.label, "Edit lines");
        assert_eq!(summary.detail, "a.ts");
        assert_eq!(
            summary.outcome,
            Ok("Replaced lines 2-3 in a.ts with 1 line.".to_string())
        );
    }

    /// A line edit is a `Write`, and its read-back is more than a line long, so
    /// the panel opens onto what actually landed.
    #[test]
    fn a_line_edit_opens_onto_the_lines_it_wrote() {
        let snippet = panel_snippet(
            "Replaced lines 2-3 in a.ts with 2 lines.\n2\tTWO\n3\tTHREE\n",
            false,
        )
        .expect("shown");
        assert!(snippet.contains("3\tTHREE"), "{snippet}");
    }

    /// A refusal names the current lines so the model can read corrected numbers
    /// straight out of it -- which is why the read-back format is shared with
    /// `read_file` rather than written a second time.
    #[test]
    fn a_refused_line_edit_shows_the_lines_it_found() {
        let error = "Lines 2-3 of a.ts no longer match what you read.\n2\ttwo\n3\tthree\n";
        assert!(
            error.contains("2\ttwo"),
            "the error must be numbered as a read"
        );
    }

    /// The arguments of `write_file` carry the whole file, so nothing derived
    /// from them may reach the event. Only the path is taken.
    #[test]
    fn only_the_path_of_a_write_is_carried() {
        let summary = summarize(
            "write_file",
            &json!({ "path": "a.ts", "content": "x".repeat(10_000) }),
            "Created a.ts.",
            true,
        );
        assert_eq!(summary.detail, "a.ts");
        assert!(summary.detail.len() < 100);
    }

    /// A tool nobody described still appears. An invisible step is worse than a
    /// generic one, because the user cannot tell a run did nothing.
    #[test]
    fn an_undescribed_tool_still_gets_a_row() {
        let summary = summarize("some_new_tool", &json!({ "path": "a.ts" }), "done", true);
        assert_eq!(summary.label, "Tool call");
        assert_eq!(summary.detail, "a.ts");
    }

    /// The outcome is derived, never substituted: what the model reads is still
    /// the tool's own text.
    #[test]
    fn the_outcome_does_not_replace_the_result_text() {
        let result = "1\ta\n2\tb\n";
        let summary = summarize("read_file", &json!({ "path": "a" }), result, true);
        assert_eq!(summary.outcome, Ok("2 lines".to_string()));
        // The summary carries no result at all, so there is nothing to confuse
        // the model's copy with the user's.
        assert!(!summary.detail.contains("import"));
    }

    /// The payload the frontend reads. A `Result` serialized as-is arrives as
    /// `{"Ok": …}`, which would put the tagging rule in the view instead of the
    /// one place that builds the event.
    #[test]
    fn the_event_payload_flattens_the_outcome_and_keeps_the_failure_flag() {
        let ok = summarize("read_file", &json!({ "path": "a" }), "1\ta\n", true);
        let ok_fields = ok.event_fields();
        assert_eq!(ok_fields["label"], "Read file");
        assert_eq!(ok_fields["outcome"], "1 line");
        assert_eq!(ok_fields["failed"], false);

        let bad = summarize(
            "read_file",
            &json!({ "path": "gone" }),
            "read_file failed: no such file",
            false,
        );
        let bad_fields = bad.event_fields();
        assert_eq!(bad_fields["failed"], true);
        assert!(bad_fields["outcome"].as_str().unwrap().contains("no such"));
    }

    /// A write's diff travels on the event, tagged per line, which is the shape
    /// the panel colours by.
    #[test]
    fn an_attached_diff_reaches_the_event_with_a_kind_per_line() {
        let mut summary = summarize("edit_file", &json!({ "path": "a.ts" }), "Replaced 1.", true);
        summary.attach_diff(super::super::diff::diff("one\ntwo\n", "one\nTWO\n"));

        let fields = summary.event_fields();
        let lines = fields["diff"]["lines"].as_array().expect("diff lines");
        assert_eq!(lines.len(), 3);
        assert_eq!(lines[0]["kind"], "context");
        assert_eq!(lines[1]["kind"], "remove");
        assert_eq!(lines[2]["kind"], "add");
        assert_eq!(lines[2]["text"], "TWO");
    }

    /// Absent rather than empty for a call that changed nothing. A `diff: {lines:
    /// []}` on every read would make each row carry a field meaning "no change",
    /// which the panel would then have to special-case.
    #[test]
    fn a_call_with_no_diff_carries_no_diff_field() {
        let fields = summarize("read_file", &json!({ "path": "a" }), "1\ta\n", true).event_fields();
        assert!(fields.get("diff").is_none(), "{fields}");
    }

    /// A file that did not exist is reported as created, because the frontend
    /// cannot tell a new file from a rewritten one: both diff as every line
    /// added and none removed.
    #[test]
    fn a_new_file_is_reported_as_created() {
        let fields = summarize(
            "write_file",
            &json!({ "path": "a.ts" }),
            "Created a.ts.",
            true,
        )
        .event_fields();
        assert_eq!(fields["created"], serde_json::Value::Bool(true), "{fields}");
    }

    /// Rewriting an existing file is an edit, not a creation. The two have the
    /// same diff shape, so this is the case that would otherwise be reported
    /// wrong by every summary the reader sees.
    #[test]
    fn a_rewritten_file_is_not_reported_as_created() {
        let rewritten = "Wrote 41 bytes to a.ts, replacing its contents.";
        let fields =
            summarize("write_file", &json!({ "path": "a.ts" }), rewritten, true).event_fields();
        assert!(fields.get("created").is_none(), "{fields}");
    }

    /// Only `write_file` can bring a file into existence, so an edit's wording is
    /// never read as a creation however it is phrased.
    #[test]
    fn an_edit_is_never_reported_as_creating_a_file() {
        let created = summarize(
            "edit_file",
            &json!({ "path": "a.ts" }),
            "Created a.ts.",
            true,
        )
        .event_fields();
        assert!(created.get("created").is_none(), "{created}");
    }

    /// A failed write created nothing, whatever it was attempting.
    #[test]
    fn a_failed_write_is_not_reported_as_created() {
        let fields = summarize(
            "write_file",
            &json!({ "path": "a.ts" }),
            "Created a.ts.",
            false,
        )
        .event_fields();
        assert!(fields.get("created").is_none(), "{fields}");
    }

    /// Lines the cap left out are reported, so a truncated diff cannot read as a
    /// file that barely changed.
    #[test]
    fn a_diff_says_how_many_lines_it_left_out() {
        let mut summary = summarize("write_file", &json!({ "path": "a.ts" }), "Wrote.", true);
        summary.attach_diff(super::super::diff::Diff {
            lines: vec![super::super::diff::Line::Added { text: "x".into() }],
            hidden: 412,
        });
        assert_eq!(
            summary.event_fields()["diff"]["hidden"],
            412,
            "{}",
            summary.event_fields()
        );
    }
}
