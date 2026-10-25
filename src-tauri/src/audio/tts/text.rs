//! What is worth hearing, out of a reply written to be read.
//!
//! A reply is mostly prose with code in it. Read aloud, a fenced block becomes a
//! minute of punctuation names, so the default is to leave it out -- and the
//! switch exists for the cases where the code *is* the point. The same shape of
//! question covers markdown itself: `**` and `#` are writing instructions, not
//! words, and a synthesis engine reads them literally -- so with `skip_markdown`
//! on the voice speaks what the reader *sees*.

/// The text to synthesize.
///
/// `smart` is the master switch: off, the reply is read as written -- markdown
/// and all -- and the two skip flags are not consulted. On, code and markup are
/// removed when their switches ask, and the separators left behind by a removed
/// span are tidied away too.
pub fn for_speech(text: &str, smart: bool, skip_code: bool, skip_markdown: bool) -> String {
    if !smart {
        return collapse(text);
    }
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
        // The gap markers the inline strip leaves are only useful here.
        tidy_gaps(&kept)
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
///
/// `index` is a **byte** offset, because it slices `line` as well as walking it.
/// It used to be a character index over `chars`, which is the same number only
/// while the line is ASCII -- an em dash or an emoji made the slice start inside
/// a character and panicked the whole app mid-reply.
fn without_markdown_spans(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    let mut index = 0;
    while index < line.len() {
        let rest = &line[index..];
        // A link keeps its label and drops its target: the URL is the part a
        // voice would spell with punctuation.
        if rest.starts_with('[') {
            if let Some(open_end) = rest.find("](") {
                let label = &rest[1..open_end];
                let close = rest[open_end + 2..].find(')').map(|c| open_end + 2 + c);
                if let Some(close) = close {
                    out.push_str(label);
                    index += close + 1;
                    continue;
                }
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
            index = line.len();
            continue;
        }
        // Every arm below moves by the character's own byte width, so the walk
        // can never land inside a multi-byte character.
        let character = rest
            .chars()
            .next()
            .expect("index is inside the string, so a character follows");
        // A table column separator is not a word either.
        if character == '|' {
            index += character.len_utf8();
            continue;
        }
        // Emphasis markers collapse to what they marked -- but only *around*
        // words. `snake_case` and `a*b*c` are words of code, and eating `_` or
        // `*` from inside one changes the name, which a voice cannot know it
        // was not meant to say.
        if matches!(character, '*' | '_' | '~') {
            let previous_word = line[..index]
                .chars()
                .next_back()
                .is_some_and(char::is_alphanumeric);
            let next_word = rest[character.len_utf8()..]
                .chars()
                .next()
                .is_some_and(char::is_alphanumeric);
            if !(previous_word && next_word) {
                index += character.len_utf8();
                continue;
            }
        }
        out.push(character);
        index += character.len_utf8();
    }
    out
}

/// Marks where an inline code span was removed.
///
/// A private marker rather than nothing at all, because what is left around the
/// hole matters: `` `a` / `b` `` should not be read as "slash". `tidy_gaps`
/// consumes the markers and the separators that only existed to join them.
const GAP: char = '\u{0}';

/// Drops backtick spans, keeping the prose on either side of them.
fn without_inline_code(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    let mut in_code = false;
    for character in line.chars() {
        if character == '`' {
            if !in_code {
                out.push(GAP);
            }
            in_code = !in_code;
            continue;
        }
        if !in_code {
            out.push(character);
        }
    }
    out
}

/// Removes the gap markers and the punctuation left stranded beside them.
///
/// The cases this exists for are all one shape: a reply naming two identifiers
/// or a value in code, where the prose between them is a slash or a dash.
/// `` `firered-stream-vad-f32` / `firered-vad-f32` — *streaming* `` is read
/// "firered ... slash firered ... streaming" without this. A separator that
/// touches a removed span is decoration, not a word -- while a comma the author
/// wrote (`Call `tts_speak`, now.`) is kept, because that comma is real prose.
fn tidy_gaps(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut characters = text.chars().peekable();
    while let Some(character) = characters.next() {
        if character != GAP {
            out.push(character);
            continue;
        }
        // Drop trailing whitespace, and a separator that sat just before the gap.
        let trimmed = out.trim_end();
        let without_separator = trimmed
            .trim_end_matches(['/', '-', '—', '–', '|'])
            .trim_end();
        let keep = if without_separator.len() < trimmed.len() {
            without_separator.len()
        } else {
            trimmed.len()
        };
        out.truncate(keep);

        // Skip whitespace and a separator just after the gap.
        while characters.peek().is_some_and(|next| next.is_whitespace()) {
            characters.next();
        }
        if characters
            .peek()
            .is_some_and(|next| matches!(*next, '/' | '-' | '—' | '–' | '|'))
        {
            characters.next();
            while characters.peek().is_some_and(|next| next.is_whitespace()) {
                characters.next();
            }
        }
        // A gap between two words becomes a space; one before punctuation does not.
        let before_punctuation = characters
            .peek()
            .is_some_and(|next| matches!(*next, ',' | '.' | ';' | ':' | '!' | '?' | ')' | ']'));
        if !before_punctuation && !out.is_empty() && !out.ends_with([' ', '\n']) {
            out.push(' ');
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
        assert_eq!(for_speech(text, true, true, false), "Before. After.");
    }

    #[test]
    fn inline_code_is_left_out() {
        assert_eq!(
            for_speech("Call `tts_speak` now.", true, true, false),
            "Call now."
        );
    }

    /// Smart speak off reads the reply as written, markup and all.
    ///
    /// The two skip switches are its sub-options, so they must not act while the
    /// master switch is off -- that is the difference between "leave my text
    /// alone" and "clean it the way I configured".
    #[test]
    fn smart_speak_off_reads_the_text_as_written() {
        let text = "**bold** and `code` stay.";
        assert_eq!(for_speech(text, false, true, true), text);
    }

    #[test]
    fn the_switch_keeps_everything() {
        let text = "Call `tts_speak` now.";
        assert_eq!(
            for_speech(text, true, false, false),
            "Call `tts_speak` now."
        );
    }

    #[test]
    fn a_reply_that_is_only_code_has_nothing_to_say() {
        // The caller reports this rather than sending an empty script to Piper.
        assert!(for_speech("```\ncode\n```", true, true, false).is_empty());
    }

    #[test]
    fn paragraphs_do_not_fuse_into_one_word() {
        assert_eq!(for_speech("One.\n\nTwo.", true, true, false), "One. Two.");
    }

    #[test]
    fn markdown_symbols_leave_the_marked_words_alone() {
        assert_eq!(
            for_speech("## Head\n**bold** and *light* done.", true, false, true),
            "Head bold and light done."
        );
    }

    #[test]
    fn a_link_is_its_label_not_its_target() {
        assert_eq!(
            for_speech(
                "Read [the manual](https://example.com/docs.md).",
                true,
                false,
                true
            ),
            "Read the manual."
        );
    }

    #[test]
    fn a_bullet_and_an_order_marker_are_structure() {
        assert_eq!(
            for_speech("- first\n2. second", true, false, true),
            "first second"
        );
    }

    #[test]
    fn code_words_inside_prose_survive_the_markdown_strip() {
        // `snake_case` is a name someone wrote; eating its `_` changes the name.
        assert_eq!(
            for_speech("the `tts_speak` update", true, true, true),
            "the update"
        );
        assert_eq!(
            for_speech("a snake_case_name stays whole.", true, false, true),
            "a snake_case_name stays whole."
        );
    }

    #[test]
    fn a_table_rule_is_not_a_dashed_word() {
        assert_eq!(
            for_speech(
                "Before.\n| a | b |\n|---|---|\n| 1 | 2 |",
                true,
                false,
                true
            ),
            "Before. a b 1 2"
        );
    }

    /// A separator that joined two removed code spans is not a word.
    ///
    /// This is the reply that read "slash streaming ..." aloud: two identifiers
    /// around a slash and an em dash, where only the prose after them was meant
    /// to be heard.
    #[test]
    fn a_separator_between_removed_spans_is_dropped() {
        assert_eq!(
            for_speech(
                "`firered-stream-vad-f32` / `firered-vad-f32` — *streaming* vs. whole-file variants.",
                true,
                true,
                true
            ),
            "streaming vs. whole-file variants."
        );
    }

    /// A comma the author wrote is prose, not a stranded separator.
    #[test]
    fn punctuation_the_author_wrote_survives_a_removed_span() {
        assert_eq!(
            for_speech("Call `tts_speak`, then wait.", true, true, true),
            "Call, then wait."
        );
    }

    /// A multi-byte character is one step, not one byte.
    ///
    /// The walk used to index a `Vec<char>` and then slice the original string
    /// with that number, so the first non-ASCII character in a reply -- an em
    /// dash is enough -- started the slice inside it and panicked the app while
    /// it was reading the answer aloud.
    #[test]
    fn a_multibyte_character_does_not_split_the_walk() {
        assert_eq!(
            for_speech("Wait — then **go** now.", true, false, true),
            "Wait — then go now."
        );
        assert_eq!(
            for_speech("Speed 🙂 vs **bold** — done.", true, false, true),
            "Speed 🙂 vs bold — done."
        );
        // A link after a dash exercises both the byte walk and the label slice.
        assert_eq!(
            for_speech(
                "See — [the manual](https://example.com).",
                true,
                false,
                true
            ),
            "See — the manual."
        );
    }
}
