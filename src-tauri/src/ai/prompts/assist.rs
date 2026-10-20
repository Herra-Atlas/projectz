//! The prompts assist uses, and why there are three of them.
//!
//! # The task is no longer only "answer the question"
//!
//! A page holds more than questions: a term worth explaining, a section worth condensing, a
//! snippet worth checking, a paragraph in another language. [`system_prompt`] therefore asks
//! for a **kind of help** beside each answer, and the level decides which kinds are on offer
//! (see `assist::settings::Level`). Keeping the kinds in the prompt rather than inventing a
//! second pipeline is what lets the cheapest level stay exactly as quiet as it was.
//!
//! # Why locate and detail are separate prompts
//!
//! A small model is good at *pointing* and bad at *reading a whole page and deciding*. The
//! top level therefore runs two requests: [`locate_system_prompt`] asks for a label per block
//! and nothing else -- a task a 2B can do over short snippets -- and
//! [`detail_system_prompt`] then asks for the help on one small slice, where the context is
//! narrow enough to hold. The alternative, one request carrying everything, is the shape a
//! small model fails at: long context, many items, a long reply.
//!
//! # Why the reply is JSON, and why it is a list
//!
//! Three facts come back per block -- which block, what the answer is, how sure the model is
//! -- and a prose reply would have to be parsed back into them by guessing. The list is what
//! lets one request serve a whole page.
//!
//! # Why the hint cannot veto
//!
//! [`user_prompt`] carries the user's own note about what they care about. It is written as a
//! preference, never as a rule about what may be answered: a hint that could refuse would be
//! a feature that silently stops working.
//!
//! # Why the brief prompt is still brief
//!
//! The system prompt is the one part of a request paid for on *every* look, and it is the
//! only cost a level can move without touching what counts as a question. The cheap level
//! gets [`brief_prompt`]: the same contract, the same refusals that matter most, and a third
//! of the words. A test fails if it grows back into the long one.

use crate::assist::candidate::Candidate;
use crate::assist::offer::Kind;

/// How much of a block the locate pass sends.
///
/// The locator is deciding *what kind of thing this is*, not answering it, and a small model
/// does that better on a short slice. The detail pass sends the full block afterwards.
pub const LOCATE_CHARS: usize = 400;

/// The system prompt for one level: the full reasoning, or the short one.
///
/// `kinds` is the level's own list, so the instruction and what the parser accepts cannot
/// disagree. `detail` asks for the fuller rendering the top level's second pass uses.
pub fn system_prompt(max_answers: usize, brief: bool, kinds: &[Kind], detail: bool) -> String {
    if brief {
        brief_prompt(max_answers)
    } else {
        full_prompt(max_answers, kinds, detail)
    }
}

/// The system prompt: what the task is, and every reason to offer nothing.
fn full_prompt(max_answers: usize, kinds: &[Kind], detail: bool) -> String {
    let detail_note = if detail {
        "\n\nThis page asked for help with these blocks specifically, so give the fuller \
         version of the kind named beside each one. Still do not restate the block, and still \
         stay inside the word limit."
    } else {
        ""
    };
    format!(
        "You are shown a numbered list of text blocks from a page someone is looking at \
     in their browser right now. They have not asked you anything. Decide which of the \
     blocks, if any, is worth helping with unasked, which of the allowed kinds of help it \
     needs, and then give that help.\n\n\
     Reply with one JSON object and nothing else. No prose before or after it, no \
     code fence:\n\
     {{\"answers\": [{{\"block\": 1, \"kind\": \"answer\", \"answer\": \"<the help>\", \"subject\": \"<one word>\", \"confidence\": <0-1>}}]}}\n\n\
     Answer at most {max_answers} block{plural}, named by the number in their brackets. \
     Those numbers are the only valid ones. Usually it is none, or one.\n\
     Reply {{\"answers\": [], \"reason\": \"<at most eight words>\"}} when nothing on \
     the list is worth helping with. That is the normal answer and a good one -- most of \
     what is on a page is not a question.\n\n\
     {kinds}\n\n\
     Do not offer anything for a block when:\n\
     - it is a menu, a navigation bar, a price, a heading with nothing under it, or something \
     the person is part-way through writing;\n\
     - it asks about their own private life, files, accounts or identity;\n\
     - answering it needs information that is not in front of you -- news, a score, a \
     price, the current date;\n\
     - it already contains its answer;\n\
     - you are not sure of the answer.\n\n\
     Rules for the help itself:\n\
     - Stay inside the word limit for its kind. Shorter is better; someone glancing at their \
     own screen has no patience for a preamble.\n\
     - Give the help, not the method, unless the method is the help.\n\
     - Never restate the block. Never open with \"Sure\", \"The answer is\" or a line of setup.\n\
     - Work out arithmetic before answering. If you are not certain of the result, \
     leave that block out.\n\
     - `subject` is one lowercase word: math, logic, trivia, language, code, science \
     or other.\n\
     - `confidence` is how sure you are that the help is both correct and worth this \
     person's attention, from 0 to 1.\n\n\
     A wrong or pointless card appearing on someone's screen unasked is worse than no card, \
     because they did not ask for it and may not be looking carefully. When in doubt, \
     leave that block out.{detail_note}",
        plural = if max_answers == 1 { "" } else { "s" },
        kinds = kind_instructions(kinds),
    )
}

/// The short prompt, for the level that trades thoroughness for cost.
///
/// Answers only, because that is the one kind the cheap level offers. The `kind` field is
/// absent from the example on purpose: the parser defaults to `answer`, and a label nobody
/// needs is tokens paid for on every look.
fn brief_prompt(max_answers: usize) -> String {
    format!(
        "You are shown numbered text blocks from a page someone is looking at in their \
     browser, and they have not asked you anything. Answer a block only if it asks a question \
     and you are sure of the answer from what is in front of you.\n\n\
     Reply with one JSON object and nothing else:\n\
     {{\"answers\": [{{\"block\": 1, \"answer\": \"<the answer>\", \"subject\": \"<one word>\", \"confidence\": <0-1>}}]}}\n\n\
     At most {max_answers} block{plural}, named by the number in their brackets -- those \
     are the only valid ones. Nothing being worth answering is the usual answer: \
     reply {{\"answers\": []}}.\n\
     Leave out a block that is a statement, a heading, a menu, something the person is writing, \
     a question about their own private life or files, anything already containing its answer, \
     and anything you are not sure of -- a wrong answer nobody asked for is worse than no \
     answer.\n\
     Each answer: at most 60 words, the answer only, no preamble, no restating the question.",
        plural = if max_answers == 1 { "" } else { "s" }
    )
}

/// The label-only prompt for the top level's first pass.
///
/// It asks for one decision per block and no content at all, because that is the part a small
/// model is good at: short snippets in, a fixed vocabulary out.
pub fn locate_system_prompt(kinds: &[Kind], max_labels: usize) -> String {
    format!(
        "You are shown a numbered list of text blocks from a page someone is reading in \
     their browser. They have not asked you anything.\n\n\
     For each block, decide whether helping with it would add something the person cannot \
     already see, and which kind of help it is. Do not write the help yet -- only the label.\n\n\
     Reply with one JSON object and nothing else:\n\
     {{\"labels\": [{{\"block\": 1, \"kind\": \"answer\"}}]}}\n\n\
     The only valid kinds are:\n\
     {kinds}\n\n\
     Use \"none\" for every block that is not worth helping with. That is the normal answer \
     and a good one -- most of what is on a page is not a question.\n\
     Label at most {max_labels} block{plural}.\n\n\
     Never label a menu, a navigation bar, a price, a heading with nothing under it, something \
     the person is part-way through writing, a question about their private life or files, or \
     a block that already contains its answer. Never label something you could not answer \
     from the text in front of you.",
        kinds = kind_instructions(kinds),
        plural = if max_labels == 1 { "" } else { "s" },
    )
}

/// The prompt for the top level's second pass.
///
/// The kind is already decided and is named beside each block, so this request only has to
/// write the help -- with a narrow context instead of a whole page.
pub fn detail_system_prompt(kinds: &[Kind]) -> String {
    format!(
        "You are shown a few numbered blocks from a page someone is reading. The kind of help \
     wanted is named beside each block. Write only that help.\n\n\
     Reply with one JSON object and nothing else:\n\
     {{\"answers\": [{{\"block\": 1, \"answer\": \"<the help>\", \"subject\": \"<one word>\", \"confidence\": <0-1>}}]}}\n\n\
     Use only the numbers given. Stay inside the word limit for the kind:\n\
     {kinds}\n\n\
     Give the help itself, not the method, unless the method is the help. Never restate the \
     block and never open with a preamble. If you are not sure, leave that block out: a wrong \
     card nobody asked for is worse than no card.",
        kinds = kind_instructions(kinds),
    )
}

/// The allowed kinds, each with its one-line instruction.
fn kind_instructions(kinds: &[Kind]) -> String {
    let mut text = String::from("Allowed kinds of help:");
    for kind in kinds {
        text.push_str(&format!(
            "\n- {}: {} At most {} words.",
            kind.as_str(),
            definition(*kind),
            kind.max_words()
        ));
    }
    text
}

fn definition(kind: Kind) -> &'static str {
    match kind {
        Kind::Answer => "the block asks a question and the help is the answer.",
        Kind::Explain => {
            "the block states an idea, term or passage whose meaning is not obvious; explain it."
        }
        Kind::Summarize => "the block is a long section worth condensing.",
        Kind::Translate => {
            "the block is in a language other than the page's; translate it into English."
        }
        Kind::Check => {
            "the block is code, a calculation or a claim that can be checked; say whether it is \
             correct and why."
        }
    }
}

/// Builds the user message: the numbered blocks, and what this user cares about.
///
/// **Numbered from one, in the order they are sent -- not by the page's own index.** The page's
/// indices are sparse, because the gate drops everything that is not a candidate: a request can
/// carry exactly one block and call it `[17]`, while the reply example in the system prompt says
/// `block: 1`. A small model with one candidate and no other number in sight copied the example's
/// `1`. A dense list makes the example's number a real one, and the caller maps the answer back to
/// the block it belongs to.
pub fn user_prompt(
    candidates: &[Candidate],
    hint: &str,
    subjects: &[String],
    title: &str,
) -> String {
    let mut prompt = page_line(title);
    prompt.push_str("\n\nThe blocks on the page now:\n\n");
    for (position, candidate) in candidates.iter().enumerate() {
        prompt.push_str(&line_for(position + 1, candidate));
    }
    // Both extras are omitted entirely when empty rather than sent as an empty field:
    // a section announcing "no preferences" is noise the model has to read past on
    // every request, and it costs tokens on every page turn.
    if !subjects.is_empty() {
        prompt.push_str("\nThis person would rather be helped with these, in order: ");
        prompt.push_str(&subjects.join(", "));
        prompt.push_str(
            ".\nThat is a preference about what is worth interrupting for. It never \
             excuses a wrong answer, and a block outside the list can still be worth \
             helping with.",
        );
    }
    if !hint.trim().is_empty() {
        prompt.push_str("\n\nThey also said this about what to help with:\n");
        prompt.push_str(hint.trim());
    }
    prompt
}

/// The same list, cut for the locate pass and with no answer requested.
pub fn locate_user_prompt(candidates: &[Candidate], title: &str) -> String {
    let mut prompt = page_line(title);
    prompt.push_str("\n\nThe blocks now. Label each one, or none:\n\n");
    for (position, candidate) in candidates.iter().enumerate() {
        let text: String = candidate.text.chars().take(LOCATE_CHARS).collect();
        prompt.push_str(&format!(
            "[{}] {}{}\n",
            position + 1,
            hint_tag(candidate),
            text
        ));
    }
    prompt
}

/// The same list for the detail pass, with the chosen kind printed beside each block.
pub fn detail_user_prompt(candidates: &[Candidate], kinds: &[Kind], title: &str) -> String {
    let mut prompt = page_line(title);
    prompt.push_str("\n\nThe blocks, with the kind of help wanted beside each:\n\n");
    for (position, (candidate, kind)) in candidates.iter().zip(kinds.iter()).enumerate() {
        prompt.push_str(&format!(
            "[{}] ({}) {}{}\n",
            position + 1,
            kind.as_str(),
            hint_tag(candidate),
            candidate.text
        ));
    }
    prompt
}

/// One line of context, which a small model uses more than a large one does.
fn page_line(title: &str) -> String {
    let title = title.trim();
    if title.is_empty() {
        String::from("A page in a browser.")
    } else {
        format!(
            "The page is titled \"{}\".",
            title.chars().take(120).collect::<String>()
        )
    }
}

/// `(code) ` when the source knew something structural, otherwise nothing.
fn hint_tag(candidate: &Candidate) -> String {
    let hint = candidate.hint.trim();
    if hint.is_empty() {
        String::new()
    } else {
        format!("({hint}) ")
    }
}

fn line_for(number: usize, candidate: &Candidate) -> String {
    format!("[{number}] {}{}\n", hint_tag(candidate), candidate.text)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::assist::candidate::Shape;

    fn candidate(index: usize, text: &str) -> Candidate {
        Candidate {
            index,
            key: format!("k{index}"),
            tag: "p".to_string(),
            hint: String::new(),
            text: text.to_string(),
            shape: Shape::Question,
            distance: 0,
        }
    }

    /// The contract the parser is written against is stated in the prompt, both ways
    /// round. Checked for both lengths, because the two are chosen by a level.
    #[test]
    fn the_prompt_states_the_reply_it_expects() {
        for prompt in [
            system_prompt(4, false, &Kind::ALL, false),
            system_prompt(2, true, &[Kind::Answer], false),
        ] {
            assert!(prompt.contains("\"answers\""), "{prompt}");
            assert!(prompt.contains("\"block\""), "{prompt}");
            assert!(prompt.contains("\"confidence\""), "{prompt}");
            // Answering nothing is the outcome the feature depends on being available.
            assert!(prompt.contains("\"answers\": []"), "{prompt}");
            // And the rule that makes a wrong answer expensive.
            assert!(prompt.contains("worse than no"), "{prompt}");
        }
    }

    /// **A kind is only offered if the level allows it.** The prompt and the parser read the
    /// same slice, so a level cannot ask for a summary it would then throw away.
    #[test]
    fn only_the_allowed_kinds_are_named() {
        let answers = system_prompt(4, false, &[Kind::Answer], false);
        assert!(answers.contains("answer:"), "{answers}");
        assert!(!answers.contains("summarize:"), "{answers}");

        let all = system_prompt(4, false, &Kind::ALL, false);
        for kind in Kind::ALL {
            assert!(all.contains(&format!("- {}:", kind.as_str())), "{all}");
        }
    }

    /// The cap in the prompt is the cap the parser enforces.
    #[test]
    fn every_prompt_names_the_cap_it_was_given() {
        assert!(system_prompt(4, false, &Kind::ALL, false).contains("at most 4 blocks"));
        assert!(system_prompt(2, true, &[Kind::Answer], false).contains("At most 2 blocks"));
        assert!(system_prompt(1, true, &[Kind::Answer], false).contains("At most 1 block,"));
        assert!(locate_system_prompt(&Kind::ALL, 8).contains("at most 8 blocks"));
    }

    /// **The short prompt stays short.** It is the one fixed cost the cheap level moves.
    #[test]
    fn the_brief_prompt_is_a_fraction_of_the_full_one() {
        let full = system_prompt(4, false, &Kind::ALL, false);
        let brief = system_prompt(4, true, &[Kind::Answer], false);
        assert!(
            brief.len() * 2 < full.len(),
            "the brief prompt is {} characters against the full one's {}",
            brief.len(),
            full.len()
        );
    }

    /// The list is numbered from one, densely, and a structural hint rides beside the text.
    #[test]
    fn the_blocks_are_numbered_from_one_and_carry_their_hint() {
        let mut code = candidate(2, "print(x)");
        code.hint = "code".to_string();
        let prompt = user_prompt(&[candidate(2, "The value of PI?"), code], "", &[], "A quiz");
        assert!(prompt.contains("[1] The value of PI?"), "{prompt}");
        assert!(prompt.contains("[2] (code) print(x)"), "{prompt}");
        assert!(!prompt.contains("[7]"), "{prompt}");
    }

    /// The locate pass is dense too, and cut shorter than a detail request.
    #[test]
    fn the_locate_list_is_dense_and_cut() {
        let long = "word ".repeat(200);
        let prompt = locate_user_prompt(&[candidate(9, &long)], "A page");
        assert!(prompt.contains("[1] word"), "{prompt}");
        assert!(!prompt.contains("[9]"), "{prompt}");
        assert!(prompt.len() < 1_000, "the locate slice must stay short");
        assert!(prompt.contains("The page is titled \"A page\""));
    }

    /// The detail pass names the kind beside each block, which is what makes the second
    /// request a different job from the first.
    #[test]
    fn the_detail_prompt_names_each_kind() {
        let prompt = detail_user_prompt(
            &[candidate(1, "Some long section"), candidate(2, "x = 1")],
            &[Kind::Summarize, Kind::Check],
            "A page",
        );
        assert!(
            prompt.contains("[1] (summarize) Some long section"),
            "{prompt}"
        );
        assert!(prompt.contains("[2] (check) x = 1"), "{prompt}");
    }

    /// Both extras are absent when empty.
    #[test]
    fn empty_preferences_are_left_out_rather_than_announced() {
        let prompt = user_prompt(&[candidate(1, "The value of PI?")], "", &[], "");
        assert!(!prompt.contains("would rather be helped"));
        assert!(!prompt.contains("They also said"));
    }

    /// A hint is carried as a preference and can never read as permission to refuse.
    #[test]
    fn a_hint_is_a_preference_and_never_a_veto() {
        let prompt = user_prompt(
            &[candidate(1, "The value of PI?")],
            "let me know about science",
            &["math".to_string(), "science".to_string()],
            "A page",
        );
        assert!(prompt.contains("math, science"));
        assert!(prompt.contains("let me know about science"));
        assert!(prompt.contains("never excuses a wrong answer"));
        assert!(prompt.contains("can still be worth"));
    }
}
