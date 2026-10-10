//! Tools the model may call during a run.
//!
//! One file per tool holds that tool's schema, executor and tests; nothing else
//! needs to know how it works. [`ToolRegistry`] is the only thing the chat
//! runtime talks to.
//!
//! Executors live in Rust rather than the frontend on purpose. Filesystem and
//! process work is where the latency and the risk are, and a Tauri command
//! round trip per tool call would add latency to every step of an agent loop
//! while putting the permission boundary somewhere it could be bypassed.

mod access;
pub mod agent;
mod approval;
mod ask;
mod background;
mod cache;
mod diff;
mod export;
mod fetch;
mod file;
mod glob;
mod grep;
mod output;
mod paths;
mod policy;
mod registry;
mod run;
mod schedule;
pub mod skills;
mod terminal;
mod todo;
mod vision;
mod walk;
mod websearch;
mod write;

pub use access::AccessSet;
pub use approval::{Approval, ApprovalGate, ApprovalRequest, QuestionRequest};
pub use cache::ToolCache;
pub use policy::{Decision, PermissionMode};
pub use registry::{Effect, Sink, ToolContext, ToolRegistry, ToolSpec};
pub use run::{RunHandle, SubAgentNotice};
pub use ui::ToolSummary;

/// The path boundary, for the file tree in `panel/`.
///
/// Re-exported at crate visibility rather than the `file` module itself being
/// made public. The panel is app chrome and shares this check so the folder the
/// *user* browses cannot escape the workspace any more than a tool call can --
/// two containment checks would be two things to keep correct, and the one that
/// drifted would be the bug. Nothing else about `file` is needed outside its own
/// module, and this binary is not a library, so crate visibility is enough.
pub(crate) use file::{require_workspace, resolve_within};

/// The folder every tool path resolves against.
///
/// Public because three things outside `tools/` need it and all of them need the
/// *same* value: the runtime writes it when the user picks a workspace, the
/// command layer reports it for the header, and the chat loop puts it in the
/// prompt. Keeping those behind a private module would mean each of them
/// reaching through a tool to get it, which is how a global and a setting end up
/// disagreeing about which folder is current.
pub mod workspace;

/// How one call reads in the transcript.
///
/// Public because the chat loop builds the panel's events from it: the loop
/// knows the arguments and the result, and it is the only place both exist at
/// once. A tool author does not call this — the description is derived, not
/// declared.
pub mod ui;

/// Which set of tools a run may use.
///
/// Sent on the request rather than resolved in the frontend, so a mode that
/// claims to have no tools genuinely sends no `tools` array.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ToolMode {
    /// No tools at all. The default, and what every non-agent conversation uses.
    #[default]
    Chat,
    /// Workspace read and write access.
    Agent,
}

/// Builds the registry for one run.
///
/// Three independent switches rather than an enum per combination: web search and
/// page fetching are per-conversation toggles, Agent mode is a mode the user
/// picks, and a user may want any of them without the others.
///
/// The result is sorted by name inside `ToolRegistry`, so the order tools are
/// added here cannot affect the cached request prefix.
///
/// Test-only since the chat loop calls [`registry_for_with`] directly: it always
/// has an `allow_subagents` value to pass, so the wrapper's default of `true`
/// would only ever hide which one a caller meant.
#[cfg(test)]
pub fn registry_for(mode: ToolMode, web_search_enabled: bool) -> ToolRegistry {
    registry_for_with(mode, web_search_enabled, true)
}

/// The same set, with sub-agent spawning optionally withheld.
///
/// A sub-agent is built with `allow_subagents = false`, which is what keeps
/// delegation one level deep: a spawned agent has every workspace tool its parent
/// has *except* the one that would let it spawn again. One flag rather than a
/// depth counter because a depth of more than one was never wanted, and a counter
/// invites a limit that is only ever discovered by exceeding it.
pub fn registry_for_with(
    mode: ToolMode,
    web_search_enabled: bool,
    allow_subagents: bool,
) -> ToolRegistry {
    let mut registry = ToolRegistry::for_mode(mode);
    // Agent-only: delegation is a tool the model reaches for, so it has no place
    // in a Chat conversation that has no tools at all.
    if allow_subagents && mode == ToolMode::Agent {
        registry.add(agent::SUB_AGENT);
    }
    // Agent-only for the same reason, and one capability narrower: scheduling is only
    // reachable by a run whose access set carries `jobs`, so a job's own run never sees
    // this tool and scheduled work cannot schedule more of itself.
    if mode == ToolMode::Agent {
        registry.add(schedule::SCHEDULE_JOB);
    }
    if web_search_enabled {
        registry.add(websearch::spec());
        // Fetching a specific URL only makes sense once searching is on: a model
        // reaches for `web_fetch` after `search_web` has given it a link, and
        // advertising it on its own invites a model to guess URLs.
        registry.add(fetch::spec());
    }
    registry
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chat_mode_without_search_advertises_nothing() {
        // The common case must send no `tools` key at all. An empty array is not
        // equivalent: some providers reject the request outright.
        assert!(registry_for(ToolMode::Chat, false).specs().is_none());
    }

    #[test]
    fn web_search_alone_still_advertises_exactly_one_tool() {
        let registry = registry_for(ToolMode::Chat, true);
        // Search plus the fetch that follows from it, and nothing else.
        assert_eq!(registry.names(), vec!["search_web", "web_fetch"]);
    }

    #[test]
    fn page_fetching_arrives_with_search_and_not_on_its_own() {
        // A model reaches for `web_fetch` after a search hands it a link.
        // Advertised on its own it would invite guessed URLs.
        assert!(!registry_for(ToolMode::Chat, false)
            .names()
            .contains(&"web_fetch"));
        assert!(!registry_for(ToolMode::Agent, false)
            .names()
            .contains(&"web_fetch"));
    }

    #[test]
    fn agent_mode_carries_read_file() {
        assert!(registry_for(ToolMode::Agent, false)
            .names()
            .contains(&"read_file"));
    }

    /// The composer sends a checked skill's body inline, so the tools are for the
    /// skills the user did not check. That path only exists with tools, so Chat
    /// mode must not advertise them.
    #[test]
    fn the_skill_tools_are_agent_only() {
        for mode in [ToolMode::Chat] {
            let names = registry_for(mode, true).names();
            assert!(!names.contains(&"skill_read"), "{names:?}");
            assert!(!names.contains(&"skill_manage"), "{names:?}");
        }
        let names = registry_for(ToolMode::Agent, false).names();
        assert!(names.contains(&"skill_read"), "{names:?}");
        assert!(names.contains(&"skill_manage"), "{names:?}");
    }

    /// Reading and managing skills have the expected effects for cache behavior;
    /// access levels explicitly determine whether either tool asks.
    #[test]
    fn reading_a_skill_is_a_read_and_saving_one_is_a_write() {
        let registry = registry_for(ToolMode::Agent, false);
        let effect = |name: &str| registry.get(name).expect("registered").effect;
        assert_eq!(effect("skill_read"), Effect::Read);
        assert_eq!(effect("skill_manage"), Effect::Write);
    }

    #[test]
    fn search_is_added_alongside_the_agent_tools_rather_than_replacing_them() {
        assert_eq!(
            registry_for(ToolMode::Agent, true).names(),
            vec![
                "ask_user",
                "delete_file",
                "edit_file",
                "edit_lines",
                "glob",
                "grep",
                "list_dir",
                "move_file",
                "read_file",
                "run_terminal",
                "schedule_job",
                "search_web",
                "skill_manage",
                "skill_read",
                "sub_agent",
                "terminal_kill",
                "terminal_output",
                "todo",
                "web_fetch",
                "write_document",
                "write_file"
            ]
        );
    }

    /// A sub-agent gets every tool its parent has except the one that would let it
    /// delegate again. Delegation is one level deep by construction, not by a depth
    /// counter that has to be checked at runtime.
    #[test]
    fn a_sub_agent_cannot_spawn_another_sub_agent() {
        let names = registry_for_with(ToolMode::Agent, true, false).names();
        assert!(!names.contains(&"sub_agent"), "{names:?}");
        // Everything else it needs to do work is still there.
        for tool in ["read_file", "write_file", "run_terminal", "search_web"] {
            assert!(names.contains(&tool), "{tool} missing: {names:?}");
        }
    }

    /// The gap between a parent's tool set and a sub-agent's is exactly one tool,
    /// so a future addition to the agent tools is not silently withheld.
    #[test]
    fn the_only_difference_is_the_spawn_tool() {
        let parent = registry_for_with(ToolMode::Agent, true, true).names();
        let child = registry_for_with(ToolMode::Agent, true, false).names();
        let difference = parent
            .into_iter()
            .filter(|name| !child.contains(name))
            .collect::<Vec<_>>();
        assert_eq!(difference, vec!["sub_agent"]);
    }

    #[test]
    fn the_tool_order_is_by_name_and_not_registration_order() {
        // Cache invariant: the same tool set must always serialize identically,
        // whichever order the switches happened to add them in.
        let names = |mode: ToolMode, search: bool| registry_for(mode, search).names();
        assert_eq!(
            names(ToolMode::Agent, true),
            vec![
                "ask_user",
                "delete_file",
                "edit_file",
                "edit_lines",
                "glob",
                "grep",
                "list_dir",
                "move_file",
                "read_file",
                "run_terminal",
                "schedule_job",
                "search_web",
                "skill_manage",
                "skill_read",
                "sub_agent",
                "terminal_kill",
                "terminal_output",
                "todo",
                "web_fetch",
                "write_document",
                "write_file"
            ]
        );
        assert_eq!(
            names(ToolMode::Agent, false),
            vec![
                "ask_user",
                "delete_file",
                "edit_file",
                "edit_lines",
                "glob",
                "grep",
                "list_dir",
                "move_file",
                "read_file",
                "run_terminal",
                "schedule_job",
                "skill_manage",
                "skill_read",
                "sub_agent",
                "terminal_kill",
                "terminal_output",
                "todo",
                "write_document",
                "write_file"
            ]
        );
    }
}
