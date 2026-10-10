//! The parts of Office Open XML that both writers need.
//!
//! A `.docx` and an `.xlsx` are the same shape: a zip of XML parts, referenced
//! from `[Content_Types].xml` and wired together by `.rels` files. The escaping,
//! the zip building and the column arithmetic are identical between them, so they
//! live here rather than being written twice and drifting.
//!
//! Nothing here knows what a document or a sheet *is* -- see `docx.rs` and
//! `xlsx.rs` for that.

use std::io::Write;

/// Escapes the five characters that end an XML text node early.
pub(crate) fn xml_escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

/// Formats a number without a trailing `.0` for whole values.
pub(crate) fn number_to_string(number: f64) -> String {
    if number.fract() == 0.0 && number.abs() < 1e15 {
        format!("{}", number as i64)
    } else {
        number.to_string()
    }
}

/// The spreadsheet column name for a 1-based index: 1 -> `A`, 27 -> `AA`.
pub(crate) fn column_letters(mut column: usize) -> String {
    let mut name = String::new();
    while column > 0 {
        let remainder = (column - 1) % 26;
        name.insert(0, (b'A' + remainder as u8) as char);
        column = (column - 1) / 26;
    }
    name
}

/// The point size of 11pt, in the half-points Word stores sizes in.
pub(crate) const BODY_HALF_POINTS: u32 = 22;

/// The twips (1/20 pt) of an inch-wide margin, for the page setup.
pub(crate) const PAGE_MARGIN_TWIPS: u32 = 1440;

/// Builds a zip archive in memory from named parts.
///
/// Entries are bytes rather than text because a `.docx` carries its images in the
/// same archive, and a picture is not text.
pub(crate) fn zip_parts(entries: &[(&str, &[u8])]) -> Vec<u8> {
    let mut cursor = std::io::Cursor::new(Vec::new());
    {
        let mut zip = zip::ZipWriter::new(&mut cursor);
        let options: zip::write::FileOptions<'_, ()> =
            zip::write::FileOptions::default().compression_method(zip::CompressionMethod::Deflated);
        for (name, body) in entries {
            zip.start_file(*name, options).expect("start an entry");
            zip.write_all(body).expect("write an entry");
        }
        zip.finish().expect("finish the archive");
    }
    cursor.into_inner()
}

/// Runs a list of zip entries through [`zip_parts`], escaping nothing.
///
/// A small adapter so a caller holding `String` parts does not have to build the
/// `&[u8]` list by hand.
pub(crate) fn zip_text_parts(entries: &[(&str, String)]) -> Vec<u8> {
    let borrowed: Vec<(&str, &[u8])> = entries
        .iter()
        .map(|(name, body)| (*name, body.as_bytes()))
        .collect();
    zip_parts(&borrowed)
}

/// The XML declaration every part begins with.
pub(crate) const XML_HEADER: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>";

/// The namespace attribute a Word document's root element opens with.
pub(crate) const WORD_NAMESPACE: &str =
    "xmlns:w=\"http://schemas.openxmlformats.org/wordprocessingml/2006/main\"";

/// The namespaces a document needs once it may contain a drawing.
///
/// Word resolves `w:` on its own, but the drawing namespaces are what a picture
/// is expressed in; a document that declares them and holds no picture is still
/// valid, so they are declared only when an image is actually embedded.
pub(crate) const DRAWING_NAMESPACES: &str =
    "xmlns:r=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships\" \
xmlns:wp=\"http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing\" \
xmlns:a=\"http://schemas.openxmlformats.org/drawingml/2006/main\" \
xmlns:pic=\"http://schemas.openxmlformats.org/drawingml/2006/picture\"";

#[cfg(test)]
mod tests {
    use super::*;

    /// The inverse has to agree with the forward direction, because a cell
    /// reference is written with one and read back with the other. The reading
    /// side lives in `documents::ooxml`; these values are the shared contract.
    #[test]
    fn the_written_letters_are_the_ones_the_reader_expects() {
        assert_eq!(column_letters(1), "A");
        assert_eq!(column_letters(26), "Z");
        assert_eq!(column_letters(27), "AA");
        assert_eq!(column_letters(52), "AZ");
        assert_eq!(column_letters(53), "BA");
    }

    #[test]
    fn the_five_xml_characters_are_escaped() {
        assert_eq!(
            xml_escape("a & b <c> \"d\" 'e'"),
            "a &amp; b &lt;c&gt; &quot;d&quot; &apos;e&apos;"
        );
    }

    #[test]
    fn whole_numbers_lose_their_trailing_zero() {
        assert_eq!(number_to_string(42.0), "42");
        assert_eq!(number_to_string(42.5), "42.5");
    }
}
