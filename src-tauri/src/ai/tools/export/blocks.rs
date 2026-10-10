//! What a document is made of, and how the tool's `content` becomes it.
//!
//! A writer needs structure -- this paragraph is a heading, this run is bold,
//! this is a table -- and a model writes text. Asking the model for a block
//! schema would mean teaching it a format it has never seen; asking it for
//! Markdown means it can use the one it already writes all day. So `content` is
//! read as a small, fixed subset of Markdown and turned into blocks here, and
//! both writers consume blocks rather than text.
//!
//! # The subset, and what is deliberately missing
//!
//! Headings (`#`..`######`), fenced code, `>` quotes, `-`/`*`/`+` and `1.` lists,
//! pipe tables with a `|---|` separator, `---` rules, images as
//! `![alt](path)`, and `**bold**`, `*italic*` and `` `code` `` inline. A link
//! keeps its label and shows its target in parentheses, because a Word hyperlink
//! needs a relationship per URL and the label alone would silently drop where it
//! pointed. Nesting, footnotes and reference links are not implemented: a
//! generated document does not need them, and guessing at them badly would be
//! worse than leaving the characters in the text.
//!
//! Anything the subset does not recognise stays literal -- an unmatched `**` is
//! two asterisks, not a swallowed rest-of-line.
//!
//! # One line is one paragraph
//!
//! Markdown would join consecutive lines into one paragraph; this does not. A
//! model writing a document breaks the line where the paragraph ends, and joining
//! those lines would run a bullet list's introduction into the list. Each line
//! standing alone is both what the text says and what the previous writer did.

/// A span of text sharing one set of emphases.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Run {
    pub text: String,
    pub bold: bool,
    pub italic: bool,
    /// Monospaced, and rendered with a shaded background in Word.
    pub code: bool,
}

impl Run {
    /// A run with no emphasis, from any string.
    pub(crate) fn text(text: &str) -> Self {
        Self::plain(text)
    }

    fn plain(text: &str) -> Self {
        Self {
            text: text.to_string(),
            bold: false,
            italic: false,
            code: false,
        }
    }
}

/// One piece of a document.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Block {
    /// `level` is 1-6.
    Heading {
        level: u8,
        runs: Vec<Run>,
    },
    Paragraph(Vec<Run>),
    List {
        ordered: bool,
        items: Vec<Vec<Run>>,
    },
    Quote(Vec<Run>),
    /// Fenced code, kept line by line so the writer can preserve the breaks.
    Code {
        lines: Vec<String>,
    },
    /// A pipe table. `head` is absent only for a table written without one.
    Table {
        head: Option<Vec<String>>,
        rows: Vec<Vec<String>>,
    },
    /// A picture, by workspace-relative or absolute path on disk.
    Image {
        alt: String,
        target: String,
    },
    Rule,
}

/// Reads `content` as the Markdown subset, in order.
pub(crate) fn parse(content: &str) -> Vec<Block> {
    let lines: Vec<&str> = content.lines().collect();
    let mut blocks = Vec::new();
    let mut index = 0;

    while index < lines.len() {
        let trimmed = lines[index].trim();
        if trimmed.is_empty() {
            index += 1;
            continue;
        }

        if let Some(fence) = trimmed.strip_prefix("```") {
            let _ = fence;
            index += 1;
            let mut code = Vec::new();
            while index < lines.len() && !lines[index].trim().starts_with("```") {
                code.push(lines[index].to_string());
                index += 1;
            }
            // Past the closing fence -- or past the end, for a block the model
            // left open, which is still code rather than an error.
            index += 1;
            blocks.push(Block::Code { lines: code });
            continue;
        }

        if let Some((level, text)) = heading(trimmed) {
            blocks.push(Block::Heading {
                level,
                runs: inline(text),
            });
            index += 1;
            continue;
        }

        if is_rule(trimmed) {
            blocks.push(Block::Rule);
            index += 1;
            continue;
        }

        if let Some((alt, target)) = only_image(trimmed) {
            blocks.push(Block::Image { alt, target });
            index += 1;
            continue;
        }

        // A table is only a table when the line under it is the `|---|` rule;
        // otherwise a paragraph that happens to contain a pipe is just text.
        if trimmed.contains('|')
            && lines
                .get(index + 1)
                .is_some_and(|next| is_table_separator(next.trim()))
        {
            let head = table_row(trimmed);
            index += 2;
            let mut rows = Vec::new();
            while index < lines.len() && lines[index].trim().contains('|') {
                rows.push(table_row(lines[index].trim()));
                index += 1;
            }
            blocks.push(Block::Table {
                head: Some(head),
                rows,
            });
            continue;
        }

        if trimmed.starts_with('>') {
            let mut runs = Vec::new();
            while index < lines.len() && lines[index].trim().starts_with('>') {
                if !runs.is_empty() {
                    runs.push(Run::plain(" "));
                }
                let text = lines[index].trim().trim_start_matches('>').trim();
                runs.extend(inline(text));
                index += 1;
            }
            blocks.push(Block::Quote(runs));
            continue;
        }

        if let Some((ordered, _)) = list_item(trimmed) {
            let mut items = Vec::new();
            while index < lines.len() {
                match list_item(lines[index].trim()) {
                    // A change of marker ends this list and starts another, so a
                    // numbered list after a bulleted one stays numbered.
                    Some((item_ordered, text)) if item_ordered == ordered => {
                        items.push(inline(text));
                        index += 1;
                    }
                    _ => break,
                }
            }
            blocks.push(Block::List { ordered, items });
            continue;
        }

        blocks.push(Block::Paragraph(inline(trimmed)));
        index += 1;
    }

    blocks
}

/// A heading's level and text, when the line is one.
fn heading(line: &str) -> Option<(u8, &str)> {
    let hashes = line
        .chars()
        .take_while(|character| *character == '#')
        .count();
    if hashes == 0 || hashes > 6 {
        return None;
    }
    let rest = &line[hashes..];
    // `#Heading` is not a heading -- the space is what separates the marker from
    // the text, and without it the line is more likely a tag or a comment.
    let text = rest.strip_prefix(' ')?;
    Some((hashes as u8, text.trim()))
}

/// True for three or more of the same rule character, spaced or not.
fn is_rule(line: &str) -> bool {
    let mut marks = 0;
    for character in line.chars() {
        match character {
            '-' | '*' | '_' => marks += 1,
            ' ' | '\t' => {}
            _ => return false,
        }
    }
    marks >= 3 && line.contains(['-', '*', '_'])
}

/// An image alone on its line, as `![alt](target)`.
fn only_image(line: &str) -> Option<(String, String)> {
    let rest = line.strip_prefix("![")?;
    let (alt, rest) = rest.split_once("](")?;
    let target = rest.strip_suffix(')')?;
    if target.trim().is_empty() {
        return None;
    }
    Some((alt.trim().to_string(), target.trim().to_string()))
}

/// True for the `|---|---|` line under a table's header.
fn is_table_separator(line: &str) -> bool {
    if !line.contains(['-', '|']) {
        return false;
    }
    let cells = split_row(line);
    !cells.is_empty()
        && cells.iter().all(|cell| {
            let body = cell.trim().trim_matches(':');
            !body.is_empty() && body.chars().all(|character| character == '-')
        })
}

/// The cells of one pipe-table line, without the outer pipes.
fn table_row(line: &str) -> Vec<String> {
    split_row(line)
        .into_iter()
        .map(|cell| cell.trim().replace("\\|", "|"))
        .collect()
}

/// Splits on unescaped pipes and drops the empty ends an outer pipe creates.
fn split_row(line: &str) -> Vec<String> {
    let mut cells = Vec::new();
    let mut current = String::new();
    let mut characters = line.chars();
    while let Some(character) = characters.next() {
        match character {
            '\\' => {
                // Keep the escape so `table_row` can unescape it; a backslash
                // before anything else is just a backslash.
                current.push(character);
                if let Some(next) = characters.next() {
                    current.push(next);
                }
            }
            '|' => {
                cells.push(std::mem::take(&mut current));
            }
            _ => current.push(character),
        }
    }
    cells.push(current);
    if cells.first().is_some_and(|cell| cell.trim().is_empty()) {
        cells.remove(0);
    }
    if cells.last().is_some_and(|cell| cell.trim().is_empty()) {
        cells.pop();
    }
    cells
}

/// A list marker and the item's text, when the line is one.
fn list_item(line: &str) -> Option<(bool, &str)> {
    for marker in ["- ", "* ", "+ "] {
        if let Some(text) = line.strip_prefix(marker) {
            return Some((false, text.trim()));
        }
    }
    let digits = line
        .chars()
        .take_while(|character| character.is_ascii_digit())
        .count();
    if digits == 0 {
        return None;
    }
    let rest = &line[digits..];
    let text = rest
        .strip_prefix(". ")
        .or_else(|| rest.strip_prefix(") "))?;
    Some((true, text.trim()))
}

/// Splits a line into runs, recognising `**bold**`, `*italic*`, `` `code` `` and
/// `[label](target)`.
///
/// A scanner rather than a regex because the three markers nest badly in a
/// regex and the rule here is simply "the first marker that is closed wins". An
/// unclosed marker is left in the text.
pub(crate) fn inline(text: &str) -> Vec<Run> {
    let mut runs = Vec::new();
    let mut plain = String::new();
    let mut rest = text;

    while !rest.is_empty() {
        match marker(rest) {
            Some((run, length)) => {
                if !plain.is_empty() {
                    runs.push(Run::plain(&plain));
                    plain.clear();
                }
                runs.push(run);
                rest = &rest[length..];
            }
            None => {
                let character = rest.chars().next().expect("non-empty");
                plain.push(character);
                rest = &rest[character.len_utf8()..];
            }
        }
    }
    if !plain.is_empty() {
        runs.push(Run::plain(&plain));
    }
    runs
}

/// The run starting at the front of `rest`, with the bytes it consumed.
fn marker(rest: &str) -> Option<(Run, usize)> {
    if let Some(body) = rest.strip_prefix("**") {
        if let Some(end) = body.find("**") {
            if end > 0 {
                return Some((
                    Run {
                        text: body[..end].to_string(),
                        bold: true,
                        italic: false,
                        code: false,
                    },
                    end + 4,
                ));
            }
        }
    }
    if let Some(body) = rest.strip_prefix('*') {
        // `**` was tried above, so this is a single star.
        if let Some(end) = body.find('*') {
            if end > 0 {
                return Some((
                    Run {
                        text: body[..end].to_string(),
                        bold: false,
                        italic: true,
                        code: false,
                    },
                    end + 2,
                ));
            }
        }
    }
    if let Some(body) = rest.strip_prefix('`') {
        if let Some(end) = body.find('`') {
            if end > 0 {
                return Some((
                    Run {
                        text: body[..end].to_string(),
                        bold: false,
                        italic: false,
                        code: true,
                    },
                    end + 2,
                ));
            }
        }
    }
    if let Some(body) = rest.strip_prefix('[') {
        if let Some((label, tail)) = body.split_once("](") {
            if let Some((target, _)) = tail.split_once(')') {
                // The target is kept in the text: a Word hyperlink needs a
                // relationship per URL, and a label on its own would hide where
                // it pointed.
                let text = if label.trim() == target.trim() {
                    label.to_string()
                } else {
                    format!("{label} ({target})")
                };
                // The bytes consumed: `[`, the label, `](`, the target and `)`.
                return Some((Run::plain(&text), label.len() + target.len() + 4));
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_heading_becomes_a_heading_block() {
        assert_eq!(
            parse("## Findings"),
            vec![Block::Heading {
                level: 2,
                runs: vec![Run::plain("Findings")]
            }]
        );
    }

    /// `#Heading` is not a heading: the space is the marker's delimiter, and a
    /// tag or a colour value should stay text.
    #[test]
    fn a_hash_without_a_space_is_not_a_heading() {
        assert_eq!(
            parse("#FF0000 is red"),
            vec![Block::Paragraph(vec![Run::plain("#FF0000 is red")])]
        );
    }

    #[test]
    fn bullets_and_numbers_become_two_lists() {
        let blocks = parse("- one\n- two\n1. first\n2. second");
        assert_eq!(
            blocks,
            vec![
                Block::List {
                    ordered: false,
                    items: vec![vec![Run::plain("one")], vec![Run::plain("two")]]
                },
                Block::List {
                    ordered: true,
                    items: vec![vec![Run::plain("first")], vec![Run::plain("second")]]
                },
            ]
        );
    }

    #[test]
    fn a_pipe_table_needs_its_separator_line() {
        let table = parse("| Name | Score |\n|---|---|\n| Ada | 42 |");
        assert_eq!(
            table,
            vec![Block::Table {
                head: Some(vec![String::from("Name"), String::from("Score")]),
                rows: vec![vec![String::from("Ada"), String::from("42")]],
            }]
        );
        // No separator under it, so it is a paragraph that contains pipes.
        assert!(matches!(
            parse("| not | a table |").as_slice(),
            [Block::Paragraph(_)]
        ));
    }

    #[test]
    fn bold_italic_and_code_split_a_line_into_runs() {
        let runs = inline("plain **bold** and *italic* and `code`");
        assert_eq!(
            runs,
            vec![
                Run::plain("plain "),
                Run {
                    text: "bold".into(),
                    bold: true,
                    italic: false,
                    code: false
                },
                Run::plain(" and "),
                Run {
                    text: "italic".into(),
                    bold: false,
                    italic: true,
                    code: false
                },
                Run::plain(" and "),
                Run {
                    text: "code".into(),
                    bold: false,
                    italic: false,
                    code: true
                },
            ]
        );
    }

    /// An unclosed marker stays literal rather than swallowing the rest of the
    /// line.
    #[test]
    fn an_unclosed_marker_is_text() {
        assert_eq!(inline("2 * 3 = 6"), vec![Run::plain("2 * 3 = 6")]);
        assert_eq!(inline("**oops"), vec![Run::plain("**oops")]);
    }

    #[test]
    fn a_link_keeps_its_label_and_shows_its_target() {
        assert_eq!(
            inline("see [the docs](https://example.com)"),
            vec![
                Run::plain("see "),
                Run::plain("the docs (https://example.com)")
            ]
        );
        // A label that repeats the target would only be printed twice.
        assert_eq!(
            inline("[https://example.com](https://example.com)"),
            vec![Run::plain("https://example.com")]
        );
    }

    #[test]
    fn a_fenced_block_keeps_its_lines_and_leaves_the_fence_out() {
        assert_eq!(
            parse("```rust\nlet x = 1;\n```"),
            vec![Block::Code {
                lines: vec!["let x = 1;".to_string()]
            }]
        );
    }

    #[test]
    fn an_image_alone_on_a_line_is_a_picture_block() {
        assert_eq!(
            parse("![Sales chart](charts/q3.png)"),
            vec![Block::Image {
                alt: "Sales chart".into(),
                target: "charts/q3.png".into()
            }]
        );
        // An image inside a sentence is left as text rather than promoted.
        assert!(matches!(
            parse("see ![x](y.png) above").as_slice(),
            [Block::Paragraph(_)]
        ));
    }

    #[test]
    fn a_rule_is_a_rule_in_any_of_its_spellings() {
        for line in ["---", "***", "_ _ _"] {
            assert_eq!(parse(line), vec![Block::Rule], "{line}");
        }
    }

    /// Each line stands alone, so a paragraph is never joined with the one after
    /// it -- which is what keeps an introduction off the list below it.
    #[test]
    fn consecutive_lines_stay_separate_paragraphs() {
        assert_eq!(parse("one\ntwo").len(), 2);
    }
}
