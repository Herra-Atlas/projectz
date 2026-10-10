//! Writing a real document file into the workspace.
//!
//! A chat answer is prose in a window; a *document* is a file the user can send,
//! open and print. This tool turns a model's description of a document into an
//! actual `.docx`, `.xlsx`, `.pdf` or `.csv` on disk -- the thing people leave
//! other assistants to get, and the thing this app could not produce.
//!
//! # Why one tool and not four
//!
//! The model's decision is "make a spreadsheet of this", not "call the xlsx
//! writer". One tool whose `format` chooses the writer keeps that decision in one
//! place, and the writers themselves live in `export/`, one file per format,
//! because a Word document and a spreadsheet have almost nothing in common once
//! past the argument parsing: what they share is the zip-of-XML plumbing, which
//! is in `export/ooxml.rs`.
//!
//! # Why the formats are built by hand
//!
//! A `.docx` and an `.xlsx` are zip files of XML, and `zip` is already a
//! dependency (the PDF reader uses it), so the writers are XML documents rather
//! than more libraries in the tree. PDF uses `lopdf`, also already present. Every
//! writer produces bytes that are written through the same atomic temp-and-rename
//! as an edit, because a half-written `.docx` is a corrupt one.
//!
//! # What the documents look like
//!
//! The rich formatting lives in the writers: `export/docx.rs` emits named styles,
//! real lists, bordered tables and embedded pictures, and `export/xlsx.rs` emits a
//! frozen header row, column widths and number formats. What the model writes is
//! still text -- `export/blocks.rs` reads it as a small Markdown subset and turns
//! it into blocks, so no new syntax has to be learned to get a styled document.

use std::path::PathBuf;

use serde_json::Value;

use super::ToolSpec;

mod blocks;
mod docx;
mod image;
mod ooxml;
#[cfg(test)]
mod test_support;
mod xlsx;

/// The body font a document is written in when none is asked for.
const DEFAULT_FONT: &str = "Calibri";

/// The worksheet name a spreadsheet is given when none is asked for.
const DEFAULT_SHEET: &str = "Sheet1";

/// Create a document file from described content.
pub const WRITE_DOCUMENT: ToolSpec = ToolSpec {
    name: "write_document",
    description: "Create a document file in the workspace: a Word file (.docx), a spreadsheet \
                  (.xlsx), a PDF, a CSV, or a plain text or Markdown file. Use this when the user \
                  asks for a report, a document, a spreadsheet or an export rather than an answer \
                  in chat. For docx, pdf, md and txt, give `content` as Markdown: `#`..`######` \
                  headings, `-` or `1.` lists, `> ` quotes, ``` fences, `| a | b |` tables with a \
                  `|---|---|` line, `---` rules, `**bold**`, `*italic*`, `` `code` `` and \
                  `![alt](path/to/image.png)` to place a picture from the workspace. Headings, \
                  lists, tables and pictures are rendered properly in .docx, so use them instead of \
                  plain lines. For xlsx and csv, give `rows` instead.",
    parameters: r#"{
        "type": "object",
        "properties": {
            "path": {
                "type": "string",
                "description": "Where to write it, relative to the workspace root, including the extension, for example `reports/summary.docx`."
            },
            "format": {
                "type": "string",
                "enum": ["docx", "xlsx", "pdf", "csv", "md", "txt"],
                "description": "The file format. Inferred from the path's extension when omitted."
            },
            "title": {
                "type": "string",
                "description": "A heading shown at the top of a docx or pdf. Optional."
            },
            "content": {
                "type": "string",
                "description": "The body for docx, pdf, md and txt, written as Markdown. Headings, lists, tables, quotes, code and images are rendered as such in .docx."
            },
            "font": {
                "type": "string",
                "description": "The document's body font for docx, for example `Georgia`. Optional; the default is Calibri."
            },
            "rows": {
                "type": "array",
                "description": "Rows of cells for xlsx and csv. Each row is a list of cell values (strings or numbers).",
                "items": {
                    "type": "array",
                    "items": { "type": ["string", "number", "boolean", "null"] }
                }
            },
            "header": {
                "type": "boolean",
                "description": "For xlsx: treat the first row as a heading, which styles it, freezes it and adds a filter. Defaults to true."
            },
            "sheet": {
                "type": "string",
                "description": "For xlsx: the worksheet's name. Defaults to `Sheet1`."
            }
        },
        "required": ["path"],
        "additionalProperties": false
    }"#,
    effect: super::Effect::Write,
    command_argument: None,
    execute: |arguments, context| {
        Box::pin(std::future::ready(
            super::file::require_workspace().and_then(|root| build(arguments, root, &context)),
        ))
    },
};

/// The formats this tool can write.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Format {
    Docx,
    Xlsx,
    Pdf,
    Csv,
    Markdown,
    Text,
}

impl Format {
    fn parse(name: &str) -> Option<Self> {
        match name {
            "docx" => Some(Format::Docx),
            "xlsx" => Some(Format::Xlsx),
            "pdf" => Some(Format::Pdf),
            "csv" => Some(Format::Csv),
            "md" | "markdown" => Some(Format::Markdown),
            "txt" | "text" => Some(Format::Text),
            _ => None,
        }
    }
}

fn build(arguments: Value, root: PathBuf, context: &super::ToolContext) -> Result<String, String> {
    let path = arguments
        .get("path")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|path| !path.is_empty())
        .ok_or("write_document requires a `path`")?
        .to_string();

    // The format is the explicit field, or the extension when it is omitted, so a
    // model that just names a `.csv` path still gets a CSV.
    let format = match arguments.get("format").and_then(Value::as_str) {
        Some(name) => Format::parse(name.trim().to_lowercase().as_str())
            .ok_or_else(|| format!("`{name}` is not a format this tool writes."))?,
        None => path
            .rsplit('.')
            .next()
            .and_then(|extension| Format::parse(&extension.to_lowercase()))
            .ok_or("Give a `format`, or a `path` with an extension this tool knows (docx, xlsx, pdf, csv, md, txt).")?,
    };

    let title = string_of(&arguments, "title");
    let content = string_of(&arguments, "content");
    let rows = rows_of(&arguments)?;

    let bytes = match format {
        Format::Docx => docx::bytes(
            title.as_deref(),
            &blocks::parse(&require_content(&content, "docx")?),
            &string_of(&arguments, "font").unwrap_or_else(|| DEFAULT_FONT.to_string()),
            &root,
        )?,
        Format::Pdf => pdf_bytes(title.as_deref(), &require_content(&content, "pdf")?)?,
        Format::Markdown => {
            let mut text = String::new();
            if let Some(title) = &title {
                text.push_str(&format!("# {title}\n\n"));
            }
            text.push_str(&require_content(&content, "md")?);
            text.into_bytes()
        }
        Format::Text => {
            let mut text = String::new();
            if let Some(title) = &title {
                text.push_str(&format!("{title}\n\n"));
            }
            text.push_str(&require_content(&content, "txt")?);
            text.into_bytes()
        }
        Format::Csv => csv_bytes(&require_rows(&rows, "csv")?),
        Format::Xlsx => xlsx::bytes(
            &require_rows(&rows, "xlsx")?,
            &string_of(&arguments, "sheet").unwrap_or_else(|| DEFAULT_SHEET.to_string()),
            // A first row of labels is what a generated table almost always has,
            // and styling it is the default for that reason -- with a way out for
            // the sheet that is a plain grid of values.
            arguments
                .get("header")
                .and_then(Value::as_bool)
                .unwrap_or(true),
        )?,
    };

    let resolved = super::file::resolve_within(&root, &path)?;
    if let Some(parent) = resolved.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|error| format!("Could not create {}: {error}", parent.display()))?;
    }
    // Read before the write, while the old contents still exist, so the panel can
    // diff a document that replaced one.
    let previous = std::fs::read_to_string(&resolved).unwrap_or_default();
    super::write::atomic_write(&resolved, &bytes)?;

    // A text format's bytes are its lines, so the panel can show a real diff and
    // the reply's "what changed" figure includes it. A binary document
    // (`.docx`/`.xlsx`/`.pdf`) has none: its bytes are a zip or a drawing stream,
    // and a line diff of them would be noise, so it is skipped and appears only
    // in the activity panel.
    if matches!(format, Format::Markdown | Format::Text | Format::Csv) {
        let text = String::from_utf8_lossy(&bytes);
        context.report_diff(super::diff::diff(&previous, &text));
    }
    Ok(format!("Wrote {path} ({} bytes).", bytes.len()))
}

fn require_content(content: &Option<String>, format: &str) -> Result<String, String> {
    content
        .clone()
        .filter(|text| !text.trim().is_empty())
        .ok_or_else(|| format!("A `{format}` document needs `content` — the text to write."))
}

fn require_rows(rows: &Option<Vec<Vec<Cell>>>, format: &str) -> Result<Vec<Vec<Cell>>, String> {
    rows.clone()
        .filter(|rows| !rows.is_empty())
        .ok_or_else(|| format!("A `{format}` file needs `rows` — the cells to write."))
}

fn string_of(arguments: &Value, field: &str) -> Option<String> {
    arguments
        .get(field)
        .and_then(Value::as_str)
        .map(str::to_string)
}

/// One cell, kept as text plus an "is a number" flag so a spreadsheet keeps
/// numbers as numbers rather than as text that looks like one.
#[derive(Clone)]
enum Cell {
    Text(String),
    Number(f64),
}

impl Cell {
    /// The cell's value as text, which is what CSV needs and what a text cell
    /// stores.
    fn as_text(&self) -> String {
        match self {
            Cell::Text(text) => text.clone(),
            Cell::Number(number) => ooxml::number_to_string(*number),
        }
    }
}

fn rows_of(arguments: &Value) -> Result<Option<Vec<Vec<Cell>>>, String> {
    let Some(value) = arguments.get("rows") else {
        return Ok(None);
    };
    let rows = value.as_array().ok_or("`rows` must be an array of rows")?;
    let mut out = Vec::with_capacity(rows.len());
    for row in rows {
        let cells = row
            .as_array()
            .ok_or("each entry in `rows` must be an array")?;
        out.push(
            cells
                .iter()
                .map(|cell| match cell {
                    Value::Number(number) => Cell::Number(number.as_f64().unwrap_or(0.0)),
                    Value::Bool(flag) => Cell::Text(flag.to_string()),
                    Value::String(text) => Cell::Text(text.clone()),
                    Value::Null => Cell::Text(String::new()),
                    _ => Cell::Text(cell.to_string()),
                })
                .collect(),
        );
    }
    Ok(Some(out))
}

/// Escapes one field for CSV, quoting when it contains a comma, a quote or a
/// newline -- the only three cases a reader is allowed to misread.
fn csv_bytes(rows: &[Vec<Cell>]) -> Vec<u8> {
    let mut out = String::new();
    for row in rows {
        let line = row
            .iter()
            .map(|cell| {
                let text = cell.as_text();
                if text.contains(',')
                    || text.contains('"')
                    || text.contains('\n')
                    || text.contains('\r')
                {
                    format!("\"{}\"", text.replace('"', "\"\""))
                } else {
                    text
                }
            })
            .collect::<Vec<_>>()
            .join(",");
        out.push_str(&line);
        out.push('\n');
    }
    out.into_bytes()
}

/// A simple, text-only PDF: one line of text per line, paginated.
///
/// Built by hand with `lopdf` because a PDF's text is a content stream of drawing
/// operators, and this needs nothing more than Helvetica at a fixed size. There is
/// no wrapping beyond a character estimate and no styling, which is what a
/// generated report needs and no more.
fn pdf_bytes(title: Option<&str>, content: &str) -> Result<Vec<u8>, String> {
    use lopdf::content::{Content, Operation};
    use lopdf::{dictionary, Document, Object, Stream};

    /// A page is US Letter: 612 by 792 points.
    const PAGE_WIDTH: f64 = 612.0;
    const MARGIN: f64 = 64.0;
    const TOP: f64 = 756.0;
    const BOTTOM: f64 = 48.0;
    const LEADING: f64 = 16.0;
    /// Roughly how many characters fit in the text width at 11pt Helvetica.
    const WRAP: usize = 88;

    // Every line to draw, in order. The title is wrapped too, so a long one does
    // not run off the page.
    let mut lines: Vec<String> = Vec::new();
    if let Some(title) = title {
        lines.extend(wrap_text(title, WRAP));
        lines.push(String::new());
    }
    for line in content.lines() {
        lines.extend(wrap_text(line, WRAP));
    }

    let per_page = (((TOP - BOTTOM) / LEADING).floor() as usize).max(1);
    let page_count = lines.len().div_ceil(per_page).max(1);

    let mut document = Document::with_version("1.4");
    let font_id = document.add_object(dictionary! {
        "Type" => "Font",
        "Subtype" => "Type1",
        "BaseFont" => "Helvetica",
    });
    let resources = document.add_object(dictionary! {
        "Font" => dictionary! { "F1" => font_id },
    });
    let pages_id = document.new_object_id();

    let mut page_ids = Vec::new();
    for page_index in 0..page_count {
        let start = page_index * per_page;
        let end = (start + per_page).min(lines.len());
        let mut operations = Vec::new();
        for (offset, line) in lines[start..end].iter().enumerate() {
            if line.is_empty() {
                continue;
            }
            let y = TOP - (offset as f64 * LEADING);
            operations.push(Operation::new("BT", vec![]));
            operations.push(Operation::new(
                "Tf",
                vec![Object::Name(b"F1".to_vec()), Object::Integer(11)],
            ));
            operations.push(Operation::new(
                "Tm",
                vec![
                    Object::Integer(1),
                    Object::Integer(0),
                    Object::Integer(0),
                    Object::Integer(1),
                    Object::Real(MARGIN as f32),
                    Object::Real(y as f32),
                ],
            ));
            operations.push(Operation::new(
                "Tj",
                vec![Object::String(
                    escape_pdf_text(line).into_bytes(),
                    lopdf::StringFormat::Literal,
                )],
            ));
            operations.push(Operation::new("ET", vec![]));
        }
        let encoded = Content { operations }
            .encode()
            .map_err(|error| format!("Could not lay out the PDF: {error}"))?;
        let content_id = document.add_object(Stream::new(dictionary! {}, encoded));
        let page_id = document.add_object(dictionary! {
            "Type" => "Page",
            "Parent" => pages_id,
            "MediaBox" => vec![0.into(), 0.into(), PAGE_WIDTH.into(), (TOP + MARGIN).into()],
            "Resources" => resources,
            "Contents" => content_id,
        });
        page_ids.push(page_id);
    }

    document.objects.insert(
        pages_id,
        Object::Dictionary(dictionary! {
            "Type" => "Pages",
            "Kids" => page_ids.into_iter().map(Object::Reference).collect::<Vec<_>>(),
            "Count" => Object::Integer(page_count as i64),
        }),
    );
    let catalog_id = document.add_object(dictionary! {
        "Type" => "Catalog",
        "Pages" => pages_id,
    });
    document.trailer.set("Root", catalog_id);
    document.compress();
    let mut bytes = Vec::new();
    document
        .save_to(&mut bytes)
        .map_err(|error| format!("Could not write the PDF: {error}"))?;
    Ok(bytes)
}

/// Wraps text at a character count, breaking on the last space where there is one.
///
/// A character count rather than a measured width because Helvetica's metrics are
/// not worth embedding for a generated report; the estimate is close enough that a
/// line rarely overflows the margin.
fn wrap_text(text: &str, width: usize) -> Vec<String> {
    if text.trim().is_empty() {
        return vec![String::new()];
    }
    let mut lines = Vec::new();
    let mut current = String::new();
    for word in text.split_whitespace() {
        if current.is_empty() {
            current.push_str(word);
        } else if current.chars().count() + 1 + word.chars().count() <= width {
            current.push(' ');
            current.push_str(word);
        } else {
            lines.push(std::mem::take(&mut current));
            current.push_str(word);
        }
    }
    if !current.is_empty() {
        lines.push(current);
    }
    lines
}

/// Escapes the three characters that end a PDF literal string early.
fn escape_pdf_text(text: &str) -> String {
    text.replace('\\', "\\\\")
        .replace('(', "\\(")
        .replace(')', "\\)")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn discarding() -> super::super::ToolContext {
        super::super::ToolContext::default()
    }

    fn root() -> PathBuf {
        use std::sync::atomic::{AtomicU32, Ordering};
        static COUNTER: AtomicU32 = AtomicU32::new(0);
        let unique = COUNTER.fetch_add(1, Ordering::Relaxed);
        let dir =
            std::env::temp_dir().join(format!("projectz-export-{}-{unique}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("temp dir");
        dir
    }

    /// The arguments that shape the output have to reach the writers: a document
    /// asked for in Georgia in a sheet named "Q3" is the whole point of them.
    #[test]
    fn the_formatting_arguments_reach_the_writers() {
        let root = root();
        build(
            json!({ "path": "a.docx", "content": "Body", "font": "Georgia" }),
            root.clone(),
            &discarding(),
        )
        .expect("write");
        let styles = super::test_support::part(
            &std::fs::read(root.join("a.docx")).expect("read back"),
            "word/styles.xml",
        );
        assert!(styles.contains("Georgia"), "{styles}");

        build(
            json!({
                "path": "a.xlsx",
                "rows": [["Name", "Score"], ["Ada", 42]],
                "sheet": "Q3 [draft]",
                "header": false
            }),
            root.clone(),
            &discarding(),
        )
        .expect("write");
        let bytes = std::fs::read(root.join("a.xlsx")).expect("read back");
        let workbook = super::test_support::part(&bytes, "xl/workbook.xml");
        assert!(workbook.contains("Q3 draft"), "{workbook}");
        // `header: false` is the way out for a sheet that is a grid of values,
        // so there is no frozen row and nothing drawn round the cells.
        let sheet = super::test_support::part(&bytes, "xl/worksheets/sheet1.xml");
        assert!(!sheet.contains("state=\"frozen\""), "{sheet}");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn csv_quotes_only_what_needs_it() {
        let rows = vec![
            vec![Cell::Text("name".into()), Cell::Text("note".into())],
            vec![Cell::Text("Ada".into()), Cell::Text("has, a comma".into())],
        ];
        let text = String::from_utf8(csv_bytes(&rows)).expect("utf8");
        assert!(text.contains("name,note"), "{text}");
        assert!(text.contains("\"has, a comma\""), "{text}");
    }

    #[test]
    fn csv_escapes_a_quote_by_doubling_it() {
        let rows = vec![vec![Cell::Text("say \"hi\"".into())]];
        let text = String::from_utf8(csv_bytes(&rows)).expect("utf8");
        assert!(text.contains("\"say \"\"hi\"\"\""), "{text}");
    }

    /// The docx writer's output is read back by the extractor, so the two are
    /// checked against each other rather than against a fixture: if the writer
    /// produces XML the reader cannot pull text from, that is the bug.
    #[test]
    fn a_written_docx_reads_back_through_the_extractor() {
        let root = root();
        build(
            json!({ "path": "a.docx", "title": "Report", "content": "First line.\nSecond line." }),
            root.clone(),
            &discarding(),
        )
        .expect("write");
        let text = crate::documents::extract(&root.join("a.docx"))
            .expect("a document")
            .expect("read");
        assert!(text.contains("Report"), "{text}");
        assert!(text.contains("First line."), "{text}");
        assert!(text.contains("Second line."), "{text}");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_written_xlsx_reads_back_with_numbers_and_shared_strings() {
        let root = root();
        build(
            json!({ "path": "a.xlsx", "rows": [["Name", "Score"], ["Ada", 42]] }),
            root.clone(),
            &discarding(),
        )
        .expect("write");
        let text = crate::documents::extract(&root.join("a.xlsx"))
            .expect("a document")
            .expect("read");
        assert!(text.contains("Name"), "{text}");
        assert!(text.contains("Ada"), "{text}");
        assert!(text.contains("42"), "{text}");
        let _ = std::fs::remove_dir_all(&root);
    }

    /// A generated PDF has to be a PDF: the test re-parses the bytes rather than
    /// trusting the header.
    #[test]
    fn a_written_pdf_parses() {
        let root = root();
        build(
            json!({ "path": "a.pdf", "title": "Report", "content": "Hello.\nWorld." }),
            root.clone(),
            &discarding(),
        )
        .expect("write");
        let bytes = std::fs::read(root.join("a.pdf")).expect("read back");
        assert!(bytes.starts_with(b"%PDF"), "not a PDF header");
        lopdf::Document::load_mem(&bytes).expect("the PDF must parse");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn the_format_is_inferred_from_the_extension() {
        let root = root();
        build(
            json!({ "path": "notes.md", "content": "body" }),
            root.clone(),
            &discarding(),
        )
        .expect("write");
        assert_eq!(
            std::fs::read_to_string(root.join("notes.md")).unwrap(),
            "body"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn an_xlsx_without_rows_is_refused() {
        let root = root();
        let error = build(
            json!({ "path": "a.xlsx", "content": "text" }),
            root.clone(),
            &discarding(),
        )
        .expect_err("no rows");
        assert!(error.contains("rows"), "{error}");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn an_unknown_format_is_refused() {
        let root = root();
        let error = build(
            json!({ "path": "a.weird", "content": "x" }),
            root.clone(),
            &discarding(),
        )
        .expect_err("no format");
        assert!(
            error.contains("format") || error.contains("extension"),
            "{error}"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_document_outside_the_workspace_is_refused() {
        let root = root();
        assert!(build(
            json!({ "path": "../out.md", "content": "x" }),
            root.clone(),
            &discarding(),
        )
        .expect_err("escape")
        .contains("outside"));
        let _ = std::fs::remove_dir_all(&root);
    }
}
