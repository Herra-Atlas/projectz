//! What is worth hearing, out of a reply written to be read.
//!
//! A reply is mostly prose with code in it. Read aloud, a fenced block becomes a
//! minute of punctuation names, so the default is to leave it out -- and the
//! switch exists for the cases where the code *is* the point.

/// The text to synthesize: code removed when `skip_code` is on, and whitespace
/// collapsed either way.
pub fn for_speech(text: &str, skip_code: bool) -> String {
    if !skip_code {
        return collapse(text);
    }

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
    collapse(&kept)
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
        assert_eq!(for_speech(text, true), "Before. After.");
    }

    #[test]
    fn inline_code_is_left_out() {
        assert_eq!(
            for_speech("Call `tts_speak` now.", true),
            "Call now."
        );
    }

    #[test]
    fn the_switch_keeps_everything() {
        let text = "Call `tts_speak` now.";
        assert_eq!(for_speech(text, false), "Call `tts_speak` now.");
    }

    #[test]
    fn a_reply_that_is_only_code_has_nothing_to_say() {
        // The caller reports this rather than sending an empty script to Piper.
        assert!(for_speech("```\ncode\n```", true).is_empty());
    }

    #[test]
    fn paragraphs_do_not_fuse_into_one_word() {
        assert_eq!(for_speech("One.\n\nTwo.", true), "One. Two.");
    }
}
