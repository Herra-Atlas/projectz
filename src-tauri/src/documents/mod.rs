//! Reading files that are not plain text.
//!
//! Two callers need this, which is why it is a module of its own rather than a
//! corner of the AI tools: `read_file` pulls a document's text out for a model,
//! and the right panel draws the same file for the user. A second reader for the
//! panel would be a second opinion about what a `.docx` contains.
//!
//! # Two kinds of "not text"
//!
//! A document has a text layer that can be pulled out and handed to the model as
//! text: a PDF, a Word file, a spreadsheet. Extraction lives here.
//!
//! An image has no text layer; the only way to know what is in it is to show it
//! to a model that can see. That needs a vision model, which is a preference the
//! user picks, so it lives with the AI tools -- see `ai/tools/vision.rs`.
//!
//! # Why the extensions, and not content sniffing
//!
//! A `.pdf` and a `.docx` are both detected by name, which is the weaker test.
//! The files being read are ones the user named, so the name is the honest
//! signal; sniffing magic bytes would only move the guess.

mod docx;
mod ooxml;
mod pdf;
#[cfg(test)]
mod test_support;
mod xlsx;

use std::path::Path;

pub use xlsx::SheetGrid;

/// Largest document read in one go.
///
/// A cap because extraction holds the whole file in memory and a PDF can be
/// arbitrarily large; the shared output cap would trim the text afterwards, but
/// not before the read had already loaded all of it.
const MAX_DOCUMENT_BYTES: u64 = 32 * 1024 * 1024;

/// The media type of an image, or `None` for anything else.
///
/// The four types every vision model accepts. An extension not in this list is
/// left to the text path, which will report it as unreadable rather than sending
/// bytes a model cannot decode.
pub fn image_media_type(path: &Path) -> Option<&'static str> {
    match extension(path).as_deref() {
        Some("png") => Some("image/png"),
        Some("jpg") | Some("jpeg") => Some("image/jpeg"),
        Some("webp") => Some("image/webp"),
        Some("gif") => Some("image/gif"),
        _ => None,
    }
}

/// The extracted text of a document, or `None` when the file is not one.
///
/// `None` is the signal to fall back to reading the file as text, so this must
/// return `None` -- not an error -- for every extension that is not a document.
/// Returning an error for a `.txt` would break the common case to serve the rare
/// one.
pub fn extract(path: &Path) -> Option<Result<String, String>> {
    match extension(path).as_deref() {
        Some("pdf") => Some(pdf::text(path)),
        Some("docx") => Some(docx::text(path)),
        Some("xlsx") => Some(xlsx::text(path)),
        _ => None,
    }
}

/// Extracts a document's text from bytes a caller already holds.
///
/// A file pasted into the composer arrives as bytes and a name, with no path:
/// the webview is not given one. Every reader here streams from a path, so the
/// bytes are written to a short-lived temp file carrying the name's extension --
/// which is what tells `extract` which format it has -- and removed either way.
///
/// `None` means the name is not a document, so the caller can pass the bytes on
/// as text instead. The temp file never outlives the call, and its name is
/// unique, so two pastes of the same document cannot read each other's bytes.
pub fn extract_bytes(name: &str, bytes: &[u8]) -> Option<Result<String, String>> {
    let extension = extension(Path::new(name))?;
    if !matches!(extension.as_str(), "pdf" | "docx" | "xlsx") {
        return None;
    }
    let path = std::env::temp_dir().join(format!(
        "projectz-attachment-{}.{extension}",
        uuid::Uuid::new_v4()
    ));
    if let Err(error) = std::fs::write(&path, bytes) {
        return Some(Err(format!(
            "Could not read {name}: the pasted bytes could not be staged ({error})."
        )));
    }
    let result = extract(&path).unwrap_or_else(|| {
        Err(format!(
            "{name} is not a document that can be read as text."
        ))
    });
    let _ = std::fs::remove_file(&path);
    Some(result)
}

/// The sheets of a spreadsheet, as grids rather than as text.
///
/// The panel's view of the same reader `extract` uses: a model wants the values
/// tab-separated, and a person wants them in columns.
pub fn spreadsheet(path: &Path) -> Result<Vec<SheetGrid>, String> {
    xlsx::grid(path)
}

/// The lowercased extension, or `None` for a path without one.
pub(crate) fn extension(path: &Path) -> Option<String> {
    path.extension()
        .map(|extension| extension.to_string_lossy().to_lowercase())
}

/// A refusal for a file too large to read, or `None` when it is fine.
pub(crate) fn too_big(path: &Path) -> Option<String> {
    let size = std::fs::metadata(path).ok()?.len();
    (size > MAX_DOCUMENT_BYTES).then(|| {
        format!(
            "{} is {} MB, larger than the {} MB limit for reading a document.",
            path.display(),
            size / (1024 * 1024),
            MAX_DOCUMENT_BYTES / (1024 * 1024)
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::documents::test_support::{temp, write_zip};

    /// Bytes reach the same reader a path does, which is what makes a pasted
    /// document identical to a dropped one.
    #[test]
    fn bytes_are_extracted_through_the_document_reader() {
        let path = temp("pasted.docx");
        let body = "<w:document><w:body>\
            <w:p><w:r><w:t>Pasted paragraph.</w:t></w:r></w:p>\
            </w:body></w:document>";
        write_zip(&path, &[("word/document.xml", body)]);
        let bytes = std::fs::read(&path).expect("read fixture");
        let _ = std::fs::remove_file(&path);

        let text = extract_bytes("notes.docx", &bytes)
            .expect("a .docx is a document")
            .expect("extracted");
        assert_eq!(text, "Pasted paragraph.");
    }

    /// Anything that is not a document is left to the text path, which is what
    /// keeps a pasted `.txt` from being written to disk and read back.
    #[test]
    fn a_name_that_is_not_a_document_is_not_claimed() {
        assert!(extract_bytes("notes.txt", b"hello").is_none());
        assert!(extract_bytes("README", b"hello").is_none());
    }
}
