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
