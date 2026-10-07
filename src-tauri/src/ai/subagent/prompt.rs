//! What a sub-agent is told it is.
//!
//! # Why this is a user message, not a system one
//!
//! The loop already puts the environment note into a single leading `system`
//! turn, and llama.cpp's templates reject a `system` message that is not first.
//! A second one would therefore work against a remote provider and fail against a
//! local model — the worst kind of difference, because it would look like the
//! local model was simply worse at the job.
//!
//! So the framing travels as the first user message, the same way a skill's
//! instructions ride inside the user's own message rather than as their own
//! turn. The one leading `system` turn is left to the loop.
//!
//! # Why the framing says so much
//!
//! A sub-agent is the one kind of run with nobody watching and no way to ask.
//! Left to infer that, a model behaves like a chat assistant: it asks a
//! clarifying question, offers options, or ends with "let me know if you want me
//! to continue" — and all of that is *lost*, because the only thing that leaves
//! the run is its final message. The instructions below exist to make the failure
//! mode impossible rather than to describe the situation.

/// The framed task, as the sub-agent's opening message.
pub fn instruction(label: &str, prompt: &str) -> String {
    format!(
        "You are a sub-agent working on one task for another agent. You have the same workspace \
tools it has. No one is watching this conversation and you cannot ask questions: the only thing \
that leaves here is your final message, so a question or a suggestion for what to do next is \
lost work.\n\n\
Do the task completely and on your own, then finish with a concise report of what you found or \
did. State the result plainly and name the files, commands or facts that matter. Do not open \
with a restatement of the task, do not close by offering to continue, and do not pad the report \
with what you were asked.\n\n\
Your name for this task is `{label}`.\n\n\
Task:\n{prompt}"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The task has to reach the agent intact; everything else is framing.
    #[test]
    fn the_instruction_carries_the_task_and_the_label() {
        let text = instruction("car research", "Find out how many cars exist");
        assert!(text.contains("car research"), "{text}");
        assert!(text.contains("Find out how many cars exist"), "{text}");
    }

    /// Two things a run with no audience must not do. Both are asserted because
    /// the whole reason this framing exists is that the failure is invisible: the
    /// question is swallowed and the parent is told nothing went wrong.
    #[test]
    fn the_instruction_forbids_questions_and_sign_offs() {
        let text = instruction("grep", "Count the TODOs");
        assert!(text.contains("cannot ask questions"), "{text}");
        assert!(text.contains("offering to continue"), "{text}");
    }
}
