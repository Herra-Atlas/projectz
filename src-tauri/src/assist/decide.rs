//! The requests: which blocks are worth helping with, and what the help is.
//!
//! # Why one call decides and answers -- and why the top level makes two
//!
//! A separate classifier in front of the answer would be a second model call per look, and
//! -- because the app runs one local model at a time -- a second local model would evict the
//! one the user is chatting with. Asking a single model to *return an empty list when nothing
//! is warranted* costs one request in the case that fires and a handful of tokens in the case
//! that does not, and it has the answers already in hand when it decides yes.
//!
//! The top level is the exception, and deliberately: [`locate`] asks only for a label per
//! block, then [`detail`] asks for the help on a narrow slice. That is two requests instead of
//! one, but each is a task a small model can actually do -- pointing at short text, then
//! writing about a handful of blocks -- rather than one long task it fails at.
//!
//! # Why the blocks are numbered and the answer is a list
//!
//! A page is not one question. A quiz page has five, and one request that answers all of them
//! costs what one answer costs -- while five requests would cost five times as much. So the
//! page is sent numbered and the answer comes back numbered, and the number is what puts each
//! card beside its own block.
//!
//! # Why the reply is treated as untrusted
//!
//! Models wrap JSON in a code fence, preface it with "Here is", answer with a bare sentence
//! when the instruction was missed, or invent a block number that was never sent. All of that
//! is normal, and only the last one is dangerous -- an invented number would draw an answer
//! over the wrong paragraph -- so an answer naming a block that was not sent is dropped rather
//! than placed.

use serde_json::Value;

use super::candidate::Candidate;
use super::debug;
use super::offer::Kind;
use super::settings::AssistSettings;
#[cfg(test)]
use super::settings::{MAX_ANSWERS, MAX_CANDIDATES};
use crate::ai::remote::client::{ReportedUsage, Thinking};

/// Longest answer that reaches the screen.
///
/// Over the longest kind's word limit, because a model that overshoots by a sentence should
/// be cut rather than thrown away -- but well under a wall of text, since this is drawn on
/// top of something the user was already reading.
///
/// Not a level's number: this is how much prose the card can hold, which is a fact about the
/// card rather than about what the request may cost.
const ANSWER_CAP: usize = 800;

/// One block's answer.
#[derive(Debug, Clone, PartialEq)]
pub struct Answer {
    /// The number of the block this answers, as the request numbered it.
    pub index: usize,
    /// What kind of help this is. Defaults to `answer` when the model names none.
    pub kind: Kind,
    /// The answer, cleaned. Never empty.
    pub text: String,
    /// One lowercase word naming the subject, or `other`.
    pub subject: String,
    /// How sure the model is, clamped to 0-1.
    pub confidence: f32,
}

/// What the model decided about one page.
#[derive(Debug, Clone, PartialEq)]
pub struct Verdict {
    /// The blocks it chose to help with, in the order it gave them.
    pub answers: Vec<Answer>,
    /// Its own words for helping with nothing. Empty when it answered something.
    pub reason: String,
}

impl Verdict {
    /// A decision to help with nothing.
    pub fn declined(reason: impl Into<String>) -> Self {
        Self {
            answers: Vec::new(),
            reason: reason.into(),
        }
    }
}

/// One block the locate pass chose, and the kind it chose.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Choice {
    /// The block's place in the request, 1-based.
    pub index: usize,
    pub kind: Kind,
}

/// Tries allowed at one decision.
///
/// A model that answers with a bare sentence instead of the object it was asked for has
/// usually just had a bad sample -- the same request a moment later follows the contract. One
/// retry covers that without giving a model that cannot produce the shape at all an unbounded
/// budget. The transport's own retry is separate: this is spent on replies that *arrived* and
/// could not be read.
///
/// **The second attempt is not always the same request.** A reply cut off at the token ceiling
/// is not a bad sample: it is a request that does not fit, and repeating it unchanged is
/// repeating something already shown not to work. So that case -- and only that case -- is
/// retried with the model's thinking switched off.
const ATTEMPTS: u32 = 2;

/// What the salvage attempt tells the model, in words every provider reads.
///
/// The portable half of switching thinking off: `chat_template_kwargs` is llama.cpp's, and a
/// provider that has never heard of it still has this sentence. It is stated as an instruction
/// rather than a plea because the rambling that filled the budget was the model reading the
/// rules back to itself, and a small model does what the last line it read says.
const NO_THINKING_NOTE: &str =
    "\n\nAnswer now, without deliberating: no reasoning, no restating of \
     these rules, no explanation. Reply with the JSON object and nothing else.";

/// The reason to report when a reply came back and could not be used.
///
/// A reply cut off at the ceiling is named as that rather than as a model that answered badly:
/// the text it produced was *thinking*, not an answer, so the parser's complaint about a
/// sentence that was never finished says nothing the user can act on. What they can act on is
/// the shape of the model -- this budget is sized for one that thinks out loud, and one that
/// reasons for longer than it will always stop here.
fn failure_reason(error: &str, truncated: bool, limit: u32) -> String {
    if !truncated {
        return error.to_string();
    }
    format!(
        "The model used all {limit} tokens without finishing. That budget fits a model that \
         thinks out loud; one that reasons for longer than it, or one with no reasoning \
         channel, is what changes this."
    )
}

/// How the thinking switch was set for one attempt, in the dump's own words.
fn thinking_note(thinking: Thinking, salvage: bool) -> &'static str {
    match (thinking, salvage) {
        (Thinking::Suppress, true) => "suppressed - the salvage attempt after a truncated reply",
        (Thinking::Suppress, false) => "suppressed - thinking is off for this request",
        (Thinking::Allow, _) => "allowed",
    }
}

/// Whether the setting allows the first attempt to think.
fn preferred_thinking(settings: &AssistSettings) -> Thinking {
    if settings.thinking {
        Thinking::Allow
    } else {
        Thinking::Suppress
    }
}

/// Runs one structured request, retrying a reply that could not be read.
///
/// `parse` is what decides whether the reply is usable, so the retry logic lives here once
/// rather than beside each caller's own idea of a valid reply. Both a decision and a list of
/// labels are read through this same path.
async fn complete_json<T>(
    endpoint: &crate::ai::remote::types::Endpoint,
    model: &str,
    base_prompt: String,
    user_prompt: &str,
    max_tokens: u32,
    preferred: Thinking,
    settings: &AssistSettings,
    parse: impl Fn(&str) -> Result<T, String>,
) -> Result<(T, ReportedUsage), String> {
    let mut last_error = "The model did not reply with a decision".to_string();
    let mut spent = ReportedUsage::default();
    // Whether the reply *before* this attempt was cut off at the ceiling.
    let mut truncated = false;
    for attempt in 1..=ATTEMPTS {
        // Whether *this* attempt is the salvage one: decided by the reply that came before it.
        let salvage = truncated;
        let thinking = if salvage || matches!(preferred, Thinking::Suppress) {
            Thinking::Suppress
        } else {
            Thinking::Allow
        };
        // Both switches, because which one a server has is not knowable from here: the field
        // the client sends is llama.cpp's, and this sentence is the one every other provider
        // reads.
        let system_prompt = match thinking {
            Thinking::Allow => base_prompt.clone(),
            Thinking::Suppress => format!("{base_prompt}{NO_THINKING_NOTE}"),
        };
        let messages = [
            serde_json::json!({ "role": "system", "content": &system_prompt }),
            serde_json::json!({ "role": "user", "content": user_prompt }),
        ];
        let (raw, usage, payload) = crate::ai::remote::client::complete_once_raw(
            endpoint,
            model,
            &messages,
            Some(max_tokens),
            thinking,
        )
        .await?;
        spent = spent.plus(usage);
        truncated = crate::ai::remote::client::finish_reason(&payload) == Some("length");
        match parse(&raw) {
            Ok(value) => return Ok((value, spent)),
            Err(error) => {
                tracing::info!(
                    attempt,
                    truncated,
                    suppressing_thinking =
                        (truncated || matches!(preferred, Thinking::Suppress)) && attempt < ATTEMPTS,
                    %error,
                    "the assist reply could not be read; asking again"
                );
                // **Temporary.** `assist::debug` says where the dump goes and why; delete it,
                // this call and the module's declaration together.
                if let Some(path) = debug::write(&debug::Failure {
                    attempt,
                    why: &error,
                    thinking: thinking_note(thinking, salvage),
                    finish_reason: crate::ai::remote::client::finish_reason(&payload),
                    endpoint,
                    model,
                    settings,
                    system_prompt: &system_prompt,
                    user_prompt,
                    reply: &raw,
                    payload: &payload,
                    usage: &usage,
                }) {
                    tracing::warn!(path = %path.display(), "assist wrote a debug dump");
                }
                last_error = failure_reason(&error, truncated, max_tokens);
                // **A retry that would be the same request is not made.** With thinking already
                // off, a reply cut off at the ceiling is deterministic -- same page, same budget,
                // same model -- and the second request would spend the whole budget again to
                // learn what the first one already said.
                if truncated && matches!(thinking, Thinking::Suppress) {
                    break;
                }
            }
        }
    }
    Err(last_error)
}

/// Asks the model about one page's short list, in one request.
///
/// Returns what the model decided *and what the request cost*, because a retry means the
/// figures belong to the whole exchange rather than to one attempt.
pub async fn ask(
    endpoint: &crate::ai::remote::types::Endpoint,
    model: &str,
    candidates: &[Candidate],
    settings: &AssistSettings,
    title: &str,
) -> Result<(Verdict, ReportedUsage), String> {
    let system_prompt = crate::ai::prompts::assist::system_prompt(
        settings.max_answers(),
        settings.brief_prompt(),
        settings.allowed_kinds(),
        // The one-pass path is never the "detail" variant: the detail note belongs to
        // `detail_system_prompt`, which is a different request.
        false,
    );
    let user_prompt = crate::ai::prompts::assist::user_prompt(
        candidates,
        &settings.hint,
        &settings.subjects,
        title,
    );
    // The valid numbers are the list's own 1..n, not the page's indices.
    let allowed: Vec<usize> = (1..=candidates.len()).collect();
    let cap = settings.max_answers();
    let (verdict, usage) = complete_json(
        endpoint,
        model,
        system_prompt,
        &user_prompt,
        settings.max_tokens(),
        preferred_thinking(settings),
        settings,
        |raw| parse_verdict(raw, &allowed, cap),
    )
    .await?;
    Ok((finish(verdict, settings), usage))
}

/// The top level's first pass: which blocks are worth helping with, and how.
///
/// Returns an empty list when the model found nothing -- which is a normal, successful
/// answer. An unreadable reply is an error, so the caller can fall back to the single-pass
/// pipeline rather than going silent because the locator had a bad sample.
pub async fn locate(
    endpoint: &crate::ai::remote::types::Endpoint,
    model: &str,
    candidates: &[Candidate],
    settings: &AssistSettings,
    title: &str,
) -> Result<(Vec<Choice>, ReportedUsage), String> {
    let kinds = settings.allowed_kinds();
    if kinds.len() <= 1 {
        // Nothing to choose between: the only allowed kind is what every candidate gets.
        return Ok((Vec::new(), ReportedUsage::default()));
    }
    // The locator may pick more than one reply can carry, because the detail pass is a
    // sequence of requests; it may not pick the whole page.
    let max_labels = settings.max_answers().saturating_mul(2).max(1);
    let system_prompt = crate::ai::prompts::assist::locate_system_prompt(kinds, max_labels);
    let user_prompt = crate::ai::prompts::assist::locate_user_prompt(candidates, title);
    let allowed: Vec<usize> = (1..=candidates.len()).collect();
    let (choices, usage) = complete_json(
        endpoint,
        model,
        system_prompt,
        &user_prompt,
        // Labels are short. A bigger budget is only room for a thinking model to wander.
        500,
        Thinking::Suppress,
        settings,
        |raw| parse_labels(raw, &allowed, kinds),
    )
    .await?;
    Ok((choices, usage))
}

/// The top level's second pass: write the help for a small, already-chosen slice.
pub async fn detail(
    endpoint: &crate::ai::remote::types::Endpoint,
    model: &str,
    candidates: &[Candidate],
    kinds: &[Kind],
    settings: &AssistSettings,
    title: &str,
) -> Result<(Verdict, ReportedUsage), String> {
    let system_prompt = crate::ai::prompts::assist::detail_system_prompt(settings.allowed_kinds());
    let user_prompt = crate::ai::prompts::assist::detail_user_prompt(candidates, kinds, title);
    let allowed: Vec<usize> = (1..=candidates.len()).collect();
    let cap = settings.max_answers();
    let (mut verdict, usage) = complete_json(
        endpoint,
        model,
        system_prompt,
        &user_prompt,
        settings.max_tokens(),
        preferred_thinking(settings),
        settings,
        |raw| parse_verdict(raw, &allowed, cap),
    )
    .await?;
    // **The kind was decided by the locate pass.** A kind the detail reply invents is not
    // allowed to change what the card is called, or a summary could come back as an answer.
    for answer in &mut verdict.answers {
        if let Some(kind) = answer
            .index
            .checked_sub(1)
            .and_then(|position| kinds.get(position))
        {
            answer.kind = *kind;
        }
    }
    Ok((finish(verdict, settings), usage))
}

/// Applies the level's allowed kinds and the confidence floor.
///
/// A kind the level does not allow is *coerced* to `answer` rather than dropped: the content
/// is still what the reader asked for, and a 2B may label an explanation as an answer without
/// being wrong about the substance. Confidence is different -- it is the one number the
/// feature uses to decide whether to speak at all, so an answer under the floor is dropped.
fn finish(mut verdict: Verdict, settings: &AssistSettings) -> Verdict {
    let allowed = settings.allowed_kinds();
    for answer in &mut verdict.answers {
        if !allowed.contains(&answer.kind) {
            answer.kind = Kind::Answer;
        }
    }
    let floor = settings.min_confidence();
    let before = verdict.answers.len();
    verdict.answers.retain(|answer| answer.confidence >= floor);
    if before > 0 && verdict.answers.is_empty() {
        verdict.reason = format!(
            "every answer was below the {}% confidence floor",
            (floor * 100.0).round()
        );
    }
    verdict
}

/// Reads a decision out of a model's reply.
///
/// `cap` is how many blocks the reply may answer, which is the level's number. `allowed` is
/// the set of addresses the reply may name -- at the call site, `1..=n` for the list that was
/// actually sent -- so a number outside it is one the model invented.
///
/// Returns `Err` only when nothing usable is there. An empty list is a *success* with no
/// answers, because answering nothing is the normal, expected outcome.
pub fn parse_verdict(raw: &str, allowed: &[usize], cap: usize) -> Result<Verdict, String> {
    let parsed =
        read_decision(raw).ok_or_else(|| "The model did not reply with a decision".to_string())?;

    let entries: Vec<&Value> = match parsed.get("answers").and_then(Value::as_array) {
        Some(answers) => answers.iter().collect(),
        // A single answer without the list around it.
        None => parsed
            .get("answer")
            .filter(|answer| answer.is_object())
            .map(|answer| vec![answer])
            .unwrap_or_default(),
    };

    let mut answers: Vec<Answer> = Vec::new();
    let mut misplaced = 0usize;
    let mut unaddressed = 0usize;
    for entry in entries {
        if answers.len() >= cap {
            break;
        }
        let Some(index) = entry
            .get("block")
            .and_then(Value::as_u64)
            .map(|it| it as usize)
        else {
            unaddressed += 1;
            continue;
        };
        // A number that was not sent is a number the app cannot place. Dropped rather than
        // clamped, which would put an answer over the wrong paragraph.
        if !allowed.contains(&index) {
            misplaced += 1;
            continue;
        }
        let text = entry
            .get("answer")
            .and_then(Value::as_str)
            .map(sanitize_answer)
            .unwrap_or_default();
        if text.is_empty() {
            continue;
        }
        // A block answered twice in one reply gets one card, the first one.
        if answers.iter().any(|answer| answer.index == index) {
            continue;
        }
        answers.push(Answer {
            index,
            // A missing label is an answer, because that is what the brief prompt asks for
            // and what every level allows.
            kind: entry
                .get("kind")
                .and_then(Value::as_str)
                .and_then(Kind::parse)
                .unwrap_or(Kind::Answer),
            text,
            subject: sanitize_subject(
                entry
                    .get("subject")
                    .and_then(Value::as_str)
                    .unwrap_or_default(),
            ),
            confidence: entry
                .get("confidence")
                .and_then(Value::as_f64)
                .unwrap_or(0.0)
                .clamp(0.0, 1.0) as f32,
        });
    }

    if answers.is_empty() {
        if misplaced > 0 {
            return Err(format!(
                "The model answered {misplaced} block{} that was not in the list",
                if misplaced == 1 { "" } else { "s" }
            ));
        }
        if unaddressed > 0 {
            return Err(format!(
                "The model answered {} without saying which block it was about, so nothing \
                 could be placed",
                if unaddressed == 1 {
                    "once".to_string()
                } else {
                    format!("{unaddressed} times")
                }
            ));
        }
        return Ok(Verdict::declined(
            parsed
                .get("reason")
                .and_then(Value::as_str)
                .map(sanitize_reason)
                .unwrap_or_default(),
        ));
    }
    Ok(Verdict {
        answers,
        reason: String::new(),
    })
}

/// Reads the locate pass's labels.
///
/// A label outside the allowed kinds, an unknown number, or a `none` is simply absent from
/// the result. An empty list is a success: the model decided the page needed nothing.
pub fn parse_labels(raw: &str, allowed: &[usize], kinds: &[Kind]) -> Result<Vec<Choice>, String> {
    let parsed =
        read_decision(raw).ok_or_else(|| "The model did not reply with labels".to_string())?;
    let Some(entries) = parsed.get("labels").and_then(Value::as_array) else {
        // A locator that answered with something else has not given a usable decision, and
        // the caller should fall back rather than read it as "nothing here".
        return Err("The model did not reply with a list of labels".to_string());
    };
    let mut choices: Vec<Choice> = Vec::new();
    for entry in entries {
        let Some(index) = entry
            .get("block")
            .and_then(Value::as_u64)
            .map(|it| it as usize)
        else {
            continue;
        };
        if !allowed.contains(&index) {
            continue;
        }
        let Some(kind) = entry
            .get("kind")
            .and_then(Value::as_str)
            .and_then(Kind::parse)
        else {
            continue;
        };
        if !kinds.contains(&kind) {
            continue;
        }
        if choices.iter().any(|choice| choice.index == index) {
            continue;
        }
        choices.push(Choice { index, kind });
    }
    Ok(choices)
}

/// The model's decision, dug out of whatever it wrapped it in.
///
/// **The first JSON *value*, not the first brace to the last one.** Models repeat themselves,
/// and taking everything between the outermost braces joins two objects into text that is not
/// JSON at all. Reading the first value takes that first object and ignores whatever follows.
fn read_decision(raw: &str) -> Option<Value> {
    let trimmed = raw.trim();
    if let Ok(value) = serde_json::from_str::<Value>(trimmed) {
        return Some(value);
    }
    let start = trimmed.find('{')?;
    serde_json::Deserializer::from_str(&trimmed[start..])
        .into_iter::<Value>()
        .next()
        .and_then(Result::ok)
}

/// An answer reduced to what should be drawn.
pub fn sanitize_answer(raw: &str) -> String {
    let mut text = raw.trim().to_string();
    // A fence around the answer, which is the commonest decoration.
    if let Some(rest) = text.strip_prefix("```") {
        let rest = rest.split_once('\n').map(|(_, body)| body).unwrap_or(rest);
        text = rest.trim_end_matches("```").trim().to_string();
    }
    // Surrounding quotes and emphasis.
    text = text
        .trim_matches(|character: char| matches!(character, '"' | '\'' | '`' | '*'))
        .trim()
        .to_string();
    // A leading label the prompt asked it not to write.
    for prefix in ["Answer:", "answer:", "The answer is", "the answer is"] {
        if let Some(rest) = text.strip_prefix(prefix) {
            text = rest.trim_start().to_string();
        }
    }
    // Cap on characters rather than bytes, so a multi-byte answer is never cut in half.
    let capped: String = text.chars().take(ANSWER_CAP).collect();
    capped.trim().to_string()
}

/// A decline's reason, reduced to something that fits on a settings row.
fn sanitize_reason(raw: &str) -> String {
    raw.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .take(80)
        .collect::<String>()
        .trim_end_matches(['.', ','])
        .to_string()
}

/// A subject reduced to the single lowercase word the settings weights use.
fn sanitize_subject(raw: &str) -> String {
    let word: String = raw
        .trim()
        .to_lowercase()
        .chars()
        .take_while(|character| character.is_alphanumeric() || *character == '-')
        .take(24)
        .collect();
    if word.is_empty() {
        "other".to_string()
    } else {
        word
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The block numbers a page might have offered, in the order they were sent.
    fn allowed() -> Vec<usize> {
        vec![2, 3, 7]
    }

    /// Reads a reply as the middle level would.
    fn parse_verdict(raw: &str, allowed: &[usize]) -> Result<Verdict, String> {
        super::parse_verdict(raw, allowed, MAX_ANSWERS)
    }

    /// A plain reply, exactly as the prompt asks for it.
    #[test]
    fn reads_a_numbered_answer() {
        let verdict = parse_verdict(
            r#"{"answers": [{"block": 2, "kind": "answer", "answer": "56", "subject": "math", "confidence": 0.95}]}"#,
            &allowed(),
        )
        .expect("a plain decision must parse");
        assert_eq!(verdict.answers.len(), 1);
        assert_eq!(verdict.answers[0].index, 2);
        assert_eq!(verdict.answers[0].kind, Kind::Answer);
        assert_eq!(verdict.answers[0].text, "56");
        assert_eq!(verdict.answers[0].subject, "math");
        assert!((verdict.answers[0].confidence - 0.95).abs() < f32::EPSILON);
    }

    /// A kind reaches the card, and an unknown one falls back to an answer.
    #[test]
    fn a_kind_is_read_and_an_unknown_one_is_an_answer() {
        let verdict = parse_verdict(
            r#"{"answers": [
                {"block": 2, "kind": "summary", "answer": "A summary", "confidence": 0.9},
                {"block": 3, "kind": "baffling", "answer": "An answer", "confidence": 0.9}
            ]}"#,
            &allowed(),
        )
        .expect("parses");
        assert_eq!(verdict.answers[0].kind, Kind::Summarize);
        assert_eq!(verdict.answers[1].kind, Kind::Answer);
    }

    /// A page with several questions gets several answers, each on its own block.
    #[test]
    fn answers_several_blocks_at_once() {
        let verdict = parse_verdict(
            r#"{"answers": [
                {"block": 7, "answer": "A riddle's answer", "subject": "logic", "confidence": 0.8},
                {"block": 3, "answer": "3.14159", "subject": "math", "confidence": 0.9}
            ]}"#,
            &allowed(),
        )
        .expect("a list must parse");
        assert_eq!(verdict.answers.len(), 2);
        assert_eq!(verdict.answers[0].index, 7);
        assert_eq!(verdict.answers[1].index, 3);
    }

    /// Answering nothing is a normal outcome, not a failure.
    #[test]
    fn an_empty_list_is_a_decline_with_its_reason() {
        let verdict = parse_verdict(
            r#"{"answers": [], "reason": "nothing here asks anything"}"#,
            &allowed(),
        )
        .expect("an empty list is not an error");
        assert!(verdict.answers.is_empty());
        assert_eq!(verdict.reason, "nothing here asks anything");
    }

    /// **The one failure that would put an answer on the wrong paragraph.**
    #[test]
    fn an_answer_to_a_block_that_was_not_sent_is_dropped() {
        let verdict = parse_verdict(
            r#"{"answers": [
                {"block": 2, "answer": "56", "subject": "math", "confidence": 1},
                {"block": 99, "answer": "wrong paragraph", "subject": "math", "confidence": 1}
            ]}"#,
            &allowed(),
        )
        .expect("the usable answer survives");
        assert_eq!(verdict.answers.len(), 1);
        assert_eq!(verdict.answers[0].text, "56");
    }

    #[test]
    fn answers_to_nothing_but_unknown_blocks_are_reported_as_a_failure() {
        let error = parse_verdict(
            r#"{"answers": [{"block": 42, "answer": "56", "subject": "math", "confidence": 1}]}"#,
            &allowed(),
        )
        .expect_err("nothing could be placed");
        assert!(error.contains("was not in the list"), "{error}");
    }

    #[test]
    fn a_reply_with_no_decision_at_all_is_an_error() {
        assert!(parse_verdict("I am not sure what you mean.", &allowed()).is_err());
        assert!(parse_verdict("", &allowed()).is_err());
    }

    #[test]
    fn an_answer_with_no_text_is_skipped() {
        let verdict = parse_verdict(
            r#"{"answers": [
                {"block": 2, "answer": "   ", "subject": "math", "confidence": 1},
                {"block": 3, "answer": "3.14159", "subject": "math", "confidence": 1}
            ]}"#,
            &allowed(),
        )
        .expect("the usable answer survives");
        assert_eq!(verdict.answers.len(), 1);
        assert_eq!(verdict.answers[0].index, 3);
    }

    #[test]
    fn the_same_block_twice_gets_one_answer() {
        let verdict = parse_verdict(
            r#"{"answers": [
                {"block": 2, "answer": "56", "subject": "math", "confidence": 1},
                {"block": 2, "answer": "56, or maybe 55", "subject": "math", "confidence": 1}
            ]}"#,
            &allowed(),
        )
        .expect("the first answer wins");
        assert_eq!(verdict.answers.len(), 1);
        assert_eq!(verdict.answers[0].text, "56");
    }

    #[test]
    fn a_flood_of_answers_is_capped() {
        let blocks = [2usize, 3, 7, 11, 12, 13];
        let reply = format!(
            r#"{{"answers": [{}]}}"#,
            (0..12)
                .map(|position| format!(
                    r#"{{"block": {}, "answer": "answer number {position}", "subject": "math", "confidence": 1}}"#,
                    blocks[position % blocks.len()]
                ))
                .collect::<Vec<_>>()
                .join(",")
        );
        let verdict = parse_verdict(&reply, &blocks).expect("a list must parse");
        assert_eq!(verdict.answers.len(), MAX_ANSWERS);
    }

    #[test]
    fn finds_the_decision_inside_decoration() {
        for reply in [
            "```json\n{\"answers\": [{\"block\": 2, \"answer\": \"56\", \"subject\": \"math\", \"confidence\": 1}]}\n```",
            "Sure, here is my decision:\n{\"answers\": [{\"block\": 2, \"answer\": \"56\", \"subject\": \"math\", \"confidence\": 1}]}",
            "{\"answers\": [{\"block\": 2, \"answer\": \"56\", \"subject\": \"math\", \"confidence\": 1}]}\nLet me know if that helps!",
        ] {
            let verdict = parse_verdict(reply, &allowed()).expect("the decision should be found");
            assert_eq!(verdict.answers.len(), 1, "wrong reading of {reply:?}");
            assert_eq!(verdict.answers[0].text, "56");
        }
    }

    #[test]
    fn a_single_answer_without_the_list_is_read() {
        let verdict = parse_verdict(
            r#"{"answer": {"block": 2, "answer": "56", "subject": "math", "confidence": 1}}"#,
            &allowed(),
        )
        .expect("a single answer must parse");
        assert_eq!(verdict.answers.len(), 1);
        assert_eq!(verdict.answers[0].index, 2);
    }

    #[test]
    fn decoration_is_stripped_from_the_answer() {
        assert_eq!(sanitize_answer("\"56\""), "56");
        assert_eq!(sanitize_answer("  **56**  "), "56");
        assert_eq!(sanitize_answer("```\n56\n```"), "56");
        assert_eq!(sanitize_answer("Answer: 56"), "56");
        assert_eq!(sanitize_answer("The answer is 56"), "56");
    }

    #[test]
    fn a_long_answer_is_capped_without_breaking_a_character() {
        let long = "é".repeat(ANSWER_CAP + 100);
        let capped = sanitize_answer(&long);
        assert_eq!(capped.chars().count(), ANSWER_CAP);
    }

    #[test]
    fn a_missing_confidence_is_zero_and_an_out_of_range_one_is_clamped() {
        assert_eq!(
            parse_verdict(r#"{"answers": [{"block": 2, "answer": "56"}]}"#, &allowed())
                .expect("parses")
                .answers[0]
                .confidence,
            0.0
        );
        assert_eq!(
            parse_verdict(
                r#"{"answers": [{"block": 2, "answer": "56", "confidence": 4}]}"#,
                &allowed()
            )
            .expect("parses")
            .answers[0]
                .confidence,
            1.0
        );
        assert_eq!(
            parse_verdict(
                r#"{"answers": [{"block": 2, "answer": "56", "confidence": -3}]}"#,
                &allowed()
            )
            .expect("parses")
            .answers[0]
                .confidence,
            0.0
        );
    }

    #[test]
    fn a_subject_becomes_one_lowercase_word() {
        assert_eq!(sanitize_subject("Math"), "math");
        assert_eq!(sanitize_subject("  Math problem "), "math");
        assert_eq!(sanitize_subject(""), "other");
        assert_eq!(sanitize_subject("   "), "other");
    }

    #[test]
    fn a_truncated_reply_is_reported_as_running_out_of_room() {
        let named = failure_reason(
            "The model's decision could not be read: EOF while parsing",
            true,
            1_500,
        );
        assert!(named.contains("1500 tokens"), "{named}");
        assert!(named.contains("thinks out loud"), "{named}");
        assert_eq!(
            failure_reason("The model did not reply with a decision", false, 1_500),
            "The model did not reply with a decision"
        );
    }

    #[test]
    fn the_salvage_attempt_tells_the_model_not_to_deliberate() {
        assert!(NO_THINKING_NOTE.contains("without deliberating"));
        assert!(NO_THINKING_NOTE.contains("JSON object"));
    }

    /// **The confidence floor is the level's, and it is enforced rather than requested.**
    #[test]
    fn an_answer_under_the_floor_is_dropped() {
        let settings = AssistSettings {
            min_confidence: 0.8,
            level: crate::assist::settings::Level::Custom,
            ..AssistSettings::default()
        };
        let verdict = parse_verdict(
            r#"{"answers": [
                {"block": 2, "answer": "sure", "confidence": 0.9},
                {"block": 3, "answer": "unsure", "confidence": 0.2}
            ]}"#,
            &allowed(),
        )
        .expect("parses");
        let finished = finish(verdict, &settings);
        assert_eq!(finished.answers.len(), 1);
        assert_eq!(finished.answers[0].text, "sure");

        // And a reply that was entirely under the floor says why, rather than reading as
        // the model declining.
        let all_unsure = parse_verdict(
            r#"{"answers": [{"block": 2, "answer": "unsure", "confidence": 0.1}]}"#,
            &allowed(),
        )
        .expect("parses");
        let finished = finish(all_unsure, &settings);
        assert!(finished.answers.is_empty());
        assert!(
            finished.reason.contains("confidence floor"),
            "{}",
            finished.reason
        );
    }

    /// A kind the level does not allow keeps its content but becomes an answer.
    #[test]
    fn a_disallowed_kind_is_coerced_rather_than_dropped() {
        let settings = AssistSettings {
            level: crate::assist::settings::Level::Efficient,
            ..AssistSettings::default()
        };
        let verdict = parse_verdict(
            r#"{"answers": [{"block": 2, "kind": "summarize", "answer": "x", "confidence": 1}]}"#,
            &allowed(),
        )
        .expect("parses");
        let finished = finish(verdict, &settings);
        assert_eq!(finished.answers[0].kind, Kind::Answer);
    }

    /// **A label is read, and an unknown one is not.** The locate pass may also say `none`
    /// for everything, which is an empty list rather than an error.
    #[test]
    fn labels_are_read_and_unknown_ones_are_skipped() {
        let choices = parse_labels(
            r#"{"labels": [
                {"block": 2, "kind": "answer"},
                {"block": 3, "kind": "none"},
                {"block": 7, "kind": "check"},
                {"block": 99, "kind": "answer"},
                {"block": 2, "kind": "explain"}
            ]}"#,
            &allowed(),
            &Kind::ALL,
        )
        .expect("labels must parse");
        assert_eq!(
            choices,
            vec![
                Choice {
                    index: 2,
                    kind: Kind::Answer
                },
                Choice {
                    index: 7,
                    kind: Kind::Check
                },
            ]
        );

        assert_eq!(
            parse_labels(r#"{"labels": []}"#, &allowed(), &Kind::ALL),
            Ok(Vec::new())
        );
        // A locator that answered with something else is a failure the caller falls back on.
        assert!(parse_labels(r#"{"answers": []}"#, &allowed(), &Kind::ALL).is_err());
    }

    /// **The two caps agree at every level.**
    #[test]
    fn the_two_caps_agree_at_every_level() {
        use crate::assist::settings::Level;
        assert!(MAX_ANSWERS <= MAX_CANDIDATES);
        for level in [Level::Efficient, Level::Normal, Level::High, Level::Custom] {
            let settings = AssistSettings {
                level,
                ..AssistSettings::default()
            }
            .normalized();
            assert!(
                settings.max_answers() <= settings.max_candidates(),
                "{level:?} would let a reply answer more blocks than the look carries"
            );
            assert!(
                settings.candidates_per_request() <= settings.max_candidates(),
                "{level:?} would send more candidates than the look considers"
            );
            assert!(
                settings.max_answers() <= settings.candidates_per_request(),
                "{level:?} would let a reply answer more blocks than its own request carried"
            );
        }
    }

    #[test]
    fn the_cap_a_reply_is_held_to_is_the_one_it_was_given() {
        let reply = r#"{"answers": [
            {"block": 2, "answer": "56", "subject": "math", "confidence": 1},
            {"block": 3, "answer": "3.14159", "subject": "math", "confidence": 1},
            {"block": 7, "answer": "a riddle's answer", "subject": "logic", "confidence": 1}
        ]}"#;
        assert_eq!(
            super::parse_verdict(reply, &allowed(), 2)
                .expect("parses")
                .answers
                .len(),
            2
        );
        assert_eq!(
            super::parse_verdict(reply, &allowed(), MAX_ANSWERS)
                .expect("parses")
                .answers
                .len(),
            3
        );
    }
}
