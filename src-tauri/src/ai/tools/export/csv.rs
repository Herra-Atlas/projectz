//! Reading and writing CSV.
//!
//! One file for both directions because they are one format: the sheet's `rows`
//! can arrive as JSON from the model, or as a CSV file the model built a section
//! at a time. Writing the file and parsing one back have to agree about quoting,
//! and keeping them beside each other is what makes that checkable.
//!
//! # Why a parser here
//!
//! The `csv` crate would be the obvious answer, but this app already builds its
//! own documents and the format's rules are four lines long: a field may be
//! quoted, a quoted field may hold commas, doubled quotes and newlines, and
//! anything else is literal. A dependency would be more code in the tree than the
//! parser it replaces.
//!
//! # Numbers are inferred, carefully
//!
//! A CSV has no types. A plain decimal (`42`, `-3.5`, `0`, `0.25`) becomes a
//! spreadsheet number so totals are real numbers; a value that merely looks
//! numeric keeps its text, because `007` is an identifier and turning it into
//! seven would silently corrupt data. The same rule as a human reading it.

use super::Cell;

/// Renders rows as CSV, quoting a field only when the format would otherwise
/// misread it: a comma, a quote or a line break.
pub(super) fn bytes(rows: &[Vec<Cell>]) -> Vec<u8> {
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

/// Parses a CSV file into rows of cells.
///
/// An empty line produces no row rather than one empty cell, so a file that ends
/// in a newline does not grow a phantom row at the bottom -- the commonest way a
/// spreadsheet gains a stray line.
pub(super) fn parse(text: &str) -> Result<Vec<Vec<Cell>>, String> {
    // Excel writes UTF-8 with a byte-order mark, and leaving it on the first
    // header would make that column's name start with an invisible character --
    // the kind of bug that shows up as a column nobody can match.
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let mut rows: Vec<Vec<Cell>> = Vec::new();
    let mut row: Vec<Cell> = Vec::new();
    let mut field = String::new();
    let mut quoted = false;
    let mut started = false;

    let mut characters = text.chars().peekable();
    while let Some(character) = characters.next() {
        match character {
            '"' if quoted => {
                // `""` inside a quoted field is one literal quote; a lone `"` ends
                // the field. Both are decided here because the second quote is only
                // knowable by looking ahead.
                if characters.peek() == Some(&'"') {
                    characters.next();
                    field.push('"');
                } else {
                    quoted = false;
                }
                started = true;
            }
            // A quote only opens a field at its start. Anywhere else it is a
            // literal character, which is what a spreadsheet written by hand does.
            '"' if field.is_empty() && !started => {
                quoted = true;
                started = true;
            }
            ',' if !quoted => {
                row.push(cell_from(std::mem::take(&mut field)));
                started = false;
            }
            '\r' if !quoted => {
                // Half of CRLF. The `\n` that follows ends the row, so consuming
                // this one here is what keeps a Windows file from gaining a blank
                // first cell in every line.
            }
            '\n' if !quoted => {
                if started || !field.is_empty() || !row.is_empty() {
                    row.push(cell_from(std::mem::take(&mut field)));
                    rows.push(std::mem::take(&mut row));
                }
                started = false;
            }
            other => {
                field.push(other);
                if !other.is_whitespace() {
                    started = true;
                }
            }
        }
    }
    // A final line with no trailing newline still ends a row.
    if started || !field.is_empty() || !row.is_empty() {
        row.push(cell_from(field));
        rows.push(row);
    }

    if rows.is_empty() {
        return Err("The CSV file is empty; it has no rows to put in a sheet.".to_string());
    }
    Ok(rows)
}

/// One field: a number when it is unambiguously a number, text otherwise.
fn cell_from(text: String) -> Cell {
    match number(&text) {
        Some(value) => Cell::Number(value),
        None => Cell::Text(text),
    }
}

/// A plain decimal, or nothing.
///
/// `0`, `42`, `-3.5` and `0.25` qualify. `007`, `1e5`, `1,000` and `3.` do not:
/// the first is an identifier and the rest are shapes a spreadsheet reader would
/// rather see verbatim than guess at.
fn number(text: &str) -> Option<f64> {
    let unsigned = text.strip_prefix(['-', '+']).unwrap_or(text);
    if unsigned.is_empty() {
        return None;
    }
    let (whole, fraction) = match unsigned.split_once('.') {
        Some((whole, fraction)) => (whole, Some(fraction)),
        None => (unsigned, None),
    };
    if whole.is_empty() || !whole.chars().all(|digit| digit.is_ascii_digit()) {
        return None;
    }
    // A leading zero means an identifier or an old-style decimal (`.5`), never the
    // number the digits would suggest.
    if whole.len() > 1 && whole.starts_with('0') {
        return None;
    }
    if let Some(fraction) = fraction {
        if fraction.is_empty() || !fraction.chars().all(|digit| digit.is_ascii_digit()) {
            return None;
        }
    }
    text.parse::<f64>().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text_rows(rows: &[Vec<Cell>]) -> Vec<Vec<String>> {
        rows.iter()
            .map(|row| row.iter().map(Cell::as_text).collect())
            .collect()
    }

    #[test]
    fn quoted_fields_keep_their_commas_and_newlines() {
        let rows = parse("name,note\nAda,\"has, a comma\"\nBob,\"two\nlines\"\n").expect("parse");
        assert_eq!(
            text_rows(&rows),
            vec![
                vec!["name".to_string(), "note".to_string()],
                vec!["Ada".to_string(), "has, a comma".to_string()],
                vec!["Bob".to_string(), "two\nlines".to_string()],
            ]
        );
    }

    #[test]
    fn a_doubled_quote_is_one_quote() {
        let rows = parse("\"say \"\"hi\"\"\"").expect("parse");
        assert_eq!(text_rows(&rows), vec![vec!["say \"hi\"".to_string()]]);
    }

    #[test]
    fn a_trailing_newline_does_not_add_a_row() {
        assert_eq!(parse("a,b\n").expect("parse").len(), 1);
        assert_eq!(parse("a,b").expect("parse").len(), 1);
    }

    #[test]
    fn a_windows_line_ending_is_one_row_break() {
        let rows = parse("a,b\r\nc,d\r\n").expect("parse");
        assert_eq!(text_rows(&rows), vec![vec!["a", "b"], vec!["c", "d"]]);
    }

    #[test]
    fn plain_decimals_become_numbers_and_look_alikes_stay_text() {
        for value in ["0", "42", "-3.5", "0.25"] {
            assert!(
                matches!(cell_from(value.to_string()), Cell::Number(_)),
                "{value}"
            );
        }
        for value in ["007", "1e5", "1,000", "3.", "none"] {
            assert!(
                matches!(cell_from(value.to_string()), Cell::Text(_)),
                "{value}"
            );
        }
    }

    #[test]
    fn a_byte_order_mark_is_not_part_of_the_first_cell() {
        let rows = parse("\u{feff}Name,Score\nAda,42\n").expect("parse");
        assert_eq!(text_rows(&rows)[0][0], "Name");
    }

    #[test]
    fn an_empty_file_is_refused_rather_than_written_as_an_empty_sheet() {
        assert!(parse("\n").is_err());
    }

    /// The writer and the reader agree, which is the only contract this file has.
    #[test]
    fn what_is_written_parses_back_to_the_same_cells() {
        let rows = vec![
            vec![Cell::Text("name".into()), Cell::Text("note".into())],
            vec![
                Cell::Text("Ada".into()),
                Cell::Text("has, a \"comma\"".into()),
            ],
            vec![Cell::Number(42.0), Cell::Number(-1.5)],
        ];
        let text = String::from_utf8(bytes(&rows)).expect("utf8");
        let parsed = parse(&text).expect("parse");
        assert_eq!(text_rows(&parsed), text_rows(&rows));
    }
}
