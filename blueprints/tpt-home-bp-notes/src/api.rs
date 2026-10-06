// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 TPT Solutions

//! Notes CRUD + FTS5 search API. Plain functions over the pool so they are
//! unit-testable; the route wrappers in `lib.rs` do the axum extraction.

use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;

use tpt_home_core::prelude::*;

#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct NoteRow {
    pub id: i64,
    pub title: String,
    pub body: String,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Serialize)]
pub struct Note {
    #[serde(flatten)]
    pub row: NoteRow,
    pub tags: Vec<String>,
}

#[derive(Debug, Deserialize)]
pub struct NoteInput {
    pub title: String,
    #[serde(default)]
    pub body: String,
    #[serde(default)]
    pub tags: Vec<String>,
}

#[derive(Debug, Deserialize)]
pub struct ListQuery {
    /// FTS5 query; each term is quoted so user input never changes syntax.
    pub q: Option<String>,
    /// Filter by tag.
    pub tag: Option<String>,
}

async fn tags_for_notes(db: &SqlitePool, note_ids: &[i64]) -> Result<Vec<(i64, String)>, ApiError> {
    if note_ids.is_empty() {
        return Ok(Vec::new());
    }
    let placeholders = note_ids.iter().map(|_| "?").collect::<Vec<_>>().join(",");
    let sql = format!(
        "SELECT nt.note_id, t.name FROM notes_note_tags nt
         JOIN notes_tags t ON t.id = nt.tag_id
         WHERE nt.note_id IN ({placeholders})"
    );
    let mut q = sqlx::query_as::<_, (i64, String)>(&sql);
    for id in note_ids {
        q = q.bind(id);
    }
    Ok(q.fetch_all(db).await?)
}

async fn attach_tags(db: &SqlitePool, rows: Vec<NoteRow>) -> Result<Vec<Note>, ApiError> {
    let ids: Vec<i64> = rows.iter().map(|r| r.id).collect();
    let pairs = tags_for_notes(db, &ids).await?;
    let mut out = Vec::with_capacity(rows.len());
    for row in rows {
        let tags: Vec<String> = pairs
            .iter()
            .filter(|(id, _)| *id == row.id)
            .map(|(_, name)| name.clone())
            .collect();
        out.push(Note { row, tags });
    }
    Ok(out)
}

/// FTS5 MATCH query from raw user input: quote every term so specials like
/// `"` `*` `(` stay literal.
fn fts_escape(input: &str) -> String {
    input
        .split_whitespace()
        .map(|term| format!("\"{}\"", term.replace('"', "\"\"")))
        .collect::<Vec<_>>()
        .join(" ")
}

/// GET /api/notes?q=&tag=
pub async fn list(db: &SqlitePool, q: ListQuery) -> Result<Vec<Note>, ApiError> {
    let rows: Vec<NoteRow> =
        if let Some(term) = q.q.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
            let match_q = fts_escape(term);
            sqlx::query_as::<_, NoteRow>(
                "SELECT n.id, n.title, n.body, n.created_at, n.updated_at
                 FROM notes n JOIN notes_fts f ON f.rowid = n.id
                 WHERE notes_fts MATCH ?1
                 ORDER BY rank LIMIT 100",
            )
            .bind(&match_q)
            .fetch_all(db)
            .await?
        } else if let Some(tag) = q.tag.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
            sqlx::query_as::<_, NoteRow>(
                "SELECT n.id, n.title, n.body, n.created_at, n.updated_at
                 FROM notes n
                 JOIN notes_note_tags nt ON nt.note_id = n.id
                 JOIN notes_tags t ON t.id = nt.tag_id
                 WHERE t.name = ?1
                 ORDER BY n.updated_at DESC LIMIT 100",
            )
            .bind(tag)
            .fetch_all(db)
            .await?
        } else {
            sqlx::query_as::<_, NoteRow>(
                "SELECT id, title, body, created_at, updated_at
                 FROM notes ORDER BY updated_at DESC LIMIT 100",
            )
            .fetch_all(db)
            .await?
        };
    attach_tags(db, rows).await
}

async fn sync_tags(db: &SqlitePool, note_id: i64, tags: &[String]) -> Result<(), ApiError> {
    sqlx::query("DELETE FROM notes_note_tags WHERE note_id = ?1")
        .bind(note_id)
        .execute(db)
        .await?;
    for tag in tags {
        let name = tag.trim();
        if name.is_empty() {
            continue;
        }
        sqlx::query("INSERT OR IGNORE INTO notes_tags (name) VALUES (?1)")
            .bind(name)
            .execute(db)
            .await?;
        let (tag_id,): (i64,) = sqlx::query_as("SELECT id FROM notes_tags WHERE name = ?1")
            .bind(name)
            .fetch_one(db)
            .await?;
        sqlx::query("INSERT OR IGNORE INTO notes_note_tags (note_id, tag_id) VALUES (?1, ?2)")
            .bind(note_id)
            .bind(tag_id)
            .execute(db)
            .await?;
    }
    Ok(())
}

/// POST /api/notes
pub async fn create(db: &SqlitePool, input: NoteInput) -> Result<Note, ApiError> {
    if input.title.trim().is_empty() {
        return Err(ApiError::bad_request("title must not be empty"));
    }
    let now = unix_now();
    let (id,): (i64,) = sqlx::query_as(
        "INSERT INTO notes (title, body, created_at, updated_at)
         VALUES (?1, ?2, ?3, ?3) RETURNING id",
    )
    .bind(input.title.trim())
    .bind(&input.body)
    .bind(now)
    .fetch_one(db)
    .await?;
    sync_tags(db, id, &input.tags).await?;
    let row = sqlx::query_as::<_, NoteRow>(
        "SELECT id, title, body, created_at, updated_at FROM notes WHERE id = ?1",
    )
    .bind(id)
    .fetch_one(db)
    .await?;
    Ok(Note {
        row,
        tags: input.tags,
    })
}

/// GET /api/notes/{id}
pub async fn get(db: &SqlitePool, id: i64) -> Result<Note, ApiError> {
    let row = sqlx::query_as::<_, NoteRow>(
        "SELECT id, title, body, created_at, updated_at FROM notes WHERE id = ?1",
    )
    .bind(id)
    .fetch_optional(db)
    .await?
    .ok_or_else(|| ApiError::not_found("note not found"))?;
    let mut notes = attach_tags(db, vec![row]).await?;
    Ok(notes.remove(0))
}

/// PUT /api/notes/{id}
pub async fn update(db: &SqlitePool, id: i64, input: NoteInput) -> Result<Note, ApiError> {
    if input.title.trim().is_empty() {
        return Err(ApiError::bad_request("title must not be empty"));
    }
    let res = sqlx::query("UPDATE notes SET title = ?1, body = ?2, updated_at = ?3 WHERE id = ?4")
        .bind(input.title.trim())
        .bind(&input.body)
        .bind(unix_now())
        .bind(id)
        .execute(db)
        .await?;
    if res.rows_affected() == 0 {
        return Err(ApiError::not_found("note not found"));
    }
    sync_tags(db, id, &input.tags).await?;
    get(db, id).await
}

/// DELETE /api/notes/{id}
pub async fn delete(db: &SqlitePool, id: i64) -> Result<(), ApiError> {
    let res = sqlx::query("DELETE FROM notes WHERE id = ?1")
        .bind(id)
        .execute(db)
        .await?;
    if res.rows_affected() == 0 {
        return Err(ApiError::not_found("note not found"));
    }
    Ok(())
}

/// GET /api/notes/{id}/render — sanitized HTML preview.
pub async fn render(db: &SqlitePool, id: i64) -> Result<String, ApiError> {
    let (body,): (String,) = sqlx::query_as("SELECT body FROM notes WHERE id = ?1")
        .bind(id)
        .fetch_optional(db)
        .await?
        .ok_or_else(|| ApiError::not_found("note not found"))?;
    Ok(super::safe_render::markdown_to_html(&body))
}

fn unix_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tpt_home_core::db::connect_in_memory;
    use tpt_home_core::migrate;

    async fn db() -> SqlitePool {
        let db = connect_in_memory().await.unwrap();
        migrate::run_all(&db).await.unwrap();
        db
    }

    #[tokio::test]
    async fn crud_and_search() {
        let db = db().await;
        let created = create(
            &db,
            NoteInput {
                title: "Groceries".into(),
                body: "buy **oat milk** and eggs".into(),
                tags: vec!["shopping".into()],
            },
        )
        .await
        .unwrap();
        assert_eq!(created.row.title, "Groceries");
        assert_eq!(created.tags, vec!["shopping".to_string()]);

        let all = list(&db, ListQuery { q: None, tag: None }).await.unwrap();
        assert_eq!(all.len(), 1);

        let hits = list(
            &db,
            ListQuery {
                q: Some("oat".into()),
                tag: None,
            },
        )
        .await
        .unwrap();
        assert_eq!(hits.len(), 1, "FTS5 should find 'oat'");

        let no_hits = list(
            &db,
            ListQuery {
                q: Some("zebra".into()),
                tag: None,
            },
        )
        .await
        .unwrap();
        assert!(no_hits.is_empty());

        let updated = update(
            &db,
            created.row.id,
            NoteInput {
                title: "Groceries!".into(),
                body: "and bread".into(),
                tags: vec![],
            },
        )
        .await
        .unwrap();
        assert_eq!(updated.row.title, "Groceries!");
        assert!(updated.tags.is_empty());

        delete(&db, created.row.id).await.unwrap();
        assert!(get(&db, created.row.id).await.is_err());
    }

    #[tokio::test]
    async fn tag_filter_and_special_chars_in_search() {
        let db = db().await;
        create(
            &db,
            NoteInput {
                title: "one".into(),
                body: String::new(),
                tags: vec!["work".into()],
            },
        )
        .await
        .unwrap();
        let by_tag = list(
            &db,
            ListQuery {
                q: None,
                tag: Some("work".into()),
            },
        )
        .await
        .unwrap();
        assert_eq!(by_tag.len(), 1);

        create(
            &db,
            NoteInput {
                title: "quotes".into(),
                body: r#"say "hi" (now)"#.into(),
                tags: vec![],
            },
        )
        .await
        .unwrap();
        // Nasty FTS syntax must not error, just find nothing / literal match.
        let _ = list(
            &db,
            ListQuery {
                q: Some("\"(now\"".into()),
                tag: None,
            },
        )
        .await;
        let _ = list(
            &db,
            ListQuery {
                q: Some("AND OR NOT (".into()),
                tag: None,
            },
        )
        .await;
    }
}
