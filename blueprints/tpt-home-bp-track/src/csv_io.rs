// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 TPT Solutions

//! CSV import and export for Track tables.
//!
//! Export writes a header row of field names, then one line per row in
//! insertion order, with CRLF line endings (RFC 4180). Import reads the same
//! shape back: the header names the columns (case-insensitive; fields missing
//! from the file are left empty), every cell is checked against its field's
//! kind, and the whole file is validated before anything is written. If any
//! row is bad, nothing is saved and the message lists the problems. Accepted
//! rows are inserted in one transaction, so an import is all or nothing.

use sqlx::{Row, SqlitePool};
use tpt_home_core::prelude::*;

use crate::api::{self, FieldRow};

/// Most rows one import may add.
pub const MAX_IMPORT_ROWS: usize = 5_000;
/// Most problems listed in one error message.
const MAX_REPORTED: usize = 20;

/// Parse CSV text into rows of fields. Handles quoted fields with `""`
/// escapes, embedded commas and newlines, and LF or CRLF line endings. A quote
/// in the middle of an unquoted field is kept as a literal character.
pub fn parse(text: &str) -> Result<Vec<Vec<String>>, String> {
    let mut rows = Vec::new();
    let mut row: Vec<String> = Vec::new();
    let mut field = String::new();
    let mut quoted = false; // the current field began with a quote
    let mut in_quotes = false;
    let mut quote_line = 1usize;
    let mut line = 1usize;
    let mut chars = text.chars().peekable();

    while let Some(c) = chars.next() {
        if in_quotes {
            match c {
                '"' if chars.peek() == Some(&'"') => {
                    chars.next();
                    field.push('"');
                }
                '"' => in_quotes = false,
                '\n' => {
                    line += 1;
                    field.push(c);
                }
                _ => field.push(c),
            }
            continue;
        }
        match c {
            '"' if field.is_empty() && !quoted => {
                in_quotes = true;
                quoted = true;
                quote_line = line;
            }
            ',' => {
                row.push(std::mem::take(&mut field));
                quoted = false;
            }
            '\r' => {}
            '\n' => {
                row.push(std::mem::take(&mut field));
                rows.push(std::mem::take(&mut row));
                quoted = false;
                line += 1;
            }
            _ => field.push(c),
        }
    }
    if in_quotes {
        return Err(format!("line {quote_line}: quoted field is never closed"));
    }
    if quoted || !field.is_empty() || !row.is_empty() {
        row.push(field);
        rows.push(row);
    }
    Ok(rows)
}

/// Append one CSV line (CRLF-terminated) to `out`.
pub fn write_row(out: &mut String, fields: &[String]) {
    for (i, field) in fields.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        if field.contains([',', '"', '\n', '\r']) {
            out.push('"');
            out.push_str(&field.replace('"', "\"\""));
            out.push('"');
        } else {
            out.push_str(field);
        }
    }
    out.push_str("\r\n");
}

/// A cell after it has been checked against its field's kind.
#[derive(Debug, PartialEq)]
enum Cell {
    Null,
    Text(String),
    Int(i64),
    Real(f64),
    Bool(bool),
}

/// Check one raw cell. An empty cell is NULL for every kind.
fn parse_cell(kind: &str, raw: &str) -> Result<Cell, String> {
    if raw.is_empty() {
        return Ok(Cell::Null);
    }
    let trimmed = raw.trim();
    match kind {
        "text" => Ok(Cell::Text(raw.to_string())),
        "integer" => trimmed
            .parse::<i64>()
            .map(Cell::Int)
            .map_err(|_| format!("'{raw}' is not a whole number")),
        "real" => match trimmed.parse::<f64>() {
            Ok(x) if x.is_finite() => Ok(Cell::Real(x)),
            _ => Err(format!("'{raw}' is not a number")),
        },
        "bool" => match trimmed.to_ascii_lowercase().as_str() {
            "true" | "yes" | "y" | "1" => Ok(Cell::Bool(true)),
            "false" | "no" | "n" | "0" => Ok(Cell::Bool(false)),
            _ => Err(format!("'{raw}' is not true or false")),
        },
        other => Err(format!("unknown kind {other}")),
    }
}

/// Table as CSV. Returns the table name (for the file name) and the text.
pub async fn export(db: &SqlitePool, table_id: i64) -> Result<(String, String), ApiError> {
    let table = api::load_table(db, table_id).await?;
    let mut out = String::new();
    let header: Vec<String> = table.fields.iter().map(|f| f.name.clone()).collect();
    write_row(&mut out, &header);

    let select = if table.fields.is_empty() {
        "SELECT id FROM ".to_string()
    } else {
        let cols: Vec<String> = table.fields.iter().map(|f| api::col(f.id)).collect();
        format!("SELECT {} FROM ", cols.join(", "))
    };
    let sql = format!("{select}{} ORDER BY id", api::data_table(table.id));
    let rows = sqlx::query(&sql).fetch_all(db).await?;

    for row in &rows {
        let mut cells = Vec::with_capacity(table.fields.len());
        for (i, field) in table.fields.iter().enumerate() {
            cells.push(export_cell(row, i, field)?);
        }
        write_row(&mut out, &cells);
    }
    Ok((table.name, out))
}

fn export_cell(
    row: &sqlx::sqlite::SqliteRow,
    index: usize,
    field: &FieldRow,
) -> Result<String, ApiError> {
    let text = match field.kind.as_str() {
        "text" => row.try_get::<Option<String>, _>(index)?.unwrap_or_default(),
        "integer" => row
            .try_get::<Option<i64>, _>(index)?
            .map(|n| n.to_string())
            .unwrap_or_default(),
        "real" => row
            .try_get::<Option<f64>, _>(index)?
            .map(|x| x.to_string())
            .unwrap_or_default(),
        "bool" => row
            .try_get::<Option<bool>, _>(index)?
            .map(|b| if b { "true" } else { "false" }.to_string())
            .unwrap_or_default(),
        other => return Err(ApiError::internal(format!("unknown kind {other}"))),
    };
    Ok(text)
}

/// Import CSV text into a table. Returns the number of rows added.
pub async fn import(db: &SqlitePool, table_id: i64, text: &str) -> Result<usize, ApiError> {
    let table = api::load_table(db, table_id).await?;
    let mut rows = parse(text)
        .map_err(ApiError::bad_request)?
        .into_iter()
        .enumerate()
        .map(|(i, cells)| (i + 1, cells))
        // Blank lines carry no data.
        .filter(|(_, cells)| !(cells.len() == 1 && cells[0].is_empty()));

    let Some((_, header)) = rows.next() else {
        return Err(ApiError::bad_request("the file has no header row"));
    };
    let mut columns: Vec<&FieldRow> = Vec::with_capacity(header.len());
    let mut unknown: Vec<String> = Vec::new();
    for name in &header {
        let wanted = name.trim();
        match table
            .fields
            .iter()
            .find(|f| f.name.eq_ignore_ascii_case(wanted))
        {
            Some(field) if columns.iter().any(|c| c.id == field.id) => {
                return Err(ApiError::bad_request(format!(
                    "column '{wanted}' appears twice"
                )));
            }
            Some(field) => columns.push(field),
            None => unknown.push(wanted.to_string()),
        }
    }
    if !unknown.is_empty() {
        return Err(ApiError::bad_request(format!(
            "unknown column(s): {}",
            unknown.join(", ")
        )));
    }

    let mut problems: Vec<String> = Vec::new();
    let mut count = 0usize;
    let mut accepted: Vec<Vec<(i64, Cell)>> = Vec::new();
    for (line, cells) in rows {
        if cells.len() != header.len() {
            problems.push(format!(
                "line {line}: expected {} columns, found {}",
                header.len(),
                cells.len()
            ));
            continue;
        }
        count += 1;
        if count > MAX_IMPORT_ROWS {
            return Err(ApiError::bad_request(format!(
                "more than {MAX_IMPORT_ROWS} rows; split the file and import it in parts"
            )));
        }
        let mut values = Vec::with_capacity(columns.len());
        for (field, raw) in columns.iter().zip(&cells) {
            match parse_cell(&field.kind, raw) {
                Ok(cell) => values.push((field.id, cell)),
                Err(msg) => problems.push(format!("line {line}, column '{}': {msg}", field.name)),
            }
        }
        accepted.push(values);
    }
    if !problems.is_empty() {
        let shown: Vec<&str> = problems
            .iter()
            .take(MAX_REPORTED)
            .map(String::as_str)
            .collect();
        let more = problems.len().saturating_sub(MAX_REPORTED);
        let tail = if more > 0 {
            format!(" (and {more} more)")
        } else {
            String::new()
        };
        return Err(ApiError::bad_request(format!(
            "nothing was imported; fix these and try again: {}{tail}",
            shown.join("; ")
        )));
    }

    let mut tx = db.begin().await?;
    for values in &accepted {
        let mut qb = sqlx::QueryBuilder::new("INSERT INTO ");
        qb.push(api::data_table(table.id));
        if values.is_empty() {
            qb.push(" DEFAULT VALUES");
        } else {
            qb.push(" (");
            for (i, (field_id, _)) in values.iter().enumerate() {
                if i > 0 {
                    qb.push(", ");
                }
                qb.push(api::col(*field_id));
            }
            qb.push(") VALUES (");
            for (i, (_, cell)) in values.iter().enumerate() {
                if i > 0 {
                    qb.push(", ");
                }
                push_cell(&mut qb, cell);
            }
            qb.push(")");
        }
        qb.build().execute(&mut *tx).await?;
    }
    tx.commit().await?;
    Ok(accepted.len())
}

fn push_cell(qb: &mut sqlx::QueryBuilder<'_, sqlx::Sqlite>, cell: &Cell) {
    match cell {
        Cell::Null => {
            qb.push_bind(Option::<String>::None);
        }
        Cell::Text(s) => {
            qb.push_bind(s.clone());
        }
        Cell::Int(n) => {
            qb.push_bind(*n);
        }
        Cell::Real(x) => {
            qb.push_bind(*x);
        }
        Cell::Bool(b) => {
            qb.push_bind(*b);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_quotes_commas_and_both_line_endings() {
        let rows = parse("a,\"b,c\",\"say \"\"hi\"\"\"\r\n1,2,3\n").unwrap();
        assert_eq!(
            rows,
            vec![
                vec!["a".to_string(), "b,c".into(), "say \"hi\"".into()],
                vec!["1".to_string(), "2".into(), "3".into()],
            ]
        );
    }

    #[test]
    fn keeps_newlines_inside_quotes_and_literal_quotes_inside_fields() {
        let rows = parse("note,x\n\"two\nlines\",5\nsize 6\"\n").unwrap();
        assert_eq!(rows[1], vec!["two\nlines".to_string(), "5".into()]);
        assert_eq!(rows[2], vec!["size 6\"".to_string()]);
    }

    #[test]
    fn unterminated_quote_reports_its_line() {
        let err = parse("a,b\n\"open,\n").unwrap_err();
        assert!(err.contains("line 2"), "{err}");
    }

    #[test]
    fn writer_escapes_what_the_parser_reads_back() {
        let fields = vec![
            "plain".to_string(),
            "a,b".into(),
            "q\"uote".into(),
            "x\ny".into(),
        ];
        let mut out = String::new();
        write_row(&mut out, &fields);
        assert_eq!(parse(&out).unwrap(), vec![fields]);
    }

    #[test]
    fn cells_are_checked_against_their_kind() {
        assert_eq!(parse_cell("integer", " 42 "), Ok(Cell::Int(42)));
        assert!(parse_cell("integer", "4.5").is_err());
        assert_eq!(parse_cell("real", "2.5"), Ok(Cell::Real(2.5)));
        assert!(parse_cell("real", "NaN").is_err());
        assert_eq!(parse_cell("bool", "YES"), Ok(Cell::Bool(true)));
        assert!(parse_cell("bool", "maybe").is_err());
        assert_eq!(parse_cell("text", ""), Ok(Cell::Null));
        assert_eq!(parse_cell("integer", ""), Ok(Cell::Null));
    }
}
