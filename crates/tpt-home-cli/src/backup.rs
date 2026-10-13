// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 TPT Solutions

//! Backup and restore: `VACUUM INTO` for a consistent database snapshot
//! (safe while the server is running) plus a copy of the uploads tree.

use std::path::{Path, PathBuf};

use anyhow::Context;

use tpt_home_core::config::Config;

fn ts_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Copy a directory tree recursively.
fn copy_dir(src: &Path, dst: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dst)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let file_type = entry.file_type()?;
        let target = dst.join(entry.file_name());
        if file_type.is_dir() {
            copy_dir(&entry.path(), &target)?;
        } else {
            std::fs::copy(entry.path(), &target)?;
        }
    }
    Ok(())
}

fn sql_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "''"))
}

/// `tpt-home backup` — writes one folder per backup:
/// `<out>/<ts>/tpt-<ts>.db` and `<out>/<ts>/uploads/`. Each folder holds its own
/// copy of the uploads, so an older backup is never overwritten by a newer one.
pub async fn backup(cfg: &Config, out_dir: Option<PathBuf>) -> anyhow::Result<()> {
    let out = out_dir.unwrap_or_else(|| cfg.data_dir.join("backups"));
    std::fs::create_dir_all(&out)?;

    let ts = ts_now();
    let folder = out.join(ts.to_string());
    // Not create_dir_all: two backups in the same second must not share a folder.
    std::fs::create_dir(&folder).with_context(|| {
        format!(
            "create backup folder {} (only one backup per second)",
            folder.display()
        )
    })?;
    let db_dest = folder.join(format!("tpt-{ts}.db"));

    let db = tpt_home_api::init_server(cfg.clone()).await?;
    let dest_sql = db_dest.display().to_string();
    sqlx::query(&format!("VACUUM INTO {}", sql_quote(&dest_sql)))
        .execute(&db)
        .await
        .with_context(|| format!("VACUUM INTO {}", db_dest.display()))?;

    let uploads_dest = folder.join("uploads");
    if cfg.uploads_dir().is_dir() {
        copy_dir(&cfg.uploads_dir(), &uploads_dest)?;
    }

    println!("database -> {}", db_dest.display());
    println!("uploads  -> {}", uploads_dest.display());
    Ok(())
}

/// Refuse a file that is not a tpt-home database. Opened read-only, so the
/// check leaves no WAL or journal files next to the backup.
async fn check_backup(path: &Path) -> anyhow::Result<()> {
    let opts = sqlx::sqlite::SqliteConnectOptions::new()
        .filename(path)
        .read_only(true);
    let pool = sqlx::sqlite::SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(opts)
        .await
        .with_context(|| format!("{} is not a readable database", path.display()))?;

    let integrity: String = sqlx::query_scalar("PRAGMA quick_check")
        .fetch_one(&pool)
        .await
        .with_context(|| format!("{} is not a SQLite database", path.display()))?;
    anyhow::ensure!(
        integrity == "ok",
        "{} failed its integrity check: {integrity}",
        path.display()
    );

    let users: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = 'users'",
    )
    .fetch_one(&pool)
    .await?;
    pool.close().await;
    anyhow::ensure!(
        users == 1,
        "{} is not a tpt-home backup (no users table)",
        path.display()
    );
    Ok(())
}

/// `tpt-home restore` — replace the live database (and optionally uploads)
/// from a backup. The server must be stopped; we do the swap via a temp file
/// + rename so a crash cannot corrupt the live database.
pub async fn restore(cfg: &Config, backup_db: &Path, yes: bool) -> anyhow::Result<()> {
    if !backup_db.is_file() {
        anyhow::bail!("backup not found: {}", backup_db.display());
    }
    // Check the file before it can replace the live database.
    check_backup(backup_db).await?;
    if !yes {
        println!(
            "This overwrites {} (and restores uploads if present next to the backup). Continue? [y/N]",
            cfg.db_path().display()
        );
        let mut line = String::new();
        std::io::stdin().read_line(&mut line)?;
        if !matches!(line.trim().to_ascii_lowercase().as_str(), "y" | "yes") {
            println!("aborted");
            return Ok(());
        }
    }

    let live = cfg.db_path();
    let tmp = live.with_extension("db.restore-tmp");
    std::fs::copy(backup_db, &tmp).with_context(|| "copying backup")?;
    // The live database runs in WAL mode. A leftover write-ahead log would be
    // replayed on top of the restored file, so drop the sidecars first.
    for sidecar in ["-wal", "-shm"] {
        let path = PathBuf::from(format!("{}{sidecar}", live.display()));
        if path.exists() {
            std::fs::remove_file(&path).with_context(|| format!("removing {}", path.display()))?;
        }
    }
    std::fs::rename(&tmp, &live).with_context(|| "swapping live database")?;

    let backup_uploads = backup_db.parent().unwrap_or(Path::new(".")).join("uploads");
    if backup_uploads.is_dir() {
        let live_uploads = cfg.uploads_dir();
        if live_uploads.is_dir() {
            let trash = live_uploads.with_extension("old");
            let _ = std::fs::remove_dir_all(&trash);
            std::fs::rename(&live_uploads, &trash)?;
            copy_dir(&backup_uploads, &live_uploads)?;
            let _ = std::fs::remove_dir_all(&trash);
        } else {
            copy_dir(&backup_uploads, &live_uploads)?;
        }
    }

    println!("restored from {}", backup_db.display());
    Ok(())
}
