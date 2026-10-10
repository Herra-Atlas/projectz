//! Writing a spreadsheet that reads as a table rather than as a wall of cells.
//!
//! An `.xlsx` is a zip of XML parts, and the previous writer produced the
//! smallest one that Excel would open: every value present, every cell wearing
//! the default style, every column the default width. It worked, and it looked
//! like a CSV someone had opened by accident.
//!
//! What makes a generated sheet usable is layout, and Excel stores all of it in
//! `styles.xml` plus a handful of elements in the sheet itself:
//!
//! - a **bold, shaded, bordered header row** with the pane frozen under it, so
//!   the headings stay on screen while the data scrolls;
//! - an **auto-filter** on that row, so the columns are sortable on arrival;
//! - **column widths** measured from the content, because the default width cuts
//!   off every label worth reading;
//! - **number formats**, so a column of totals is right-aligned with thousands
//!   separators and keeps its decimals.
//!
//! # Text is stored inline
//!
//! Excel normally keeps strings in one `sharedStrings.xml` table and stores an
//! index in each cell. Inline strings are used instead: a generated sheet is
//! written once and read by a person, so the de-duplication a shared table buys
//! costs a second part and a second pass for nothing. See `documents.rs`, which
//! reads both forms back.

use super::ooxml::{self, xml_escape, XML_HEADER};
use super::Cell;

/// The narrowest and widest a column is sized to, in Excel's character units.
///
/// The floor keeps a short column from looking like a crack; the ceiling keeps one
/// long sentence from pushing every other column off the screen.
const MIN_WIDTH: usize = 9;
const MAX_WIDTH: usize = 60;

/// The style index of each look, as numbered in `styles.xml` below.
mod style {
    /// A number with thousands separators and no decimals.
    pub const INTEGER: u32 = 3;
    /// A number with thousands separators and two decimals.
    pub const DECIMAL: u32 = 4;
    /// A body cell in the table's grid.
    pub const TEXT: u32 = 2;
    /// The bold, shaded, centred header cell.
    pub const HEADER: u32 = 1;
}

/// Builds the whole package.
///
/// `header` is what decides whether row one is a heading: when it is, the sheet
/// gets the grid, the fill, the freeze and the filter, and when it is not the
/// values are written plain. It is an argument rather than a guess because a
/// first row of labels and a first row of data are indistinguishable from the
/// outside, and styling the wrong one is worse than not styling at all.
pub(crate) fn bytes(rows: &[Vec<Cell>], sheet: &str, header: bool) -> Result<Vec<u8>, String> {
    if rows.is_empty() {
        return Err("A spreadsheet needs at least one row.".to_string());
    }
    let name = sheet_name(sheet);
    let columns = rows.iter().map(Vec::len).max().unwrap_or(0);
    if columns == 0 {
        return Err("A spreadsheet needs at least one column.".to_string());
    }
    let last_column = ooxml::column_letters(columns);
    let dimension = format!("A1:{last_column}{}", rows.len());

    let sheet_xml = format!(
        "{header}\
<worksheet xmlns=\"http://schemas.openxmlformats.org/spreadsheetml/2006/main\">\
<dimension ref=\"{dimension}\"/>\
<sheetViews><sheetView workbookViewId=\"0\">{view}</sheetView></sheetViews>\
<sheetFormatPr defaultRowHeight=\"15\"/>\
{cols}<sheetData>{data}</sheetData>{filter}</worksheet>",
        header = XML_HEADER,
        view = if header {
            // The split is one row down, so row one never scrolls away.
            "<pane ySplit=\"1\" topLeftCell=\"A2\" activePane=\"bottomLeft\" state=\"frozen\"/>\
             <selection pane=\"bottomLeft\" activeCell=\"A2\" sqref=\"A2\"/>"
        } else {
            ""
        },
        cols = columns_element(rows),
        data = sheet_data(rows, header),
        filter = if header {
            format!("<autoFilter ref=\"{dimension}\"/>")
        } else {
            String::new()
        },
    );

    Ok(ooxml::zip_text_parts(&[
        ("[Content_Types].xml", content_types()),
        ("_rels/.rels", package_rels()),
        ("xl/workbook.xml", workbook(&name)),
        ("xl/_rels/workbook.xml.rels", workbook_rels()),
        ("xl/styles.xml", styles()),
        ("xl/worksheets/sheet1.xml", sheet_xml),
    ]))
}

/// The `<cols>` block, sizing each column from the longest value in it.
fn columns_element(rows: &[Vec<Cell>]) -> String {
    let width = rows.first().map_or(0, Vec::len);
    let mut out = String::new();
    for index in 0..width {
        let longest = rows
            .iter()
            .filter_map(|row| row.get(index))
            .map(|cell| cell.as_text().chars().count())
            .max()
            .unwrap_or(0);
        let size = (longest + 2).clamp(MIN_WIDTH, MAX_WIDTH);
        out.push_str(&format!(
            "<col min=\"{min}\" max=\"{min}\" width=\"{size}\" customWidth=\"1\"/>",
            min = index + 1,
        ));
    }
    out
}

/// Every row of cells.
fn sheet_data(rows: &[Vec<Cell>], header: bool) -> String {
    let mut out = String::new();
    for (index, row) in rows.iter().enumerate() {
        let number = index + 1;
        // Row one is taller when it is a heading: the fill needs the room to read
        // as a band rather than as a tight line of text.
        let attributes = if header && index == 0 {
            format!(" ht=\"20\" customHeight=\"1\"")
        } else {
            String::new()
        };
        out.push_str(&format!("<row r=\"{number}\"{attributes}>"));
        for (column, cell) in row.iter().enumerate() {
            let reference = format!("{}{number}", ooxml::column_letters(column + 1));
            let is_header_row = header && index == 0;
            out.push_str(&cell_xml(&reference, cell, is_header_row, header));
        }
        out.push_str("</row>");
    }
    out
}

/// One cell: its value, and the style that decides how it looks.
fn cell_xml(reference: &str, cell: &Cell, is_header: bool, table: bool) -> String {
    let style = if is_header {
        style::HEADER
    } else if !table {
        // No header was asked for, so there is no table to draw round the cells.
        0
    } else {
        match cell {
            Cell::Number(number) => {
                if number.fract() == 0.0 {
                    style::INTEGER
                } else {
                    style::DECIMAL
                }
            }
            Cell::Text(_) => style::TEXT,
        }
    };
    // Style zero is the default, and an `s="0"` on every cell is noise in a part
    // that is already long.
    let style_attribute = if style == 0 {
        String::new()
    } else {
        format!(" s=\"{style}\"")
    };
    match cell {
        Cell::Number(number) => format!(
            "<c r=\"{reference}\"{style_attribute}><v>{}</v></c>",
            ooxml::number_to_string(*number)
        ),
        // An inline string, so no shared-string table has to be built and
        // referenced. `xml:space` keeps a leading or trailing space, which a
        // value pasted from a report often has.
        Cell::Text(text) => format!(
            "<c r=\"{reference}\"{style_attribute} t=\"inlineStr\"><is><t xml:space=\"preserve\">{}</t></is></c>",
            xml_escape(text)
        ),
    }
}

/// The sheet's name, made safe for Excel.
///
/// Excel refuses a name containing `[]:*?/\`, will not accept one longer than 31
/// characters, and will not accept an empty one. A refused name is an error the
/// user sees as "the file is corrupt", so the name is cleaned here instead.
fn sheet_name(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .filter(|character| !"[]:*?/\\".contains(*character))
        .take(31)
        .collect();
    let cleaned = cleaned.trim();
    if cleaned.is_empty() {
        "Sheet1".to_string()
    } else {
        cleaned.to_string()
    }
}

/// The parts this writer produces.
fn content_types() -> String {
    format!(
        "{XML_HEADER}\
<Types xmlns=\"http://schemas.openxmlformats.org/package/2006/content-types\">\
<Default Extension=\"rels\" ContentType=\"application/vnd.openxmlformats-package.relationships+xml\"/>\
<Default Extension=\"xml\" ContentType=\"application/xml\"/>\
<Override PartName=\"/xl/workbook.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml\"/>\
<Override PartName=\"/xl/worksheets/sheet1.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml\"/>\
<Override PartName=\"/xl/styles.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.spreadsheetml.styles+xml\"/>\
</Types>"
    )
}

fn package_rels() -> String {
    format!(
        "{XML_HEADER}\
<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">\
<Relationship Id=\"rId1\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument\" Target=\"xl/workbook.xml\"/>\
</Relationships>"
    )
}

fn workbook(name: &str) -> String {
    format!(
        "{XML_HEADER}\
<workbook xmlns=\"http://schemas.openxmlformats.org/spreadsheetml/2006/main\" \
xmlns:r=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships\">\
<sheets><sheet name=\"{}\" sheetId=\"1\" r:id=\"rId1\"/></sheets></workbook>",
        xml_escape(name)
    )
}

fn workbook_rels() -> String {
    format!(
        "{XML_HEADER}\
<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">\
<Relationship Id=\"rId1\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet\" Target=\"worksheets/sheet1.xml\"/>\
<Relationship Id=\"rId2\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles\" Target=\"styles.xml\"/>\
</Relationships>"
    )
}

/// The cell formats, in the order `style` numbers them.
///
/// Two parts of this are load-bearing rather than cosmetic. The `fills` and
/// `borders` lists must include the empty first entry, because index 0 means
/// "none" everywhere in the format and shifting the list would repoint every
/// style. And a format id over 163 is a custom one, which is why the number
/// formats start at 164.
fn styles() -> String {
    format!(
        "{XML_HEADER}\
<styleSheet xmlns=\"http://schemas.openxmlformats.org/spreadsheetml/2006/main\">\
<numFmts count=\"2\">\
<numFmt numFmtId=\"164\" formatCode=\"#,##0\"/>\
<numFmt numFmtId=\"165\" formatCode=\"#,##0.00\"/>\
</numFmts>\
<fonts count=\"2\">\
<font><sz val=\"11\"/><name val=\"Calibri\"/></font>\
<font><b/><sz val=\"11\"/><color rgb=\"FF111827\"/><name val=\"Calibri\"/></font>\
</fonts>\
<fills count=\"3\">\
<fill><patternFill patternType=\"none\"/></fill>\
<fill><patternFill patternType=\"gray125\"/></fill>\
<fill><patternFill patternType=\"solid\"><fgColor rgb=\"FFF3F4F6\"/><bgColor indexed=\"64\"/></patternFill></fill>\
</fills>\
<borders count=\"2\">\
<border><left/><right/><top/><bottom/><diagonal/></border>\
<border><left style=\"thin\"><color rgb=\"FFD1D5DB\"/></left>\
<right style=\"thin\"><color rgb=\"FFD1D5DB\"/></right>\
<top style=\"thin\"><color rgb=\"FFD1D5DB\"/></top>\
<bottom style=\"thin\"><color rgb=\"FFD1D5DB\"/></bottom><diagonal/></border>\
</borders>\
<cellStyleXfs count=\"1\"><xf numFmtId=\"0\" fontId=\"0\" fillId=\"0\" borderId=\"0\"/></cellStyleXfs>\
<cellXfs count=\"5\">\
<xf numFmtId=\"0\" fontId=\"0\" fillId=\"0\" borderId=\"0\" xfId=\"0\"/>\
<xf numFmtId=\"0\" fontId=\"1\" fillId=\"2\" borderId=\"1\" xfId=\"0\" applyFont=\"1\" applyFill=\"1\" applyBorder=\"1\" applyAlignment=\"1\">\
<alignment horizontal=\"center\" vertical=\"center\" wrapText=\"1\"/></xf>\
<xf numFmtId=\"0\" fontId=\"0\" fillId=\"0\" borderId=\"1\" xfId=\"0\" applyBorder=\"1\" applyAlignment=\"1\">\
<alignment vertical=\"top\" wrapText=\"1\"/></xf>\
<xf numFmtId=\"164\" fontId=\"0\" fillId=\"0\" borderId=\"1\" xfId=\"0\" applyNumberFormat=\"1\" applyBorder=\"1\" applyAlignment=\"1\">\
<alignment vertical=\"top\"/></xf>\
<xf numFmtId=\"165\" fontId=\"0\" fillId=\"0\" borderId=\"1\" xfId=\"0\" applyNumberFormat=\"1\" applyBorder=\"1\" applyAlignment=\"1\">\
<alignment vertical=\"top\"/></xf>\
</cellXfs>\
<cellStyles count=\"1\"><cellStyle name=\"Normal\" xfId=\"0\" builtinId=\"0\"/></cellStyles>\
</styleSheet>"
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai::tools::export::test_support::part;

    fn rows() -> Vec<Vec<Cell>> {
        vec![
            vec![Cell::Text("Name".into()), Cell::Text("Score".into())],
            vec![Cell::Text("Ada".into()), Cell::Number(42.0)],
            vec![Cell::Text("Grace".into()), Cell::Number(7.5)],
        ]
    }

    #[test]
    fn a_header_row_is_styled_frozen_and_filterable() {
        let bytes = bytes(&rows(), "Sheet1", true).expect("write");
        let sheet = part(&bytes, "xl/worksheets/sheet1.xml");
        // Row one in the header format, the pane split under it, and a filter
        // over the used range.
        assert!(sheet.contains("<c r=\"A1\" s=\"1\""), "{sheet}");
        assert!(sheet.contains("state=\"frozen\""), "{sheet}");
        assert!(sheet.contains("<autoFilter ref=\"A1:B3\"/>"), "{sheet}");
        // The style the sheet points at has to exist, and a header that is not
        // bold is not a header.
        let styles = part(&bytes, "xl/styles.xml");
        assert!(styles.contains("<b/>"), "{styles}");
        assert!(styles.contains("FFF3F4F6"), "{styles}");
    }

    #[test]
    fn whole_and_fractional_numbers_get_different_formats() {
        let bytes = bytes(&rows(), "Sheet1", true).expect("write");
        let sheet = part(&bytes, "xl/worksheets/sheet1.xml");
        assert!(
            sheet.contains("<c r=\"B2\" s=\"3\"><v>42</v></c>"),
            "{sheet}"
        );
        assert!(
            sheet.contains("<c r=\"B3\" s=\"4\"><v>7.5</v></c>"),
            "{sheet}"
        );
        let styles = part(&bytes, "xl/styles.xml");
        assert!(styles.contains("formatCode=\"#,##0\""), "{styles}");
        assert!(styles.contains("formatCode=\"#,##0.00\""), "{styles}");
    }

    /// Without a header there is no table to draw, so the cells keep the default
    /// format instead of arriving with boxes round them.
    #[test]
    fn no_header_means_no_grid() {
        let bytes = bytes(&rows(), "Sheet1", false).expect("write");
        let sheet = part(&bytes, "xl/worksheets/sheet1.xml");
        assert!(sheet.contains("<c r=\"A1\""), "{sheet}");
        assert!(!sheet.contains("s=\"1\""), "{sheet}");
        assert!(!sheet.contains("state=\"frozen\""), "{sheet}");
    }

    #[test]
    fn columns_are_sized_from_their_longest_value() {
        let bytes = bytes(
            &[vec![Cell::Text("A very long heading indeed".into())]],
            "Sheet1",
            false,
        )
        .expect("write");
        let sheet = part(&bytes, "xl/worksheets/sheet1.xml");
        assert!(sheet.contains("width=\"28\""), "{sheet}");
        assert!(sheet.contains("customWidth=\"1\""), "{sheet}");
    }

    /// A name Excel would refuse is cleaned rather than passed through: a rejected
    /// name surfaces as "the file is corrupt".
    #[test]
    fn a_sheet_name_is_made_safe() {
        assert_eq!(sheet_name("Q3 [draft]/final"), "Q3 draftfinal");
        assert_eq!(sheet_name(""), "Sheet1");
        assert_eq!(sheet_name("   "), "Sheet1");
        assert_eq!(sheet_name(&"x".repeat(50)).chars().count(), 31);
    }

    #[test]
    fn an_empty_grid_is_refused_rather_than_written() {
        assert!(bytes(&[], "Sheet1", true).is_err());
        assert!(bytes(&[vec![]], "Sheet1", true).is_err());
    }

    /// A value that starts like a formula is stored as text, so a cell holding
    /// `=SUM(A1)` is not silently turned into a formula.
    #[test]
    fn text_that_looks_like_a_formula_stays_text() {
        let bytes = bytes(&[vec![Cell::Text("=SUM(A1)".into())]], "Sheet1", false).expect("write");
        let sheet = part(&bytes, "xl/worksheets/sheet1.xml");
        assert!(sheet.contains("t=\"inlineStr\""), "{sheet}");
        assert!(!sheet.contains("<f>"), "{sheet}");
    }
}
