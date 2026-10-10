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
//! model that invents a name -- [`crate::ai::tools::policy::decide_with_access`]
//! refuses it and the refusal is reported back as the tool's result, so the model
//! is told why rather than silently ignored.

use serde::{Deserialize, Serialize};

/// The capabilities one run may use.
///
/// Grouped, not per-tool: the user picks a handful of switches, and a tool added
/// later falls into a group rather than needing a new switch in the UI.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
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
}

/// Maps a tool name to its group.
///
/// An unrecognised name is treated as a *read*, which is the conservative choice:
/// it is the one group whose failure mode is a refused read rather than a
/// permitted write, and it means a tool added later is reachable only by a run
/// that may already read.
fn capability(tool_name: &str) -> Capability {
    match tool_name {
        "write_file" | "edit_file" | "edit_lines" | "skill_manage" | "move_file"
        | "delete_file" | "write_document" => Capability::Write,
        // The whole terminal group, so a run without the terminal cannot read or
        // stop what another run left running either.
        "run_terminal" | "terminal_output" | "terminal_kill" => Capability::Terminal,
        "search_web" | "web_fetch" => Capability::Web,
        "sub_agent" => Capability::Subagents,
        "schedule_job" => Capability::Jobs,
        // Asking the user reads their intent; it changes nothing, so it belongs
        // with the reads even though it is not cacheable.
        "ask_user" => Capability::Read,
        // Recording the agent's own checklist is likewise not a change to the
        // world; it is a note to itself.
        "todo" => Capability::Read,
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
        assert!(access.is_allowed("skill_read"));
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
