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
//!
//! The framing also carries the guardrails such a run needs most: no destructive
//! or irreversible commands, no sweeping edits outside the task, and edits kept
//! local because another agent may be writing to the same workspace at the same
//! moment. The tools refuse a path outside the root on their own; these
//! instructions cover what the boundary cannot -- an in-workspace action that is
//! nonetheless not what the task asked for.

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
Stay strictly inside the task and the workspace. Do not run destructive or irreversible \
commands: no deleting files or folders, no `git reset`, `git push --force` or history rewrites, \
no killing processes, and nothing that reaches a path outside the workspace. Do not make sweeping \
changes -- no reformatting, renaming or restructuring files the task did not name. Another agent \
may be working in this same workspace at the same time, so keep your edits local to the files the \
task names and never rewrite a shared file wholesale; if the task calls for no change, \
investigate and report it instead of editing.\n\n\
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

    /// A sub-agent has no one to stop it, so it is told the two ways it could do
    /// damage: an irreversible command, and an edit that tramples work another
    /// agent may be doing in the same workspace at the same time.
    #[test]
    fn the_instruction_forbids_destructive_and_sweeping_changes() {
        let text = instruction("tidy", "Remove the dead code in src/a.rs");
        assert!(text.contains("destructive"), "{text}");
        assert!(text.contains("outside the workspace"), "{text}");
        assert!(text.contains("Another agent"), "{text}");
        assert!(text.contains("local to the files"), "{text}");
    }
}
