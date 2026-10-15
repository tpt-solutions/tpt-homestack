// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 TPT Solutions

//! Track API. See the crate docs for the dynamic-SQL safety model.

use serde::{Deserialize, Serialize};
use sqlx::{Row, SqlitePool};

use tpt_home_core::prelude::*;

const KINDS: [&str; 4] = ["text", "integer", "real", "bool"];
const MAX_FIELDS: usize = 64;

#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct FieldRow {
    pub id: i64,
    pub name: String,
    pub kind: String,
    pub position: i64,
}

#[derive(Debug, Serialize)]
pub struct TableDef {
    pub id: i64,
    pub name: String,
    pub fields: Vec<FieldRow>,
}

#[derive(Debug, Deserialize)]
pub struct FieldInput {
    pub name: String,
    pub kind: String,
}

#[derive(Debug, Deserialize)]
pub struct CreateTable {
    pub name: String,
    pub fields: Vec<FieldInput>,
}

pub(crate) fn data_table(table_id: i64) -> String {
    // Integer from our own PK — cannot contain SQL syntax.
    format!("track_data_{table_id}")
}

pub(crate) fn col(field_id: i64) -> String {
    format!("f_{field_id}")
}

fn validate_label(label: &str, what: &str) -> Result<(), ApiError> {
    let len = label.trim().chars().count();
    if !(1..=64).contains(&len) {
        return Err(ApiError::bad_request(format!(
            "{what} must be 1-64 characters"
        )));
    }
    if label.contains(['\0', '\n', '\r']) {
        return Err(ApiError::bad_request(format!(
            "{what} contains invalid characters"
        )));
    }
    Ok(())
}

async fn load_fields(db: &SqlitePool, table_id: i64) -> Result<Vec<FieldRow>, ApiError> {
    let fields = sqlx::query_as::<_, FieldRow>(
        "SELECT id, name, kind, position FROM track_fields
         WHERE table_id = ?1 ORDER BY position, id",
    )
    .bind(table_id)
    .fetch_all(db)
    .await?;
    Ok(fields)
}

pub(crate) async fn load_table(db: &SqlitePool, table_id: i64) -> Result<TableDef, ApiError> {
    let (id, name): (i64, String) =
        sqlx::query_as("SELECT id, name FROM track_tables WHERE id = ?1")
            .bind(table_id)
            .fetch_optional(db)
            .await?
            .ok_or_else(|| ApiError::not_found("table not found"))?;
    Ok(TableDef {
        id,
        name,
        fields: load_fields(db, table_id).await?,
    })
}

/// GET /api/track/tables
pub async fn list_tables(db: &SqlitePool) -> Result<Vec<TableDef>, ApiError> {
    let tables: Vec<(i64, String)> =
        sqlx::query_as("SELECT id, name FROM track_tables ORDER BY id")
            .fetch_all(db)
            .await?;
    let mut out = Vec::with_capacity(tables.len());
    for (id, name) in tables {
        out.push(TableDef {
            id,
            name,
            fields: load_fields(db, id).await?,
        });
    }
    Ok(out)
}

/// POST /api/track/tables — create a table definition + its backing table.
pub async fn create_table(db: &SqlitePool, input: CreateTable) -> Result<TableDef, ApiError> {
    validate_label(&input.name, "table name")?;
    if input.fields.is_empty() {
        return Err(ApiError::bad_request("at least one field is required"));
    }
    if input.fields.len() > MAX_FIELDS {
        return Err(ApiError::bad_request(format!(
            "at most {MAX_FIELDS} fields"
        )));
    }
    for f in &input.fields {
        validate_label(&f.name, "field name")?;
        if !KINDS.contains(&f.kind.as_str()) {
            return Err(ApiError::bad_request(format!(
                "field '{}' kind must be one of: {}",
                f.name,
                KINDS.join(", ")
            )));
        }
    }
    let names: Vec<String> = input
        .fields
        .iter()
        .map(|f| f.name.trim().to_lowercase())
        .collect();
    if names.iter().collect::<std::collections::HashSet<_>>().len() != names.len() {
        return Err(ApiError::bad_request("duplicate field names"));
    }

    let now = unix_now();
    let (table_id,): (i64,) =
        sqlx::query_as("INSERT INTO track_tables (name, created_at) VALUES (?1, ?2) RETURNING id")
            .bind(input.name.trim())
            .bind(now)
            .fetch_one(db)
            .await?;

    let mut cols = String::from("id INTEGER PRIMARY KEY");
    for (pos, f) in input.fields.iter().enumerate() {
        let sql_type = match f.kind.as_str() {
            "text" => "TEXT",
            "integer" => "INTEGER",
            "real" => "REAL",
            "bool" => "INTEGER",
            _ => unreachable!("validated above"),
        };
        let res = sqlx::query(
            "INSERT INTO track_fields (table_id, name, kind, position) VALUES (?1, ?2, ?3, ?4)",
        )
        .bind(table_id)
        .bind(f.name.trim())
        .bind(&f.kind)
        .bind(pos as i64)
        .execute(db)
        .await;
        match res {
            Ok(done) => {
                cols.push_str(&format!(", {} {sql_type}", col(done.last_insert_rowid())));
            }
            Err(e) => {
                // Roll back the definition so we don't leave orphans.
                let _ = sqlx::query("DELETE FROM track_tables WHERE id = ?1")
                    .bind(table_id)
                    .execute(db)
                    .await;
                return Err(e.into());
            }
        }
    }

    let create_sql = format!("CREATE TABLE {} ({})", data_table(table_id), cols);
    sqlx::raw_sql(&create_sql).execute(db).await?;

    let table = load_table(db, table_id).await?;
    Ok(table)
}

/// DELETE /api/track/tables/{id}
pub async fn delete_table(db: &SqlitePool, id: i64) -> Result<(), ApiError> {
    load_table(db, id).await?;
    sqlx::raw_sql(&format!("DROP TABLE IF EXISTS {}", data_table(id)))
        .execute(db)
        .await?;
    sqlx::query("DELETE FROM track_tables WHERE id = ?1")
        .bind(id)
        .execute(db)
        .await?;
    Ok(())
}

/// Validate + bind one cell value according to the field kind.
fn bind_value<'q>(
    qb: &mut sqlx::QueryBuilder<'q, sqlx::Sqlite>,
    kind: &str,
    value: &serde_json::Value,
) -> Result<(), ApiError> {
    match kind {
        "text" => {
            let s = value.as_str().ok_or_else(|| {
                ApiError::bad_request(format!("expected a string for {kind} field, got {value}"))
            })?;
            // Owned binds keep the builder's 'q independent of the input.
            qb.push_bind(s.to_owned());
        }
        "integer" => {
            let n = value.as_i64().ok_or_else(|| {
                ApiError::bad_request(format!("expected an integer, got {value}"))
            })?;
            qb.push_bind(n);
        }
        "real" => {
            let x = value
                .as_f64()
                .ok_or_else(|| ApiError::bad_request(format!("expected a number, got {value}")))?;
            qb.push_bind(x);
        }
        "bool" => {
            let b = value
                .as_bool()
                .ok_or_else(|| ApiError::bad_request(format!("expected a boolean, got {value}")))?;
            qb.push_bind(b);
        }
        other => return Err(ApiError::bad_request(format!("unknown kind {other}"))),
    }
    Ok(())
}

/// GET /api/track/tables/{id}/rows — raw query string for `?f_<id>=<value>`
/// filters plus limit/offset.
pub async fn list_rows(
    db: &SqlitePool,
    id: i64,
    query: Option<String>,
) -> Result<Vec<serde_json::Value>, ApiError> {
    let table = load_table(db, id).await?;
    let mut qb = sqlx::QueryBuilder::new("SELECT ");
    let cols: Vec<String> = std::iter::once("id".to_string())
        .chain(table.fields.iter().map(|f| col(f.id)))
        .collect();
    qb.push(cols.join(", "));
    qb.push(" FROM ");
    qb.push(data_table(table.id));

    let mut filters: Vec<(i64, String)> = Vec::new();
    if let Some(qs) = query.as_deref() {
        for pair in qs.split('&') {
            let mut it = pair.splitn(2, '=');
            let key = it.next().unwrap_or_default();
            let value = it.next().unwrap_or_default().replace("%3D", "=");
            let value = urldecode(&value);
            if key == "limit" || key == "offset" {
                continue;
            }
            // Accept either the internal `f_<id>` column name or the human
            // field name; only known fields become SQL (and values are bound,
            // never interpolated).
            let fid = if let Some(rest) = key.strip_prefix("f_") {
                rest.parse::<i64>().ok()
            } else {
                table
                    .fields
                    .iter()
                    .find(|f| f.name.eq_ignore_ascii_case(key))
                    .map(|f| f.id)
            };
            if let Some(fid) = fid
                && table.fields.iter().any(|f| f.id == fid)
            {
                filters.push((fid, value));
            }
        }
    }

    if !filters.is_empty() {
        qb.push(" WHERE ");
        for (i, (fid, value)) in filters.iter().enumerate() {
            if i > 0 {
                qb.push(" AND ");
            }
            let field = table
                .fields
                .iter()
                .find(|f| f.id == *fid)
                .expect("checked above");
            qb.push(col(*fid));
            qb.push(" = ");
            bind_value(
                &mut qb,
                &field.kind,
                &serde_json::Value::String(value.clone()),
            )?;
        }
    }
    qb.push(" ORDER BY id DESC LIMIT ");
    qb.push_bind(200i64);

    let rows = qb.build().fetch_all(db).await?;
    let out = rows_to_json(&table.fields, &rows);
    Ok(out)
}

fn rows_to_json(fields: &[FieldRow], rows: &[sqlx::sqlite::SqliteRow]) -> Vec<serde_json::Value> {
    rows.iter()
        .map(|row| {
            let mut obj = serde_json::Map::new();
            obj.insert("id".into(), serde_json::json!(row.get::<i64, _>("id")));
            for f in fields {
                let c = col(f.id);
                let value = match f.kind.as_str() {
                    "text" => serde_json::json!(row.get::<Option<String>, _>(c.as_str())),
                    "integer" => serde_json::json!(row.get::<Option<i64>, _>(c.as_str())),
                    "real" => serde_json::json!(row.get::<Option<f64>, _>(c.as_str())),
                    "bool" => serde_json::json!(row.get::<Option<bool>, _>(c.as_str())),
                    _ => serde_json::Value::Null,
                };
                obj.insert(c, value);
            }
            serde_json::Value::Object(obj)
        })
        .collect()
}

fn collect_values<'a>(
    fields: &'a [FieldRow],
    values: &'a serde_json::Map<String, serde_json::Value>,
) -> Result<Vec<(&'a FieldRow, &'a serde_json::Value)>, ApiError> {
    let mut out = Vec::new();
    for (key, value) in values {
        let field = fields
            .iter()
            .find(|f| col(f.id) == *key || f.name.eq_ignore_ascii_case(key))
            .ok_or_else(|| ApiError::bad_request(format!("unknown field '{key}'")))?;
        if !value.is_null() {
            out.push((field, value));
        }
    }
    Ok(out)
}

/// POST /api/track/tables/{id}/rows
pub async fn create_row(
    db: &SqlitePool,
    id: i64,
    values: serde_json::Map<String, serde_json::Value>,
) -> Result<(), ApiError> {
    let table = load_table(db, id).await?;
    let pairs = collect_values(&table.fields, &values)?;

    let mut qb = sqlx::QueryBuilder::new("INSERT INTO ");
    qb.push(data_table(table.id));
    qb.push(" (");
    for (i, (field, _)) in pairs.iter().enumerate() {
        if i > 0 {
            qb.push(", ");
        }
        qb.push(col(field.id));
    }
    qb.push(") VALUES (");
    for (i, (field, value)) in pairs.iter().enumerate() {
        if i > 0 {
            qb.push(", ");
        }
        bind_value(&mut qb, &field.kind, value)?;
    }
    qb.push(")");

    qb.build().execute(db).await?;
    Ok(())
}

/// PATCH /api/track/tables/{id}/rows/{row}
pub async fn update_row(
    db: &SqlitePool,
    id: i64,
    row: i64,
    values: serde_json::Map<String, serde_json::Value>,
) -> Result<(), ApiError> {
    let table = load_table(db, id).await?;
    let pairs = collect_values(&table.fields, &values)?;
    if pairs.is_empty() {
        return Err(ApiError::bad_request("no fields to update"));
    }

    let mut qb = sqlx::QueryBuilder::new("UPDATE ");
    qb.push(data_table(table.id));
    qb.push(" SET ");
    for (i, (field, value)) in pairs.iter().enumerate() {
        if i > 0 {
            qb.push(", ");
        }
        qb.push(col(field.id));
        qb.push(" = ");
        bind_value(&mut qb, &field.kind, value)?;
    }
    qb.push(" WHERE id = ");
    qb.push_bind(row);

    let res = qb.build().execute(db).await?;
    if res.rows_affected() == 0 {
        return Err(ApiError::not_found("row not found"));
    }
    Ok(())
}

/// DELETE /api/track/tables/{id}/rows/{row}
pub async fn delete_row(db: &SqlitePool, id: i64, row: i64) -> Result<(), ApiError> {
    let table = load_table(db, id).await?;
    let mut qb = sqlx::QueryBuilder::new("DELETE FROM ");
    qb.push(data_table(table.id));
    qb.push(" WHERE id = ");
    qb.push_bind(row);
    let res = qb.build().execute(db).await?;
    if res.rows_affected() == 0 {
        return Err(ApiError::not_found("row not found"));
    }
    Ok(())
}

fn urldecode(s: &str) -> String {
    let mut out = Vec::with_capacity(s.len());
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'%' if i + 2 < bytes.len() + 1 && i + 2 < bytes.len() + 1 => {
                let hex = bytes.get(i + 1..i + 3).and_then(|h| {
                    std::str::from_utf8(h)
                        .ok()
                        .and_then(|h| u8::from_str_radix(h, 16).ok())
                });
                match hex {
                    Some(b) => {
                        out.push(b);
                        i += 3;
                    }
                    None => {
                        out.push(bytes[i]);
                        i += 1;
                    }
                }
            }
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            b => {
                out.push(b);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn unix_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}
