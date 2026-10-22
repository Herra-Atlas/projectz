//! Reading and writing the small YAML header a `SKILL.md` carries.
//!
//! # Why not a YAML crate
//!
//! The header is a fixed, tiny shape: four top-level strings and one flat
//! `metadata` map. A full YAML parser would be a new dependency for a format the
//! app itself writes, and the one field that could contain punctuation -- the
//! description -- is quoted on the way out and unquoted on the way in. Unknown
//! keys are ignored rather than rejected, so a skill written for another host
//! still loads here.
//!
//! The frontmatter follows the Agent Skills specification: `name` is the slug and
//! must match the folder, `description` says when to use the skill, and anything
//! the app adds for itself lives under `metadata`.

use std::path::Path;

use super::SkillOrigin;

/// The fields this app understands.
#[derive(Clone, Debug, Default)]
pub struct Frontmatter {
    /// The spec's identifier: lowercase letters, digits and hyphens.
    pub name: String,
    /// What the card is called. Optional; falls back to a title-cased slug.
    pub title: Option<String>,
    pub description: String,
    pub skill_type: Option<String>,
    /// Set only for a skill the agent drafted. A bundled skill's origin comes
    /// from the root it lives in, not from its text.
    pub origin: Option<SkillOrigin>,
}

impl Frontmatter {
    /// The name to draw: the stored title, or the slug made readable.
    pub fn display_name(&self) -> String {
        self.title
            .as_deref()
            .map(str::trim)
            .filter(|title| !title.is_empty())
            .map(str::to_string)
            .unwrap_or_else(|| humanize(&self.name))
    }
}

/// Parses a `SKILL.md`: the header, then the body exactly as written.
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
    let mut metadata_indent: Option<usize> = None;
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
            parse_line(line, &mut header, &mut metadata_indent);
        }
    }
    if !closed {
        return Err("The skill header was not closed with `---`".to_string());
    }

    Ok((header, body.join("\n").trim_start_matches('\n').to_string()))
}

/// Reads one header line into the struct.
///
/// Indentation is what separates a `metadata` entry from a top-level field, so
/// the parser tracks it rather than assuming a fixed two spaces.
fn parse_line(line: &str, header: &mut Frontmatter, metadata_indent: &mut Option<usize>) {
    let trimmed = line.trim();
    if trimmed.is_empty() || trimmed.starts_with('#') {
        return;
    }
    let Some((key, value)) = trimmed.split_once(':') else {
        return;
    };
    let key = key.trim();
    let value = unquote(value.trim());
    let indent = line.len() - line.trim_start().len();

    if trimmed == "metadata:" {
        *metadata_indent = Some(indent);
        return;
    }
    // A metadata entry is indented under `metadata:`; anything at the header's
    // own indentation ends the block.
    let in_metadata = metadata_indent.is_some_and(|base| indent > base);
    if in_metadata {
        match key {
            "type" => header.skill_type = non_empty(value),
            "title" => header.title = non_empty(value),
            "origin" => header.origin = Some(SkillOrigin::from_str(&value)),
            _ => {}
        }
        return;
    }
    *metadata_indent = None;
    match key {
        "name" => header.name = value,
        "title" => header.title = non_empty(value),
        "description" => header.description = value,
        "type" => header.skill_type = non_empty(value),
        "origin" => header.origin = Some(SkillOrigin::from_str(&value)),
        _ => {}
    }
}

/// Renders the header and body in the shape [`parse`] reads.
///
/// Quoted values are the only escaping the parser has to understand, and the
/// description is the one field that routinely contains a colon or a quote.
pub fn render(header: &Frontmatter, body: &str) -> String {
    let mut out = String::from("---\n");
    out.push_str(&format!("name: {}\n", header.name));
    if !header.description.trim().is_empty() {
        out.push_str(&format!("description: {}\n", quote(&header.description)));
    }
    let has_metadata = header.title.is_some()
        || header.skill_type.is_some()
        || header
            .origin
            .is_some_and(|origin| origin == SkillOrigin::Generated);
    if has_metadata {
        out.push_str("metadata:\n");
        if let Some(title) = header
            .title
            .as_deref()
            .map(str::trim)
            .filter(|title| !title.is_empty())
        {
            out.push_str(&format!("  title: {}\n", quote(title)));
        }
        if let Some(skill_type) = header
            .skill_type
            .as_deref()
            .map(str::trim)
            .filter(|label| !label.is_empty())
        {
            out.push_str(&format!("  type: {}\n", quote(skill_type)));
        }
        if header.origin == Some(SkillOrigin::Generated) {
            out.push_str("  origin: generated\n");
        }
    }
    out.push_str("---\n\n");
    out.push_str(body.trim());
    if !out.ends_with('\n') {
        out.push('\n');
    }
    out
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

fn non_empty(value: String) -> Option<String> {
    let value = value.trim().to_string();
    (!value.is_empty()).then_some(value)
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

/// `code-review` becomes `Code review`, for a skill that carries no title.
fn humanize(slug: &str) -> String {
    let spaced = slug.replace('-', " ");
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
        let header = Frontmatter {
            name: "code-review".into(),
            title: Some("Code review".into()),
            description: "Use when reviewing a diff".into(),
            skill_type: Some("code".into()),
            origin: Some(SkillOrigin::Generated),
        };
        let rendered = render(&header, "# Steps\n\nRead the diff.");
        let (parsed, body) = parse(&rendered).expect("parse");
        assert_eq!(parsed.name, "code-review");
        assert_eq!(parsed.display_name(), "Code review");
        assert_eq!(parsed.description, "Use when reviewing a diff");
        assert_eq!(parsed.skill_type.as_deref(), Some("code"));
        assert_eq!(parsed.origin, Some(SkillOrigin::Generated));
        assert!(body.contains("Read the diff."), "{body}");
    }

    /// The description is the one field that routinely contains punctuation, so
    /// the quoting has to survive a colon and a quote.
    #[test]
    fn a_quoted_description_comes_back_whole() {
        let header = Frontmatter {
            name: "x".into(),
            description: "Use when the user says \"ship it\": carefully".into(),
            ..Frontmatter::default()
        };
        let (parsed, _) = parse(&render(&header, "body")).expect("parse");
        assert_eq!(
            parsed.description,
            "Use when the user says \"ship it\": carefully"
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

    #[test]
    fn an_empty_slug_is_humanized_rather_than_shown_bare() {
        assert_eq!(humanize("code-review"), "Code review");
        assert_eq!(humanize(""), "");
    }
}
