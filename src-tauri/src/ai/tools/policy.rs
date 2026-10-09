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
    /// The listed read and write tools run without asking.
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
    "search_files",
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
    "sub_agent",
    "schedule_job",
];

/// Decides whether this tool name runs automatically in the selected mode.
pub fn decide(mode: PermissionMode, tool_name: &str) -> Decision {
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

/// Decides whether a tool may run, given the level **and** the run's access set.
///
/// The access set is checked first and refuses outright: a capability the run does
/// not hold is not something a prompt can grant, and in an unattended job there is
/// nobody to prompt anyway. Only when the tool is permitted does the level decide
/// whether it runs now or waits.
///
/// This is the function the gate calls; [`decide`] stays the pure level question
/// and is what the tests exercise.
pub fn decide_with_access(mode: PermissionMode, tool_name: &str, access: &AccessSet) -> Decision {
    if !access.is_allowed(tool_name) {
        return Decision::Deny(format!(
            "this run is not allowed to use `{tool_name}`"
        ));
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
            decide_with_access(PermissionMode::Full, "run_terminal", &access),
            Decision::Deny("this run is not allowed to use `run_terminal`".to_string())
        );
    }

    #[test]
    fn a_permitted_tool_falls_through_to_the_level() {
        assert_eq!(
            decide_with_access(PermissionMode::Full, "write_file", &AccessSet::ALL),
            Decision::Allow
        );
        assert_eq!(
            decide_with_access(PermissionMode::Ask, "write_file", &AccessSet::ALL),
            Decision::Ask
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
            "search_files",
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
            "run_terminal",
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
            "search_files",
            "skill_read",
            "search_web",
            "web_fetch",
            "write_file",
            "edit_file",
            "edit_lines",
            "skill_manage",
            "sub_agent",
        ] {
            assert_eq!(decide(PermissionMode::AutoWrites, tool), Decision::Allow);
        }
        for tool in ["run_terminal", "unknown_tool"] {
            assert_eq!(decide(PermissionMode::AutoWrites, tool), Decision::Ask);
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
