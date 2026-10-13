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
//!
//! A database that has core migrations this binary does not know was written
//! by a newer version. The runner refuses it rather than run an older schema
//! over it. Blueprint migrations this binary does not know (a blueprint that
//! was removed from the build) only produce a warning.

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

const CORE_PREFIX: &str = "tpt-home-core:";

fn sort_key(key: &str) -> (u8, &str) {
    // Core schema first, blueprints after, both lexicographic within group.
    if key.starts_with(CORE_PREFIX) {
        (0, key)
    } else {
        (1, key)
    }
}

/// Every registered migration, in the order they run.
fn ordered_entries() -> Vec<&'static MigrationEntry> {
    let mut entries: Vec<&MigrationEntry> = inventory::iter::<MigrationEntry>().collect();
    entries.sort_by_key(|e| sort_key(e.key));
    entries
}

async fn ensure_table(db: &DbPool) -> anyhow::Result<()> {
    sqlx::query(
        "CREATE TABLE IF NOT EXISTS _tpt_schema_migrations (
            key        TEXT PRIMARY KEY,
            applied_at INTEGER NOT NULL
        )",
    )
    .execute(db)
    .await?;
    Ok(())
}

/// Keys of every migration already applied to this database.
pub async fn applied_keys(db: &DbPool) -> anyhow::Result<HashSet<String>> {
    ensure_table(db).await?;
    let keys = sqlx::query_as::<_, (String,)>("SELECT key FROM _tpt_schema_migrations")
        .fetch_all(db)
        .await?
        .into_iter()
        .map(|(k,)| k)
        .collect();
    Ok(keys)
}

/// Keys this binary would apply now, in order. Empty when up to date.
pub async fn pending_keys(db: &DbPool) -> anyhow::Result<Vec<String>> {
    let applied = applied_keys(db).await?;
    let mut seen = HashSet::new();
    let mut pending = Vec::new();
    for entry in ordered_entries() {
        if !seen.insert(entry.key) {
            anyhow::bail!("duplicate migration key: {}", entry.key);
        }
        if !applied.contains(entry.key) {
            pending.push(entry.key.to_string());
        }
    }
    Ok(pending)
}

/// True when the database has any migration history. A new, empty database
/// has none, so nothing needs saving before its first migration.
pub async fn has_history(db: &DbPool) -> anyhow::Result<bool> {
    Ok(!applied_keys(db).await?.is_empty())
}

/// Refuse a database that has core migrations this binary does not know, and
/// warn about blueprint migrations it does not know.
pub async fn check_compatible(db: &DbPool) -> anyhow::Result<()> {
    let applied = applied_keys(db).await?;
    let known: HashSet<&str> = ordered_entries().iter().map(|e| e.key).collect();
    let mut unknown_core: Vec<&String> = applied
        .iter()
        .filter(|k| k.starts_with(CORE_PREFIX) && !known.contains(k.as_str()))
        .collect();
    if !unknown_core.is_empty() {
        unknown_core.sort();
        anyhow::bail!(
            "this database was upgraded by a newer tpt-homestack (unknown core migrations: {}). \
             Install the newer version, or restore a backup taken before that upgrade",
            unknown_core
                .iter()
                .map(|k| k.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        );
    }
    for key in applied
        .iter()
        .filter(|k| !k.starts_with(CORE_PREFIX) && !known.contains(k.as_str()))
    {
        tracing::warn!(
            key = key.as_str(),
            "database has a blueprint migration this build does not include"
        );
    }
    Ok(())
}

/// Apply all collected migrations that have not run yet. Returns the keys
/// applied in this call, in order. Refuses a database from a newer version.
pub async fn run_all(db: &DbPool) -> anyhow::Result<Vec<String>> {
    check_compatible(db).await?;
    let pending: HashSet<String> = pending_keys(db).await?.into_iter().collect();

    let mut applied_now = Vec::new();
    for entry in ordered_entries() {
        if !pending.contains(entry.key) {
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
            first.iter().any(|k| k.starts_with(CORE_PREFIX)),
            "core migrations should be present: {first:?}"
        );
        let second = run_all(&db).await.unwrap();
        assert!(second.is_empty(), "second run must apply nothing");
    }

    #[tokio::test]
    async fn pending_lists_what_run_all_would_apply() {
        let db = connect_in_memory().await.unwrap();
        assert!(!has_history(&db).await.unwrap());
        let pending = pending_keys(&db).await.unwrap();
        assert!(pending.iter().any(|k| k.starts_with(CORE_PREFIX)));
        run_all(&db).await.unwrap();
        assert!(pending_keys(&db).await.unwrap().is_empty());
        assert!(has_history(&db).await.unwrap());
    }

    #[tokio::test]
    async fn a_database_from_a_newer_version_is_refused_untouched() {
        let db = connect_in_memory().await.unwrap();
        run_all(&db).await.unwrap();
        sqlx::query("INSERT INTO _tpt_schema_migrations (key, applied_at) VALUES ('tpt-home-core:999_future', 0)")
            .execute(&db)
            .await
            .unwrap();
        let err = run_all(&db).await.unwrap_err().to_string();
        assert!(err.contains("newer tpt-homestack"), "{err}");
        assert!(err.contains("tpt-home-core:999_future"), "{err}");
        // Nothing new was applied.
        assert!(pending_keys(&db).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn an_unknown_blueprint_migration_is_tolerated() {
        let db = connect_in_memory().await.unwrap();
        run_all(&db).await.unwrap();
        sqlx::query("INSERT INTO _tpt_schema_migrations (key, applied_at) VALUES ('tpt-home-bp-gone:001', 0)")
            .execute(&db)
            .await
            .unwrap();
        assert!(run_all(&db).await.unwrap().is_empty());
    }
}
