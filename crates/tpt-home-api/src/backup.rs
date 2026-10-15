// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 TPT Solutions

//! Backups of the database and uploads, shared by the CLI, the admin API and
//! the scheduler. Each backup is one folder named after the unix time it was
//! taken: `<dir>/<ts>/tpt-<ts>.db` plus a copy of the uploads in `uploads/`.
//! `VACUUM INTO` takes a consistent snapshot, so a backup can run while the
//! server is serving requests.

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::Context;
use serde::Serialize;

use tpt_home_core::config::Config;
use tpt_home_core::db::DbPool;

/// One backup as the API and the admin page show it.
#[derive(Debug, Clone, Serialize)]
pub struct BackupInfo {
    /// Folder name: the unix time the backup was taken.
    pub name: String,
    pub created_at: i64,
    pub db_bytes: u64,
    pub has_uploads: bool,
    /// Full path to the database file, for `tpt-home restore`.
    pub path: String,
}

/// The default backups folder: `<data_dir>/backups`.
pub fn default_dir(cfg: &Config) -> PathBuf {
    cfg.data_dir.join("backups")
}

fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// Only all-digit folder names are backups. This keeps a request from naming
/// a path outside the backups folder.
pub fn valid_name(name: &str) -> bool {
    !name.is_empty() && name.len() <= 20 && name.bytes().all(|b| b.is_ascii_digit())
}

fn db_file(folder: &Path, name: &str) -> PathBuf {
    folder.join(format!("tpt-{name}.db"))
}

fn info(dir: &Path, name: &str) -> Option<BackupInfo> {
    let folder = dir.join(name);
    let db = db_file(&folder, name);
    let meta = std::fs::metadata(&db).ok()?;
    Some(BackupInfo {
        name: name.to_string(),
        created_at: name.parse().ok()?,
        db_bytes: meta.len(),
        has_uploads: folder.join("uploads").is_dir(),
        path: db.display().to_string(),
    })
}

/// Backups in `dir`, newest first. Other folders are ignored.
pub fn list(dir: &Path) -> Vec<BackupInfo> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut out: Vec<BackupInfo> = entries
        .flatten()
        .filter_map(|e| {
            let name = e.file_name().into_string().ok()?;
            if !valid_name(&name) {
                return None;
            }
            info(dir, &name)
        })
        .collect();
    out.sort_by(|a, b| b.created_at.cmp(&a.created_at).then(b.name.cmp(&a.name)));
    out
}

/// The database file of backup `name`, if that backup exists.
pub fn db_file_for(dir: &Path, name: &str) -> Option<PathBuf> {
    if !valid_name(name) {
        return None;
    }
    let path = db_file(&dir.join(name), name);
    path.is_file().then_some(path)
}

/// Create the next backup folder. Two backups in the same second take the
/// next free second, so an older backup is never overwritten.
fn new_folder(dir: &Path) -> anyhow::Result<(String, PathBuf)> {
    std::fs::create_dir_all(dir)?;
    let mut ts = now();
    loop {
        let folder = dir.join(ts.to_string());
        match std::fs::create_dir(&folder) {
            Ok(()) => return Ok((ts.to_string(), folder)),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => ts += 1,
            Err(e) => return Err(e).context("create backup folder"),
        }
    }
}

fn sql_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "''"))
}

/// Copy a directory tree recursively.
fn copy_dir(src: &Path, dst: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dst)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let target = dst.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_dir(&entry.path(), &target)?;
        } else {
            std::fs::copy(entry.path(), &target)?;
        }
    }
    Ok(())
}

/// Take a backup of the live database and uploads into `dir`.
pub async fn create(db: &DbPool, cfg: &Config, dir: &Path) -> anyhow::Result<BackupInfo> {
    let (name, folder) = new_folder(dir)?;
    let result = write_backup(db, cfg, &name, &folder).await;
    if result.is_err() {
        // Do not leave a half-written folder behind; it would look like a backup.
        let _ = std::fs::remove_dir_all(&folder);
    }
    result?;
    info(dir, &name).context("backup was written but cannot be read back")
}

async fn write_backup(db: &DbPool, cfg: &Config, name: &str, folder: &Path) -> anyhow::Result<()> {
    let dest = db_file(folder, name);
    let sql = format!("VACUUM INTO {}", sql_quote(&dest.display().to_string()));
    sqlx::query(&sql)
        .execute(db)
        .await
        .with_context(|| format!("VACUUM INTO {}", dest.display()))?;
    anyhow::ensure!(
        dest.is_file(),
        "VACUUM INTO wrote no file at {}",
        dest.display()
    );

    let uploads = cfg.uploads_dir();
    if uploads.is_dir() {
        let target = folder.join("uploads");
        tokio::task::spawn_blocking(move || copy_dir(&uploads, &target))
            .await?
            .context("copy uploads")?;
    }
    Ok(())
}

/// Copy the database (not the uploads) before migrations change it. The copy
/// goes in `<data>/backups/upgrade/`, which [`list`] and [`prune`] ignore, so
/// it stays until someone deletes it. Delete it once the upgrade is confirmed.
pub async fn upgrade_snapshot(db: &DbPool, cfg: &Config) -> anyhow::Result<PathBuf> {
    let dir = default_dir(cfg).join("upgrade");
    std::fs::create_dir_all(&dir)?;
    let path = dir.join(format!("{}-{}.db", now(), std::process::id()));
    let sql = format!("VACUUM INTO {}", sql_quote(&path.display().to_string()));
    sqlx::query(&sql)
        .execute(db)
        .await
        .with_context(|| format!("VACUUM INTO {}", path.display()))?;
    anyhow::ensure!(
        path.is_file(),
        "VACUUM INTO wrote no file at {}",
        path.display()
    );
    Ok(path)
}

/// Delete all but the newest `keep` backups in `dir`. Returns how many were
/// deleted. `keep` is raised to 1: the newest backup is never deleted.
pub fn prune(dir: &Path, keep: usize) -> anyhow::Result<usize> {
    let mut removed = 0;
    for old in list(dir).iter().skip(keep.max(1)) {
        std::fs::remove_dir_all(dir.join(&old.name))
            .with_context(|| format!("delete backup {}", old.name))?;
        removed += 1;
    }
    Ok(removed)
}

/// Refuse a file that is not a tpt-home database. Opened read-only, so the
/// check leaves no WAL or journal files next to it.
pub async fn check_db_file(path: &Path) -> anyhow::Result<()> {
    let opts = sqlx::sqlite::SqliteConnectOptions::new()
        .filename(path)
        .read_only(true);
    let pool = sqlx::sqlite::SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(opts)
        .await
        .with_context(|| format!("{} is not a readable database", path.display()))?;
    // Close the pool on every path. An open handle keeps the file locked on
    // Windows, so a rejected upload could not be deleted.
    let result = check_open(&pool, path).await;
    pool.close().await;
    result
}

async fn check_open(pool: &sqlx::SqlitePool, path: &Path) -> anyhow::Result<()> {
    let integrity: String = sqlx::query_scalar("PRAGMA quick_check")
        .fetch_one(pool)
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
    .fetch_one(pool)
    .await?;
    anyhow::ensure!(
        users == 1,
        "{} is not a tpt-home backup (no users table)",
        path.display()
    );
    Ok(())
}

/// Keep a database file received from outside (an upload) as a new backup in
/// `dir`. The file is checked first and is moved, not copied. Uploads are not
/// part of it: restoring that database with `tpt-home restore` does not bring
/// back the uploads.
pub async fn import(dir: &Path, staged: &Path) -> anyhow::Result<BackupInfo> {
    check_db_file(staged).await?;
    let (name, folder) = new_folder(dir)?;
    let dest = db_file(&folder, &name);
    if let Err(e) = std::fs::rename(staged, &dest) {
        let _ = std::fs::remove_dir_all(&folder);
        return Err(e).context("store uploaded backup");
    }
    info(dir, &name).context("uploaded backup cannot be read back")
}

/// Run scheduled backups every `backup_every_hours` and prune to
/// `backup_keep`. Returns at once when the interval is 0. The first backup is
/// taken one interval after start, not at start.
pub async fn schedule_loop(db: DbPool, cfg: Config) {
    if cfg.backup_every_hours == 0 {
        return;
    }
    let every = Duration::from_secs(cfg.backup_every_hours * 3600);
    let dir = default_dir(&cfg);
    let mut ticker = tokio::time::interval_at(tokio::time::Instant::now() + every, every);
    loop {
        ticker.tick().await;
        match create(&db, &cfg, &dir).await {
            Ok(info) => {
                tracing::info!(backup = %info.name, "scheduled backup written");
                match prune(&dir, cfg.backup_keep) {
                    Ok(0) => {}
                    Ok(n) => tracing::info!(removed = n, "old backups deleted"),
                    Err(e) => tracing::error!(error = %e, "pruning old backups failed"),
                }
            }
            Err(e) => tracing::error!(error = %e, "scheduled backup failed"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fake_backup(dir: &Path, name: &str) {
        let folder = dir.join(name);
        std::fs::create_dir_all(&folder).unwrap();
        std::fs::write(db_file(&folder, name), b"x").unwrap();
    }

    #[test]
    fn only_digit_names_are_backups() {
        assert!(valid_name("1700000000"));
        assert!(!valid_name(""));
        assert!(!valid_name(".."));
        assert!(!valid_name("12/34"));
        assert!(!valid_name("12a"));
        assert!(!valid_name(&"1".repeat(21)));
    }

    #[test]
    fn list_is_newest_first_and_ignores_other_folders() {
        let tmp = tempfile::tempdir().unwrap();
        fake_backup(tmp.path(), "100");
        fake_backup(tmp.path(), "300");
        fake_backup(tmp.path(), "200");
        std::fs::create_dir_all(tmp.path().join("staging")).unwrap();
        std::fs::create_dir_all(tmp.path().join("400")).unwrap(); // no database inside

        let names: Vec<String> = list(tmp.path()).into_iter().map(|b| b.name).collect();
        assert_eq!(names, ["300", "200", "100"]);
    }

    #[test]
    fn prune_keeps_the_newest_and_never_below_one() {
        let tmp = tempfile::tempdir().unwrap();
        for ts in ["100", "200", "300", "400"] {
            fake_backup(tmp.path(), ts);
        }
        assert_eq!(prune(tmp.path(), 2).unwrap(), 2);
        let names: Vec<String> = list(tmp.path()).into_iter().map(|b| b.name).collect();
        assert_eq!(names, ["400", "300"]);

        // keep = 0 still keeps the newest one.
        assert_eq!(prune(tmp.path(), 0).unwrap(), 1);
        let names: Vec<String> = list(tmp.path()).into_iter().map(|b| b.name).collect();
        assert_eq!(names, ["400"]);
    }

    #[test]
    fn db_file_for_refuses_traversal() {
        let tmp = tempfile::tempdir().unwrap();
        fake_backup(tmp.path(), "100");
        assert!(db_file_for(tmp.path(), "100").is_some());
        assert!(db_file_for(tmp.path(), "..").is_none());
        assert!(db_file_for(tmp.path(), "999").is_none());
    }
}
