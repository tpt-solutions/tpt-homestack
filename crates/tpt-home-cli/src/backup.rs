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

/// `tpt-home backup` — writes `<out>/tpt-<ts>.db` and `<out>/uploads/`.
pub async fn backup(cfg: &Config, out_dir: Option<PathBuf>) -> anyhow::Result<()> {
    let out = out_dir.unwrap_or_else(|| cfg.data_dir.join("backups"));
    std::fs::create_dir_all(&out)?;

    let ts = ts_now();
    let db_dest = out.join(format!("tpt-{ts}.db"));

    let db = tpt_home_api::init_server(cfg.clone()).await?;
    let dest_sql = db_dest.display().to_string();
    sqlx::query(&format!("VACUUM INTO {}", sql_quote(&dest_sql)))
        .execute(&db)
        .await
        .with_context(|| format!("VACUUM INTO {}", db_dest.display()))?;

    let uploads_dest = out.join("uploads");
    if cfg.uploads_dir().is_dir() {
        copy_dir(&cfg.uploads_dir(), &uploads_dest)?;
    }

    println!("database -> {}", db_dest.display());
    println!("uploads  -> {}", uploads_dest.display());
    Ok(())
}

/// `tpt-home restore` — replace the live database (and optionally uploads)
/// from a backup. The server must be stopped; we do the swap via a temp file
/// + rename so a crash cannot corrupt the live database.
pub async fn restore(cfg: &Config, backup_db: &Path, yes: bool) -> anyhow::Result<()> {
    if !backup_db.is_file() {
        anyhow::bail!("backup not found: {}", backup_db.display());
    }
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
