// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 TPT Solutions

//! `tpt-home backup` and `tpt-home restore`. Taking and checking backups is
//! shared with the server (see `tpt_home_api::backup`); restoring replaces the
//! live database, so it is done here with the server stopped.

use std::path::{Path, PathBuf};

use anyhow::Context;

use tpt_home_core::config::Config;

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

/// `tpt-home backup` — writes one folder per backup:
/// `<out>/<ts>/tpt-<ts>.db` and `<out>/<ts>/uploads/`. Each folder holds its own
/// copy of the uploads, so an older backup is never overwritten by a newer one.
pub async fn backup(cfg: &Config, out_dir: Option<PathBuf>) -> anyhow::Result<()> {
    let out = out_dir.unwrap_or_else(|| tpt_home_api::backup::default_dir(cfg));
    let db = tpt_home_api::init_server(cfg.clone()).await?;
    let info = tpt_home_api::backup::create(&db, cfg, &out)
        .await
        .context("take backup")?;
    println!("database -> {}", info.path);
    if info.has_uploads {
        println!(
            "uploads  -> {}",
            Path::new(&info.path).with_file_name("uploads").display()
        );
    }
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
    tpt_home_api::backup::check_db_file(backup_db).await?;
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
