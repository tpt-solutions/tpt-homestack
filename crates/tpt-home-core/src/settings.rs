// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 TPT Solutions

//! Server-wide settings kept in the `core_kv` table: the server's name and
//! whether the first-run wizard is finished.

use crate::db::DbPool;

pub const SERVER_NAME: &str = "server_name";
pub const WIZARD_DONE: &str = "wizard_done";
pub const DEFAULT_SERVER_NAME: &str = "homestack";

const MAX_NAME_CHARS: usize = 64;

pub async fn get(db: &DbPool, key: &str) -> Result<Option<String>, sqlx::Error> {
    let row: Option<(String,)> = sqlx::query_as("SELECT value FROM core_kv WHERE key = ?")
        .bind(key)
        .fetch_optional(db)
        .await?;
    Ok(row.map(|r| r.0))
}

pub async fn set(db: &DbPool, key: &str, value: &str) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO core_kv (key, value) VALUES (?, ?)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
    )
    .bind(key)
    .bind(value)
    .execute(db)
    .await?;
    Ok(())
}

/// The server's name, or the default when none is set (or the read fails).
pub async fn server_name(db: &DbPool) -> String {
    get(db, SERVER_NAME)
        .await
        .ok()
        .flatten()
        .unwrap_or_else(|| DEFAULT_SERVER_NAME.to_string())
}

pub async fn wizard_done(db: &DbPool) -> bool {
    matches!(get(db, WIZARD_DONE).await, Ok(Some(v)) if v == "1")
}

fn must_change_key(user_id: i64) -> String {
    format!("must_change_password:{user_id}")
}

/// Mark a user whose password was set by someone else (an admin), so they must
/// choose their own before using the app. `false` clears the mark.
pub async fn set_must_change_password(
    db: &DbPool,
    user_id: i64,
    required: bool,
) -> Result<(), sqlx::Error> {
    if required {
        set(db, &must_change_key(user_id), "1").await
    } else {
        sqlx::query("DELETE FROM core_kv WHERE key = ?")
            .bind(must_change_key(user_id))
            .execute(db)
            .await?;
        Ok(())
    }
}

pub async fn must_change_password(db: &DbPool, user_id: i64) -> bool {
    matches!(get(db, &must_change_key(user_id)).await, Ok(Some(v)) if v == "1")
}

/// A usable server name: trimmed, 1 to 64 characters, no control characters.
pub fn valid_server_name(name: &str) -> Option<String> {
    let name = name.trim();
    let ok = !name.is_empty()
        && name.chars().count() <= MAX_NAME_CHARS
        && !name.chars().any(char::is_control);
    ok.then(|| name.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn server_names_are_trimmed_and_bounded() {
        assert_eq!(
            valid_server_name("  Hall House ").as_deref(),
            Some("Hall House")
        );
        assert_eq!(valid_server_name(""), None);
        assert_eq!(valid_server_name("   "), None);
        assert_eq!(valid_server_name("tab\there"), None);
        assert_eq!(
            valid_server_name(&"a".repeat(64)).map(|s| s.len()),
            Some(64)
        );
        assert_eq!(valid_server_name(&"a".repeat(65)), None);
    }
}
