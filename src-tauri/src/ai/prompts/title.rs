/// Instructions used when asking a model to name a conversation.
/// Kept separate from `chat` prompts because this is a short, single-shot
/// classification task rather than a conversational system prompt.
pub fn system_prompt() -> &'static str {
    "You write short titles for chat conversations."
}

/// Builds the user message asking for a title. `prompt` is the user's opening
/// message, already truncated by the caller. Only the request is summarized —
/// the assistant has not replied yet when this runs.
pub fn user_prompt(prompt: &str) -> String {
    format!(
        "Write a title for a conversation that starts with this request.\n\n\
         Rules:\n\
         - Between 3 and 6 words.\n\
         - Describe the topic, not the phrasing of the request.\n\
         - No surrounding quotes, no trailing punctuation.\n\
         - Reply with the title only.\n\n\
         Request:\n{prompt}"
    )
}

/// Fewest words a title may have. The prompt asks for at least this many, but a
/// model that answers with one or two words has produced something that reads as
/// a label rather than a description of the conversation, so it is rejected and
/// the next attempt runs.
const MIN_TITLE_WORDS: usize = 3;

/// Normalizes raw model output into a usable title: strips quotes and
/// markdown decoration, collapses whitespace, and enforces the length and word
/// count. Returns `None` when nothing usable is left.
pub fn sanitize_title(raw: &str) -> Option<String> {
    let cleaned = raw
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .unwrap_or_default()
        .trim_matches(|character: char| {
            matches!(character, '"' | '\'' | '`' | '*' | '#' | '>' | '|')
        })
        .trim()
        .trim_end_matches('.')
        .trim();
    if cleaned.is_empty() {
        return None;
    }
    // Cap on characters rather than bytes so multi-byte titles stay valid.
    let capped: String = cleaned.chars().take(60).collect();
    if capped.split_whitespace().count() < MIN_TITLE_WORDS {
        return None;
    }
    Some(capped)
}

#[cfg(test)]
mod tests {
    use super::{sanitize_title, user_prompt};

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
        assert!(prompt.contains("Between 3 and 6 words"));
    }

    /// A one or two word answer is a label, not a description of the
    /// conversation, so it is treated as unusable and the next attempt runs.
    #[test]
    fn rejects_a_title_with_too_few_words() {
        assert_eq!(sanitize_title("Rust"), None);
        assert_eq!(sanitize_title("Async patterns"), None);
        assert_eq!(sanitize_title("  Fixing   the  "), None);
        // Three words is the boundary and is accepted.
        assert_eq!(
            sanitize_title("Fixing the layout"),
            Some("Fixing the layout".into())
        );
    }

    /// Word count is measured after decoration is stripped, so markup does not
    /// pad a short title past the limit.
    #[test]
    fn counts_words_after_stripping_decoration() {
        assert_eq!(sanitize_title("**Rust async**"), None);
    }
}
