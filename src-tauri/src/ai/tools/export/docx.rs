//! Writing a Word document that looks like someone made it on purpose.
//!
//! A `.docx` is a zip of XML parts. The previous writer emitted a body of
//! unstyled paragraphs, which Word renders in its own default 11pt body text --
//! legible, and indistinguishable from a plain text file. This one writes the
//! parts that carry appearance: a `styles.xml` holding the named styles the body
//! refers to, a `numbering.xml` that makes lists actually list, a
//! `document.xml` whose runs carry their own emphasis, and `word/media/` with any
//! pictures that were asked for.
//!
//! # Why styles rather than direct formatting
//!
//! Every appearance could be written onto each paragraph instead. Styles are used
//! because they are what makes the document *editable*: opening it in Word shows
//! "Heading 1" in the style box, so the user can restyle the whole document by
//! changing one style, and the navigation pane works. Direct formatting produces
//! a document that merely looks right.
//!
//! # Coordinate systems
//!
//! Word measures page furniture in twips (1/20 pt, 1440 to the inch) and pictures
//! in EMU (914400 to the inch). Both are written as bare integers with no unit;
//! mixing them up is the classic way to produce a document that opens wrong, so
//! the constants here name the unit they are in.

use std::path::Path;

use super::blocks::{Block, Run};
use super::image;
use super::ooxml::{self, xml_escape};

/// The point size a code run is written at, in half-points.
const CODE_HALF_POINTS: u32 = 19;

/// The style a picture's paragraph carries, so it is centred and spaced apart.
const PICTURE_STYLE: &str = "Picture";

/// The text column's width in twips: US Letter (12240) less two one-inch
/// margins.
const TABLE_WIDTH_TWIPS: u32 = 12_240 - 2 * 1_440;

/// The deepest heading level `styles.xml` defines.
///
/// A generated document has no use for a sixth-level heading, and a style that
/// does not exist would leave the paragraph unstyled rather than merely smaller.
const MAX_HEADING: u8 = 4;

/// One picture collected for the package.
struct Media {
    /// The part name inside `word/`, for example `media/image1.png`.
    part: String,
    content_type: &'static str,
    extension: &'static str,
    bytes: Vec<u8>,
}

/// Builds the whole package.
///
/// `root` is the workspace, which every picture's path is resolved against --
/// through the same containment check the file tools use, so a document cannot
/// embed something outside the workspace any more than `read_file` could read it.
pub(crate) fn bytes(
    title: Option<&str>,
    blocks: &[Block],
    font: &str,
    root: &Path,
) -> Result<Vec<u8>, String> {
    let mut body = String::new();
    if let Some(title) = title {
        body.push_str(&paragraph(Some("Title"), &[Run::text(title)], ""));
    }

    let mut media: Vec<Media> = Vec::new();
    for block in blocks {
        match block {
            Block::Heading { level, runs } => {
                let level = (*level).clamp(1, MAX_HEADING);
                body.push_str(&paragraph(Some(&format!("Heading{level}")), runs, ""));
            }
            Block::Paragraph(runs) => body.push_str(&paragraph(None, runs, "")),
            Block::List { ordered, items } => {
                // Two numbering definitions are declared in `numbering.xml`: one
                // bulleted, one decimal. Which list uses which is the only thing
                // this picks.
                let number_id = if *ordered { 2 } else { 1 };
                for item in items {
                    let properties = format!(
                        "<w:numPr><w:ilvl w:val=\"0\"/><w:numId w:val=\"{number_id}\"/></w:numPr>"
                    );
                    body.push_str(&paragraph(Some("ListParagraph"), item, &properties));
                }
            }
            Block::Quote(runs) => body.push_str(&paragraph(Some("Quote"), runs, "")),
            Block::Code { lines } => {
                // One paragraph per line rather than one paragraph with breaks:
                // it is what the style's spacing expects, and what a reader
                // pulling text back out of the file sees as separate lines.
                for line in lines {
                    body.push_str(&paragraph(Some("CodeBlock"), &[Run::text(line)], ""));
                }
            }
            Block::Table { head, rows } => body.push_str(&table(head.as_deref(), rows)),
            Block::Rule => body.push_str(&paragraph(Some("Rule"), &[], "")),
            Block::Image { alt, target } => {
                let (drawing, media_part) = picture(alt, target, root, media.len() + 1)?;
                media.push(media_part);
                // The drawing is a run, not a paragraph property: it sits in the
                // body beside the text runs, and the style is what centres it.
                body.push_str(&format!(
                    "<w:p><w:pPr><w:pStyle w:val=\"{PICTURE_STYLE}\"/></w:pPr>{drawing}</w:p>"
                ));
            }
        }
    }

    body.push_str(&section_properties());

    // The drawing namespaces are declared only when there is a drawing to name.
    // A picture element without them is not a document Word will open.
    let drawing = if media.is_empty() {
        String::new()
    } else {
        format!(" {}", ooxml::DRAWING_NAMESPACES)
    };
    let document = format!(
        "{header}<w:document {namespace}{drawing}><w:body>{body}</w:body></w:document>",
        header = ooxml::XML_HEADER,
        namespace = ooxml::WORD_NAMESPACE,
    );

    let mut parts: Vec<(String, Vec<u8>)> = vec![
        (
            "[Content_Types].xml".to_string(),
            content_types(&media).into_bytes(),
        ),
        (
            "_rels/.rels".to_string(),
            package_rels().to_string().into_bytes(),
        ),
        ("word/document.xml".to_string(), document.into_bytes()),
        ("word/styles.xml".to_string(), styles(font).into_bytes()),
        (
            "word/numbering.xml".to_string(),
            NUMBERING.as_bytes().to_vec(),
        ),
        (
            "word/_rels/document.xml.rels".to_string(),
            document_rels(&media).into_bytes(),
        ),
    ];
    for part in &media {
        parts.push((format!("word/{}", part.part), part.bytes.clone()));
    }

    let borrowed: Vec<(&str, &[u8])> = parts
        .iter()
        .map(|(name, body)| (name.as_str(), body.as_slice()))
        .collect();
    Ok(ooxml::zip_parts(&borrowed))
}

/// One paragraph: an optional style, optional extra paragraph properties, and
/// its runs.
fn paragraph(style: Option<&str>, runs: &[Run], extra: &str) -> String {
    let mut properties = String::new();
    if let Some(style) = style {
        properties.push_str(&format!("<w:pStyle w:val=\"{style}\"/>"));
    }
    properties.push_str(extra);

    let mut out = String::from("<w:p>");
    if !properties.is_empty() {
        out.push_str("<w:pPr>");
        out.push_str(&properties);
        out.push_str("</w:pPr>");
    }
    for run in runs {
        out.push_str(&run_xml(run));
    }
    out.push_str("</w:p>");
    out
}

/// One run, with only the properties it actually needs.
///
/// An unstyled run is written without a `w:rPr` at all: an empty property block
/// is not wrong, but it is XML that says nothing and makes the part harder to
/// read when something has gone wrong.
fn run_xml(run: &Run) -> String {
    let mut properties = String::new();
    if run.code {
        properties.push_str(
            "<w:rFonts w:ascii=\"Consolas\" w:hAnsi=\"Consolas\"/>\
             <w:shd w:val=\"clear\" w:color=\"auto\" w:fill=\"F3F4F6\"/>",
        );
    }
    if run.bold {
        properties.push_str("<w:b/>");
    }
    if run.italic {
        properties.push_str("<w:i/>");
    }
    let text = xml_escape(&run.text);
    if properties.is_empty() {
        format!("<w:r><w:t xml:space=\"preserve\">{text}</w:t></w:r>")
    } else {
        format!("<w:r><w:rPr>{properties}</w:rPr><w:t xml:space=\"preserve\">{text}</w:t></w:r>")
    }
}

/// A table with a shaded header row and thin borders.
///
/// Borders are written per table rather than defined as a table style: a style
/// lives in `styles.xml`, needs a `tblStyle` reference and a matching `w:style`
/// of type `table`, and for one table shape that is three places to keep in step
/// for no gain.
fn table(head: Option<&[String]>, rows: &[Vec<String>]) -> String {
    let columns = head
        .map(<[String]>::len)
        .into_iter()
        .chain(rows.iter().map(Vec::len))
        .max()
        .unwrap_or(0);
    if columns == 0 {
        return String::new();
    }
    // The text width of a US Letter page with one-inch margins, in twips. The
    // grid is divided evenly because column *content* widths are Word's job to
    // compute from the rendered text; guessing at them here would fight it.
    let width = TABLE_WIDTH_TWIPS;
    let column_width = width / columns as u32;

    let mut out = String::from(
        "<w:tbl><w:tblPr><w:tblW w:w=\"0\" w:type=\"auto\"/>\
         <w:tblBorders>\
         <w:top w:val=\"single\" w:sz=\"4\" w:space=\"0\" w:color=\"D1D5DB\"/>\
         <w:left w:val=\"single\" w:sz=\"4\" w:space=\"0\" w:color=\"D1D5DB\"/>\
         <w:bottom w:val=\"single\" w:sz=\"4\" w:space=\"0\" w:color=\"D1D5DB\"/>\
         <w:right w:val=\"single\" w:sz=\"4\" w:space=\"0\" w:color=\"D1D5DB\"/>\
         <w:insideH w:val=\"single\" w:sz=\"4\" w:space=\"0\" w:color=\"D1D5DB\"/>\
         <w:insideV w:val=\"single\" w:sz=\"4\" w:space=\"0\" w:color=\"D1D5DB\"/>\
         </w:tblBorders></w:tblPr><w:tblGrid>",
    );
    for _ in 0..columns {
        out.push_str(&format!("<w:gridCol w:w=\"{column_width}\"/>"));
    }
    out.push_str("</w:tblGrid>");

    if let Some(head) = head {
        out.push_str(&table_row(head, true));
    }
    for row in rows {
        out.push_str(&table_row(row, false));
    }
    out.push_str("</w:tbl>");
    out
}

/// One table row, with every run emboldened when it is the header.
fn table_row(cells: &[String], header: bool) -> String {
    let mut out = String::from("<w:tr>");
    for cell in cells {
        out.push_str("<w:tc><w:tcPr><w:tcW w:w=\"0\" w:type=\"auto\"/>");
        if header {
            // Shaded rather than merely bold: at a glance the eye finds the
            // header row by its block of colour, not by reading it.
            out.push_str("<w:shd w:val=\"clear\" w:color=\"auto\" w:fill=\"F3F4F6\"/>");
        }
        out.push_str("</w:tcPr>");
        // Cells go through the inline scanner, so a cell may carry its own
        // emphasis without the table needing a second syntax. An empty cell
        // still gets its paragraph -- a cell with no block in it is not valid --
        // and that is what the loop leaves behind when there are no runs.
        let runs = super::blocks::inline(cell);
        out.push_str("<w:p><w:pPr><w:spacing w:after=\"0\"/></w:pPr>");
        for run in &runs {
            out.push_str(&run_xml(&Run {
                bold: run.bold || header,
                ..run.clone()
            }));
        }
        out.push_str("</w:p></w:tc>");
    }
    out.push_str("</w:tr>");
    out
}

/// A picture: the drawing XML for the body, and the media part to carry it.
///
/// `index` is 1-based and becomes both the file name and the relationship id, so
/// the body and the `.rels` part can be built from the same list without a second
/// numbering scheme.
fn picture(alt: &str, target: &str, root: &Path, index: usize) -> Result<(String, Media), String> {
    let resolved = super::super::file::resolve_within(root, target)?;
    let extension = image::extension_for(&resolved)
        .ok_or_else(|| format!("`{target}` cannot be embedded: use a .png, .jpg or .gif image."))?;
    let content_type =
        crate::documents::image_media_type(&resolved).expect("extension_for knows the same types");
    let bytes = std::fs::read(&resolved)
        .map_err(|error| format!("Could not read the image `{target}`: {error}"))?;
    let size = image::dimensions(&bytes).ok_or_else(|| {
        format!("Could not read the size of `{target}`; it may not be a valid {extension} file.")
    })?;

    // Scaled down to the text width when it is wider, never up: enlarging a small
    // picture only makes it blurry.
    let mut cx = image::emu(size.width);
    let mut cy = image::emu(size.height);
    if cx > image::TEXT_WIDTH_EMU {
        cy = cy * image::TEXT_WIDTH_EMU / cx;
        cx = image::TEXT_WIDTH_EMU;
    }

    // Two relationships are taken by `styles.xml` and `numbering.xml`, so the
    // first picture is `rId3`. See `document_rels`.
    let relationship = format!("rId{}", index + 2);
    let name = format!("Picture {index}");
    let file_name = format!("image{index}.{extension}");
    let drawing = format!(
        "<w:r><w:drawing><wp:inline distT=\"0\" distB=\"0\" distL=\"0\" distR=\"0\">\
         <wp:extent cx=\"{cx}\" cy=\"{cy}\"/>\
         <wp:docPr id=\"{index}\" name=\"{name}\" descr=\"{alt_escaped}\"/>\
         <a:graphic><a:graphicData uri=\"http://schemas.openxmlformats.org/drawingml/2006/picture\">\
         <pic:pic><pic:nvPicPr><pic:cNvPr id=\"{index}\" name=\"{file_name}\"/><pic:cNvPicPr/></pic:nvPicPr>\
         <pic:blipFill><a:blip r:embed=\"{relationship}\"/><a:stretch><a:fillRect/></a:stretch></pic:blipFill>\
         <pic:spPr><a:xfrm><a:off x=\"0\" y=\"0\"/><a:ext cx=\"{cx}\" cy=\"{cy}\"/></a:xfrm>\
         <a:prstGeom prst=\"rect\"><a:avLst/></a:prstGeom></pic:spPr>\
         </pic:pic></a:graphicData></a:graphic></wp:inline></w:drawing></w:r>",
        alt_escaped = xml_escape(alt),
    );

    Ok((
        drawing,
        Media {
            part: format!("media/{file_name}"),
            content_type,
            extension,
            bytes,
        },
    ))
}

/// The page setup at the end of the body: US Letter with one-inch margins.
fn section_properties() -> String {
    format!(
        "<w:sectPr><w:pgSz w:w=\"12240\" w:h=\"15840\"/>\
         <w:pgMar w:top=\"{margin}\" w:right=\"{margin}\" w:bottom=\"{margin}\" \
         w:left=\"{margin}\" w:header=\"720\" w:footer=\"720\" w:gutter=\"0\"/></w:sectPr>",
        margin = ooxml::PAGE_MARGIN_TWIPS,
    )
}

/// The package's content types: the parts this writer produces, and one default
/// per image extension actually used.
fn content_types(media: &[Media]) -> String {
    let mut defaults = String::new();
    let mut seen: Vec<&str> = Vec::new();
    for part in media {
        if seen.contains(&part.extension) {
            continue;
        }
        seen.push(part.extension);
        defaults.push_str(&format!(
            "<Default Extension=\"{}\" ContentType=\"{}\"/>",
            part.extension, part.content_type
        ));
    }

    format!(
        "{header}\
<Types xmlns=\"http://schemas.openxmlformats.org/package/2006/content-types\">\
<Default Extension=\"rels\" ContentType=\"application/vnd.openxmlformats-package.relationships+xml\"/>\
<Default Extension=\"xml\" ContentType=\"application/xml\"/>{defaults}\
<Override PartName=\"/word/document.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml\"/>\
<Override PartName=\"/word/styles.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.wordprocessingml.styles+xml\"/>\
<Override PartName=\"/word/numbering.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.wordprocessingml.numbering+xml\"/>\
</Types>",
        header = ooxml::XML_HEADER,
    )
}

/// The package root relationships: one pointer to the document part.
fn package_rels() -> String {
    format!(
        "{header}\
<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">\
<Relationship Id=\"rId1\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument\" Target=\"word/document.xml\"/>\
</Relationships>",
        header = ooxml::XML_HEADER,
    )
}

/// The document's own relationships: styles, numbering, then one per picture.
///
/// The ids are positional, and `picture` builds its `r:embed` from the same
/// arithmetic -- the two must agree or Word reports a broken link instead of
/// drawing the image.
fn document_rels(media: &[Media]) -> String {
    let mut out = format!(
        "{header}\
<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">\
<Relationship Id=\"rId1\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles\" Target=\"styles.xml\"/>\
<Relationship Id=\"rId2\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/numbering\" Target=\"numbering.xml\"/>",
        header = ooxml::XML_HEADER,
    );
    for (index, part) in media.iter().enumerate() {
        out.push_str(&format!(
            "<Relationship Id=\"rId{id}\" \
             Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/image\" \
             Target=\"{target}\"/>",
            id = index + 3,
            target = part.part,
        ));
    }
    out.push_str("</Relationships>");
    out
}

/// The style definitions the body refers to.
///
/// `docDefaults` sets the document's font, which is what makes the `font`
/// argument work: every style that does not name a font of its own inherits it,
/// so one attribute restyles the whole document.
fn styles(font: &str) -> String {
    let font = xml_escape(font);
    format!(
        "{header}\
<w:styles {namespace}>\
<w:docDefaults>\
<w:rPrDefault><w:rPr><w:rFonts w:ascii=\"{font}\" w:hAnsi=\"{font}\" w:cs=\"{font}\"/>\
<w:sz w:val=\"{body}\"/><w:szCs w:val=\"{body}\"/></w:rPr></w:rPrDefault>\
<w:pPrDefault><w:pPr><w:spacing w:after=\"160\" w:line=\"276\" w:lineRule=\"auto\"/></w:pPr></w:pPrDefault>\
</w:docDefaults>\
<w:style w:type=\"paragraph\" w:default=\"1\" w:styleId=\"Normal\"><w:name w:val=\"Normal\"/></w:style>\
<w:style w:type=\"paragraph\" w:styleId=\"Title\"><w:name w:val=\"Title\"/><w:basedOn w:val=\"Normal\"/>\
<w:pPr><w:spacing w:before=\"0\" w:after=\"280\"/></w:pPr>\
<w:rPr><w:b/><w:sz w:val=\"34\"/><w:color w:val=\"1F2937\"/></w:rPr></w:style>\
<w:style w:type=\"paragraph\" w:styleId=\"Heading1\"><w:name w:val=\"heading 1\"/><w:basedOn w:val=\"Normal\"/>\
<w:pPr><w:keepNext/><w:spacing w:before=\"320\" w:after=\"140\"/></w:pPr>\
<w:rPr><w:b/><w:sz w:val=\"28\"/><w:color w:val=\"111827\"/></w:rPr></w:style>\
<w:style w:type=\"paragraph\" w:styleId=\"Heading2\"><w:name w:val=\"heading 2\"/><w:basedOn w:val=\"Normal\"/>\
<w:pPr><w:keepNext/><w:spacing w:before=\"280\" w:after=\"120\"/></w:pPr>\
<w:rPr><w:b/><w:sz w:val=\"26\"/><w:color w:val=\"1F2937\"/></w:rPr></w:style>\
<w:style w:type=\"paragraph\" w:styleId=\"Heading3\"><w:name w:val=\"heading 3\"/><w:basedOn w:val=\"Normal\"/>\
<w:pPr><w:keepNext/><w:spacing w:before=\"240\" w:after=\"100\"/></w:pPr>\
<w:rPr><w:b/><w:sz w:val=\"24\"/><w:color w:val=\"374151\"/></w:rPr></w:style>\
<w:style w:type=\"paragraph\" w:styleId=\"Heading4\"><w:name w:val=\"heading 4\"/><w:basedOn w:val=\"Normal\"/>\
<w:pPr><w:keepNext/><w:spacing w:before=\"200\" w:after=\"80\"/></w:pPr>\
<w:rPr><w:b/><w:sz w:val=\"{body}\"/><w:color w:val=\"4B5563\"/></w:rPr></w:style>\
<w:style w:type=\"paragraph\" w:styleId=\"Quote\"><w:name w:val=\"Quote\"/><w:basedOn w:val=\"Normal\"/>\
<w:pPr><w:ind w:left=\"360\"/><w:pBdr><w:left w:val=\"single\" w:sz=\"18\" w:space=\"10\" w:color=\"D1D5DB\"/></w:pBdr>\
<w:spacing w:after=\"160\"/></w:pPr>\
<w:rPr><w:i/><w:color w:val=\"4B5563\"/></w:rPr></w:style>\
<w:style w:type=\"paragraph\" w:styleId=\"CodeBlock\"><w:name w:val=\"Code Block\"/><w:basedOn w:val=\"Normal\"/>\
<w:pPr><w:spacing w:after=\"0\" w:line=\"240\" w:lineRule=\"auto\"/>\
<w:shd w:val=\"clear\" w:color=\"auto\" w:fill=\"F9FAFB\"/></w:pPr>\
<w:rPr><w:rFonts w:ascii=\"Consolas\" w:hAnsi=\"Consolas\"/><w:sz w:val=\"{code}\"/></w:rPr></w:style>\
<w:style w:type=\"paragraph\" w:styleId=\"ListParagraph\"><w:name w:val=\"List Paragraph\"/><w:basedOn w:val=\"Normal\"/>\
<w:pPr><w:spacing w:after=\"80\"/></w:pPr></w:style>\
<w:style w:type=\"paragraph\" w:styleId=\"Picture\"><w:name w:val=\"Picture\"/><w:basedOn w:val=\"Normal\"/>\
<w:pPr><w:jc w:val=\"center\"/><w:spacing w:before=\"160\" w:after=\"200\"/></w:pPr></w:style>\
<w:style w:type=\"paragraph\" w:styleId=\"Rule\"><w:name w:val=\"Rule\"/><w:basedOn w:val=\"Normal\"/>\
<w:pPr><w:pBdr><w:bottom w:val=\"single\" w:sz=\"6\" w:space=\"1\" w:color=\"D1D5DB\"/></w:pBdr>\
<w:spacing w:before=\"120\" w:after=\"200\"/></w:pPr></w:style>\
</w:styles>",
        header = ooxml::XML_HEADER,
        namespace = ooxml::WORD_NAMESPACE,
        body = ooxml::BODY_HALF_POINTS,
        code = CODE_HALF_POINTS,
    )
}

/// Two list definitions -- one bulleted, one decimal -- that `ListParagraph`
/// paragraphs point at.
///
/// Without this part a list is a set of indented paragraphs with no marker at
/// all: the numbering is what draws the bullet, not the style.
const NUMBERING: &str = concat!(
    "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>",
    "<w:numbering xmlns:w=\"http://schemas.openxmlformats.org/wordprocessingml/2006/main\">",
    "<w:abstractNum w:abstractNumId=\"0\"><w:multiLevelType w:val=\"hybridMultilevel\"/>",
    "<w:lvl w:ilvl=\"0\"><w:start w:val=\"1\"/><w:numFmt w:val=\"bullet\"/>",
    "<w:lvlText w:val=\"\u{2022}\"/><w:lvlJc w:val=\"left\"/>",
    "<w:pPr><w:ind w:left=\"720\" w:hanging=\"360\"/></w:pPr>",
    "<w:rPr><w:rFonts w:ascii=\"Arial\" w:hAnsi=\"Arial\" w:hint=\"default\"/></w:rPr></w:lvl>",
    "</w:abstractNum>",
    "<w:abstractNum w:abstractNumId=\"1\"><w:multiLevelType w:val=\"hybridMultilevel\"/>",
    "<w:lvl w:ilvl=\"0\"><w:start w:val=\"1\"/><w:numFmt w:val=\"decimal\"/>",
    "<w:lvlText w:val=\"%1.\"/><w:lvlJc w:val=\"left\"/>",
    "<w:pPr><w:ind w:left=\"720\" w:hanging=\"360\"/></w:pPr></w:lvl>",
    "</w:abstractNum>",
    "<w:num w:numId=\"1\"><w:abstractNumId w:val=\"0\"/></w:num>",
    "<w:num w:numId=\"2\"><w:abstractNumId w:val=\"1\"/></w:num>",
    "</w:numbering>"
);

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai::tools::export::test_support::{names, part, png, temp_root as root};

    #[test]
    fn a_document_carries_its_styles_and_numbering() {
        let blocks = super::super::blocks::parse("# Title\n\n- one\n- two");
        let bytes = bytes(Some("Report"), &blocks, "Calibri", &root()).expect("write");
        let document = part(&bytes, "word/document.xml");
        assert!(
            document.contains("<w:pStyle w:val=\"Title\"/>"),
            "{document}"
        );
        assert!(
            document.contains("<w:pStyle w:val=\"Heading1\"/>"),
            "{document}"
        );
        assert!(document.contains("<w:numId w:val=\"1\"/>"), "{document}");
        // The style the body points at has to exist, or Word shows the paragraph
        // in body text and the numbering never draws.
        let styles = part(&bytes, "word/styles.xml");
        assert!(styles.contains("w:styleId=\"Heading1\""), "{styles}");
        assert!(part(&bytes, "word/numbering.xml").contains("w:numId=\"1\""));
    }

    /// The document's font is applied once, in `docDefaults`, so every style that
    /// does not name its own inherits it.
    #[test]
    fn the_chosen_font_reaches_the_defaults() {
        let bytes = bytes(None, &[], "Georgia", &root()).expect("write");
        let styles = part(&bytes, "word/styles.xml");
        assert!(styles.contains("w:ascii=\"Georgia\""), "{styles}");
    }

    #[test]
    fn a_table_is_a_real_table_with_a_shaded_header() {
        let blocks = super::super::blocks::parse("| Name | Score |\n|---|---|\n| Ada | 42 |");
        let bytes = bytes(None, &blocks, "Calibri", &root()).expect("write");
        let document = part(&bytes, "word/document.xml");
        assert!(document.contains("<w:tbl>"), "{document}");
        assert!(document.contains("w:fill=\"F3F4F6\""), "{document}");
        assert!(document.contains(">Name<"), "{document}");
        assert!(document.contains(">42<"), "{document}");
    }

    /// A picture has to arrive three ways at once: in the body as a drawing, in
    /// the package as a media part, and in the content types so Word knows what
    /// the part is. Any one missing produces a document that opens with a
    /// placeholder or not at all.
    #[test]
    fn a_picture_is_embedded_in_all_three_places() {
        let root = root();
        std::fs::write(root.join("chart.png"), png(64, 32)).expect("fixture");
        let blocks = super::super::blocks::parse("![A chart](chart.png)");
        let bytes = bytes(None, &blocks, "Calibri", &root).expect("write");

        assert!(names(&bytes).contains(&"word/media/image1.png".to_string()));
        let document = part(&bytes, "word/document.xml");
        assert!(document.contains("<w:drawing>"), "{document}");
        assert!(document.contains("r:embed=\"rId3\""), "{document}");
        assert!(document.contains("descr=\"A chart\""), "{document}");
        let rels = part(&bytes, "word/_rels/document.xml.rels");
        assert!(rels.contains("Id=\"rId3\""), "{rels}");
        assert!(rels.contains("media/image1.png"), "{rels}");
        assert!(part(&bytes, "[Content_Types].xml").contains("Extension=\"png\""));
        let _ = std::fs::remove_dir_all(&root);
    }

    /// A picture wider than the text column is scaled to fit; a narrow one keeps
    /// its size rather than being stretched.
    #[test]
    fn a_wide_picture_is_scaled_down_and_a_small_one_is_not() {
        let root = root();
        std::fs::write(root.join("wide.png"), png(2000, 1000)).expect("fixture");
        std::fs::write(root.join("small.png"), png(64, 32)).expect("fixture");

        let wide = bytes(
            None,
            &super::super::blocks::parse("![w](wide.png)"),
            "Calibri",
            &root,
        )
        .expect("write");
        let document = part(&wide, "word/document.xml");
        assert!(
            document.contains(&format!("cx=\"{}\"", image::TEXT_WIDTH_EMU)),
            "{document}"
        );

        let small = bytes(
            None,
            &super::super::blocks::parse("![s](small.png)"),
            "Calibri",
            &root,
        )
        .expect("write");
        let document = part(&small, "word/document.xml");
        assert!(
            document.contains(&format!("cx=\"{}\"", image::emu(64))),
            "{document}"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    /// A path outside the workspace is refused by the same check the file tools
    /// use, so a document cannot embed something `read_file` could not read.
    #[test]
    fn a_picture_outside_the_workspace_is_refused() {
        let root = root();
        let error = bytes(
            None,
            &super::super::blocks::parse("![x](../outside.png)"),
            "Calibri",
            &root,
        )
        .expect_err("refused");
        assert!(error.contains("outside"), "{error}");
        let _ = std::fs::remove_dir_all(&root);
    }
}
