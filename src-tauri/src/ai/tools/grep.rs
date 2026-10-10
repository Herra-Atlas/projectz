//! Searching the workspace by regular expression.
//!
//! The tool a model reaches for instead of shelling out to `rg`. Shelling out
//! would route every search through the permission gate as a command, and the
//! output would not be shaped for a model; walking the tree here keeps search a
//! `Read` tool and its behaviour identical everywhere.
//!
//! # Why regex, and why a literal fallback
//!
//! Models are trained to pass regular expressions to a search tool, so accepting
//! one is what they expect. But they also, constantly, pass a fragment of real
//! code -- `fn main(` -- which is not a valid pattern. Refusing that would cost a
//! turn for a mistake the tool can absorb, so a pattern that will not compile is
//! searched for literally instead. The two behaviours are indistinguishable for a
//! simple word and the fallback only ever turns a dead end into a result.
//!
//! A literal search is also *worse* than a regex search for the case that made the
//! old `search_files` frustrating: `name` found `Filename` and could not be told
//! to stop. Now `\bname\b` does.

use std::ops::ControlFlow;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use serde_json::Value;

use super::walk::{walk_files, FoundFile, MAX_WALK_SECS};
use super::ToolSpec;

/// Search the workspace for a pattern.
pub const GREP: ToolSpec = ToolSpec {
    name: "grep",
    description: "Search file contents in the workspace for a pattern and return matching lines \
                  with their file path and line number. The pattern is a regular expression; a \
                  pattern that is not valid regex is searched for literally. Use `context` to see \
                  surrounding lines, `glob` to narrow to certain files (for example `*.rs`), and \
                  `output_mode` to list only the files or the counts. Prefer this to `run_terminal` \
                  for finding things: it is a read, so it needs no approval.",
    parameters: r#"{
        "type": "object",
        "properties": {
            "pattern": {
                "type": "string",
                "description": "Regular expression to search for. Falls back to a literal match if it is not valid regex."
            },
            "path": {
                "type": "string",
                "description": "Directory to search, relative to the workspace root. Defaults to the whole workspace."
            },
            "glob": {
                "type": "string",
                "description": "Only search files matching this glob, for example `*.rs` or `src/**/*.ts`."
            },
            "case_insensitive": {
                "type": "boolean",
                "description": "Match without regard to case. Defaults to false."
            },
            "context": {
                "type": "integer",
                "description": "Lines of context to show before and after each match. Defaults to 0."
            },
            "output_mode": {
                "type": "string",
                "enum": ["content", "files_with_matches", "count"],
                "description": "What to return: matching lines (default), only the paths that matched, or a count of matching lines per file."
            },
            "max_results": {
                "type": "integer",
                "description": "Maximum matches, files or counts to return depending on output_mode. Defaults to 100, capped at 500."
            }
        },
        "required": ["pattern"],
        "additionalProperties": false
    }"#,
    effect: super::Effect::Read,
    command_argument: None,
    execute: |arguments, context| {
        let _ = &context;
        Box::pin(std::future::ready(
            super::file::require_workspace().and_then(|root| grep(arguments, root)),
        ))
    },
};

/// Default and ceiling on reported results.
///
/// The ceiling matters more than the default: an unbounded search over a large
/// tree produces megabytes of matches, and a model given those reads none of them.
const DEFAULT_MAX_RESULTS: usize = 100;
const MAX_RESULTS: usize = 500;

/// Files larger than this are not opened.
///
/// A single large binary or lockfile can hold a megabyte on one line, which would
/// make a match line useless rather than informative.
const MAX_FILE_BYTES: u64 = 2 * 1024 * 1024;

/// Ceiling on requested context lines, on each side.
///
/// A model asking for a hundred lines around every match has asked for a read,
/// not a search. The cap keeps one match from spending a whole context window.
const MAX_CONTEXT: usize = 20;

/// How the caller wants the search reported.
#[derive(Clone, Copy, PartialEq, Eq)]
enum OutputMode {
    Content,
    FilesWithMatches,
    Count,
}

/// A compiled pattern: a regex, or the literal it fell back to.
enum Matcher {
    Regex(regex::Regex),
    /// Lowercased when the search is case-insensitive, so the comparison is on
    /// lowercased text either way.
    Literal(String),
}

impl Matcher {
    /// Whether one line contains the pattern.
    fn matches(&self, line: &str) -> bool {
        match self {
            Matcher::Regex(pattern) => pattern.is_match(line),
            Matcher::Literal(needle) => line.to_lowercase().contains(needle),
        }
    }
}

fn grep(arguments: Value, root: PathBuf) -> Result<String, String> {
    let pattern = arguments
        .get("pattern")
        .and_then(Value::as_str)
        .filter(|pattern| !pattern.is_empty())
        .ok_or("grep requires a non-empty string `pattern`")?;
    let case_insensitive = arguments
        .get("case_insensitive")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let matcher = build_matcher(pattern, case_insensitive);
    let mode = output_mode(&arguments)?;
    let context = arguments
        .get("context")
        .and_then(Value::as_u64)
        .map(|value| value.min(MAX_CONTEXT as u64) as usize)
        .unwrap_or(0);
    let limit = arguments
        .get("max_results")
        .and_then(Value::as_u64)
        .map(|value| value.clamp(1, MAX_RESULTS as u64) as usize)
        .unwrap_or(DEFAULT_MAX_RESULTS);
    let filter = filename_filter(&arguments)?;

    let start = arguments
        .get("path")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|path| !path.is_empty())
        .unwrap_or(".");
    let directory = super::file::resolve_within(&root, start)?;

    let deadline = Instant::now() + Duration::from_secs(MAX_WALK_SECS);
    let mut report = Report {
        limit,
        mode,
        context,
        matcher: &matcher,
        matches: Vec::new(),
        emitted: 0,
        scanned: 0,
        hit_limit: false,
    };
    let completed = walk_files(&root, &directory, deadline, &mut |file| {
        if filter
            .as_ref()
            .is_some_and(|filter| !filter.allows(&file.relative))
        {
            return ControlFlow::Continue(());
        }
        report.visit(file)
    });

    Ok(report.render(completed))
}

/// Builds the matcher, falling back to a literal when the pattern will not compile.
///
/// The fallback is the whole reason a model can pass real code as a pattern. A
/// failed compile is not reported as an error because a literal search is nearly
/// always what was meant -- `Vec<String>` is a common thing to look for and a
/// valid thing to search for, and it is not a valid pattern.
fn build_matcher(pattern: &str, case_insensitive: bool) -> Matcher {
    let mut builder = regex::RegexBuilder::new(pattern);
    builder.case_insensitive(case_insensitive);
    match builder.build() {
        Ok(compiled) => Matcher::Regex(compiled),
        Err(_) => Matcher::Literal(if case_insensitive {
            pattern.to_lowercase()
        } else {
            pattern.to_string()
        }),
    }
}

fn output_mode(arguments: &Value) -> Result<OutputMode, String> {
    match arguments.get("output_mode").and_then(Value::as_str) {
        None | Some("content") => Ok(OutputMode::Content),
        Some("files_with_matches") => Ok(OutputMode::FilesWithMatches),
        Some("count") => Ok(OutputMode::Count),
        Some(other) => Err(format!(
            "`{other}` is not an output_mode. Use \"content\", \"files_with_matches\" or \"count\"."
        )),
    }
}

/// The filename filter named by `glob`, if any.
///
/// A pattern with no `/` is matched against the file's name, so `*.rs` selects
/// every Rust file anywhere; one with a `/` is matched against the whole relative
/// path, so `src/**/*.ts` bounds the search to a subtree. That split is what makes
/// the common `*.rs` mean what a model expects rather than only matching the
/// workspace root.
fn filename_filter(arguments: &Value) -> Result<Option<Pattern>, String> {
    let Some(raw) = arguments
        .get("glob")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|glob| !glob.is_empty())
    else {
        return Ok(None);
    };
    Ok(Some(Pattern::new(raw)?))
}

/// A glob pattern, matched against a name or a path depending on its shape.
///
/// Shared with `glob` so the two tools agree about what `*.rs` and `src/**/*.ts`
/// mean, which is what lets a model move between searching and finding without
/// relearning the syntax.
pub(crate) struct Pattern {
    compiled: glob::Pattern,
    /// Matched against the whole relative path rather than the bare name.
    on_path: bool,
}

impl Pattern {
    pub(crate) fn new(raw: &str) -> Result<Self, String> {
        glob::Pattern::new(raw)
            .map(|compiled| Self {
                compiled,
                on_path: raw.contains('/'),
            })
            .map_err(|error| format!("`{raw}` is not a valid glob pattern: {error}"))
    }

    pub(crate) fn allows(&self, relative: &str) -> bool {
        if self.on_path {
            self.compiled.matches(relative)
        } else {
            let name = relative.rsplit('/').next().unwrap_or(relative);
            self.compiled.matches(name)
        }
    }
}

/// The search's running state, so the walk closure stays one line.
struct Report<'a> {
    limit: usize,
    mode: OutputMode,
    context: usize,
    matcher: &'a Matcher,
    /// Rendered blocks, one per file that matched.
    matches: Vec<String>,
    /// Result units counted toward the limit: matching lines in `content` mode,
    /// files in the other two. Kept apart from `matches.len()` because one content
    /// block can hold many lines, and the cap is on lines.
    emitted: usize,
    /// Files whose contents were read, so "no matches" can say how much was seen.
    scanned: usize,
    /// The limit was reached and results were dropped.
    hit_limit: bool,
}

impl Report<'_> {
    fn visit(&mut self, file: FoundFile<'_>) -> ControlFlow<()> {
        if self.emitted >= self.limit {
            self.hit_limit = true;
            return ControlFlow::Break(());
        }
        if file.bytes > MAX_FILE_BYTES {
            return ControlFlow::Continue(());
        }
        let Ok(body) = std::fs::read_to_string(file.absolute) else {
            return ControlFlow::Continue(());
        };
        self.scanned += 1;

        let lines: Vec<&str> = body.lines().collect();
        let matching: Vec<usize> = lines
            .iter()
            .enumerate()
            .filter(|(_, line)| self.matcher.matches(line))
            .map(|(index, _)| index)
            .collect();
        if matching.is_empty() {
            return ControlFlow::Continue(());
        }

        match self.mode {
            OutputMode::FilesWithMatches => {
                self.matches.push(file.relative);
                self.emitted += 1;
            }
            OutputMode::Count => {
                self.matches
                    .push(format!("{}:{}", file.relative, matching.len()));
                self.emitted += 1;
            }
            OutputMode::Content => {
                // The cap is on matching lines, not files, so one file with more
                // matches than the limit is cut too -- otherwise a single busy file
                // could return more than the caller asked for.
                let room = self.limit - self.emitted;
                let take = matching.len().min(room);
                if take < matching.len() {
                    self.hit_limit = true;
                }
                self.matches.push(render_content(
                    &file.relative,
                    &lines,
                    &matching[..take],
                    self.context,
                ));
                self.emitted += take;
            }
        }
        if self.emitted >= self.limit {
            self.hit_limit = true;
            return ControlFlow::Break(());
        }
        ControlFlow::Continue(())
    }

    fn render(self, completed: bool) -> String {
        if self.matches.is_empty() {
            return if completed {
                format!("No matches in {} files.", self.scanned)
            } else {
                "No matches in the part of the workspace searched before the time limit. \
                 Narrow the search with `path` or `glob`."
                    .to_string()
            };
        }

        let mut out = self.matches.join("\n");
        match self.mode {
            OutputMode::Content => out.push_str(&format!(
                "\n\n{} matching lines in {} files",
                self.emitted, self.scanned
            )),
            OutputMode::FilesWithMatches | OutputMode::Count => {
                out.push_str(&format!("\n\n{} files", self.emitted))
            }
        }
        if self.hit_limit {
            // Saying the result was cut is what stops a model concluding the code
            // it wanted is not there. Point at the real fix rather than the cap.
            out.push_str(" (more exist; narrow the search with `path` or `glob`)");
        } else if !completed {
            out.push_str(" (search stopped at the time limit; narrow it with `path` or `glob`)");
        }
        out
    }
}

/// Renders one file's matches, with context, using grep's own two prefixes.
///
/// A match is `path:line: text` and a context line is `path-line- text`. The
/// change of separator is the only thing distinguishing the two, which is exactly
/// what `grep -C` trained every reader of this output to expect -- so a model sees
/// which lines matched without being told.
fn render_content(path: &str, lines: &[&str], matching: &[usize], context: usize) -> String {
    let mut chosen = std::collections::BTreeSet::new();
    for &index in matching {
        let from = index.saturating_sub(context);
        let to = (index + context).min(lines.len().saturating_sub(1));
        chosen.extend(from..=to);
    }
    chosen
        .into_iter()
        .map(|index| {
            let separator = if matching.contains(&index) { ':' } else { '-' };
            format!(
                "{path}{separator}{}{separator} {}",
                index + 1,
                lines[index].trim()
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn root_with(files: &[(&str, &str)]) -> PathBuf {
        use std::sync::atomic::{AtomicU32, Ordering};
        static COUNTER: AtomicU32 = AtomicU32::new(0);
        let unique = COUNTER.fetch_add(1, Ordering::Relaxed);
        let root =
            std::env::temp_dir().join(format!("projectz-grep-{}-{unique}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).expect("temp dir");
        for (path, body) in files {
            let full = root.join(path);
            if let Some(parent) = full.parent() {
                std::fs::create_dir_all(parent).expect("parent");
            }
            std::fs::write(full, body).expect("write");
        }
        root
    }

    #[test]
    fn a_match_is_reported_with_path_and_line() {
        let root = root_with(&[("a.rs", "fn main() {\n    println!(\"hi\");\n}")]);
        let out = grep(json!({ "pattern": "println" }), root.clone()).expect("grep");
        assert!(out.contains("a.rs:2:"), "{out}");
        assert!(out.contains("hi"), "{out}");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_pattern_is_a_regex_by_default() {
        let root = root_with(&[("a.rs", "let n = 42;\nlet s = \"x\";")]);
        let out = grep(json!({ "pattern": "\\d+" }), root.clone()).expect("grep");
        assert!(out.contains("a.rs:1:"), "{out}");
        assert!(!out.contains("a.rs:2:"), "{out}");
        let _ = std::fs::remove_dir_all(&root);
    }

    /// The fallback that lets a model pass real code: `Vec<String>` is not a valid
    /// pattern, and refusing it would cost a turn for a mistake the tool can absorb.
    #[test]
    fn an_invalid_regex_falls_back_to_a_literal_search() {
        let root = root_with(&[("a.rs", "let v: Vec<String> = Vec::new();")]);
        let out = grep(json!({ "pattern": "Vec<String>" }), root.clone()).expect("grep");
        assert!(out.contains("a.rs:1:"), "{out}");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn case_insensitive_search_ignores_case_only_when_asked() {
        let root = root_with(&[("a.rs", "const NAME: &str = \"x\";")]);
        assert!(grep(json!({ "pattern": "name" }), root.clone())
            .expect("grep")
            .contains("No matches"));
        assert!(grep(
            json!({ "pattern": "name", "case_insensitive": true }),
            root.clone()
        )
        .expect("grep")
        .contains("a.rs:1:"));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn context_lines_are_shown_and_marked_with_the_context_separator() {
        let root = root_with(&[("a.rs", "before\nMATCH\nafter")]);
        let out = grep(json!({ "pattern": "MATCH", "context": 1 }), root.clone()).expect("grep");
        assert!(out.contains("a.rs:2: MATCH"), "{out}");
        assert!(out.contains("a.rs-1- before"), "{out}");
        assert!(out.contains("a.rs-3- after"), "{out}");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn files_with_matches_lists_paths_without_contents() {
        let root = root_with(&[("src/a.rs", "needle"), ("src/b.rs", "needle")]);
        let out = grep(
            json!({ "pattern": "needle", "output_mode": "files_with_matches" }),
            root.clone(),
        )
        .expect("grep");
        assert!(out.contains("src/a.rs"), "{out}");
        assert!(out.contains("src/b.rs"), "{out}");
        assert!(!out.contains("1:"), "paths only, no line numbers: {out}");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn count_mode_reports_lines_per_file() {
        let root = root_with(&[("a.rs", "x\nx\nx\n")]);
        let out = grep(
            json!({ "pattern": "x", "output_mode": "count" }),
            root.clone(),
        )
        .expect("grep");
        assert!(out.contains("a.rs:3"), "{out}");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_glob_narrows_by_name_or_by_subtree() {
        let root = root_with(&[("src/a.rs", "needle"), ("src/b.txt", "needle")]);
        let only_rs =
            grep(json!({ "pattern": "needle", "glob": "*.rs" }), root.clone()).expect("grep");
        assert!(only_rs.contains("src/a.rs"), "{only_rs}");
        assert!(!only_rs.contains("b.txt"), "{only_rs}");

        let subtree = grep(
            json!({ "pattern": "needle", "glob": "src/**/*.txt" }),
            root.clone(),
        )
        .expect("grep");
        assert!(subtree.contains("src/b.txt"), "{subtree}");
        assert!(!subtree.contains("a.rs"), "{subtree}");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn results_are_capped_and_say_more_exist() {
        let root = root_with(&[("a.rs", "needle\nneedle\nneedle\n")]);
        let out = grep(
            json!({ "pattern": "needle", "max_results": 1 }),
            root.clone(),
        )
        .expect("grep");
        assert!(out.contains("more exist"), "{out}");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn no_matches_says_how_much_was_searched() {
        let root = root_with(&[("a.rs", "nothing")]);
        let out = grep(json!({ "pattern": "absent" }), root.clone()).expect("grep");
        assert!(out.contains("No matches in 1 files"), "{out}");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn an_unknown_output_mode_is_refused_by_name() {
        let root = root_with(&[("a.rs", "x")]);
        assert!(grep(
            json!({ "pattern": "x", "output_mode": "json" }),
            root.clone()
        )
        .expect_err("bad mode")
        .contains("json"));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_search_path_outside_the_workspace_is_refused() {
        let root = root_with(&[("a.rs", "x")]);
        assert!(
            grep(json!({ "pattern": "x", "path": "../.." }), root.clone())
                .expect_err("escape")
                .contains("outside")
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn an_empty_pattern_is_refused() {
        let root = root_with(&[("a.rs", "x")]);
        assert!(grep(json!({ "pattern": "" }), root.clone())
            .expect_err("empty")
            .contains("pattern"));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn generated_trees_are_skipped() {
        let root = root_with(&[("src/a.rs", "needle"), ("target/b.rs", "needle")]);
        let out = grep(json!({ "pattern": "needle" }), root.clone()).expect("grep");
        assert!(out.contains("src/a.rs"), "{out}");
        assert!(!out.contains("target"), "{out}");
        let _ = std::fs::remove_dir_all(&root);
    }
}
