// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 TPT Solutions

//! Core database schema: users, roles, sessions and the migration tracker.
//!
//! These are registered directly (no macro needed inside core) and are always
//! applied before any blueprint migration.

use crate::migrate::MigrationEntry;

inventory::submit! {
    MigrationEntry {
        key: "tpt-home-core:001_users",
        sql: r#"
            CREATE TABLE IF NOT EXISTS users (
                id            INTEGER PRIMARY KEY,
                username      TEXT    NOT NULL UNIQUE COLLATE NOCASE,
                password_hash TEXT    NOT NULL,
                role          TEXT    NOT NULL DEFAULT 'user' CHECK (role IN ('admin', 'user')),
                disabled      INTEGER NOT NULL DEFAULT 0,
                created_at    INTEGER NOT NULL
            );
        "#,
    }
}

inventory::submit! {
    MigrationEntry {
        key: "tpt-home-core:002_sessions",
        sql: r#"
            CREATE TABLE IF NOT EXISTS sessions (
                token_hash TEXT    PRIMARY KEY,
                user_id    INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
                created_at INTEGER NOT NULL,
                expires_at INTEGER NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_sessions_user ON sessions(user_id);
            CREATE INDEX IF NOT EXISTS idx_sessions_expiry ON sessions(expires_at);
        "#,
    }
}

inventory::submit! {
    MigrationEntry {
        key: "tpt-home-core:003_kv",
        sql: r#"
            CREATE TABLE IF NOT EXISTS core_kv (
                key   TEXT PRIMARY KEY,
                value TEXT NOT NULL
            );
        "#,
    }
}

inventory::submit! {
    MigrationEntry {
        key: "tpt-home-core:004_api_tokens",
        sql: r#"
            CREATE TABLE IF NOT EXISTS api_tokens (
                id           INTEGER PRIMARY KEY,
                user_id      INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
                name         TEXT    NOT NULL,
                token_hash   TEXT    NOT NULL UNIQUE,
                scopes       TEXT    NOT NULL,
                created_at   INTEGER NOT NULL,
                expires_at   INTEGER,
                last_used_at INTEGER
            );
            CREATE INDEX IF NOT EXISTS idx_api_tokens_user ON api_tokens(user_id);
        "#,
    }
}
