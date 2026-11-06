/// Instructions used when asking a model to name a conversation.
/// Kept separate from `chat` prompts because this is a short, single-shot
/// classification task rather than a conversational system prompt.
pub fn system_prompt() -> &'static str {
    "You name chat conversations. You answer with a title and nothing else."
}

/// Builds the user message asking for a title. `prompt` is the user's opening
/// message, already truncated by the caller. Only the request is summarized —
/// the assistant has not replied yet when this runs.
///
/// **A worked example and a trailing `Title:` cue, deliberately.** Small local
/// models copy the shape of what they are shown far more reliably than they
/// follow prose rules, and the bare cue stops them prefacing the answer with
/// "Sure, here is a title" -- which the sanitiser would otherwise have to guess
/// its way through.
pub fn user_prompt(prompt: &str) -> String {
    format!(
        "Write a title for a conversation that starts with the request below.\n\n\
         Rules:\n\
         - 3 to 6 words.\n\
         - Describe the topic, not the wording of the request.\n\
         - Plain text only: no quotes, no punctuation, no \"Title:\" prefix.\n\
         - Answer with the title only.\n\n\
         Example:\n\
         Request: How do I center a div in CSS?\n\
         Title: Centering a div with CSS\n\n\
         Request:\n{prompt}\n\n\
         Title:"
    )
}

/// The answer a model gave, collapsed to one bounded line for a log.
///
/// A rejected title is only diagnosable with the text in hand: an empty string, a
/// one-word label and a paragraph of prose all fail the same check but need
/// different fixes. Bounded so a model that ignores the task cannot flood the
/// console with a whole essay.
pub fn preview(raw: &str) -> String {
    let collapsed = raw.split_whitespace().collect::<Vec<_>>().join(" ");
    if collapsed.is_empty() {
        return "<empty>".to_string();
    }
    collapsed.chars().take(200).collect()
}

/// Fewest words a title may have.
///
/// Two rather than three: a small model often answers `Async patterns` or
/// `Fixing layout`, and rejecting those cost the whole feature for anyone who
/// picked a 1B model to name their chats. A single word is still refused -- that
/// is a label, not a description.
const MIN_TITLE_WORDS: usize = 2;

/// Normalizes raw model output into a usable title.
///
/// Strips a leading `Title:` label, quotes and markdown decoration, then enforces
/// the length and word count. Returns `None` when nothing usable is left.
pub fn sanitize_title(raw: &str) -> Option<String> {
    // The first non-empty line, because a model that rambles before answering
    // still tends to put the title first.
    let line = raw.lines().map(str::trim).find(|line| !line.is_empty())?;
    let labelled = without_label(line);
    let cleaned = labelled
        .trim_matches(|character: char| {
            matches!(character, '"' | '\'' | '`' | '*' | '#' | '>' | '|')
        })
        .trim()
        .trim_end_matches('.')
        .trim();
    // Collapse runs of whitespace: a model that double-spaces its answer should
    // still produce a title that reads normally, and `Fixing   the` is the same
    // title as `Fixing the`.
    let collapsed = cleaned.split_whitespace().collect::<Vec<_>>().join(" ");
    if collapsed.is_empty() {
        return None;
    }
    // Cap on characters rather than bytes so multi-byte titles stay valid.
    let capped: String = collapsed.chars().take(60).collect();
    if capped.split_whitespace().count() < MIN_TITLE_WORDS {
        return None;
    }
    Some(capped)
}

/// The text after a leading label such as `Title:` or `Here is a title:`.
///
/// The prompt already asks for no prefix, and the trailing cue makes one
/// unlikely, but a model that saw the cue echoed can repeat it. Removing it here
/// means a good answer is not thrown away for a label in front of it. Leading
/// markdown (`**`, `#`, `>`) is skipped first so the decorated form is caught too.
fn without_label(line: &str) -> String {
    let trimmed = line.trim_start_matches(|character: char| {
        character.is_whitespace() || matches!(character, '*' | '#' | '>' | '`')
    });
    let lower = trimmed.to_ascii_lowercase();
    let after = if lower.starts_with("here is a title") {
        &trimmed["here is a title".len()..]
    } else if lower.starts_with("here's a title") {
        &trimmed["here's a title".len()..]
    } else if lower.starts_with("title")
        // The next character must not continue the word, or `Titles are hard`
        // would lose its first five letters.
        && trimmed
            .as_bytes()
            .get(5)
            .map(|byte| !byte.is_ascii_alphanumeric())
            .unwrap_or(true)
    {
        &trimmed[5..]
    } else {
        return trimmed.to_string();
    };
    after
        .trim_start_matches(|character: char| {
            character.is_whitespace() || matches!(character, ':' | '-' | '*' | '`')
        })
        .trim()
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::{preview, sanitize_title, user_prompt};

    #[test]
    fn keeps_a_plain_title() {
        assert_eq!(
            sanitize_title("Rust async patterns"),
            Some("Rust async patterns".into())
        );
    }

    #[test]
    fn strips_quotes_and_markdown_decoration() {
        assert_eq!(
            sanitize_title("**Rust async patterns**"),
            Some("Rust async patterns".into())
        );
        assert_eq!(
            sanitize_title("\"Rust async patterns\""),
            Some("Rust async patterns".into())
        );
        assert_eq!(
            sanitize_title("## Rust async patterns"),
            Some("Rust async patterns".into())
        );
    }

    #[test]
    fn trims_trailing_punctuation_and_whitespace() {
        assert_eq!(
            sanitize_title("  Rust async patterns.  "),
            Some("Rust async patterns".into())
        );
    }

    #[test]
    fn uses_the_first_non_empty_line() {
        assert_eq!(
            sanitize_title("\n\nRust async patterns\n\nSure, here you go!"),
            Some("Rust async patterns".into())
        );
    }

    #[test]
    fn rejects_output_with_nothing_usable() {
        assert_eq!(sanitize_title(""), None);
        assert_eq!(sanitize_title("   \n  \n"), None);
        assert_eq!(sanitize_title("**\n\n```"), None);
    }

    #[test]
    fn caps_long_titles_without_breaking_multibyte_text() {
        // Three short words, each long enough that the 60-character cap lands
        // inside the third one. The cap counts characters rather than bytes, so
        // a multi-byte character is never split in half.
        let long = ["é".repeat(25), "ü".repeat(25), "ö".repeat(25)].join(" ");
        let capped = sanitize_title(&long).expect("title should survive capping");
        assert_eq!(capped.chars().count(), 60);
        assert!(capped.starts_with(&"é".repeat(25)));
        assert!(capped.contains('ü'));
        assert!(capped
            .chars()
            .all(|character| matches!(character, 'é' | 'ü' | 'ö' | ' ')));
    }

    #[test]
    fn prompt_carries_only_the_users_request() {
        let prompt = user_prompt("How do I center a div?");
        assert!(prompt.contains("How do I center a div?"));
        // The assistant has not replied when this runs, so the prompt must not
        // imply a transcript or ask about a conversation already in progress.
        assert!(!prompt.contains("Conversation:"));
        assert!(prompt.contains("Request:"));
        assert!(prompt.contains("3 to 6 words"));
    }

    /// A single word is a label, not a description of the conversation, so it is
    /// treated as unusable and the next attempt runs. Two words are accepted:
    /// small models answer that way often, and rejecting them was worse than
    /// keeping a terse title.
    #[test]
    fn rejects_a_title_with_too_few_words() {
        assert_eq!(sanitize_title("Rust"), None);
        assert_eq!(
            sanitize_title("  Fixing   the  "),
            Some("Fixing the".into())
        );
        assert_eq!(
            sanitize_title("Async patterns"),
            Some("Async patterns".into())
        );
    }

    /// Word count is measured after decoration is stripped, so markup does not
    /// pad a short title past the limit.
    #[test]
    fn counts_words_after_stripping_decoration() {
        assert_eq!(sanitize_title("**Rust**"), None);
    }

    /// A rejected answer is logged as one bounded line, so an empty completion
    /// can be told from a rambling one in the console.
    #[test]
    fn a_rejected_answer_is_logged_as_one_bounded_line() {
        assert_eq!(preview(""), "<empty>");
        assert_eq!(preview("  \n\t "), "<empty>");
        assert_eq!(preview("Sure!  Here\nis a title"), "Sure! Here is a title");
        assert_eq!(preview(&"word ".repeat(100)).chars().count(), 200);
    }

    /// A model that echoes the cue's label is not throwing away a good title.
    #[test]
    fn strips_a_title_label() {
        assert_eq!(
            sanitize_title("Title: Fixing the layout"),
            Some("Fixing the layout".into())
        );
        assert_eq!(
            sanitize_title("**Title:** Rust async patterns"),
            Some("Rust async patterns".into())
        );
        assert_eq!(
            sanitize_title("Here is a title: Fixing the layout"),
            Some("Fixing the layout".into())
        );
        // A word that merely starts with "title" keeps its letters.
        assert_eq!(
            sanitize_title("Titles are hard"),
            Some("Titles are hard".into())
        );
    }
}
