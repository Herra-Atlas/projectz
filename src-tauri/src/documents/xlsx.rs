//! Reading a spreadsheet, as text for a model and as a grid for the panel.
//!
//! Both come from the same walking of `xl/worksheets/*.xml`, because a second
//! reader for the panel would be a second opinion about where a cell sits. What
//! differs is only the shape handed back: a model wants the values tab-separated
//! under a sheet's name, and a person wants them in columns.
//!
//! # Where a cell sits
//!
//! A cell carries its own reference (`r="B7"`), and a row that skips a cell --
//! which is what an empty middle column is -- simply omits it. Trusting the
//! position in the file would shift every value after the gap one column to the
//! left, so the column is read from the reference and the gaps are filled with
//! empty cells.

use std::path::Path;

use serde::Serialize;

use super::ooxml::{attribute, column_index, read_zip_entry, tag_text, zip_entry_names};

/// Rows and columns a grid is cut to before the panel draws it.
///
/// A cap because the panel is a preview and a generated sheet can be arbitrarily
/// long: past this the reader stops and says so rather than building a list the
/// UI cannot use.
const MAX_GRID_ROWS: usize = 500;
const MAX_GRID_COLUMNS: usize = 64;

/// One sheet, as a grid for the panel.
#[derive(Serialize)]
pub struct SheetGrid {
    pub name: String,
    pub rows: Vec<Vec<String>>,
    /// True when rows or columns were left out, so the panel can say so rather
    /// than quietly showing a partial sheet.
    pub truncated: bool,
}

/// The spreadsheet's text: each sheet as tab-separated rows.
///
/// Values are tab-separated within a row and rows newline-separated, which is
/// what a model reading a table wants. Cell references that point into the shared
/// string table are resolved, so the output is the strings the user sees rather
/// than the indices the file stores.
pub(super) fn text(path: &Path) -> Result<String, String> {
    let text = sheets(path)?
        .into_iter()
        .map(|(name, rows)| {
            let body = rows
                .iter()
                .map(|row| row.join("\t"))
                .filter(|row| !row.trim().is_empty())
                .collect::<Vec<_>>()
                .join("\n");
            (name, body)
        })
        .filter(|(_, body)| !body.trim().is_empty())
        .map(|(_, body)| body)
        .collect::<Vec<_>>()
        .join("\n\n");
    if text.trim().is_empty() {
        return Err(format!("{} contained no readable cells.", path.display()));
    }
    Ok(text)
}

/// Every sheet as a grid, cut to the preview's limits.
pub(super) fn grid(path: &Path) -> Result<Vec<SheetGrid>, String> {
    let grids = sheets(path)?
        .into_iter()
        .map(|(name, rows)| {
            let wide = rows.iter().map(Vec::len).max().unwrap_or(0);
            let truncated = rows.len() > MAX_GRID_ROWS || wide > MAX_GRID_COLUMNS;
            let rows = rows
                .into_iter()
                .take(MAX_GRID_ROWS)
                .map(|row| {
                    let mut row = row;
                    row.truncate(MAX_GRID_COLUMNS);
                    row
                })
                .collect();
            SheetGrid {
                name,
                rows,
                truncated,
            }
        })
        .collect::<Vec<_>>();
    if grids.is_empty() {
        return Err(format!("{} contained no readable cells.", path.display()));
    }
    Ok(grids)
}

/// Every sheet's name and rows, in the workbook's own order.
fn sheets(path: &Path) -> Result<Vec<(String, Vec<Vec<String>>)>, String> {
    let shared = shared_strings(path)?;
    let mut out = Vec::new();
    for (name, entry) in sheet_parts(path)? {
        let Some(xml) = read_zip_entry(path, &entry)? else {
            continue;
        };
        out.push((name, rows(&xml, &shared)));
    }
    Ok(out)
}

/// The shared string table, in index order.
///
/// Only one of the two ways a string is stored: a cell may also carry its text
/// inline in `<is>`, which `value` handles.
fn shared_strings(path: &Path) -> Result<Vec<String>, String> {
    Ok(read_zip_entry(path, "xl/sharedStrings.xml")?
        .map(|xml| tag_text(&xml, &["t"]))
        .unwrap_or_default())
}

/// Every sheet's name and the part it lives in.
///
/// Read from the workbook and its relationships rather than from the file names,
/// because the number in `sheet1.xml` is an internal id and has nothing to do with
/// the order the tabs appear in -- a workbook written by Excel can name them
/// `sheet7.xml` and `sheet2.xml` in that order.
fn sheet_parts(path: &Path) -> Result<Vec<(String, String)>, String> {
    let workbook = read_zip_entry(path, "xl/workbook.xml")?.unwrap_or_default();
    let rels = read_zip_entry(path, "xl/_rels/workbook.xml.rels")?.unwrap_or_default();
    let mut out = Vec::new();
    for chunk in workbook.split("<sheet ").skip(1) {
        let head = chunk.split('>').next().unwrap_or("");
        let name = attribute(head, "name").unwrap_or_else(|| format!("Sheet{}", out.len() + 1));
        let Some(entry) = attribute(head, "r:id")
            .and_then(|id| relationship_target(&rels, &id))
            .map(|target| normalize_target(&target))
        else {
            continue;
        };
        out.push((name, entry));
    }
    if out.is_empty() {
        // A workbook with no readable map -- so the parts are taken as they are
        // found. Sorted, because the archive's own order is arbitrary and a sheet
        // whose position changes between reads would be worse than one misnamed.
        let mut entries: Vec<String> = zip_entry_names(path)?
            .into_iter()
            .filter(|name| name.starts_with("xl/worksheets/") && name.ends_with(".xml"))
            .collect();
        entries.sort();
        out = entries
            .into_iter()
            .enumerate()
            .map(|(index, entry)| (format!("Sheet{}", index + 1), entry))
            .collect();
    }
    Ok(out)
}

/// The target a relationship id points at.
fn relationship_target(rels: &str, id: &str) -> Option<String> {
    for chunk in rels.split("<Relationship ").skip(1) {
        let head = chunk.split('>').next().unwrap_or("");
        if attribute(head, "Id").as_deref() == Some(id) {
            return attribute(head, "Target");
        }
    }
    None
}

/// A relationship target as an archive path.
///
/// Excel writes either `worksheets/sheet1.xml`, relative to the workbook's own
/// folder, or `/xl/worksheets/sheet1.xml`, from the package root.
fn normalize_target(target: &str) -> String {
    if let Some(absolute) = target.strip_prefix('/') {
        absolute.to_string()
    } else if target.starts_with("xl/") {
        target.to_string()
    } else {
        format!("xl/{target}")
    }
}

/// One sheet's rows, with each value in the column its reference names.
fn rows(xml: &str, shared: &[String]) -> Vec<Vec<String>> {
    let data = sheet_data(xml);
    let mut out: Vec<Vec<String>> = Vec::new();
    for row in data.split("</row>") {
        let mut cells: Vec<String> = Vec::new();
        for (position, chunk) in row.split("<c").skip(1).enumerate() {
            let head = chunk.split('>').next().unwrap_or("");
            let column = attribute(head, "r")
                .and_then(|reference| {
                    let letters: String = reference
                        .chars()
                        .take_while(char::is_ascii_alphabetic)
                        .collect();
                    column_index(&letters)
                })
                .unwrap_or(position);
            if cells.len() <= column {
                cells.resize(column + 1, String::new());
            }
            cells[column] = value(chunk, shared);
        }
        // A row's trailing empties are the columns Excel pads to; they are not
        // values, and keeping them would widen every grid to the widest sheet.
        while cells.last().is_some_and(String::is_empty) {
            cells.pop();
        }
        out.push(cells);
    }
    while out.last().is_some_and(Vec::is_empty) {
        out.pop();
    }
    out
}

/// Just the `<sheetData>` body, so nothing outside it is read as a cell.
///
/// `<cols>` is the reason: it is a sibling element whose name starts with `<c`,
/// and a naive split on `<c` finds it.
fn sheet_data(xml: &str) -> &str {
    let Some(after) = xml.split("<sheetData").nth(1) else {
        return xml;
    };
    let Some((_, body)) = after.split_once('>') else {
        return xml;
    };
    body.split("</sheetData>").next().unwrap_or(body)
}

/// One cell's displayed value, following a shared-string reference.
fn value(cell: &str, shared: &[String]) -> String {
    let head = cell.split('>').next().unwrap_or("");
    // An inline string carries its text in `<is><t>`; a shared string is an index
    // into the table; anything else is a literal value in `<v>`.
    if cell.contains("<is>") {
        return tag_text(cell, &["t"]).join("");
    }
    let Some((_, rest)) = cell.split_once("<v>") else {
        return String::new();
    };
    let Some((value, _)) = rest.split_once("</v>") else {
        return String::new();
    };
    if head.contains("t=\"s\"") {
        return value
            .trim()
            .parse::<usize>()
            .ok()
            .and_then(|index| shared.get(index).cloned())
            .unwrap_or_default();
    }
    crate::documents::ooxml::decode_entities(value)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::documents::test_support::{temp, write_zip};

    /// The shared-string table is resolved, so the reader sees the strings rather
    /// than the indices the file stores.
    #[test]
    fn shared_strings_are_resolved_and_sheets_are_named() {
        let path = temp("a.xlsx");
        write_zip(
            &path,
            &[
                ("xl/workbook.xml", "<workbook><sheets><sheet name=\"Q3\" sheetId=\"1\" r:id=\"rId1\"/></sheets></workbook>"),
                (
                    "xl/_rels/workbook.xml.rels",
                    "<Relationships><Relationship Id=\"rId1\" Target=\"worksheets/sheet1.xml\"/></Relationships>",
                ),
                ("xl/sharedStrings.xml", "<sst><si><t>Name</t></si><si><t>Ada</t></si></sst>"),
                (
                    "xl/worksheets/sheet1.xml",
                    "<worksheet><cols><col min=\"1\" max=\"1\" width=\"18\"/></cols><sheetData>\
                     <row r=\"1\"><c r=\"A1\" t=\"s\"><v>0</v></c><c r=\"B1\" t=\"s\"><v>1</v></c></row>\
                     <row r=\"2\"><c r=\"B2\"><v>42</v></c></row>\
                     </sheetData></worksheet>",
                ),
            ],
        );
        let text = text(&path).expect("text");
        assert!(text.contains("Name\tAda"), "{text}");
        assert!(text.contains("42"), "{text}");

        let grids = grid(&path).expect("grid");
        assert_eq!(grids[0].name, "Q3");
        assert!(!grids[0].truncated);
        // The skipped `A2` leaves an empty cell rather than pulling `42` left.
        assert_eq!(grids[0].rows[1], vec!["", "42"]);
        let _ = std::fs::remove_file(&path);
    }

    /// `<cols>` is a sibling element whose name starts like a cell's, so it must
    /// not be read as one.
    #[test]
    fn the_column_block_is_not_read_as_a_cell() {
        let xml = "<worksheet><cols><col min=\"1\" max=\"1\" width=\"18\"/></cols>\
                   <sheetData><row r=\"1\"><c r=\"A1\"><v>1</v></c></row></sheetData></worksheet>";
        assert_eq!(rows(xml, &[]), vec![vec!["1"]]);
    }

    /// With no workbook map the parts are used in name order rather than in
    /// whatever order the archive happens to list them.
    #[test]
    fn a_workbook_with_no_map_falls_back_to_the_parts() {
        let path = temp("no-map.xlsx");
        write_zip(
            &path,
            &[(
                "xl/worksheets/sheet1.xml",
                "<worksheet><sheetData><row><c><v>7</v></c></row></sheetData></worksheet>",
            )],
        );
        let grids = grid(&path).expect("grid");
        assert_eq!(grids[0].name, "Sheet1");
        assert_eq!(grids[0].rows, vec![vec!["7"]]);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn a_target_is_normalised_to_an_archive_path() {
        assert_eq!(
            normalize_target("worksheets/sheet1.xml"),
            "xl/worksheets/sheet1.xml"
        );
        assert_eq!(
            normalize_target("/xl/worksheets/sheet1.xml"),
            "xl/worksheets/sheet1.xml"
        );
        assert_eq!(
            normalize_target("xl/worksheets/sheet1.xml"),
            "xl/worksheets/sheet1.xml"
        );
    }
}
