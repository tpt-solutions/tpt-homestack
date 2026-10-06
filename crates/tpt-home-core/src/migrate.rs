// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 TPT Solutions

//! Migration runner.
//!
//! Migrations arrive from two sources:
//! - the core's own schema ([`crate::migrations`], registered directly)
//! - every blueprint, via the `#[bp_migration]` macro (also registered as
//!   [`MigrationEntry`] through `inventory`)
//!
//! Entries are keyed `"<crate>:<name>"`; core migrations are applied first so
//! blueprints can rely on the users/sessions schema existing.

use std::collections::HashSet;

use crate::db::DbPool;

/// One forward-only SQL migration. Register with `inventory::submit!` (what
/// `#[bp_migration]` generates) or the `#[bp_migration]` macro directly.
pub struct MigrationEntry {
    /// Globally unique id, conventionally `"<crate>:<name>"`.
    pub key: &'static str,
    /// SQL batch; may contain multiple statements separated by `;`.
    pub sql: &'static str,
}

inventory::collect!(MigrationEntry);

fn sort_key(key: &str) -> (u8, &str) {
    // Core schema first, blueprints after, both lexicographic within group.
    if key.starts_with("tpt-home-core:") {
        (0, key)
    } else {
        (1, key)
    }
}

/// Apply all collected migrations that have not run yet. Returns the keys
/// applied in this call, in order.
pub async fn run_all(db: &DbPool) -> anyhow::Result<Vec<String>> {
    sqlx::query(
        "CREATE TABLE IF NOT EXISTS _tpt_schema_migrations (
            key        TEXT PRIMARY KEY,
            applied_at INTEGER NOT NULL
        )",
    )
    .execute(db)
    .await?;

    let mut entries: Vec<&MigrationEntry> = inventory::iter::<MigrationEntry>().collect();
    entries.sort_by_key(|e| sort_key(e.key));

    let applied: HashSet<String> =
        sqlx::query_as::<_, (String,)>("SELECT key FROM _tpt_schema_migrations")
            .fetch_all(db)
            .await?
            .into_iter()
            .map(|(k,)| k)
            .collect();

    let mut applied_now = Vec::new();
    let mut seen = HashSet::new();
    for entry in entries {
        if !seen.insert(entry.key) {
            anyhow::bail!("duplicate migration key: {}", entry.key);
        }
        if applied.contains(entry.key) {
            continue;
        }
        let mut tx = db.begin().await?;
        sqlx::raw_sql(entry.sql).execute(&mut *tx).await?;
        let now = chrono::Utc::now().timestamp();
        sqlx::query("INSERT INTO _tpt_schema_migrations (key, applied_at) VALUES (?1, ?2)")
            .bind(entry.key)
            .bind(now)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        tracing::info!(key = entry.key, "applied migration");
        applied_now.push(entry.key.to_string());
    }
    Ok(applied_now)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::connect_in_memory;

    #[tokio::test]
    async fn runs_core_migrations_and_is_idempotent() {
        let db = connect_in_memory().await.unwrap();
        let first = run_all(&db).await.unwrap();
        assert!(
            first.iter().any(|k| k.starts_with("tpt-home-core:")),
            "core migrations should be present: {first:?}"
        );
        let second = run_all(&db).await.unwrap();
        assert!(second.is_empty(), "second run must apply nothing");
    }
}
