//! Reading a Word document's text.
//!
//! A `.docx` is a zip of XML. The body is `word/document.xml`, where runs are
//! `<w:t>` elements grouped into paragraphs by `<w:p>` -- so the paragraph break
//! is restored as a newline, which is the only structure worth keeping. Headings,
//! lists and tables all reach the reader as ordinary paragraphs, which is what a
//! model wants: the words in the order they appear.

use std::path::Path;

use super::ooxml::{read_zip_entry, tag_text};

/// Extracts the text of a Word document.
pub(super) fn text(path: &Path) -> Result<String, String> {
    let xml = read_zip_entry(path, "word/document.xml")?.ok_or_else(|| {
        format!(
            "{} is not a readable .docx (no document body).",
            path.display()
        )
    })?;
    let text = xml
        .split("</w:p>")
        .map(|paragraph| tag_text(paragraph, &["w:t"]).join(""))
        .filter(|paragraph| !paragraph.trim().is_empty())
        .collect::<Vec<_>>()
        .join("\n");
    if text.trim().is_empty() {
        return Err(format!("{} contained no text.", path.display()));
    }
    Ok(text)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::documents::test_support::{temp, write_zip};

    /// Paragraphs are separated and runs within one are joined, which is the only
    /// structure the reader keeps.
    #[test]
    fn a_docx_is_extracted_as_paragraphs() {
        let path = temp("a.docx");
        let body = "<w:document><w:body>\
            <w:p><w:r><w:t>First paragraph.</w:t></w:r></w:p>\
            <w:p><w:r><w:t>Second </w:t></w:r><w:r><w:t>line.</w:t></w:r></w:p>\
            </w:body></w:document>";
        write_zip(&path, &[("word/document.xml", body)]);
        let text = text(&path).expect("read");
        assert!(text.contains("First paragraph."), "{text}");
        assert!(text.contains("Second line."), "{text}");
        assert_eq!(text.lines().count(), 2, "{text}");
        let _ = std::fs::remove_file(&path);
    }

    /// The styles this app writes live in `w:pPr`, beside the runs rather than in
    /// them, so a styled paragraph must still read as its text.
    #[test]
    fn a_styled_paragraph_reads_as_its_text() {
        let path = temp("styled.docx");
        let body = "<w:document><w:body><w:p>\
            <w:pPr><w:pStyle w:val=\"Heading1\"/></w:pPr>\
            <w:r><w:rPr><w:b/></w:rPr><w:t>Findings</w:t></w:r>\
            </w:p></w:body></w:document>";
        write_zip(&path, &[("word/document.xml", body)]);
        assert_eq!(text(&path).expect("read"), "Findings");
        let _ = std::fs::remove_file(&path);
    }

    /// A zip with no document part is not a Word file, and saying so is better
    /// than an empty string that reads as an empty document.
    #[test]
    fn a_zip_without_a_body_is_refused() {
        let path = temp("empty.docx");
        write_zip(&path, &[("word/styles.xml", "<w:styles/>")]);
        let error = text(&path).expect_err("no body");
        assert!(error.contains("no document body"), "{error}");
        let _ = std::fs::remove_file(&path);
    }
}
