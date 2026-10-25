//! What a run is allowed to touch.
//!
//! # Where this sits
//!
//! [`crate::ai::tools::policy`] answers *"may this run do this without asking?"* --
//! a question about prompting. This answers a different one: *"may this run do
//! this at all?"* The two are independent. A job that runs at 3am unattended has
//! nobody to ask, so its `Ask` level would park forever on the first write; and a
//! capability the user switched off for a job must not become available just
//! because the level is `Full`.
//!
//! # Why it rides on the approval gate
//!
//! A sub-agent is handed its parent's `ApprovalGate` (`run.approval.clone()`), and
//! that is the one value every level of a run already shares. Carrying the access
//! set there means a delegated run inherits its parent's capabilities with no
//! second argument to thread and no way for the two to disagree -- which is the
//! requirement: a job's sub-agents may do exactly what the job may.
//!
//! # Both halves are enforced
//!
//! A disallowed tool is removed from the registry, so it is never advertised and
//! the model cannot ask for it. If one is asked for anyway -- a cached call, a
//! model that invents a name -- [`crate::ai::tools::policy::decide_with_host_access`]
//! refuses it and the refusal is reported back as the tool's result, so the model
//! is told why rather than silently ignored.

use serde::{Deserialize, Serialize};

/// The capabilities one run may use.
///
/// Grouped, not per-tool: the user picks a handful of switches, and a tool added
/// later falls into a group rather than needing a new switch in the UI.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct AccessSet {
    /// Filesystem and skill reads.
    pub read: bool,
    /// Filesystem writes, edits, and skill management.
    pub write: bool,
    /// Running programs in the workspace.
    pub terminal: bool,
    /// Web search and page fetching.
    pub web: bool,
    /// Delegating to a sub-agent.
    pub subagents: bool,
    /// Scheduling work for later.
    ///
    /// The only capability whose effect outlives the run that used it: everything
    /// else here limits what happens while the user is watching, and a job runs with
    /// nobody there. It is also the one a job's own run is never given, which is what
    /// keeps scheduled work one generation deep.
    pub jobs: bool,
    /// Tools offered by the configured MCP servers.
    ///
    /// Its own switch rather than riding on `write`: an MCP tool is classified as a
    /// write because the protocol does not say what it changes, but a job or a phone
    /// should still be able to turn that whole surface off without losing ordinary
    /// file writes.
    pub mcp: bool,
}

impl Default for AccessSet {
    fn default() -> Self {
        Self::ALL
    }
}

impl AccessSet {
    /// Everything on, which is what a normal conversation gets.
    ///
    /// The default on purpose: a run the user started by hand is not restricted,
    /// and only a job narrows this.
    pub const ALL: Self = Self {
        read: true,
        write: true,
        terminal: true,
        web: true,
        subagents: true,
        jobs: true,
        mcp: true,
    };

    /// Whether a tool name may be used by this run.
    pub fn is_allowed(&self, tool_name: &str) -> bool {
        match capability(tool_name) {
            Capability::Read => self.read,
            Capability::Write => self.write,
            Capability::Terminal => self.terminal,
            Capability::Web => self.web,
            Capability::Subagents => self.subagents,
            Capability::Jobs => self.jobs,
            Capability::Mcp => self.mcp,
        }
    }

    /// The capabilities both the host and this run allow.
    ///
    /// Narrowing is the only operation here. A per-run set can take away what the
    /// host grants, never add to it, so a job record or a channel setting cannot
    /// become a way around the installation-wide switches.
    pub fn within(self, host: Self) -> Self {
        Self {
            read: self.read && host.read,
            write: self.write && host.write,
            terminal: self.terminal && host.terminal,
            web: self.web && host.web,
            subagents: self.subagents && host.subagents,
            jobs: self.jobs && host.jobs,
            mcp: self.mcp && host.mcp,
        }
    }

    /// The user-facing name of the group a tool belongs to.
    ///
    /// Used in the host-disabled refusal so a model and the person reading it hear
    /// the capability by the name the Settings page uses, not by a tool name.
    pub fn label_for(tool_name: &str) -> &'static str {
        match capability(tool_name) {
            Capability::Read => "file reading",
            Capability::Write => "file writing",
            Capability::Terminal => "terminal",
            Capability::Web => "web access",
            Capability::Subagents => "subagents",
            Capability::Jobs => "jobs",
            Capability::Mcp => "MCP tools",
        }
    }
}

/// The group a tool belongs to.
enum Capability {
    Read,
    Write,
    Terminal,
    Web,
    Subagents,
    Jobs,
    Mcp,
}

/// Maps a tool name to its group.
///
/// An unrecognised name is treated as a *read*, which is the conservative choice:
/// it is the one group whose failure mode is a refused read rather than a
/// permitted write, and it means a tool added later is reachable only by a run
/// that may already read.
fn capability(tool_name: &str) -> Capability {
    match tool_name {
        "write_file" | "edit_file" | "edit_lines" | "skill_manage" | "skill_write"
        | "move_file" | "delete_file" | "write_document" => Capability::Write,
        // A skill's script is a program, so it follows the terminal switch rather
        // than the write one: a run that may not start a process may not start one
        // through a skill either. This is the group's whole meaning.
        "run_terminal" | "terminal_output" | "terminal_kill" | "terminal_status" | "skill_run" => {
            Capability::Terminal
        }
        "search_web" | "web_fetch" => Capability::Web,
        "sub_agent" => Capability::Subagents,
        "schedule_job" => Capability::Jobs,
        // Asking the user reads their intent; it changes nothing, so it belongs
        // with the reads even though it is not cacheable.
        "ask_user" => Capability::Read,
        // Recording the agent's own checklist is likewise not a change to the
        // world; it is a note to itself.
        "todo" => Capability::Read,
        // A tool from an MCP server has its own switch. It is still treated as a
        // write by policy -- the protocol does not report whether it changes
        // anything -- but the whole server surface can be taken away without also
        // taking away ordinary file writes.
        name if name.starts_with(crate::mcp::tools::TOOL_PREFIX) => Capability::Mcp,
        _ => Capability::Read,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_default_is_everything() {
        let access = AccessSet::default();
        for tool in [
            "read_file",
            "write_file",
            "run_terminal",
            "search_web",
            "sub_agent",
            "schedule_job",
            "mcp__files__read_file",
        ] {
            assert!(
                access.is_allowed(tool),
                "{tool} should be allowed by default"
            );
        }
    }

    #[test]
    fn a_switch_gates_its_whole_group() {
        let access = AccessSet {
            terminal: false,
            ..AccessSet::ALL
        };
        assert!(!access.is_allowed("run_terminal"));
        // The other groups are untouched.
        assert!(access.is_allowed("read_file"));
        assert!(access.is_allowed("write_file"));
    }

    /// Skill management is a write: it changes stored state, so it follows the
    /// write switch rather than the read one.
    #[test]
    fn skill_management_follows_the_write_switch() {
        let access = AccessSet {
            write: false,
            ..AccessSet::ALL
        };
        assert!(!access.is_allowed("skill_manage"));
        assert!(!access.is_allowed("skill_write"));
        assert!(access.is_allowed("skill_read"));
    }

    /// A skill's script is a program, so it follows the terminal switch. A run
    /// allowed to write files is not thereby allowed to start a process through a
    /// skill, which is the point of the two capabilities being separate.
    #[test]
    fn running_a_skill_script_follows_the_terminal_switch() {
        let writer = AccessSet {
            terminal: false,
            ..AccessSet::ALL
        };
        assert!(!writer.is_allowed("skill_run"));
        assert!(writer.is_allowed("skill_write"));

        let reader = AccessSet {
            terminal: false,
            write: false,
            ..AccessSet::ALL
        };
        assert!(!reader.is_allowed("skill_run"));
        assert!(reader.is_allowed("read_file"));
    }

    /// Scheduling work for later is its own switch, because it is the one thing a run
    /// can do whose effect outlives it -- the job runs when nobody is watching.
    #[test]
    fn scheduling_is_its_own_switch() {
        let access = AccessSet {
            jobs: false,
            ..AccessSet::ALL
        };
        assert!(!access.is_allowed("schedule_job"));
        // And the rest of the run is untouched by that narrowing.
        assert!(access.is_allowed("write_file"));
        assert!(access.is_allowed("sub_agent"));
    }

    /// MCP tools have their own switch, so a job can keep ordinary file writes and
    /// still turn off every server-provided tool.
    #[test]
    fn mcp_tools_follow_their_own_switch() {
        let access = AccessSet {
            mcp: false,
            ..AccessSet::ALL
        };
        assert!(!access.is_allowed("mcp__files__read_file"));
        assert!(access.is_allowed("write_file"));

        let writes_off = AccessSet {
            write: false,
            ..AccessSet::ALL
        };
        assert!(writes_off.is_allowed("mcp__files__read_file"));
        assert!(!writes_off.is_allowed("write_file"));
    }

    /// The host can narrow a run, and a run can never widen itself back out.
    #[test]
    fn host_access_is_an_outer_ceiling() {
        let host = AccessSet {
            terminal: false,
            mcp: false,
            ..AccessSet::ALL
        };
        let narrowed = AccessSet::ALL.within(host);
        assert!(!narrowed.terminal);
        assert!(!narrowed.mcp);
        assert!(narrowed.write);

        let already_off = AccessSet {
            web: false,
            ..AccessSet::ALL
        };
        assert!(!already_off.within(AccessSet::ALL).web);
    }

    /// A name nobody knows is treated as a read, so it is denied by a run that
    /// cannot read and never granted write access by accident.
    #[test]
    fn unknown_tools_are_treated_as_reads() {
        let access = AccessSet {
            read: false,
            ..AccessSet::ALL
        };
        assert!(!access.is_allowed("some_future_tool"));
    }
}
