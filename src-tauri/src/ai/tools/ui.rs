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
    /// The result as the model actually read it, for the record a later turn sees.
    ///
    /// The panel's copy ([`panel_snippet`]) is deliberately small: a row is for
    /// reading at a glance, and it omits failures and one-line results entirely.
    /// The model's record is the opposite question -- what did this call return --
    /// and a reply that loses it cannot explain its own work one turn later. So
    /// this carries the text whole, bounded only by
    /// [`super::output::MAX_TOOL_OUTPUT`], which is the same cap the tool itself
    /// already applied before the model was handed it.
    ///
    /// Stored only when it differs from the panel's copy, so a short result is not
    /// written twice into the transcript.
    pub context: Option<String>,
    /// The run a `sub_agent` call started, when it started one.
    ///
    /// The row is what the report is drawn on, and a report arrives minutes after
    /// the call returned. Without an exact key the frontend has to guess which row
    /// a report belongs to, which is wrong as soon as two agents run in parallel.
    /// Set from the tool's own sink rather than parsed out of the acknowledgement.
    pub subagent: Option<String>,
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
        // The model's own view of what came back. Absent when the panel's snippet
        // already is that text, so the transcript stores one copy of a short result.
        if let (Some(context), Some(object)) = (&self.context, fields.as_object_mut()) {
            object.insert("context".into(), serde_json::Value::String(context.clone()));
        }
        // The id the call's own run will report under. Absent on every tool that
        // did not start a run, so the frontend reads it as "this row is a
        // delegation" without a second field to check.
        if let (Some(agent_id), Some(object)) = (&self.subagent, fields.as_object_mut()) {
            object.insert(
                "agent_id".into(),
                serde_json::Value::String(agent_id.clone()),
            );
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

    /// Records the run a `sub_agent` call started.
    ///
    /// A setter for the same reason as [`Self::attach_diff`]: the id is reported
    /// through the sink while the call runs, and the summary is built after it.
    pub fn attach_subagent_id(&mut self, agent_id: String) {
        self.subagent = Some(agent_id);
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
        label: label_for_call(tool, arguments),
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
        context: model_context(tool, arguments, result, !succeeded),
        // Filled from the sink by the loop, which is the only place that sees the
        // tool's structured report. `summarize` sees the call's text alone.
        subagent: None,
    }
}

/// The result as the model read it, kept only where the panel's copy would lose it.
///
/// Three cases need this and the panel deliberately drops all three: a failure
/// (whose reason the row already carries), a one-line result (already on the row as
/// its outcome), and anything longer than [`MAX_PANEL_OUTPUT`]. The model saw the
/// text in each of them, so the record must hold it, or the next turn is a
/// conversation that read a file and cannot say what was in it.
fn model_context(tool: &str, arguments: &Value, result: &str, failed: bool) -> Option<String> {
    let trimmed = result.trim();
    if trimmed.is_empty() {
        return None;
    }
    // A call that opted out of the shared cap was asked for whole, so the record
    // keeps it whole too. Re-capping it here would not save a token on the turn
    // that read it -- the model already received all of it -- and it would mean
    // the turn after that knew less about the file than the turn which read it.
    let whole = if super::output::is_uncapped(tool, arguments) {
        trimmed.to_string()
    } else {
        super::output::truncate_within(trimmed, super::output::MAX_TOOL_OUTPUT)
    };
    match panel_snippet(result, failed) {
        // The panel's copy is already the whole text, so one copy is enough.
        Some(snippet) if snippet == whole => None,
        _ => Some(whole),
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
    if matches!(tool, "grep" | "glob") {
        let pattern = string_at(arguments, "pattern").unwrap_or_default();
        // A search without a path covered the whole workspace, and that is worth
        // saying: it is the difference between "needle" and "needle in src".
        return match string_at(arguments, "path") {
            Some(path) => format!("{pattern} in {path}"),
            None => pattern,
        };
    }
    // A move is identified by both ends: one path alone does not say where the
    // file went, which is the fact the row is about.
    if tool == "move_file" {
        return match (
            string_at(arguments, "path"),
            string_at(arguments, "destination"),
        ) {
            (Some(from), Some(to)) => format!("{from} → {to}"),
            (Some(from), None) => from,
            _ => String::new(),
        };
    }
    // A background command is identified by the id the model was handed.
    if matches!(
        tool,
        "terminal_output" | "terminal_kill" | "terminal_status"
    ) {
        return string_at(arguments, "id").unwrap_or_default();
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
    // A scheduled job is named by the model that created it, and that name is what
    // the user will look for in the Jobs screen -- so the row is the name, not the
    // instruction it will run hours from now.
    if tool == "schedule_job" {
        return string_at(arguments, "name").unwrap_or_default();
    }
    // An MCP tool is named by the server and the tool inside it. Its schema is
    // the server's, so the fields below are a guess; the two names are the one
    // thing this app knows for certain about the call.
    if let Some(rest) = tool.strip_prefix("mcp__") {
        return rest.replace("__", " · ");
    }
    // A skill tool is identified by the skill and the file inside it. Neither
    // alone is enough to judge: the skill names the folder, and the file names
    // what a run or a write will touch.
    if matches!(tool, "skill_read" | "skill_write" | "skill_run" | "skill_manage") {
        let skill = string_at(arguments, "skill")
            .or_else(|| string_at(arguments, "existing_name"))
            .or_else(|| string_at(arguments, "name"));
        let file = string_at(arguments, "path")
            .or_else(|| string_at(arguments, "script"))
            // `skill_manage` writes `SKILL.md` and nothing else: its schema has
            // no `path`, so the file is stated rather than left to be guessed
            // from the tool's description.
            .or_else(|| (tool == "skill_manage").then(|| "SKILL.md".to_string()));
        return match (skill, file) {
            (Some(skill), Some(file)) => format!("{skill} · {file}"),
            (Some(skill), None) => skill,
            (None, Some(file)) => file,
            (None, None) => String::new(),
        };
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

/// What a permission prompt must say about a call, beside its label.
///
/// The prompt used to name only the tool -- "Allow write_file" -- which tells a
/// reader nothing they can judge: not which file, not how much of it, not that a
/// delete takes a folder with it. This is the call's subject (the same detail its
/// transcript row would show) followed by the one fact that makes the decision
/// meaningful.
///
/// Built from the arguments alone, because that is all the gate holds: the file
/// has not been read or written yet, so the line count is what the call writes,
/// never the file's own size.
pub fn approval_detail(tool: &str, arguments: &Value) -> String {
    let detail = detail_for(tool, arguments);
    let fact = match tool {
        "write_file" => {
            let lines = arguments
                .get("content")
                .and_then(Value::as_str)
                .map(|content| content.lines().count())
                .unwrap_or(0);
            let append = arguments
                .get("append")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            match (append, lines) {
                (true, 0) => "append".to_string(),
                (true, count) => format!("append {}", plural(count, "line")),
                (false, 0) => "replace contents".to_string(),
                (false, count) => format!("write {}", plural(count, "line")),
            }
        }
        // The number of change blocks, which is what `edits` carries. The result
        // states how many occurrences actually matched, but that is not known
        // before the call runs.
        "edit_file" => {
            let changes = arguments
                .get("edits")
                .and_then(Value::as_array)
                .map_or(1, Vec::len);
            if arguments
                .get("replace_all")
                .and_then(Value::as_bool)
                .unwrap_or(false)
            {
                format!("{} · every occurrence", plural(changes, "change"))
            } else {
                plural(changes, "change")
            }
        }
        "edit_lines" => {
            let start = arguments.get("start_line").and_then(Value::as_u64);
            let end = arguments.get("end_line").and_then(Value::as_u64);
            let lines = arguments
                .get("new_text")
                .and_then(Value::as_str)
                .map(|text| text.lines().count())
                .unwrap_or(0);
            match start {
                Some(start) => {
                    let range = match end {
                        Some(end) if end != start => format!("lines {start}–{end}"),
                        _ => format!("line {start}"),
                    };
                    match lines {
                        0 => format!("delete {range}"),
                        count => format!("replace {range} with {}", plural(count, "line")),
                    }
                }
                None => String::new(),
            }
        }
        // A folder delete is the one that cannot be undone by looking at the path:
        // it takes everything inside it, and the prompt has to say so.
        "delete_file" => {
            if arguments
                .get("recursive")
                .and_then(Value::as_bool)
                .unwrap_or(false)
            {
                "folder and everything in it".to_string()
            } else {
                "file".to_string()
            }
        }
        "write_document" => {
            let format = string_at(arguments, "format").unwrap_or_else(|| "document".to_string());
            let lines = arguments
                .get("content")
                .and_then(Value::as_str)
                .map(|content| content.lines().count())
                .unwrap_or(0);
            if lines == 0 {
                format
            } else {
                format!("{format} · {}", plural(lines, "line"))
            }
        }
        // A delegated run is judged by what kind it is: an `explore`/`plan` agent
        // is read-only, and the prompt should not look like a write when it is not.
        "sub_agent" => format!(
            "{} agent",
            arguments
                .get("type")
                .and_then(Value::as_str)
                .unwrap_or("general")
        ),
        // A shell is judged by its command alone. `detail_for` already carries
        // it, and the prompt shows that command verbatim, so there is no second
        // fact worth adding here.
        _ => String::new(),
    };
    match (detail.is_empty(), fact.is_empty()) {
        (false, false) => format!("{detail} · {fact}"),
        (false, true) => detail,
        (true, false) => fact,
        (true, true) => String::new(),
    }
}

/// The singular verb phrase for a tool.
///
/// Public because a permission prompt names a call the same way its transcript
/// row does: two wordings for one call would make the dialog and the row it is
/// about disagree.
///
/// Matched by name rather than carried on [`super::ToolSpec`] because
/// `ToolSpec` feeds the cached `tools` array: adding a field to it would change
/// the bytes a provider caches on, which is exactly what the sorted-tools
/// invariant in `registry.rs` exists to protect.
pub fn label_for(tool: &str) -> &'static str {
    match tool {
        "read_file" => "Read file",
        "list_dir" => "List folder",
        "grep" => "Search",
        "glob" => "Find files",
        "write_file" => "Write file",
        "write_document" => "Write document",
        "edit_file" => "Edit file",
        // Its own label rather than sharing `Edit file`: the two take different
        // arguments, and a panel where two rows say the same thing for edits by
        // text and edits by line is a row that cannot be told apart at a glance.
        "edit_lines" => "Edit lines",
        "move_file" => "Move",
        "delete_file" => "Delete",
        "run_terminal" => "Run command",
        "terminal_output" => "Read output",
        "terminal_kill" => "Stop command",
        "terminal_status" => "Command status",
        "search_web" => "Search the web",
        "web_fetch" => "Open page",
        "skill_read" => "Read skill",
        "skill_manage" => "Save skill",
        "skill_write" => "Write skill file",
        "skill_run" => "Run skill script",
        "sub_agent" => "Run agent",
        // Its own label rather than sharing `Run agent`: the two read as two
        // different intentions -- one asks an agent to work, the other reads what
        // it found -- and one row per intention is what a "Worked for" list is for.
        "subagent_wait" => "Wait for agent",
        "schedule_job" => "Schedule job",
        "ask_user" => "Ask user",
        "todo" => "Checklist",
        _ => "Tool call",
    }
}

/// The verb phrase for one call, where the tool's own arguments change it.
///
/// Only `skill_manage` needs this: it is one tool covering create, update and
/// edit, and a row that says "Save skill" for a one-line correction names the
/// wrong act. Everything else is named by [`label_for`] alone.
///
/// Static strings, like [`label_for`], so the summary shape and the cached
/// `tools` array are untouched.
pub fn label_for_call(tool: &str, arguments: &Value) -> &'static str {
    if tool == "skill_manage" {
        return match arguments.get("action").and_then(Value::as_str) {
            Some("create") => "Create skill",
            Some("update") => "Rewrite skill",
            Some("edit") => "Edit skill",
            _ => label_for(tool),
        };
    }
    label_for(tool)
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
        "grep" | "search_web" => Ok(search_outcome(tool, result)),
        // A glob reports how many files it matched.
        "glob" => Ok(glob_outcome(result)),
        // A workspace listing ends with how many entries it showed.
        "list_dir" => Ok(listing_outcome(result)),
        "run_terminal" => exit_outcome(result),
        // A background command reports in its first line whether it is still
        // running; a move and a delete say what they did in their first line.
        "move_file" | "delete_file" | "terminal_output" | "terminal_kill" | "terminal_status" => {
            Ok(first_line(result).to_string())
        }
        // A write says what it did in words that read better than a figure, and
        // so does a line edit -- its own result opens with exactly that, so
        // there is nothing to count here.
        "write_file" | "write_document" | "edit_file" | "edit_lines" => {
            Ok(first_line(result).to_string())
        }
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
/// neither has a count line: `grep` appends a trailer saying how many lines
/// matched, and `search_web` numbers its entries `[1]`, `[2]`. Reading the first
/// is the number the model was given; counting the second is the only way to know
/// it, since the context it returns is prose plus numbered entries.
///
/// Anything unrecognised reports nothing rather than a guess. A wrong count is
/// worse than none: it would be read as a fact about the search.
fn search_outcome(tool: &str, result: &str) -> String {
    if result.starts_with("No matches") {
        return "no matches".to_string();
    }
    if tool == "grep" {
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

/// How many files a glob matched, from the trailer it appends.
///
/// The trailer reads "N of M files", so the count is the first word. An empty
/// result is reported as none rather than left blank, because "found nothing" is
/// exactly the fact a reader is scanning the row for.
fn glob_outcome(result: &str) -> String {
    for line in result.lines().rev().take(2) {
        if let Some((count, _)) = line.trim().split_once(" of ") {
            if let Ok(number) = count.parse::<usize>() {
                return plural(number, "file");
            }
        }
        if line.starts_with("No files match") {
            return "no matches".to_string();
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

    /// A write's prompt names the file and how much is being written, which is
    /// the difference between a prompt a reader can answer and one they cannot.
    #[test]
    fn a_write_prompt_names_its_path_and_size() {
        assert_eq!(
            approval_detail(
                "write_file",
                &json!({ "path": "src/App.tsx", "content": "a\nb\nc\n" })
            ),
            "src/App.tsx · write 3 lines"
        );
        assert_eq!(
            approval_detail(
                "write_file",
                &json!({ "path": "a.md", "content": "x", "append": true })
            ),
            "a.md · append 1 line"
        );
    }

    /// An edit prompt states the changes it carries, and a line edit states the
    /// range it replaces -- a file name alone does not say what is at stake.
    #[test]
    fn an_edit_prompt_names_its_change_or_its_range() {
        assert_eq!(
            approval_detail(
                "edit_file",
                &json!({ "path": "a.rs", "old_text": "x", "new_text": "y" })
            ),
            "a.rs · 1 change"
        );
        assert_eq!(
            approval_detail(
                "edit_file",
                &json!({ "path": "a.rs", "edits": [
                    { "old_text": "x", "new_text": "y" },
                    { "old_text": "p", "new_text": "q" }
                ] })
            ),
            "a.rs · 2 changes"
        );
        assert_eq!(
            approval_detail(
                "edit_lines",
                &json!({ "path": "a.ts", "start_line": 2, "end_line": 3, "new_text": "A\nB" })
            ),
            "a.ts · replace lines 2–3 with 2 lines"
        );
        assert_eq!(
            approval_detail(
                "edit_lines",
                &json!({ "path": "a.ts", "start_line": 9, "new_text": "" })
            ),
            "a.ts · delete line 9"
        );
    }

    /// A recursive delete takes everything inside the folder, and the prompt is
    /// the only place that says so before it happens.
    #[test]
    fn a_folder_delete_says_it_takes_the_folder() {
        assert_eq!(
            approval_detail("delete_file", &json!({ "path": "build", "recursive": true })),
            "build · folder and everything in it"
        );
        assert_eq!(
            approval_detail("delete_file", &json!({ "path": "notes.txt" })),
            "notes.txt · file"
        );
    }

    /// A skill tool is named by its skill and the file inside it -- neither alone
    /// tells a reader what will run or change.
    #[test]
    fn a_skill_prompt_names_the_skill_and_the_file() {
        assert_eq!(
            approval_detail(
                "skill_run",
                &json!({ "skill": "code-changes", "script": "scripts/build.ps1" })
            ),
            "code-changes · scripts/build.ps1"
        );
    }

    /// A shell's prompt is its command, verbatim, with no second copy beside it.
    #[test]
    fn a_terminal_prompt_is_its_command() {
        assert_eq!(
            approval_detail("run_terminal", &json!({ "command": "cargo test" })),
            "cargo test"
        );
    }

    /// An MCP tool's schema belongs to the server, so its arguments are not known
    /// here. The server and tool names are, and they are what a prompt can state
    /// without guessing.
    #[test]
    fn an_mcp_prompt_names_the_server_and_the_tool() {
        assert_eq!(
            approval_detail("mcp__playwright__browser_click", &json!({})),
            "playwright · browser_click"
        );
    }

    #[test]
    fn a_search_names_its_pattern_and_where_it_looked() {
        let root = summarize("grep", &json!({ "pattern": "needle" }), "", true);
        assert_eq!(root.label, "Search");
        assert_eq!(root.detail, "needle");

        let scoped = summarize(
            "grep",
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
            "grep",
            &json!({ "pattern": "x" }),
            "a.rs:1: x\n\n100 matching lines in 40 files",
            true,
        );
        assert_eq!(summary.outcome, Ok("100 results".to_string()));

        // The singular case, because the tool writes it that way and a row
        // reading "1 results" would look like a counting bug.
        let one = summarize(
            "grep",
            &json!({ "pattern": "x" }),
            "a.rs:1: x\n\n1 matching line in 1 files",
            true,
        );
        assert_eq!(one.outcome, Ok("1 result".to_string()));

        let none = summarize(
            "grep",
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

    /// `skill_manage` is one tool covering three acts, so the row has to say
    /// which one happened and which file it touched -- not "Save skill" for a
    /// one-line correction.
    #[test]
    fn each_skill_action_is_named_for_what_it_did() {
        let created = summarize(
            "skill_manage",
            &json!({ "action": "create", "name": "code-craft" }),
            "Created the skill `code-craft`.",
            true,
        );
        assert_eq!(created.label, "Create skill");
        assert_eq!(created.detail, "code-craft · SKILL.md");

        let edited = summarize(
            "skill_manage",
            &json!({ "action": "edit", "existing_name": "code-craft", "old_text": "a", "new_text": "b" }),
            "Changed 1 of 1 in `code-craft`.",
            true,
        );
        assert_eq!(edited.label, "Edit skill");
        assert_eq!(edited.detail, "code-craft · SKILL.md");

        let updated = summarize(
            "skill_manage",
            &json!({ "action": "update", "existing_name": "code-craft" }),
            "Updated the skill `code-craft`.",
            true,
        );
        assert_eq!(updated.label, "Rewrite skill");
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

    /// The id a `sub_agent` call started rides its own result, so the frontend can
    /// match the run's later report to this row rather than to "the last agent row
    /// still open". Two parallel agents make that guess wrong immediately.
    #[test]
    fn a_subagent_id_rides_the_call_that_started_the_run() {
        let mut summary = summarize("sub_agent", &json!({ "instructions": "x" }), "ack", true);
        summary.attach_subagent_id("agent-1".into());
        assert_eq!(summary.event_fields()["agent_id"], "agent-1");

        let read = summarize("read_file", &json!({ "path": "a" }), "1\ta\n", true);
        assert!(read.event_fields().get("agent_id").is_none());
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
            ..Default::default()
        });
        assert_eq!(
            summary.event_fields()["diff"]["hidden"],
            412,
            "{}",
            summary.event_fields()
        );
    }

    /// A one-line result never reaches the panel, but the model read it, so the
    /// record keeps it. This is the case a grep would otherwise lose entirely.
    #[test]
    fn a_single_line_result_is_kept_for_the_record() {
        let summary = summarize(
            "grep",
            &json!({ "pattern": "needle" }),
            "3 matches in a.rs",
            true,
        );
        assert!(panel_snippet("3 matches in a.rs", false).is_none());
        assert_eq!(summary.context.as_deref(), Some("3 matches in a.rs"));
        assert_eq!(
            summary.event_fields()["context"],
            "3 matches in a.rs",
            "the record travels on the step"
        );
    }

    /// A failure's text is kept too: the model was handed the reason, and the next
    /// turn has to be able to say why the call did not work.
    #[test]
    fn a_failed_call_keeps_the_text_the_model_read() {
        let summary = summarize(
            "read_file",
            &json!({ "path": "gone.rs" }),
            "no such file or directory",
            false,
        );
        assert_eq!(
            summary.context.as_deref(),
            Some("no such file or directory")
        );
    }

    /// A result longer than the panel's cap is stored whole for the model, capped
    /// only where the tool itself already capped it.
    #[test]
    fn a_long_result_is_recorded_whole_for_the_model() {
        let long = format!("{}\n{}", "a".repeat(MAX_PANEL_OUTPUT), "z".repeat(50));
        let summary = summarize("read_file", &json!({ "path": "big.rs" }), &long, true);
        let context = summary.context.expect("recorded");
        assert!(context.contains(&"z".repeat(50)), "the tail is kept");
        assert!(context.len() > MAX_PANEL_OUTPUT);
    }

    /// A full read is recorded whole, not capped a second time for the record.
    ///
    /// The model already received the whole file, so a capped record would mean
    /// the next turn knew less about it than the turn that read it -- and the
    /// record is the only thing the next turn sees.
    #[test]
    fn a_full_read_is_recorded_whole_for_the_model() {
        let long = format!(
            "{}\n{}",
            "a".repeat(super::super::output::MAX_TOOL_OUTPUT),
            "z".repeat(50)
        );
        let summary = summarize(
            "read_file",
            &json!({ "path": "big.rs", "full": true }),
            &long,
            true,
        );
        assert_eq!(summary.context.as_deref(), Some(long.trim()));
    }

    /// Where the panel's copy already is the whole result, it is not stored twice.
    #[test]
    fn a_short_result_is_stored_once() {
        let summary = summarize(
            "read_file",
            &json!({ "path": "a.rs" }),
            "1\ta\n2\tb\n",
            true,
        );
        assert!(summary.context.is_none(), "{:?}", summary.context);
        assert!(summary.event_fields().get("context").is_none());
    }
}
