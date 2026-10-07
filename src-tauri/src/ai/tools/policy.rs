//! Decides whether a tool call may run.
//!
//! This module is pure: it decides from the selected access level and tool name,
//! and never executes. The gate that applies the decision lives in the tool loop.

use serde::{Deserialize, Serialize};

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

const AUTO_READ_TOOLS: &[&str] = &["list_dir", "read_file", "search_files", "skill_read"];
const AUTO_WRITE_TOOLS: &[&str] = &["write_file", "edit_file", "edit_lines", "skill_manage"];

/// Tools that only arrange work rather than doing it, and so never prompt.
///
/// `spawn_agent` is the whole list. Starting a sub-agent changes nothing on its
/// own -- the sub-agent's own calls are what touch the workspace, and those go
/// through this same gate, one level down. Prompting here as well would ask the
/// user to approve a delegation and then ask again for each thing it does, which
/// is two prompts for one decision. Letting the leaves decide is also what makes
/// the access levels mean the same thing inside a sub-agent as outside it.
const ORCHESTRATION_TOOLS: &[&str] = &["spawn_agent"];

/// Decides whether this tool name runs automatically in the selected mode.
pub fn decide(mode: PermissionMode, tool_name: &str) -> Decision {
    if ORCHESTRATION_TOOLS.contains(&tool_name) {
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
        for tool in ["list_dir", "read_file", "search_files", "skill_read"] {
            assert_eq!(decide(PermissionMode::AutoSafe, tool), Decision::Allow);
        }
        for tool in [
            "write_file",
            "edit_file",
            "edit_lines",
            "skill_manage",
            "run_terminal",
            "search_web",
            "web_fetch",
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
            "write_file",
            "edit_file",
            "edit_lines",
            "skill_manage",
        ] {
            assert_eq!(decide(PermissionMode::AutoWrites, tool), Decision::Allow);
        }
        for tool in ["run_terminal", "search_web", "web_fetch", "unknown_tool"] {
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

    /// Delegation is allowed in every mode, because it does no work itself: the
    /// sub-agent's own calls are gated the same way its parent's are. Prompting
    /// here too would ask twice for one decision.
    #[test]
    fn spawning_is_orchestration_and_never_prompts() {
        for mode in [
            PermissionMode::Ask,
            PermissionMode::AutoSafe,
            PermissionMode::AutoWrites,
            PermissionMode::Full,
        ] {
            assert_eq!(decide(mode, "spawn_agent"), Decision::Allow, "{mode:?}");
        }
    }
}
