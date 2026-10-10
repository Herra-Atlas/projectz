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

/// The kind of work a sub-agent was asked to do.
///
/// The kinds differ in what they are allowed to do as much as in what they are
/// told: `Explore` and `Plan` are handed a narrowed access set by
/// [`super::spawn`], so the framing here and the refusal in the tool gate agree.
/// A prompt that said "do not edit" while the tool still allowed an edit would be
/// a promise the app could not keep.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    /// May do anything the parent can.
    General,
    /// Reads and searches only, and reports what it found.
    Explore,
    /// Reads only, and returns a plan rather than making changes.
    Plan,
}

impl Kind {
    /// The kind named by the call's `type` argument, defaulting to `General`.
    ///
    /// An unknown value falls back rather than failing: a model that invents a
    /// type should still get an agent, and `General` is the one that matches the
    /// tool's own default. The capability narrowing only ever applies to a kind
    /// this function positively recognised.
    pub fn parse(value: Option<&str>) -> Self {
        match value.map(str::trim) {
            Some("explore") => Kind::Explore,
            Some("plan") => Kind::Plan,
            _ => Kind::General,
        }
    }

    /// Whether this kind may change the workspace.
    pub fn may_change(&self) -> bool {
        matches!(self, Kind::General)
    }

    /// The extra framing this kind needs, or nothing for the general one.
    fn framing(&self) -> &'static str {
        match self {
            Kind::General => "",
            Kind::Explore => {
                "You are a read-only agent: you may read files, search and list, but \
you cannot write, edit, run commands or change the workspace, and you should not try. Your job is \
to investigate and report what you found.\n\n"
            }
            Kind::Plan => {
                "You are a read-only agent whose result is a plan. You may read, search \
and list, but you cannot change anything, and you should not try. Investigate enough to be sure \
of the ground, then finish with a concrete, ordered plan the caller can carry out: name the files, \
the functions and the steps, in the order they should happen.\n\n"
            }
        }
    }
}

/// The framed task, as the sub-agent's opening message.
pub fn instruction(kind: Kind, label: &str, prompt: &str) -> String {
    format!(
        "You are a sub-agent working on one task for another agent. You have the same workspace \
tools it has. No one is watching this conversation and you cannot ask questions: the only thing \
that leaves here is your final message, so a question or a suggestion for what to do next is \
lost work.\n\n\
{framing}\
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
Task:\n{prompt}",
        framing = kind.framing()
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The task has to reach the agent intact; everything else is framing.
    #[test]
    fn the_instruction_carries_the_task_and_the_label() {
        let text = instruction(
            Kind::General,
            "car research",
            "Find out how many cars exist",
        );
        assert!(text.contains("car research"), "{text}");
        assert!(text.contains("Find out how many cars exist"), "{text}");
    }

    /// Two things a run with no audience must not do. Both are asserted because
    /// the whole reason this framing exists is that the failure is invisible: the
    /// question is swallowed and the parent is told nothing went wrong.
    #[test]
    fn the_instruction_forbids_questions_and_sign_offs() {
        let text = instruction(Kind::General, "grep", "Count the TODOs");
        assert!(text.contains("cannot ask questions"), "{text}");
        assert!(text.contains("offering to continue"), "{text}");
    }

    /// A sub-agent has no one to stop it, so it is told the two ways it could do
    /// damage: an irreversible command, and an edit that tramples work another
    /// agent may be doing in the same workspace at the same time.
    #[test]
    fn the_instruction_forbids_destructive_and_sweeping_changes() {
        let text = instruction(Kind::General, "tidy", "Remove the dead code in src/a.rs");
        assert!(text.contains("destructive"), "{text}");
        assert!(text.contains("outside the workspace"), "{text}");
        assert!(text.contains("Another agent"), "{text}");
        assert!(text.contains("local to the files"), "{text}");
    }

    /// A read-only kind is told it is read-only, and a general one is not. The
    /// capability narrowing in `spawn` enforces the same thing, so this is the
    /// half that stops the agent trying and being refused.
    #[test]
    fn a_read_only_kind_is_told_so_and_a_general_one_is_not() {
        assert!(!instruction(Kind::General, "x", "y").contains("read-only agent"));
        assert!(instruction(Kind::Explore, "x", "look around").contains("read-only agent"));
        assert!(instruction(Kind::Plan, "x", "plan it").contains("ordered plan"));
    }

    #[test]
    fn the_kind_is_parsed_from_the_wire_names() {
        assert_eq!(Kind::parse(Some("explore")), Kind::Explore);
        assert_eq!(Kind::parse(Some("plan")), Kind::Plan);
        assert_eq!(Kind::parse(Some("general")), Kind::General);
        // An unknown or absent type lets the agent work, matching the tool's own
        // default rather than failing the call.
        assert_eq!(Kind::parse(None), Kind::General);
        assert_eq!(Kind::parse(Some("whatever")), Kind::General);
    }

    /// Only the general kind may change the workspace, which is what makes the
    /// access narrowing in `spawn` safe to key off this.
    #[test]
    fn only_the_general_kind_may_change_things() {
        assert!(Kind::General.may_change());
        assert!(!Kind::Explore.may_change());
        assert!(!Kind::Plan.may_change());
    }
}
