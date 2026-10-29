//! Waiting for a sub-agent that was started earlier.
//!
//! The other half of [`sub_agent`](super::agent). Delegation returns at once and
//! the report arrives on its own, which is right for independent work; this tool
//! is for the case that has no other door -- a next step that cannot be taken
//! until the delegated answer exists. Without it the parent had to finish its
//! reply and be given the report as a new turn, paying for a whole second request
//! to read one result.
//!
//! # Why `Volatile`
//!
//! The result is a fact about the present: with identical arguments the first call
//! parks and the second returns, so caching one would hand every later call the
//! first answer. It is not a `Write` either -- waiting changes nothing, so it must
//! not evict the conversation's cached reads. `Volatile` is exactly that pair, the
//! same classification `terminal_output` uses for the same reason.
//!
//! # Why it is not part of `sub_agent`
//!
//! A tool has one effect. Fused into the spawn tool, which is a `Write` because a
//! delegated run may edit files, every status check would invalidate the parent's
//! cached reads -- a poll that costs the conversation its cache.

use super::{Effect, ToolSpec};

/// Waits for a sub-agent's report, or reports how far it has got.
pub const SUBAGENT_WAIT: ToolSpec = ToolSpec {
    name: "subagent_wait",
    description:
        "Wait for a sub-agent you started earlier, and get its report back now instead of \
finishing your turn. Use this only when your next step depends on what that agent finds: the \
report arrives as its own message anyway, so waiting on work you do not need yet just holds the \
reply up. Pass the `agent_id` printed when the agent was started. The call parks up to `seconds` \
and then answers with the report, or with how many steps the agent has done and what it is doing \
now -- call it again, or carry on. Set `peek` to answer immediately with progress and never wait. \
Do not call it for an agent that already reported: its report is in the conversation.",
    parameters: r#"{
        "type": "object",
        "properties": {
            "agent_id": {
                "type": "string",
                "description": "The id `sub_agent` printed when the agent was started."
            },
            "seconds": {
                "type": "integer",
                "minimum": 1,
                "maximum": 120,
                "description": "How long to wait before answering with progress instead. Default 30, capped at 120. A longer wait is one call; repeated short polls cost a request each."
            },
            "peek": {
                "type": "boolean",
                "description": "Answer at once with the agent's progress, or its report if it has already finished. Never waits."
            }
        },
        "required": ["agent_id"],
        "additionalProperties": false
    }"#,
    effect: Effect::Volatile,
    // Waiting runs no command, so there is no line for an approval prompt to quote.
    command_argument: None,
    execute: |arguments, context| Box::pin(crate::ai::subagent::wait(arguments, context)),
};

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai::tools::ToolContext;
    use serde_json::json;

    /// The schema is a literal, so a typo would otherwise surface only as a
    /// confusing tool-call error at runtime.
    #[test]
    fn the_schema_is_valid_json_and_requires_the_agent_id() {
        let schema: serde_json::Value =
            serde_json::from_str(SUBAGENT_WAIT.parameters).expect("valid schema");
        assert_eq!(schema["required"][0], "agent_id");
        assert_eq!(schema["properties"]["seconds"]["maximum"], 120);
        assert_eq!(schema["additionalProperties"], false);
    }

    /// A poll may be repeated and must never be answered from the tool cache, and
    /// it changes nothing, so it must not invalidate the reads taken before it.
    #[test]
    fn waiting_is_volatile_for_caching() {
        assert_eq!(SUBAGENT_WAIT.effect, Effect::Volatile);
    }

    /// An id nobody holds is named rather than waited on for thirty seconds.
    #[tokio::test]
    async fn an_unknown_agent_is_refused_by_name() {
        let error = (SUBAGENT_WAIT.execute)(
            json!({ "agent_id": "no-such-agent" }),
            ToolContext::default(),
        )
        .await
        .expect_err("unknown agent");
        assert!(error.contains("no-such-agent"), "{error}");
    }

    /// A missing id is a mistake the model can correct, so it is named too.
    #[tokio::test]
    async fn a_missing_agent_id_is_reported() {
        let error = (SUBAGENT_WAIT.execute)(json!({}), ToolContext::default())
            .await
            .expect_err("no id");
        assert!(error.contains("agent_id"), "{error}");
    }
}
