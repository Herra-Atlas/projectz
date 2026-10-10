//! Delegating one task to a separate agent.
//!
//! The tool is thin on purpose: everything about *running* a sub-agent lives in
//! [`crate::ai::subagent`], and this file is only the schema the model reads and
//! the executor that hands the call over. A tool whose schema and behaviour drift
//! is a tool that lies to the model, so the two are kept as close as the shape
//! allows.
//!
//! # Why it is a `Write`
//!
//! Not because it writes files itself -- it may, through the sub-agent -- but
//! because of what `Write` *means* to the rest of the app. Two consequences, both
//! correct here:
//!
//! - Its result is never cached. The same task run twice should run twice, and a
//!   cached answer to a delegated task would be stale work returned as fresh.
//! - It invalidates the conversation's earlier cached reads. A sub-agent may have
//!   changed a file, and the parent's cached `read_file` of that file was taken
//!   before the change. Stamping the session is what stops the parent being handed
//!   pre-edit content, and it needs no reasoning about what the sub-agent did.

use super::{Effect, ToolSpec};

/// Hands one task to a separate agent and returns its final message.
pub const SUB_AGENT: ToolSpec = ToolSpec {
    name: "sub_agent",
    description:
        "Delegate one focused task to a separate agent that works in the same workspace with the \
same tools you have. Use it to keep a large or self-contained piece of work out of this \
conversation, or to run several independent tasks at once: issue one sub_agent call per agent \
in the same turn and they run in parallel. You get back only the agent's final message, so write \
the task as a complete brief — it cannot see this conversation and cannot ask you questions. \
Good fits: research, a broad search across a codebase, or any job you only need the conclusion of. \
Do not overlap them: never point two agents at the same files, and do not delegate an edit you \
are making yourself, because concurrent writes to one file race and one of them is lost. If you \
are unsure whether the work is independent, do it yourself. Set `type` to `explore` for a \
read-only investigation or `plan` for a read-only plan; both are barred from changing anything.",
    parameters: r#"{
        "type": "object",
        "properties": {
            "instructions": {
                "type": "string",
                "description": "The complete task for the agent, in enough detail to be done without follow-up questions. State what to find or do and what the final report should contain."
            },
            "label": {
                "type": "string",
                "description": "A short name for this agent, two to five words, shown to the user and used to tell its result apart from other agents'. Omit to derive one from the task."
            },
            "model": {
                "type": "string",
                "description": "Optional model id to run this agent on, for a task that does not need the main model. Omit to use the configured sub-agent model."
            },
            "type": {
                "type": "string",
                "enum": ["general", "explore", "plan"],
                "description": "The kind of agent. `general` (the default) may do anything you can. `explore` reads and searches only. `plan` reads only and returns a plan instead of making changes. Both non-general kinds are barred from writing, running commands or editing, so they cannot change the workspace."
            }
        },
        "required": ["instructions"],
        "additionalProperties": false
    }"#,
    effect: Effect::Write,
    // A sub-agent is not a shell, so there is no command line for the permission
    // prompt to quote. Its calls are gated on their own.
    command_argument: None,
    execute: |arguments, context| Box::pin(crate::ai::subagent::spawn(arguments, context)),
};

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai::tools::registry_for_with;
    use crate::ai::tools::ToolMode;

    /// The schema is a literal, so a typo in it would otherwise only surface as a
    /// confusing tool-call error at runtime.
    #[test]
    fn the_schema_is_valid_json_and_names_its_one_required_field() {
        let schema: serde_json::Value =
            serde_json::from_str(SUB_AGENT.parameters).expect("valid schema");
        assert_eq!(schema["required"][0], "instructions");
        assert!(schema["properties"]["label"].is_object());
        assert_eq!(schema["additionalProperties"], false);
    }

    /// The effect is what keeps a delegated task from being cached and makes a
    /// sub-agent's writes invalidate its parent's reads.
    #[test]
    fn spawning_is_a_write_for_caching_and_invalidation() {
        assert_eq!(SUB_AGENT.effect, Effect::Write);
        assert_eq!(SUB_AGENT.command_argument, None);
    }

    /// It is advertised in Agent mode and withheld from a sub-agent of its own.
    #[test]
    fn the_tool_is_agent_only_and_never_nests() {
        assert!(registry_for_with(ToolMode::Agent, false, true)
            .names()
            .contains(&"sub_agent"));
        assert!(!registry_for_with(ToolMode::Agent, false, false)
            .names()
            .contains(&"sub_agent"));
        assert!(!registry_for_with(ToolMode::Chat, true, true)
            .names()
            .contains(&"sub_agent"));
    }

    /// The description has to steer the model away from the one use that breaks:
    /// two agents editing the same files at once. Without it a model reads the
    /// "several at once" sentence and parallelizes work that cannot be
    /// parallelized.
    #[test]
    fn the_description_warns_against_overlapping_work() {
        let description = SUB_AGENT.description;
        assert!(description.contains("parallel"), "{description}");
        assert!(description.contains("same files"), "{description}");
        assert!(description.contains("race"), "{description}");
    }
}
