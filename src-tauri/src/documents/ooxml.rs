//! The zip-and-XML primitives every Office reader needs.
//!
//! A `.docx` and an `.xlsx` are the same shape -- a zip of XML parts -- so
//! opening the archive, listing what is in it and pulling text out of a fragment
//! are shared rather than written twice. All of it is a scanner, not a parser:
//! Office XML is regular enough that the text is always between `>` and the next
//! `<`, and an XML library would be a dependency for a substring.

use std::io::Read;
use std::path::Path;

use super::too_big;

/// Reads one entry out of a zip archive as text.
pub(crate) fn read_zip_entry(path: &Path, entry: &str) -> Result<Option<String>, String> {
    if let Some(error) = too_big(path) {
        return Err(error);
    }
    let file = std::fs::File::open(path)
        .map_err(|error| format!("Could not open {}: {error}", path.display()))?;
    let mut archive = zip::ZipArchive::new(file)
        .map_err(|error| format!("{} is not a readable Office file: {error}", path.display()))?;
    let Ok(mut handle) = archive.by_name(entry) else {
        return Ok(None);
    };
    let mut text = String::new();
    handle
        .read_to_string(&mut text)
        .map_err(|error| format!("Could not read {entry} in {}: {error}", path.display()))?;
    Ok(Some(text))
}

/// Every entry name in a zip archive.
pub(crate) fn zip_entry_names(path: &Path) -> Result<Vec<String>, String> {
    let file = std::fs::File::open(path)
        .map_err(|error| format!("Could not open {}: {error}", path.display()))?;
    let archive = zip::ZipArchive::new(file)
        .map_err(|error| format!("{} is not a readable Office file: {error}", path.display()))?;
    Ok(archive.file_names().map(str::to_string).collect())
}

/// The text of every occurrence of each named tag in a fragment of XML.
///
/// Closing tags are skipped by name so `</w:t>` does not contribute an empty
/// value -- which would double every piece of text in the document.
pub(crate) fn tag_text(xml: &str, tags: &[&str]) -> Vec<String> {
    let mut out = Vec::new();
    for chunk in xml.split('<') {
        let Some((head, rest)) = chunk.split_once('>') else {
            continue;
        };
        if head.starts_with('/') {
            continue;
        }
        let name = head.split_whitespace().next().unwrap_or("");
        if !tags.contains(&name) {
            continue;
        }
        if let Some(text) = rest.split('<').next() {
            if !text.is_empty() {
                out.push(decode_entities(text));
            }
        }
    }
    out
}

/// One attribute's value from a tag's own text, if it is there.
///
/// The leading-boundary check is what keeps a `name="` search from matching the
/// tail of `sheetName="`: an attribute has to start at the beginning of the tag
/// or after whitespace.
pub(crate) fn attribute(tag: &str, name: &str) -> Option<String> {
    let needle = format!("{name}=\"");
    let mut from = 0;
    while let Some(offset) = tag[from..].find(&needle) {
        let start = from + offset;
        let boundary = start == 0
            || tag[..start]
                .chars()
                .next_back()
                .is_some_and(char::is_whitespace);
        if boundary {
            return tag[start + needle.len()..]
                .split('"')
                .next()
                .map(str::to_string);
        }
        from = start + 1;
    }
    None
}

/// Undoes the five XML entities that appear in document text.
pub(crate) fn decode_entities(text: &str) -> String {
    text.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&amp;", "&")
}

/// The zero-based column index for a cell reference's letters: `A` -> 0,
/// `AA` -> 26.
///
/// The inverse of the writer's `column_letters`, and it lives here because this
/// is the side that reads a reference: a cell states where it is, and a row that
/// skips a column -- which is what an empty middle column is -- states nothing
/// about the gap.
pub(crate) fn column_index(letters: &str) -> Option<usize> {
    let mut index = 0usize;
    for character in letters.chars() {
        if !character.is_ascii_alphabetic() {
            return None;
        }
        index = index * 26 + (character.to_ascii_uppercase() as usize - 'A' as usize + 1);
    }
    (index > 0).then(|| index - 1)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The scanner takes the text and skips the closing tag, which is what keeps
    /// a document's text from being doubled.
    #[test]
    fn xml_text_is_read_between_tags() {
        let xml = "<w:p><w:t>Hello</w:t><w:t xml:space=\"preserve\"> world</w:t></w:p>";
        assert_eq!(tag_text(xml, &["w:t"]).join(""), "Hello world");
    }

    #[test]
    fn text_entities_are_decoded() {
        assert_eq!(decode_entities("a &amp; b &lt;c&gt;"), "a & b <c>");
    }

    /// `name` must not match `sheetName`, which is the whole reason the boundary
    /// check exists.
    #[test]
    fn an_attribute_matches_only_its_own_name() {
        let tag = r#"<sheet name="Q3" sheetId="1" r:id="rId4"/>"#;
        assert_eq!(attribute(tag, "name"), Some("Q3".to_string()));
        assert_eq!(attribute(tag, "r:id"), Some("rId4".to_string()));
        assert_eq!(attribute(tag, "id"), None);
        assert_eq!(attribute(tag, "missing"), None);
    }

    /// A reference's letters decide the column, so the first one is 0 and the
    /// twenty-seventh is 26 -- the same numbering the writer's letters produce.
    #[test]
    fn a_reference_names_its_column() {
        assert_eq!(column_index("A"), Some(0));
        assert_eq!(column_index("Z"), Some(25));
        assert_eq!(column_index("AA"), Some(26));
        assert_eq!(column_index("AZ"), Some(51));
        assert_eq!(column_index("A1"), None);
        assert_eq!(column_index(""), None);
    }
}
