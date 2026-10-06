// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 TPT Solutions

//! SQLite connection management, tuned for low-RAM home servers.

use std::path::Path;
use std::time::Duration;

/// Shared handle to the SQLite database used as router state everywhere.
pub type DbPool = sqlx::SqlitePool;

/// Open (creating if needed) the database at `path` with lean-RAM pragmas:
/// WAL journaling, `NORMAL` sync, foreign keys on, small page cache.
pub async fn connect(path: &Path) -> anyhow::Result<DbPool> {
    let opts = sqlx::sqlite::SqliteConnectOptions::new()
        .filename(path)
        .create_if_missing(true)
        .journal_mode(sqlx::sqlite::SqliteJournalMode::Wal)
        .synchronous(sqlx::sqlite::SqliteSynchronous::Normal)
        .foreign_keys(true)
        .busy_timeout(Duration::from_secs(5))
        .pragma("cache_size", "-2000") // ~2 MiB per connection
        .pragma("temp_store", "MEMORY");

    let pool = sqlx::sqlite::SqlitePoolOptions::new()
        .max_connections(4)
        .connect_with(opts)
        .await?;

    Ok(pool)
}

/// In-memory pool for tests. Each call returns an independent database.
pub async fn connect_in_memory() -> anyhow::Result<DbPool> {
    let opts = sqlx::sqlite::SqliteConnectOptions::new()
        .in_memory(true)
        .foreign_keys(true);
    let pool = sqlx::sqlite::SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(opts)
        .await?;
    Ok(pool)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn in_memory_pool_executes() {
        let db = connect_in_memory().await.unwrap();
        let n: (i64,) = sqlx::query_as("SELECT 1").fetch_one(&db).await.unwrap();
        assert_eq!(n.0, 1);
    }
}
