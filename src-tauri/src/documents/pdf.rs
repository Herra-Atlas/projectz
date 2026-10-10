//! Reading a PDF's text layer.
//!
//! The one format here that needs a real library: a PDF's text is a content
//! stream of drawing operators, so pulling it out means interpreting them.
//! `pdf-extract` is already in the tree and does exactly that.

use std::path::Path;

use super::too_big;

/// Extracts the text layer of a PDF.
pub(super) fn text(path: &Path) -> Result<String, String> {
    if let Some(error) = too_big(path) {
        return Err(error);
    }
    let text = pdf_extract::extract_text(path)
        .map_err(|error| format!("Could not read the PDF {}: {error}", path.display()))?;
    if text.trim().is_empty() {
        // A scanned PDF is images with no text layer. Saying so is more useful
        // than an empty result that reads as "the file is blank".
        return Err(format!(
            "{} has no extractable text. It may be a scan (images only); it cannot be read as text.",
            path.display()
        ));
    }
    Ok(text)
}
