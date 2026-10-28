//! Decides whether a tool call may run.
//!
//! This module is pure: it decides from the selected access level and tool name,
//! and never executes. The gate that applies the decision lives in the tool loop.

use serde::{Deserialize, Serialize};

use super::access::AccessSet;

/// How much the agent may do without asking.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PermissionMode {
    /// Every call waits for the user.
    #[default]
    Ask,
    /// Only the explicitly listed read tools run without asking.
    #[serde(rename = "auto_safe")]
    AutoSafe,
    /// The listed read and write tools run without asking. Running a program is
    /// not a write: the terminal family asks at this level and every level below
    /// `full`, because "auto-approve writes" is a statement about the workspace.
    #[serde(rename = "auto_writes")]
    AutoWrites,
    /// Every tool runs without asking.
    Full,
}

/// What the gate decided about one call.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Decision {
    /// Run it now.
    Allow,
    /// Stop and wait for the user.
    Ask,
    /// Refuse without asking.
    Deny(String),
}

/// Read tools, approved without asking from `auto_safe` upward.
///
/// `search_web` and `web_fetch` ride here because they only observe: a search
/// reads a search engine and a fetch reads one page, and both already have an
/// implicit ceiling (`search_web`'s fanout and `web_fetch`'s single URL). Asking
/// for every page a model reads would be a prompt per step of a research run,
/// which is the mode this level exists to make tolerable. `web_fetch` can reach
/// an internal address, so a user who does not want that should stay on `ask`.
const AUTO_READ_TOOLS: &[&str] = &[
    "list_dir",
    "read_file",
    "grep",
    "glob",
    "skill_read",
    "search_web",
    "web_fetch",
];
/// Write tools, approved without asking only from `auto_writes` upward.
///
/// `sub_agent` is here rather than in a class of its own. Delegation is gated
/// like the work it starts: a run in `Ask` or `AutoSafe` is asked before an agent
/// is launched, and one that already trusts writes may delegate without a prompt.
/// The sub-agent's own calls go through this same gate one level down, so its
/// leaves still decide for themselves.
const AUTO_WRITE_TOOLS: &[&str] = &[
    "write_file",
    "edit_file",
    "edit_lines",
    "skill_manage",
    "skill_write",
    "move_file",
    "delete_file",
    "write_document",
    "sub_agent",
    "schedule_job",
];

/// Tools that start, stop or observe a process.
///
/// Deliberately on **neither** allowlist: absence is the rule, so `decide` asks
/// for them at `ask`, `auto_safe` and `auto_writes`, and allows them only at
/// `full`. Running a program is not a write — it can do anything the account can,
/// not only what the tool set can — so a level that says "auto-approve writes"
/// must not hand it over. The observing pair is here with the launching pair
/// because one capability is one decision: a run with no business starting a
/// process has no business reading or stopping one either.
///
/// `skill_run` belongs to this group for the same reason: it executes a script,
/// which `access.rs` already classifies as the terminal capability.
///
/// Test-only: nothing in production consults this list -- absence from the two
/// allowlists *is* the rule -- and the tests below use it to state the exception
/// by name instead of leaving it unnamed.
#[cfg(test)]
const PROCESS_TOOLS: &[&str] = &[
    "run_terminal",
    "terminal_output",
    "terminal_status",
    "terminal_kill",
    "skill_run",
];

/// Decides whether this tool name runs automatically in the selected mode.
pub fn decide(mode: PermissionMode, tool_name: &str) -> Decision {
    // Asking the user is never itself something to approve. The prompt *is* the
    // interaction, and a dialog asking permission to show a dialog is a dead end
    // -- the model would have to be approved before it could ask, at every level
    // including `ask`. Recording the agent's own checklist is the same: it writes
    // a note to itself, not to the workspace.
    if tool_name == "ask_user" || tool_name == "todo" {
        return Decision::Allow;
    }
    let allowed = match mode {
        PermissionMode::Ask => false,
        PermissionMode::AutoSafe => AUTO_READ_TOOLS.contains(&tool_name),
        PermissionMode::AutoWrites => {
            AUTO_READ_TOOLS.contains(&tool_name) || AUTO_WRITE_TOOLS.contains(&tool_name)
        }
        PermissionMode::Full => true,
    };

    if allowed {
        Decision::Allow
    } else {
        Decision::Ask
    }
}

/// Decides whether a tool may run, given the level, the run's access set and the
/// host's own ceiling.
///
/// The host is checked first because its refusal is a different fact from a
/// narrowed run: "this job may not use the terminal" is the job's configuration,
/// while "the host has disabled the terminal" is an installation-wide decision
/// the user needs to understand cannot be changed by editing that job.
///
/// This is the function the gate calls; [`decide`] stays the pure level question.
pub fn decide_with_host_access(
    mode: PermissionMode,
    tool_name: &str,
    access: &AccessSet,
    host: &AccessSet,
) -> Decision {
    if !host.is_allowed(tool_name) {
        return Decision::Deny(format!(
            "Host has disabled {} for this account.",
            AccessSet::label_for(tool_name)
        ));
    }
    if !access.is_allowed(tool_name) {
        return Decision::Deny(format!("this run is not allowed to use `{tool_name}`"));
    }
    decide(mode, tool_name)
}

#[cfg(test)]
mod access_tests {
    use super::*;

    /// A capability the run lacks is refused, whatever the level -- including
    /// `Full`, which is the whole point: a job at `Full` may act without asking,
    /// but only within what it was allowed to touch.
    #[test]
    fn access_refuses_before_the_level_can_allow() {
        let access = AccessSet {
            terminal: false,
            ..AccessSet::ALL
        };
        assert_eq!(
            decide_with_host_access(
                PermissionMode::Full,
                "run_terminal",
                &access,
                &AccessSet::ALL
            ),
            Decision::Deny("this run is not allowed to use `run_terminal`".to_string())
        );
    }

    #[test]
    fn a_permitted_tool_falls_through_to_the_level() {
        assert_eq!(
            decide_with_host_access(
                PermissionMode::Full,
                "write_file",
                &AccessSet::ALL,
                &AccessSet::ALL
            ),
            Decision::Allow
        );
        assert_eq!(
            decide_with_host_access(
                PermissionMode::Ask,
                "write_file",
                &AccessSet::ALL,
                &AccessSet::ALL
            ),
            Decision::Ask
        );
    }

    /// A host switch is a refusal the model can report as what it is, and the
    /// wording names the capability rather than an internal tool name.
    #[test]
    fn a_host_disabled_capability_is_refused_in_the_users_words() {
        let host = AccessSet {
            terminal: false,
            ..AccessSet::ALL
        };
        assert_eq!(
            decide_with_host_access(PermissionMode::Full, "run_terminal", &AccessSet::ALL, &host),
            Decision::Deny("Host has disabled terminal for this account.".to_string())
        );
        assert_eq!(
            decide_with_host_access(
                PermissionMode::Full,
                "mcp__files__write",
                &AccessSet::ALL,
                &AccessSet {
                    mcp: false,
                    ..AccessSet::ALL
                },
            ),
            Decision::Deny("Host has disabled MCP tools for this account.".to_string())
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    // The two invariants below are driven off the registry's own classification
    // rather than a second copy of it.
    use crate::ai::tools::{registry_for, Effect, ToolMode};

    #[test]
    fn permission_modes_use_the_frontend_wire_names() {
        for (wire, mode) in [
            ("ask", PermissionMode::Ask),
            ("auto_safe", PermissionMode::AutoSafe),
            ("auto_writes", PermissionMode::AutoWrites),
            ("full", PermissionMode::Full),
        ] {
            let parsed = serde_json::from_str::<PermissionMode>(&format!("\"{wire}\""))
                .expect("frontend permission value should deserialize");
            assert_eq!(parsed, mode);
        }
    }

    #[test]
    fn ask_mode_prompts_for_every_tool() {
        for tool in ["read_file", "write_file", "run_terminal"] {
            assert_eq!(decide(PermissionMode::Ask, tool), Decision::Ask);
        }
    }

    #[test]
    fn auto_safe_only_approves_the_read_allowlist() {
        for tool in [
            "list_dir",
            "read_file",
            "grep",
            "glob",
            "skill_read",
            // Network reads live here too: a research run should not prompt once
            // per page it opens.
            "search_web",
            "web_fetch",
        ] {
            assert_eq!(decide(PermissionMode::AutoSafe, tool), Decision::Allow);
        }
        for tool in [
            "write_file",
            "edit_file",
            "edit_lines",
            "skill_manage",
            "skill_write",
            "move_file",
            "delete_file",
            "write_document",
            "sub_agent",
        ] {
            assert_eq!(decide(PermissionMode::AutoSafe, tool), Decision::Ask);
        }
    }

    #[test]
    fn auto_writes_approves_only_the_read_and_write_allowlists() {
        for tool in [
            "list_dir",
            "read_file",
            "grep",
            "glob",
            "skill_read",
            "search_web",
            "web_fetch",
            "write_file",
            "edit_file",
            "edit_lines",
            "skill_manage",
            "skill_write",
            "move_file",
            "delete_file",
            "write_document",
            "sub_agent",
            "schedule_job",
        ] {
            assert_eq!(decide(PermissionMode::AutoWrites, tool), Decision::Allow);
        }
        // The terminal family is not a write. A user who picked "auto-approve
        // writes" has approved changes to their workspace, not arbitrary
        // programs run as them.
        for tool in PROCESS_TOOLS {
            assert_eq!(
                decide(PermissionMode::AutoWrites, tool),
                Decision::Ask,
                "{tool} must ask at auto_writes"
            );
        }
        // A name no list knows still asks here, and is allowed at `full`: an
        // unrecognised tool is not an invitation to run it unprompted.
        assert_eq!(
            decide(PermissionMode::AutoWrites, "unknown_tool"),
            Decision::Ask
        );
        assert_eq!(
            decide(PermissionMode::Full, "unknown_tool"),
            Decision::Allow
        );
    }

    /// Every write tool is auto-approved at `auto_writes`, except the process
    /// tools, which are deliberately full-access-only.
    ///
    /// Driven off the registry's own `Effect` rather than a copy of the list, so
    /// a tool added later -- or one reclassified from `Read` to `Write` -- cannot
    /// end up silently prompting at a level that promises it will not. The
    /// exception is `PROCESS_TOOLS`, and it is checked separately below rather
    /// than skipped quietly: a process tool that reached an allowlist would be
    /// the regression this rule exists to prevent.
    #[test]
    fn every_write_tool_is_auto_approved_at_auto_writes_or_runs_a_process() {
        let registry = registry_for(ToolMode::Agent, true);
        let unlisted = registry
            .names()
            .into_iter()
            .filter(|name| registry.get(name).expect("registered").effect == Effect::Write)
            .filter(|name| !AUTO_WRITE_TOOLS.contains(&name.as_str()))
            .filter(|name| !PROCESS_TOOLS.contains(&name.as_str()))
            .collect::<Vec<_>>();
        assert!(
            unlisted.is_empty(),
            "these write tools would prompt at auto_writes: {unlisted:?}"
        );
    }

    /// The read half of the same invariant: every observing tool runs unprompted
    /// from `auto_safe` up, so a new read tool cannot be added and then prompt.
    /// The `PROCESS_TOOLS` exception is checked here as well as below: those
    /// tools only observe, but they reach the terminal capability, and asking is
    /// what that capability means below `full`. Their `Effect` is a separate
    /// question -- `terminal_output` and `terminal_status` are `Volatile`
    /// because a poll's answer changes without anything being written, which says
    /// nothing about whether polling needs permission.
    #[test]
    fn every_read_tool_is_auto_approved_at_auto_safe() {
        let registry = registry_for(ToolMode::Agent, true);
        let unlisted = registry
            .names()
            .into_iter()
            .filter(|name| registry.get(name).expect("registered").effect == Effect::Read)
            .filter(|name| !AUTO_READ_TOOLS.contains(&name.as_str()))
            .filter(|name| !PROCESS_TOOLS.contains(&name.as_str()))
            .collect::<Vec<_>>();
        assert!(
            unlisted.is_empty(),
            "these read tools would prompt at auto_safe: {unlisted:?}"
        );
    }

    /// The terminal family is reachable only from `full`.
    ///
    /// This is the rule a user means by "Full access": no level below it can run,
    /// stop, or read a program without a prompt. Asserted by name over the
    /// registry so renaming a tool cannot quietly take it out of the group
    /// without the list here being updated too.
    #[test]
    fn the_process_tools_ask_below_full_and_run_at_full() {
        let registry = registry_for(ToolMode::Agent, true);
        let registered = registry.names();
        for tool in PROCESS_TOOLS {
            assert!(
                registered.iter().any(|name| name == tool),
                "`{tool}` is in PROCESS_TOOLS but not registered"
            );
            for mode in [
                PermissionMode::Ask,
                PermissionMode::AutoSafe,
                PermissionMode::AutoWrites,
            ] {
                assert_eq!(
                    decide(mode, tool),
                    Decision::Ask,
                    "`{tool}` must prompt at {mode:?}"
                );
            }
            assert_eq!(
                decide(PermissionMode::Full, tool),
                Decision::Allow,
                "`{tool}` must run unprompted at full"
            );
        }
    }

    /// Asking the user is never gated: the prompt is the interaction, so a dialog
    /// asking permission to show a dialog would be unreachable at every level.
    /// The agent's own checklist is ungated for the same reason -- it writes a
    /// note to itself, and prompting for it every few steps would be noise.
    #[test]
    fn asking_the_user_and_the_checklist_are_always_allowed() {
        for mode in [
            PermissionMode::Ask,
            PermissionMode::AutoSafe,
            PermissionMode::AutoWrites,
            PermissionMode::Full,
        ] {
            for tool in ["ask_user", "todo"] {
                assert_eq!(decide(mode, tool), Decision::Allow, "{tool} at {mode:?}");
            }
        }
    }

    #[test]
    fn full_mode_approves_every_tool() {
        for tool in [
            "list_dir",
            "write_file",
            "run_terminal",
            "search_web",
            "web_fetch",
            "unknown_tool",
        ] {
            assert_eq!(decide(PermissionMode::Full, tool), Decision::Allow);
        }
    }

    /// Delegation is gated like a write. A run that must ask before touching the
    /// workspace must also ask before starting an agent that will; one that
    /// already trusts writes may delegate without a prompt.
    #[test]
    fn delegating_is_gated_like_a_write() {
        assert_eq!(decide(PermissionMode::Ask, "sub_agent"), Decision::Ask);
        assert_eq!(decide(PermissionMode::AutoSafe, "sub_agent"), Decision::Ask);
        assert_eq!(
            decide(PermissionMode::AutoWrites, "sub_agent"),
            Decision::Allow
        );
        assert_eq!(decide(PermissionMode::Full, "sub_agent"), Decision::Allow);
    }
}
