//! Declarative description of one callable tool.
//!
//! A tool is two things: a JSON Schema the model reads to decide whether and how
//! to call it, and a name that resolves to an executor. Keeping them in one
//! struct is deliberate -- a tool whose schema and executor can drift apart is a
//! tool that will eventually lie to the model.
//!
//! # Cache stability
//!
//! [`ToolRegistry::specs`] sorts by name. A provider caches on the exact bytes
//! preceding its breakpoint, so a `tools` array whose order changes between
//! turns invalidates the whole prefix and the conversation pays full price
//! again. Registration order is an implementation detail and must never leak
//! into the request.

use std::sync::Arc;

use serde_json::Value;

use super::output;
use super::run::RunHandle;

/// One tool as advertised to the model.
#[derive(Clone, Debug)]
pub struct ToolSpec {
    /// Name the model uses to call this. Must be stable: it is the cache key.
    pub name: &'static str,
    /// Told to the model verbatim. Describes *when* to use the tool, not the
    /// implementation.
    pub description: &'static str,
    /// JSON Schema for the arguments object, as a literal.
    ///
    /// A string rather than a `Value` because `ToolSpec` is a `const`: `json!`
    /// allocates and so cannot be evaluated at compile time. It is parsed once
    /// per registry, not once per request.
    pub parameters: &'static str,
    /// Whether the tool can change the world. Determines caching and cache
    /// invalidation; permission levels use the tool's name instead.
    pub effect: Effect,
    /// The argument holding a shell command line, for the tools that have one.
    ///
    /// Declared per tool rather than assumed to be `command`, so approval prompts
    /// can show the command field the tool actually reads. A tool with `None`
    /// contributes no command text, which is correct for tools that do not run a shell.
    pub command_argument: Option<&'static str>,
    /// Runs the tool.
    ///
    /// Boxed rather than a plain `fn(Value) -> Result<String, String>` because
    /// tools are not all synchronous: a web search is a network round trip, and
    /// a shell command can be waited on. A synchronous signature would force
    /// every tool either to block a runtime thread or to spawn a runtime of its
    /// own, and the second of those is how a nested-runtime bug gets in.
    ///
    /// The returned future receives the run's cancellation flag so a long tool
    /// can abandon its work when the user stops the reply.
    pub execute: fn(Value, ToolContext) -> ToolFuture,
}

/// A tool whose name and schema are not known until the app is running.
///
/// MCP tools are the only kind. They arrive from a server when it is connected,
/// so they cannot be a [`ToolSpec`], whose fields are `&'static str` and whose
/// executor is a plain function pointer. Everything else about them is the same,
/// and from [`ToolRegistry::specs`] onward the registry treats the two alike.
///
/// Not a second registry: one list the model reads, one place a call is resolved.
/// Two registries would be two answers to "is this tool available", and the
/// access set, the policy and the cache all ask that question.
#[derive(Clone)]
pub struct DynamicTool {
    pub name: String,
    pub description: String,
    /// The argument schema exactly as the server sent it.
    pub parameters: Value,
    pub effect: Effect,
    /// The same contract as [`ToolSpec::execute`], but a closure rather than a
    /// function pointer: a dynamic tool has to carry which server and which remote
    /// name it speaks for, and a `fn` has nowhere to put them.
    pub execute: Arc<dyn Fn(Value, ToolContext) -> ToolFuture + Send + Sync>,
}

/// What a tool is given when it runs.
#[derive(Clone, Default)]
pub struct ToolContext {
    /// Shared with the run. A tool that polls this can stop promptly; one that
    /// never polls finishes on its own.
    pub cancelled: std::sync::Arc<std::sync::atomic::AtomicBool>,
    /// Where a tool reports structured results for the UI to render.
    ///
    /// A callback because the event sink belongs to the chat runtime, which owns
    /// the Tauri handle, while tools are plain functions that know nothing about
    /// Tauri. Passing it here keeps that boundary.
    pub sink: Sink,
    /// The run's database, for a tool that reads or writes stored state.
    ///
    /// Present rather than reached for as a global because a tool has no business
    /// knowing which conversation it is running in — but the skill tools do need
    /// the same database the run's tool cache is scoped to, and threading one
    /// value here is how the two stay the same value. `None` for a call with no
    /// session, which is what keeps a tool unable to answer one conversation from
    /// another's rows.
    pub database: Option<std::sync::Arc<crate::database::Database>>,
    /// The run this call belongs to, for a tool that has to start work of its own.
    ///
    /// Only `sub_agent` reads it today. `None` means the call has no run behind
    /// it — a tool's own unit test — and such a tool reports a clean error rather
    /// than reaching for a model it does not have. See [`super::run::RunHandle`].
    pub run: Option<std::sync::Arc<RunHandle>>,
}

/// Sinks a tool's structured output back to the running session.
///
/// A collector rather than a callback. The alternative -- handing the run's
/// `FnMut` to the tool -- needs it to be `Send`, because a tool future may
/// complete on any thread, and that bound does not belong on the runtime's
/// event callback. Collecting and draining from the loop keeps the tool's
/// signature simple and means one place emits every event.
///
/// Deliberately narrow: only web search reports anything structured today. A
/// tool with UI state worth showing would add an arm here rather than widen this
/// to "any tool, any payload", because an untyped sink is how a tool ends up
/// inventing a payload the frontend cannot render.
#[derive(Clone, Default)]
pub struct Sink {
    searches: std::sync::Arc<std::sync::Mutex<Vec<crate::websearch::WebSearchOutput>>>,
    diffs: std::sync::Arc<std::sync::Mutex<Vec<super::diff::Diff>>>,
}

impl ToolContext {
    /// Queues a completed web search for the run to report.
    pub fn report_search(&self, output: crate::websearch::WebSearchOutput) {
        // A poisoned lock means another sink panicked. The search itself
        // succeeded, so dropping the report is better than propagating a panic
        // across an await point.
        if let Ok(mut searches) = self.sink.searches.lock() {
            searches.push(output);
        }
    }

    /// Queues what a write changed, for the panel.
    ///
    /// A side channel rather than part of the tool's return value, which is the
    /// text the *model* reads. The diff is for the user, it is far too long to
    /// send to a provider on every edit, and appending it to the result would
    /// spend tokens on something the model cannot use.
    ///
    /// Ignored when there is no context, which is the case in a tool's own tests.
    /// A diff missing from a test is not a failed write.
    pub fn report_diff(&self, diff: super::diff::Diff) {
        if let Ok(mut diffs) = self.sink.diffs.lock() {
            diffs.push(diff);
        }
    }
}

impl Sink {
    /// Takes everything reported since the last drain.
    pub fn drain_searches(&self) -> Vec<crate::websearch::WebSearchOutput> {
        self.searches
            .lock()
            .map(|mut searches| std::mem::take(&mut *searches))
            .unwrap_or_default()
    }

    /// Takes the diffs reported since the last drain.
    ///
    /// Drained per call rather than per round, because a round can contain two
    /// writes and attributing both to one row would show the wrong change.
    pub fn drain_diffs(&self) -> Vec<super::diff::Diff> {
        self.diffs
            .lock()
            .map(|mut diffs| std::mem::take(&mut *diffs))
            .unwrap_or_default()
    }
}

impl std::fmt::Debug for ToolContext {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ToolContext")
            .field("cancelled", &self.cancelled)
            .finish()
    }
}

/// What a tool returns: either the text the model will read, or why it failed.
pub type ToolFuture =
    std::pin::Pin<Box<dyn std::future::Future<Output = Result<String, String>> + Send + 'static>>;

impl ToolSpec {
    /// The parsed argument schema.
    ///
    /// A malformed literal is a programming error, not a runtime condition, so
    /// it panics at the point of use with the tool named: a schema that fails
    /// to parse would otherwise silently advertise no arguments and produce a
    /// confusing tool-call error much later.
    fn schema(&self) -> Value {
        serde_json::from_str(self.parameters)
            .unwrap_or_else(|error| panic!("tool {} has invalid parameters: {error}", self.name))
    }
}

/// What a tool does to the system.
///
/// The classification drives exactly two things -- whether a result may be
/// cached, and whether taking it invalidates what was cached before -- and those
/// are two separate questions. They used to be one: everything that was not a
/// pure read was `Write`, so `ask_user` and `todo`, which touch no file at all,
/// wiped the conversation's whole cache every time the agent asked a question or
/// ticked an item off its checklist. A run that reads a dozen files and updates
/// its list between them then re-reads all twelve.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Effect {
    /// Observes only; its result may be cached.
    Read,
    /// Never cached, and **does not invalidate**: the tool writes a note to
    /// itself rather than to the workspace.
    ///
    /// `ask_user` and `todo` are the whole of this class. Their results must not
    /// be cached -- a question has to be asked afresh and a checklist changes as
    /// work progresses -- but neither can have changed a file, so nothing read
    /// before them has gone stale. Treating them as writes cost the cache for no
    /// correctness in return.
    Volatile,
    /// Changes state; invalidates every cached read taken before it, and is never
    /// itself cached.
    Write,
}

/// The tools available for one run.
#[derive(Default)]
pub struct ToolRegistry {
    tools: Vec<ToolSpec>,
    /// Tools that arrive at runtime. Empty on every run with no MCP server
    /// configured, which is most of them.
    dynamic: Vec<DynamicTool>,
}

impl ToolRegistry {
    /// Registers one more tool.
    ///
    /// Ordering is decided by [`ToolRegistry::specs`] rather than here, so a tool
    /// added after construction cannot disturb the cached request prefix.
    pub fn add(&mut self, tool: ToolSpec) {
        self.tools.push(tool);
    }

    /// Registers one more runtime tool.
    ///
    /// Kept apart from [`ToolRegistry::add`] only because the two carry different
    /// data; from `specs` onward they are one list. A dynamic tool may shadow a
    /// static one by name -- nothing does, and the alternative is a name check on
    /// every registration for a collision that cannot happen with a prefix.
    pub fn add_dynamic(&mut self, tool: DynamicTool) {
        self.dynamic.push(tool);
    }

    /// Drops every tool the predicate rejects.
    ///
    /// The one way a run's access set narrows what is advertised: a tool removed
    /// here is not in `specs()` and so never reaches the request body. Ordering is
    /// untouched, so the cache-stable sort still holds.
    pub fn retain(&mut self, keep: impl Fn(&str) -> bool) {
        self.tools.retain(|tool| keep(tool.name));
        self.dynamic.retain(|tool| keep(&tool.name));
    }

    /// Builds a registry from a fixed tool set.
    ///
    /// `mode` selects the set so the choice stays in one place rather than at
    /// each call site. A mode with no tools registered yet returns an empty
    /// registry, which behaves exactly like Chat mode.
    pub fn for_mode(mode: crate::ai::tools::ToolMode) -> Self {
        let specs: &[ToolSpec] = match mode {
            crate::ai::tools::ToolMode::Chat => &[],
            crate::ai::tools::ToolMode::Agent => &[
                super::file::LIST_DIR,
                super::file::READ_FILE,
                // `grep` replaces the old `search_files`: one search tool, by
                // regular expression with a literal fallback, rather than two
                // that overlap.
                super::grep::GREP,
                super::glob::GLOB,
                // The skill tools are agent-only for the same reason the filesystem
                // ones are: `skill_read` is a decision the model makes for itself,
                // and a model that was handed the body inline by the composer has no
                // reason to also look one up. Chat mode reaches skills the other
                // way, through the composer.
                super::skills::SKILL_MANAGE,
                super::skills::SKILL_READ,
                // Folder work: a skill can carry scripts and references, so it
                // needs a write verb beside the read one, and a way to execute a
                // script the user has trusted.
                super::skills::SKILL_WRITE,
                super::skills::SKILL_RUN,
                super::terminal::RUN_TERMINAL,
                // Reading and stopping a command `run_terminal` left running.
                // Split so a poll is a read -- it changes nothing and asks for
                // nothing -- while stopping one is a write.
                super::terminal::TERMINAL_OUTPUT,
                super::terminal::TERMINAL_KILL,
                // A read: liveness, process id, exit code and how much output is
                // buffered, so the model can ask instead of probing a port.
                super::terminal::TERMINAL_STATUS,
                // Moving and removing files, so a refactor does not have to go
                // through the shell to rename one.
                super::paths::MOVE_FILE,
                super::paths::DELETE_FILE,
                // Line-numbered editing is the tool a `read_file` is designed to
                // feed, so it ships with the readers rather than as an
                // afterthought. `edit_file` stays for the case where the model
                // has the text but not the numbers, which is what a search hit
                // gives it.
                super::write::EDIT_LINES,
                super::write::EDIT_FILE,
                super::write::WRITE_FILE,
                // A deliverable, not an edit: it builds a document the user asked
                // for, which is why it ships with the writers rather than the tools
                // that change source.
                super::export::WRITE_DOCUMENT,
                // Asking the user, which parks the run until they answer. Not a
                // write: its result must never be cached, but it changes nothing,
                // so it has no business invalidating the reads already taken.
                super::ask::ASK_USER,
                // The agent's own checklist. `Volatile` for the same reason: the
                // list moves as work progresses so it cannot be cached, and the
                // agent ticking an item off has not made any file stale.
                super::todo::TODO,
            ],
        };
        let mut tools = specs.to_vec();
        // Sorted by name so the request bytes are identical on every turn. See
        // the cache note on `ToolSpec`.
        tools.sort_by_key(|tool| tool.name);
        Self {
            tools,
            dynamic: Vec::new(),
        }
    }

    /// The `tools` array for a chat-completions request, in stable order.
    ///
    /// Returns `None` when nothing is registered, so the caller omits `tools` and
    /// `tool_choice` entirely. An empty array is not equivalent: several
    /// providers reject a request carrying one rather than ignoring it.
    ///
    /// Sorted here rather than only at construction so the cache guarantee holds
    /// for every way a registry can be built, including a struct literal in a
    /// test.
    pub fn specs(&self) -> Option<Value> {
        if self.tools.is_empty() && self.dynamic.is_empty() {
            return None;
        }
        // Built as (name, spec) and sorted by name, because a dynamic tool's name
        // is a `String` and the sort has to cover both lists as one. Sorting the
        // pairs rather than the finished values is what keeps the two from being
        // concatenated in two separately-sorted halves.
        let mut entries: Vec<(String, Value)> = self
            .tools
            .iter()
            .map(|tool| {
                (
                    tool.name.to_string(),
                    serde_json::json!({
                        "type": "function",
                        "function": {
                            "name": tool.name,
                            "description": tool.description,
                            "parameters": tool.schema(),
                        }
                    }),
                )
            })
            .chain(self.dynamic.iter().map(|tool| {
                (
                    tool.name.clone(),
                    serde_json::json!({
                        "type": "function",
                        "function": {
                            "name": tool.name,
                            "description": tool.description,
                            "parameters": tool.parameters,
                        }
                    }),
                )
            }))
            .collect();
        entries.sort_by(|(left, _), (right, _)| left.cmp(right));
        Some(Value::Array(
            entries.into_iter().map(|(_, spec)| spec).collect(),
        ))
    }

    /// Names of the registered tools, for tests.
    ///
    /// Test-only: nothing in the app needs the bare names, since it reads
    /// [`ToolRegistry::specs`] or looks a tool up by name. Owned rather than
    /// borrowed, because a dynamic tool's name is a `String` the registry holds
    /// and a caller asking a temporary registry for its names must not be left
    /// holding references into something already dropped.
    #[cfg(test)]
    pub fn names(&self) -> Vec<String> {
        let mut names: Vec<String> = self
            .tools
            .iter()
            .map(|tool| tool.name.to_string())
            .chain(self.dynamic.iter().map(|tool| tool.name.clone()))
            .collect();
        names.sort_unstable();
        names
    }

    /// The command line this call would run, or empty for a tool that runs none.
    ///
    /// Read from the tool's own declared field rather than assuming `command`,
    /// so a tool cannot end up uninspected by naming its argument differently.
    /// Empty for an unknown name, which the parse step has already rejected.
    pub fn dangerous_string(&self, name: &str, arguments: &Value) -> String {
        self.get(name)
            .and_then(|tool| tool.command_argument)
            .and_then(|field| arguments.get(field))
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string()
    }

    /// The schema for one named argument object.
    ///
    /// Unknown fields are rejected rather than ignored, because a model that
    /// sends `path` and `directory` when only `path` exists has made a mistake
    /// worth surfacing instead of silently resolving to a default.
    pub fn parse_arguments(&self, name: &str, raw: &str) -> Result<Value, String> {
        let schema = if let Some(tool) = self.tools.iter().find(|tool| tool.name == name) {
            tool.schema()
        } else if let Some(tool) = self.dynamic.iter().find(|tool| tool.name == name) {
            tool.parameters.clone()
        } else {
            return Err(format!("Unknown tool: {name}"));
        };
        let value: Value = serde_json::from_str(raw)
            .map_err(|error| format!("Tool {name} was called with invalid JSON: {error}"))?;
        if !value.is_object() {
            return Err(format!("Tool {name} expects an object of named arguments"));
        }
        if let Some(unknown) = unknown_field(&value, &schema) {
            return Err(format!(
                "Tool {name} has no argument named {unknown}. Available: {}",
                field_names(&schema).join(", ")
            ));
        }
        Ok(value)
    }

    /// Looks up a registered tool by name.
    pub fn get(&self, name: &str) -> Option<&ToolSpec> {
        self.tools.iter().find(|tool| tool.name == name)
    }

    /// What a registered tool does to the system, whichever kind it is.
    ///
    /// Both lists, because this is what decides caching and invalidation, and a
    /// dynamic tool read as a `Read` would be answered from the cache and would
    /// leave a file it changed looking current. The registry's own `get` cannot
    /// serve this: a `DynamicTool` has no `ToolSpec` to return.
    pub fn effect_of(&self, name: &str) -> Option<Effect> {
        if let Some(tool) = self.tools.iter().find(|tool| tool.name == name) {
            return Some(tool.effect);
        }
        self.dynamic
            .iter()
            .find(|tool| tool.name == name)
            .map(|tool| tool.effect)
    }

    /// Runs a tool and returns text for the model's next turn.
    ///
    /// The result is truncated before it is returned: a tool that prints ten
    /// megabytes must not be able to blow out the context window, and a model
    /// reading an untruncated dump is no better informed than one reading the
    /// first and last screens of it.
    ///
    /// A tool's own failure comes back as an `Err` carrying text for the model
    /// rather than as a run-ending error: the model needs to see why a call
    /// failed in order to correct it, and a wrong path is its mistake to fix,
    /// not a reason to discard the conversation.
    pub async fn run(
        &self,
        name: &str,
        arguments: Value,
        context: ToolContext,
    ) -> Result<String, String> {
        // A dynamic tool's closure is cloned out before the await, so the borrow
        // of `self` does not have to be held across it.
        let outcome = if let Some(tool) = self.tools.iter().find(|tool| tool.name == name) {
            (tool.execute)(arguments, context).await
        } else if let Some(tool) = self.dynamic.iter().find(|tool| tool.name == name) {
            let execute = Arc::clone(&tool.execute);
            execute(arguments, context).await
        } else {
            return Err(format!("Unknown tool: {name}"));
        };
        match outcome {
            Ok(text) => Ok(output::truncate(text)),
            Err(error) => Err(output::truncate(format!("{name} failed: {error}"))),
        }
    }
}

/// A field the caller sent that the schema does not define.
fn unknown_field(arguments: &Value, schema: &Value) -> Option<String> {
    let properties = schema.get("properties")?.as_object()?;
    arguments
        .as_object()?
        .keys()
        .find(|key| !properties.contains_key(*key))
        .cloned()
}

/// Names the schema declares, for the error message.
fn field_names(schema: &Value) -> Vec<String> {
    schema
        .get("properties")
        .and_then(Value::as_object)
        .map(|properties| properties.keys().cloned().collect())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai::tools::ToolMode;
    use serde_json::json;

    fn spec() -> ToolSpec {
        ToolSpec {
            name: "read_file",
            description: "reads a file",
            parameters: r#"{
                "type": "object",
                "properties": { "path": { "type": "string" } },
                "required": ["path"],
                "additionalProperties": false
            }"#,
            effect: Effect::Read,
            command_argument: None,
            execute: |_, _| Box::pin(std::future::ready(Ok("body".into()))),
        }
    }

    fn registry() -> ToolRegistry {
        ToolRegistry {
            tools: vec![spec()],
            dynamic: Vec::new(),
        }
    }

    #[test]
    fn parses_a_well_formed_call() {
        let parsed = registry()
            .parse_arguments("read_file", r#"{"path":"C:\\a.rs"}"#)
            .expect("valid call");
        assert_eq!(parsed["path"], "C:\\a.rs");
    }

    #[test]
    fn an_unknown_argument_is_reported_with_the_valid_ones() {
        // Silently dropping it would let the model believe it set something it
        // did not, and the mistake would surface much later as a wrong result.
        let error = registry()
            .parse_arguments("read_file", r#"{"file":"a.rs"}"#)
            .expect_err("unknown argument");
        assert!(error.contains("file"), "{error}");
        assert!(error.contains("path"), "{error}");
    }

    #[test]
    fn an_unknown_tool_is_reported_by_name() {
        assert!(registry()
            .parse_arguments("write_file", "{}")
            .expect_err("unknown tool")
            .contains("write_file"));
    }

    #[test]
    fn malformed_json_is_reported_rather_than_panicking() {
        assert!(registry()
            .parse_arguments("read_file", "{not json")
            .expect_err("malformed")
            .contains("invalid JSON"));
    }

    #[test]
    fn a_bare_array_is_not_accepted_as_arguments() {
        assert!(registry()
            .parse_arguments("read_file", "[]")
            .expect_err("array")
            .contains("object"));
    }

    #[test]
    fn a_tool_error_reaches_the_model_as_text() {
        // The model needs to see why it failed in order to fix its call, so a
        // failing tool returns a description rather than ending the run.
        let registry = ToolRegistry {
            tools: vec![ToolSpec {
                execute: |_, _| Box::pin(std::future::ready(Err("no such file".into()))),
                ..spec()
            }],
            dynamic: Vec::new(),
        };
        let error = blocking_run(&registry, "read_file");
        assert!(error.contains("no such file"), "{error}");
        assert!(error.starts_with("read_file failed"), "{error}");
    }

    /// Names as owned strings, for comparing against `names()`.
    fn listed(names: &[&str]) -> Vec<String> {
        names.iter().map(|name| name.to_string()).collect()
    }

    /// A runtime tool, of the shape an MCP server's tools have.
    fn dynamic(name: &'static str) -> DynamicTool {
        DynamicTool {
            name: name.to_string(),
            description: "a runtime tool".into(),
            parameters: json!({ "type": "object" }),
            effect: Effect::Write,
            execute: Arc::new(|_, _| Box::pin(std::future::ready(Ok("done".into())))),
        }
    }

    /// A dynamic tool is advertised, checked and run like any other: it is the
    /// same list the model reads and the same lookup a call resolves through.
    #[test]
    fn a_dynamic_tool_is_advertised_parsed_and_run() {
        let mut registry = ToolRegistry {
            tools: Vec::new(),
            dynamic: vec![DynamicTool {
                parameters: json!({
                    "type": "object",
                    "properties": { "path": { "type": "string" } },
                    "additionalProperties": false
                }),
                ..dynamic("mcp__files__read")
            }],
        };
        registry.add_dynamic(dynamic("unused"));

        let specs = registry.specs().expect("specs");
        assert_eq!(specs[0]["function"]["name"], "mcp__files__read");
        assert_eq!(
            registry.effect_of("mcp__files__read"),
            Some(Effect::Write),
            "a dynamic tool's effect must be readable, or a write is cached as a read"
        );

        // Its own schema is what arguments are validated against.
        let error = registry
            .parse_arguments("mcp__files__read", r#"{"other":1}"#)
            .expect_err("unknown argument");
        assert!(error.contains("path"), "{error}");

        assert_eq!(blocking_run(&registry, "mcp__files__read"), "done");
    }

    /// The cache invariant holds across both kinds: one list, sorted by name.
    #[test]
    fn dynamic_tools_sort_in_one_list_with_static_ones() {
        let mut registry = ToolRegistry {
            tools: vec![ToolSpec {
                name: "mmm",
                ..spec()
            }],
            dynamic: Vec::new(),
        };
        registry.add_dynamic(dynamic("zzz"));
        registry.add_dynamic(dynamic("aaa"));
        assert_eq!(registry.names(), listed(&["aaa", "mmm", "zzz"]));
    }

    /// The access set narrows a run by removing tools, and it has to remove the
    /// runtime ones too: they are advertised from `specs`, which is what the
    /// request body is built from.
    #[test]
    fn retaining_applies_to_dynamic_tools_too() {
        let mut registry = ToolRegistry {
            tools: vec![ToolSpec {
                name: "mmm",
                ..spec()
            }],
            dynamic: vec![dynamic("aaa"), dynamic("zzz")],
        };
        registry.retain(|name| name == "mmm" || name == "zzz");
        assert_eq!(registry.names(), listed(&["mmm", "zzz"]));
    }

    /// Runs a tool to completion from a synchronous test.
    ///
    /// Uses the tokio runtime the app already depends on rather than adding an
    /// async test attribute, so this file needs no extra dev-dependency for what
    /// is a two-line helper.
    fn blocking_run(registry: &ToolRegistry, name: &str) -> String {
        let context = ToolContext {
            cancelled: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
            sink: Sink::default(),
            database: None,
            run: None,
        };
        match tokio::runtime::Builder::new_current_thread()
            .build()
            .expect("test runtime")
            .block_on(registry.run(name, json!({}), context))
        {
            Ok(text) => text,
            Err(error) => error,
        }
    }

    #[test]
    fn specs_are_ordered_by_name_whatever_the_registration_order() {
        // This is the cache invariant: identical tools must serialize to
        // identical bytes on every turn, or the cached prefix is thrown away.
        let named = |name: &'static str| ToolSpec { name, ..spec() };
        let forward = ToolRegistry {
            tools: vec![named("aaa"), named("zzz"), named("mmm")],
            dynamic: Vec::new(),
        };
        let reversed = ToolRegistry {
            tools: vec![named("zzz"), named("mmm"), named("aaa")],
            dynamic: Vec::new(),
        };
        assert_eq!(forward.specs(), reversed.specs());
        assert_eq!(forward.names(), listed(&["aaa", "mmm", "zzz"]));
    }

    /// Chat mode registers no tools, and an empty `tools` array is not
    /// equivalent to omitting it -- some providers reject the request outright.
    /// So this pins the None, which is the whole of Chat mode's tool story.
    #[test]
    fn chat_mode_advertises_no_tools() {
        assert!(ToolRegistry::for_mode(ToolMode::Chat).specs().is_none());
    }
}
