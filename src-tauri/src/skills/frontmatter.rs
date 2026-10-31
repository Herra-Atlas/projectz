//! Reading and writing the small YAML header a `SKILL.md` carries.
//!
//! # Why not a YAML crate
//!
//! The header is two fields. `name` and `description` are the whole contract
//! between a skill and the model: the folder supplies `name`, the description is
//! the trigger, and everything else a skill has to say is the body. A full YAML
//! parser would be a new dependency for a format the app itself writes, and the
//! one field that can contain punctuation -- the description -- is quoted on the
//! way out and unquoted on the way in. Unknown keys, including the `metadata`
//! block older versions wrote, are ignored rather than rejected, so a skill
//! written for another host still loads here and a skill written by an older
//! build of this app keeps working.
//!
//! # Where a skill's title lives
//!
//! The folder name is the title. `code-review/` is drawn as `Code review`, and
//! the folder is also the skill's address -- so a title that disagreed with it
//! would be a second, weaker name for the same thing. Removing the `title` and
//! `type` fields made the folder the single answer to both questions.

use std::path::Path;

/// The fields this app understands.
#[derive(Clone, Debug, Default)]
pub struct Frontmatter {
    /// The spec's identifier: lowercase letters, digits and hyphens.
    pub name: String,
    /// When to use the skill. The only text the model chooses from.
    pub description: String,
}

/// Parses a `SKILL.md`: the header, then the body exactly as written.
///
/// The identity comes from the folder, so the parsed `name` is only used by
/// callers that want what the file *claims*. [`parse`] leaves it empty when the
/// header omits it, and the folder fills it in.
///
/// A file with no header is accepted as a body-only skill, because a plain
/// markdown note dropped into the folder is a reasonable thing for a person to
/// do; the folder name supplies the identity. A header that opens but never
/// closes is a mistake worth reporting, because silently treating half a file as
/// instructions would hide it.
pub fn parse(text: &str) -> Result<(Frontmatter, String), String> {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let mut lines = text.lines();
    let Some(first) = lines.next() else {
        return Err("The skill is empty".to_string());
    };
    if first.trim() != "---" {
        return Ok((Frontmatter::default(), text.to_string()));
    }

    let mut header = Frontmatter::default();
    let mut closed = false;
    let mut body: Vec<&str> = Vec::new();

    // Lines are collected rather than sliced by byte offset. A file edited on
    // Windows is CRLF and `lines()` drops the `\r`, so an offset built from line
    // lengths would land one byte short per line and cut the body mid-word.
    for line in lines {
        if line.trim() == "---" {
            closed = true;
            continue;
        }
        if closed {
            body.push(line);
        } else {
            parse_line(line, &mut header);
        }
    }
    if !closed {
        return Err("The skill header was not closed with `---`".to_string());
    }

    Ok((header, body.join("\n").trim_start_matches('\n').to_string()))
}

/// Reads one header line into the struct.
///
/// Top-level `name` and `description` are read; an indented line belongs to a
/// nested block (`metadata:`, which is now ignored) and is skipped without
/// ending the top-level block, so a legacy header still yields its description
/// even when it sits above the block that is being ignored.
fn parse_line(line: &str, header: &mut Frontmatter) {
    let trimmed = line.trim();
    if trimmed.is_empty() || trimmed.starts_with('#') {
        return;
    }
    let indent = line.len() - line.trim_start().len();
    if indent > 0 {
        return;
    }
    let Some((key, value)) = trimmed.split_once(':') else {
        return;
    };
    let value = unquote(value.trim());
    match key.trim() {
        "name" => header.name = value,
        "description" => header.description = value,
        _ => {}
    }
}

/// Renders the header and body in the shape [`parse`] reads.
///
/// The name is written because the Agent Skills spec requires it and the install
/// path validates against it; the description is quoted because it is the one
/// field that routinely contains a colon or a quote.
pub fn render(name: &str, description: &str, body: &str) -> String {
    let mut out = String::from("---\n");
    out.push_str(&format!("name: {name}\n"));
    if !description.trim().is_empty() {
        out.push_str(&format!("description: {}\n", quote(description)));
    }
    out.push_str("---\n\n");
    out.push_str(body.trim());
    if !out.ends_with('\n') {
        out.push('\n');
    }
    out
}

/// Rewrites the `description:` field alone, leaving the rest of the file alone.
///
/// The counterpart to [`render`], which rebuilds the whole file from two fields
/// and is therefore only right for a call that says it replaces the text. Saving
/// one field through a rebuild would discard every key this parser does not
/// understand, every comment, and the author's own spacing -- material that was
/// never the thing being edited. Here only the `description:` line moves; the
/// bytes on either side of it are copied through untouched, including a CRLF
/// file's line endings.
///
/// Three cases: a replacement where the line exists, an insertion right under the
/// opening `---` where it does not, and a file with no header at all -- the
/// body-only skill [`parse`] accepts -- which has to gain one to carry the field.
/// An empty description removes the line, because the header is the only place a
/// trigger lives and an absent line is how this format says "none".
pub fn set_description(text: &str, name: &str, description: &str) -> String {
    let value = description.split_whitespace().collect::<Vec<_>>().join(" ");
    let Some((open, close)) = header_bounds(text) else {
        return if value.is_empty() {
            text.to_string()
        } else {
            render(name, &value, text)
        };
    };

    let mut offset = open;
    for chunk in text[open..close].split_inclusive('\n') {
        let line = chunk.trim_end_matches(['\r', '\n']);
        let indented = line.len() != line.trim_start().len();
        // An indented `description:` belongs to a nested block, which this parser
        // ignores; writing over it would change a key it never read.
        if !indented {
            if let Some((key, _)) = line.trim().split_once(':') {
                if key.trim() == "description" {
                    let end = offset + line.len();
                    // The newline after the line is kept, so a missing final
                    // newline cannot run the next key onto this one.
                    let replacement = if value.is_empty() {
                        String::new()
                    } else {
                        format!("description: {}", quote(&value))
                    };
                    let mut out = String::with_capacity(text.len() + replacement.len());
                    out.push_str(&text[..offset]);
                    out.push_str(&replacement);
                    out.push_str(&text[end..]);
                    return out;
                }
            }
        }
        offset += chunk.len();
    }

    if value.is_empty() {
        // Nothing to insert and nothing to remove.
        return text.to_string();
    }
    let newline = if text[..open].ends_with("\r\n") {
        "\r\n"
    } else {
        "\n"
    };
    let mut out = String::with_capacity(text.len() + value.len() + 16);
    out.push_str(&text[..open]);
    out.push_str(&format!("description: {}{newline}", quote(&value)));
    out.push_str(&text[open..]);
    out
}

/// The byte range of a file's header interior, between the two `---` lines.
///
/// `None` when the file has no header, which [`parse`] accepts as a body-only
/// skill. The same reading as [`parse`] on purpose: a first line that is not
/// `---`, after a byte-order mark, means the file starts with its instructions.
fn header_bounds(text: &str) -> Option<(usize, usize)> {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let mut lines = text.split_inclusive('\n');
    let first = lines.next()?;
    if first.trim_end_matches(['\r', '\n']).trim() != "---" {
        return None;
    }
    let mut offset = first.len();
    for chunk in lines {
        if chunk.trim_end_matches(['\r', '\n']).trim() == "---" {
            return Some((first.len(), offset));
        }
        offset += chunk.len();
    }
    None
}

/// Reads only enough of a `SKILL.md` to parse its header.
///
/// The index needs names and triggers, not bodies. Reading a whole file per skill
/// on every agent request would scale with the size of the instructions rather
/// than the number of skills; a bounded prefix scales with neither, because a
/// header is a few hundred bytes in every real skill. A header longer than the
/// window is treated as unreadable, which is the honest outcome for a file whose
/// description alone is 64 KB.
pub fn read_header(path: &Path) -> Result<Frontmatter, String> {
    use std::io::Read;

    let mut file = std::fs::File::open(path)
        .map_err(|error| format!("Could not read {}: {error}", path.display()))?;
    let mut buffer = vec![0u8; 64 * 1024];
    let read = file
        .read(&mut buffer)
        .map_err(|error| format!("Could not read {}: {error}", path.display()))?;
    buffer.truncate(read);
    let text = String::from_utf8_lossy(&buffer);
    parse(&text).map(|(header, _)| header)
}

/// Reads and parses one `SKILL.md`.
pub fn read(path: &Path) -> Result<(Frontmatter, String), String> {
    // A skill file is prose; a folder named like a skill is not a way to make a
    // listing read a disk image as text.
    let size = std::fs::metadata(path)
        .map_err(|error| format!("Could not read {}: {error}", path.display()))?
        .len();
    if size > 2_000_000 {
        return Err(format!(
            "{} is too large to read as a skill",
            path.display()
        ));
    }
    let text = std::fs::read_to_string(path)
        .map_err(|error| format!("Could not read {}: {error}", path.display()))?;
    parse(&text)
}

/// Removes one pair of surrounding quotes, if present.
fn unquote(value: &str) -> String {
    let bytes = value.as_bytes();
    if bytes.len() >= 2 {
        let first = bytes[0];
        let last = bytes[bytes.len() - 1];
        if (first == b'"' && last == b'"') || (first == b'\'' && last == b'\'') {
            let inner = &value[1..value.len() - 1];
            return if first == b'"' {
                inner.replace("\\\"", "\"").replace("\\\\", "\\")
            } else {
                inner.replace("''", "'")
            };
        }
    }
    value.to_string()
}

/// Quotes a value for the header.
fn quote(value: &str) -> String {
    let one_line = value.split_whitespace().collect::<Vec<_>>().join(" ");
    format!(
        "\"{}\"",
        one_line.replace('\\', "\\\\").replace('"', "\\\"")
    )
}

/// `code-review` becomes `Code review`.
pub fn humanize(slug: &str) -> String {
    let spaced = slug.replace(['-', '_'], " ");
    let mut characters = spaced.chars();
    match characters.next() {
        Some(first) => first.to_uppercase().collect::<String>() + characters.as_str(),
        None => spaced,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_header_and_body_round_trip() {
        let rendered = render(
            "code-review",
            "Use when reviewing a diff",
            "# Steps\n\nRead the diff.",
        );
        let (parsed, body) = parse(&rendered).expect("parse");
        assert_eq!(parsed.name, "code-review");
        assert_eq!(humanize(&parsed.name), "Code review");
        assert_eq!(parsed.description, "Use when reviewing a diff");
        assert!(body.contains("Read the diff."), "{body}");
    }

    /// The description is the one field that routinely contains punctuation, so
    /// the quoting has to survive a colon and a quote.
    #[test]
    fn a_quoted_description_comes_back_whole() {
        let rendered = render("x", "Use when the user says \"ship it\": carefully", "body");
        let (parsed, _) = parse(&rendered).expect("parse");
        assert_eq!(
            parsed.description,
            "Use when the user says \"ship it\": carefully"
        );
    }

    /// Lifted by the folded pass one line at a time, never by offset.
    #[test]
    fn a_description_line_is_replaced_in_place() {
        let text = "---\nname: x\nlicense: MIT\n# mine\ndescription: \"old\"\n---\n\nBody\n";
        let out = set_description(text, "x", "new");
        // Everything except the one line, including a key this parser ignores and
        // a comment a person wrote.
        assert_eq!(
            out,
            "---\nname: x\nlicense: MIT\n# mine\ndescription: \"new\"\n---\n\nBody\n"
        );
    }

    #[test]
    fn a_missing_description_line_is_inserted_under_the_opening_rule() {
        let out = set_description("---\nname: x\n---\n\nBody\n", "x", "when working on x");
        assert_eq!(
            out,
            "---\ndescription: \"when working on x\"\nname: x\n---\n\nBody\n"
        );
    }

    /// A body-only skill gains a header rather than the field being lost.
    #[test]
    fn a_file_without_a_header_gains_one() {
        let out = set_description("Just do the thing.", "x", "when doing it");
        let (header, body) = parse(&out).expect("parse");
        assert_eq!(header.description, "when doing it");
        assert_eq!(body, "Just do the thing.");
    }

    /// An empty description removes the line: an absent line is how this format
    /// says "no trigger", and `parse` reads it back as empty either way.
    #[test]
    fn an_empty_description_removes_the_line() {
        let out = set_description("---\nname: x\ndescription: \"old\"\n---\n\nBody\n", "x", "");
        let (header, body) = parse(&out).expect("parse");
        assert_eq!(header.description, "");
        assert_eq!(body, "Body");
    }

    /// A CRLF file keeps its line endings: the bytes on either side of the edited
    /// line are copied through rather than re-joined.
    #[test]
    fn editing_a_description_keeps_a_crlf_file_crlf() {
        let out = set_description(
            "---\r\nname: x\r\ndescription: \"old\"\r\n---\r\n\r\nBody\r\n",
            "x",
            "new",
        );
        assert_eq!(
            out,
            "---\r\nname: x\r\ndescription: \"new\"\r\n---\r\n\r\nBody\r\n"
        );
    }

    /// A file edited on Windows is CRLF. The body must not inherit the `\r` and
    /// must not be shifted by it -- the first version sliced by byte offset and
    /// landed one byte short per line.
    #[test]
    fn a_crlf_file_keeps_its_body_intact() {
        let (header, body) =
            parse("---\r\nname: x\r\ndescription: \"d\"\r\n---\r\n\r\nHello\r\nworld")
                .expect("parse");
        assert_eq!(header.name, "x");
        assert_eq!(body, "Hello\nworld");
    }

    /// A plain markdown file with no header is still a skill: the folder supplies
    /// the identity and the whole file is the body.
    #[test]
    fn a_file_without_a_header_is_all_body() {
        let (header, body) = parse("Just do the thing.").expect("parse");
        assert_eq!(header.name, "");
        assert_eq!(body, "Just do the thing.");
    }

    /// A header that opens and never closes is reported, because treating half a
    /// file as instructions would hide the mistake.
    #[test]
    fn an_unclosed_header_is_refused() {
        assert!(parse("---\nname: x\nbody")
            .expect_err("unclosed")
            .contains("closed"));
    }

    /// Unknown keys are ignored, so a skill written for another host still loads.
    #[test]
    fn unknown_fields_are_ignored() {
        let (header, body) =
            parse("---\nname: x\ndescription: d\nlicense: MIT\nallowed-tools: Bash\n---\nbody")
                .expect("parse");
        assert_eq!(header.name, "x");
        assert_eq!(body, "body");
    }

    /// A skill this app wrote before the fields were removed still loads: its
    /// `metadata` block is skipped and its description survives.
    #[test]
    fn a_legacy_metadata_block_is_skipped() {
        let (header, body) = parse(
            "---\nname: x\nmetadata:\n  title: \"X\"\n  type: code\ndescription: d\n---\nbody",
        )
        .expect("parse");
        assert_eq!(header.name, "x");
        assert_eq!(header.description, "d");
        assert_eq!(body, "body");
    }

    #[test]
    fn a_slug_is_humanized_rather_than_shown_bare() {
        assert_eq!(humanize("code-review"), "Code review");
        assert_eq!(humanize("code_review"), "Code review");
        assert_eq!(humanize(""), "");
    }
}
