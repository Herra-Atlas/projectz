//! What is worth hearing, out of a reply written to be read.
//!
//! A reply is mostly prose with code in it. Read aloud, a fenced block becomes a
//! minute of punctuation names, so the default is to leave it out -- and the
//! switch exists for the cases where the code *is* the point. The same shape of
//! question covers markdown itself: `**` and `#` are writing instructions, not
//! words, and a synthesis engine reads them literally -- so with `skip_markdown`
//! on the voice speaks what the reader *sees*.

/// The text to synthesize: code removed when `skip_code` is on, markdown
/// symbols removed when `skip_markdown` is on, and whitespace collapsed either
/// way.
pub fn for_speech(text: &str, skip_code: bool, skip_markdown: bool) -> String {
    let code_free = if skip_code {
        // Fences first, whole lines, because a fence's contents span lines and
        // an inline strip cannot see the line boundary.
        let mut kept = String::with_capacity(text.len());
        let mut in_fence = false;
        for line in text.lines() {
            if line.trim_start().starts_with("```") {
                in_fence = !in_fence;
                continue;
            }
            if in_fence {
                continue;
            }
            kept.push_str(&without_inline_code(line));
            kept.push('\n');
        }
        kept
    } else {
        text.to_string()
    };
    if skip_markdown {
        collapse(&without_markdown(&code_free))
    } else {
        collapse(&code_free)
    }
}

/// Strips markdown *markup*, keeping the words it marked.
///
/// The rules are deliberately line-based, not a Markdown grammar: a
/// regeneration of the common-loudness cases (headings, emphasis, bullets,
/// tables, block quotes, link targets) is what a voice needs, and anything
/// this misses is spoken as written rather than wrong.
fn without_markdown(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for line in text.lines() {
        let line = line.trim_start();
        // `#`, `>`, and `- `/`* ` at line start are structure, not words.
        let body = if let Some(rest) = line.strip_prefix('#') {
            rest.trim_start_matches(['#', ' '])
        } else if let Some(rest) = line.strip_prefix('>') {
            rest.trim_start_matches(['>', ' '])
        } else if let Some(rest) = line.strip_prefix("- ") {
            rest
        } else if let Some(rest) = line.strip_prefix("* ") {
            rest
        } else if let Some((digits, tail)) = line.split_once(". ") {
            // An ordered-list marker -- up to three leading digits and the
            // dot -- is enumeration structure. More than three digits is a
            // number someone wrote, and that stays.
            if digits.len() <= 3 && digits.bytes().all(|b| b.is_ascii_digit()) {
                tail
            } else {
                line
            }
        } else {
            line
        };
        out.push_str(&without_markdown_spans(body));
        out.push('\n');
    }
    out
}

/// Emphasis, links and tables inside one line, removed without touching the
/// words they carry.
fn without_markdown_spans(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    let chars: Vec<char> = line.chars().collect();
    let mut index = 0;
    while index < chars.len() {
        let rest = &line[index..];
        // A link keeps its label and drops its target: the URL is the part a
        // voice would spell with punctuation.
        if let Some(open_end) = rest.find("](").filter(|_| rest.starts_with('[')) {
            let label = &rest[1..open_end];
            let close = rest[open_end + 2..].find(')').map(|c| open_end + 2 + c);
            if let Some(close) = close {
                out.push_str(label);
                index += close + 1;
                continue;
            }
        }
        // An image `![caption](…)` keeps its caption, which is the part a
        // reader would have seen as words.
        if rest.starts_with("![") {
            if let Some(open_end) = rest.find("](") {
                if let Some(close) = rest[open_end + 2..].find(')').map(|c| open_end + 2 + c) {
                    out.push_str(rest[2..open_end].trim());
                    index += close + 1;
                    continue;
                }
            }
        }
        // Table rules are dividers, not dashes to read.
        if rest.starts_with("---")
            && rest
                .chars()
                .all(|c| c == '-' || c == '|' || c == ' ' || c == ':')
        {
            index += rest.len();
            continue;
        }
        // A table column separator is not a word either.
        if chars[index] == '|' {
            index += 1;
            continue;
        }
        // Emphasis markers collapse to what they marked -- but only *around*
        // words. `snake_case` and `a*b*c` are words of code, and eating `_` or
        // `*` from inside one changes the name, which a voice cannot know it
        // was not meant to say.
        if matches!(chars[index], '*' | '_' | '~') {
            let previous_word = index > 0 && chars[index - 1].is_alphanumeric();
            let next_word = index + 1 < chars.len() && chars[index + 1].is_alphanumeric();
            if !(previous_word && next_word) {
                index += 1;
                continue;
            }
        }
        out.push(chars[index]);
        index += 1;
    }
    out
}

/// Drops backtick spans, keeping the prose on either side of them.
fn without_inline_code(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    let mut in_code = false;
    for character in line.chars() {
        if character == '`' {
            in_code = !in_code;
            continue;
        }
        if !in_code {
            out.push(character);
        }
    }
    out
}

/// Every run of whitespace becomes one space. Removing a code span leaves the
/// spaces on either side of where it was, and Piper reads two spaces as a pause
/// no one asked for; a newline is a break either way, so the spacing only has to
/// stop two paragraphs fusing into one word.
fn collapse(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut pending_space = false;
    for character in text.chars() {
        if character.is_whitespace() {
            // Leading whitespace is dropped rather than turned into a space.
            pending_space = !out.is_empty();
            continue;
        }
        if pending_space {
            out.push(' ');
            pending_space = false;
        }
        out.push(character);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_fenced_block_is_left_out() {
        let text = "Before.\n```rust\nfn main() {}\n```\nAfter.";
        assert_eq!(for_speech(text, true, false), "Before. After.");
    }

    #[test]
    fn inline_code_is_left_out() {
        assert_eq!(
            for_speech("Call `tts_speak` now.", true, false),
            "Call now."
        );
    }

    #[test]
    fn the_switch_keeps_everything() {
        let text = "Call `tts_speak` now.";
        assert_eq!(for_speech(text, false, false), "Call `tts_speak` now.");
    }

    #[test]
    fn a_reply_that_is_only_code_has_nothing_to_say() {
        // The caller reports this rather than sending an empty script to Piper.
        assert!(for_speech("```\ncode\n```", true, false).is_empty());
    }

    #[test]
    fn paragraphs_do_not_fuse_into_one_word() {
        assert_eq!(for_speech("One.\n\nTwo.", true, false), "One. Two.");
    }

    #[test]
    fn markdown_symbols_leave_the_marked_words_alone() {
        assert_eq!(
            for_speech("## Head\n**bold** and *light* done.", false, true),
            "Head bold and light done."
        );
    }

    #[test]
    fn a_link_is_its_label_not_its_target() {
        assert_eq!(
            for_speech(
                "Read [the manual](https://example.com/docs.md).",
                false,
                true
            ),
            "Read the manual."
        );
    }

    #[test]
    fn a_bullet_and_an_order_marker_are_structure() {
        assert_eq!(
            for_speech("- first\n2. second", false, true),
            "first second"
        );
    }

    #[test]
    fn code_words_inside_prose_survive_the_markdown_strip() {
        // `snake_case` is a name someone wrote; eating its `_` changes the name.
        assert_eq!(
            for_speech("the `tts_speak` update", true, true),
            "the update"
        );
        assert_eq!(
            for_speech("a snake_case_name stays whole.", false, true),
            "a snake_case_name stays whole."
        );
    }

    #[test]
    fn a_table_rule_is_not_a_dashed_word() {
        assert_eq!(
            for_speech("Before.\n| a | b |\n|---|---|\n| 1 | 2 |", false, true),
            "Before. a b 1 2"
        );
    }
}
