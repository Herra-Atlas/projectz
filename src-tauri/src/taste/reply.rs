//! Reading one learning reply.
//!
//! # The smallest contract a model can actually keep
//!
//! One fact per line, `FACT | CONFIDENCE`, or the single word `NONE`. There is no
//! JSON envelope, no operation vocabulary, no ids and no report field: every one
//! of those was a token the model had to spend and a shape it could get wrong, and
//! a small local model got them wrong often enough that the retry ladder -- the
//! slowest part of a run -- was the common path.
//!
//! # Tolerant about decoration, strict about the separator
//!
//! Models add bullets, numbering, code fences, a "Here are the facts:" preamble
//! and a trailing summary. Those are stripped or ignored. A line without a
//! confidence marker is **not** accepted as a fact: otherwise a paragraph of prose
//! becomes a profile of sentences, which is exactly the failure this contract
//! avoids. If nothing usable arrives, the reply is an error and the caller retries
//! once with a reminder rather than storing nonsense.
//!
//! # The reply is untrusted
//!
//! The transcript is data, and so is the reply. Nothing here interprets a line as
//! an instruction; the merge applies the same validation a manually typed fact
//! goes through.

/// Most facts one reply may contribute. Kept equal to the prompt's own ceiling:
/// a model that ignores it is not allowed to turn one batch into a profile
/// rewrite, and the output budget already bounds the text.
const MAX_FACTS: usize = 8;

/// One fact read from a reply.
#[derive(Clone, Debug, PartialEq)]
pub struct RawFact {
    pub text: String,
    pub confidence: f32,
}

/// The reply, as read.
#[derive(Debug, Default)]
pub struct RawReply {
    /// Facts that could be read. A run may learn nothing.
    pub facts: Vec<RawFact>,
    /// Whether the model said there was nothing to learn.
    pub declined: bool,
    /// Lines that looked like an answer but could not be read. Counted rather than
    /// fatal: one bad line should not cost the facts beside it.
    pub ignored: usize,
}

/// Reads one reply.
///
/// `Err` means the reply contained no fact and did not say `NONE`, which is the one
/// case the caller retries. A readable `NONE` is a successful empty answer.
pub fn parse_reply(raw: &str) -> Result<RawReply, String> {
    let mut reply = RawReply::default();

    for line in raw.lines() {
        let line = strip_decoration(line);
        if line.is_empty() {
            continue;
        }
        if is_decline(&line) {
            reply.declined = true;
            continue;
        }
        if reply.facts.len() >= MAX_FACTS {
            reply.ignored += 1;
            continue;
        }
        match parse_line(&line) {
            Some(fact) => reply.facts.push(fact),
            None => reply.ignored += 1,
        }
    }

    if reply.facts.is_empty() && !reply.declined {
        return Err("the reply had no `FACT | CONFIDENCE` lines".to_string());
    }
    Ok(reply)
}

/// One line, or `None` when it carries no confidence marker.
fn parse_line(line: &str) -> Option<RawFact> {
    // A markdown table row ends in a pipe. Without this the last split is empty
    // and a correct fact inside a table is thrown away.
    let line = line.trim().trim_end_matches('|').trim_end();
    // The common shape. `rsplit_once` because a fact may contain a pipe of its
    // own ("Uses awk | sed pipelines") and the confidence is always last.
    if let Some((text, tail)) = line.rsplit_once('|') {
        let text = strip_decoration(text);
        // Two words, not one: a schema header (`FACT | CONFIDENCE`) is otherwise
        // read as a fact called `FACT` when its right-hand side is not a number.
        if text.split_whitespace().count() < 2 {
            return None;
        }
        let confidence = parse_confidence(tail)?;
        return Some(RawFact { text, confidence });
    }

    // Models also write `Fact (0.8)`, `Fact - 80%` or `Fact 0.8`. The last token
    // is the confidence; the rest is the fact.
    let (text, tail) = line.rsplit_once(char::is_whitespace)?;
    // **Only with an explicit marker.** Without this, a correct fact that ends in
    // a number is read as `("Uses Windows", 0.11)` -- the model's "11" taken as a
    // confidence and the fact silently truncated or dropped.
    if !explicit_marker(tail) {
        return None;
    }
    let confidence = parse_confidence(tail)?;
    let text = strip_decoration(text);
    if text.split_whitespace().count() < 2 {
        return None;
    }
    Some(RawFact { text, confidence })
}

/// Whether a bare last token is unmistakably a confidence rather than the end of
/// a fact: `80%`, `(0.8)`, `0.8`, `- 0.8` or `: 0.8`. A bare `11` is not.
fn explicit_marker(tail: &str) -> bool {
    let tail = tail.trim();
    tail.ends_with('%')
        || tail.contains('.')
        || tail
            .chars()
            .next()
            .is_some_and(|first| matches!(first, '(' | '[' | '{' | '-' | ':' | '—'))
}

/// A confidence from `0.9`, `90%`, `90` or a parenthesised form of any of them.
fn parse_confidence(raw: &str) -> Option<f32> {
    let text = raw
        .trim()
        .trim_matches(|character: char| {
            matches!(
                character,
                '(' | ')' | '[' | ']' | '{' | '}' | ';' | ',' | ':' | '|'
            )
        })
        .trim();
    let percent = text.ends_with('%');
    let text = text.strip_suffix('%').unwrap_or(text).trim();
    let text = text.trim_end_matches('.');
    if text.is_empty() {
        return None;
    }
    let value: f32 = text.replace(',', ".").parse().ok()?;
    let normalized = if percent {
        value / 100.0
    } else if value > 1.0 {
        // A whole number up to 100 is a percentage; `1.5` is not a confidence and
        // must not become 1.5%.
        if value <= 100.0 && value.fract() == 0.0 {
            value / 100.0
        } else {
            return None;
        }
    } else {
        value
    };
    (0.0..=1.0).contains(&normalized).then_some(normalized)
}

/// Removes a bullet, numbering, quote marks and a code fence from a line.
fn strip_decoration(line: &str) -> String {
    let mut text = line.trim();
    text = text.trim_start_matches(|character: char| {
        matches!(
            character,
            '-' | '*' | '•' | '–' | '—' | '`' | '"' | '\'' | '>' | '+' | '|'
        )
    });
    // `1.` / `1)` / `1:` before a fact.
    let digits = text.chars().take_while(char::is_ascii_digit).count();
    if digits > 0 {
        let rest = &text[digits..];
        if rest.starts_with('.') || rest.starts_with(')') || rest.starts_with(':') {
            text = rest[1..].trim_start();
        }
    }
    // A separator left over from `Fact - 80%` belongs to the marker, not the fact.
    text.trim_end_matches(|character: char| matches!(character, '-' | '–' | '—' | ':' | ';' | ','))
        .trim()
        .to_string()
}

/// Whether a line says there is nothing to learn.
///
/// Kept narrow on purpose: "Nothing annoys them more than slow tools" is a
/// dislike, not a decline, and it starts with the same word.
fn is_decline(line: &str) -> bool {
    let text = line.trim_matches('.').trim().to_lowercase();
    matches!(
        text.as_str(),
        "none" | "nothing" | "no facts" | "no new facts" | "nothing new" | "none found"
    ) || text.starts_with("no new facts")
        || text.starts_with("nothing new")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_one_fact_per_line() {
        let reply = parse_reply(
            "Uses Rust (programming language) for backend services. | 0.9\n\
             Prefers dark themes in editors. | 80%",
        )
        .expect("reply");
        assert_eq!(reply.facts.len(), 2);
        assert_eq!(reply.facts[0].confidence, 0.9);
        assert!((reply.facts[1].confidence - 0.8).abs() < 0.001);
    }

    #[test]
    fn reads_through_bullets_numbering_and_a_fence() {
        let reply = parse_reply(
            "Here are the facts:\n```\n- 1. Uses Rust (programming language). | 0.9\n\
             2) Dislikes loud notifications | 0.7\n```",
        )
        .expect("reply");
        assert_eq!(reply.facts.len(), 2);
        assert_eq!(reply.facts[0].text, "Uses Rust (programming language).");
        assert_eq!(reply.facts[1].text, "Dislikes loud notifications");
        // The preamble is one ignored line, not a fact and not a failure.
        assert_eq!(reply.ignored, 1);
    }

    #[test]
    fn reads_a_trailing_confidence_without_a_pipe() {
        let reply =
            parse_reply("Prefers concise answers. (0.8)\nUses Windows 11. - 90%").expect("reply");
        assert_eq!(reply.facts.len(), 2);
        assert_eq!(reply.facts[1].text, "Uses Windows 11.");
        assert!((reply.facts[1].confidence - 0.9).abs() < 0.001);
    }

    #[test]
    fn none_is_a_successful_empty_answer() {
        let reply = parse_reply("NONE").expect("reply");
        assert!(reply.facts.is_empty());
        assert!(reply.declined);
    }

    #[test]
    fn a_decline_is_not_mistaken_for_a_fact() {
        let reply = parse_reply("Nothing new.").expect("reply");
        assert!(reply.declined);
        assert!(reply.facts.is_empty());
    }

    /// Prose without a marker is ignored. Accepting it would turn every explanation
    /// into a list of sentences about the user.
    #[test]
    fn prose_without_a_confidence_is_not_a_fact() {
        assert!(parse_reply("The user seems to prefer short answers.").is_err());
        let mixed =
            parse_reply("The user seems to prefer short answers.\nPrefers short answers. | 0.8")
                .expect("reply");
        assert_eq!(mixed.facts.len(), 1);
        assert_eq!(mixed.ignored, 1);
    }

    /// A model that repeats the schema header must not turn it into a fact.
    #[test]
    fn the_instruction_header_is_ignored() {
        let reply = parse_reply("FACT | CONFIDENCE\nUses Rust (programming language). | 0.9")
            .expect("reply");
        assert_eq!(reply.facts.len(), 1);
        assert_eq!(reply.ignored, 1);
    }

    #[test]
    fn the_number_of_facts_is_capped() {
        let lines: Vec<String> = (0..20)
            .map(|index| format!("Fact number {index} says something durable. | 0.8"))
            .collect();
        let reply = parse_reply(&lines.join("\n")).expect("reply");
        assert_eq!(reply.facts.len(), MAX_FACTS);
        assert_eq!(reply.ignored, 20 - MAX_FACTS);
    }

    /// A marker that is not a number is not a confidence. Storing the fact with a
    /// guessed one would make the profile claim more than the reply did.
    #[test]
    fn an_unreadable_confidence_marker_drops_the_line() {
        assert!(parse_reply("Uses Rust (programming language). | high").is_err());
    }

    /// A correct fact ending in a number must not become a truncated fact with a
    /// fabricated confidence: "Uses Windows 11" is not 11 percent of anything.
    #[test]
    fn a_fact_ending_in_a_number_is_not_read_as_a_confidence() {
        assert!(parse_reply("Uses Windows 11").is_err());
        assert!(parse_reply("Uses Windows 70").is_err());
        assert!(parse_reply("Uses Rust (programming language). | 1.5").is_err());
    }

    #[test]
    fn a_markdown_table_row_is_read() {
        let reply = parse_reply("| Uses Rust (programming language). | 0.9 |").expect("reply");
        assert_eq!(reply.facts.len(), 1);
        assert_eq!(reply.facts[0].text, "Uses Rust (programming language).");
    }
}
