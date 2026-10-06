// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 TPT Solutions

//! DB-backed sessions.
//!
//! The browser holds a random 256-bit token; the database stores only its
//! SHA-256 hash, so a database leak does not leak usable session cookies.

use axum::http::header::HeaderMap;
use sha2::{Digest, Sha256};
use sqlx::SqlitePool;
use std::time::Duration;

use super::AuthError;
use super::user::{AuthUser, Role};

/// Session cookie name.
pub const COOKIE_NAME: &str = "tpt_session";

/// Hash a token for storage / lookup.
fn token_hash(token: &str) -> String {
    let digest = Sha256::digest(token.as_bytes());
    hex(&digest)
}

fn hex(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push_str(&format!("{b:02x}"));
    }
    s
}

/// Issue a new session for `user_id`, returning the opaque token to set as a
/// cookie.
pub async fn create_session(
    db: &SqlitePool,
    user_id: i64,
    ttl: Duration,
) -> Result<String, AuthError> {
    // Two v4 uuids = 256 bits of OS randomness.
    let token = format!(
        "{}{}",
        uuid::Uuid::new_v4().simple(),
        uuid::Uuid::new_v4().simple()
    );
    let now = chrono::Utc::now().timestamp();
    let expires = now + ttl.as_secs() as i64;
    // Opportunistic purge keeps the table small without a cron.
    sqlx::query("DELETE FROM sessions WHERE expires_at < ?1")
        .bind(now)
        .execute(db)
        .await?;
    sqlx::query(
        "INSERT INTO sessions (token_hash, user_id, created_at, expires_at)
         VALUES (?1, ?2, ?3, ?4)",
    )
    .bind(token_hash(&token))
    .bind(user_id)
    .bind(now)
    .bind(expires)
    .execute(db)
    .await?;
    Ok(token)
}

/// Resolve a token to an authenticated user, or `None` when unknown/expired.
pub async fn resolve_session(db: &SqlitePool, token: &str) -> Result<Option<AuthUser>, AuthError> {
    let now = chrono::Utc::now().timestamp();
    let row = sqlx::query_as::<_, (i64, String, String, bool, i64)>(
        "SELECT u.id, u.username, u.role, u.disabled, s.expires_at
         FROM sessions s JOIN users u ON u.id = s.user_id
         WHERE s.token_hash = ?1",
    )
    .bind(token_hash(token))
    .fetch_optional(db)
    .await?;
    match row {
        Some((id, username, role, disabled, expires_at)) if expires_at > now && !disabled => {
            Ok(Some(AuthUser {
                id,
                username,
                role: Role::parse(&role).unwrap_or(Role::User),
            }))
        }
        _ => Ok(None),
    }
}

/// Revoke a single session (logout).
pub async fn revoke_session(db: &SqlitePool, token: &str) -> Result<(), AuthError> {
    sqlx::query("DELETE FROM sessions WHERE token_hash = ?1")
        .bind(token_hash(token))
        .execute(db)
        .await?;
    Ok(())
}

/// Revoke every session of a user (password change, disable, delete).
pub async fn revoke_user_sessions(db: &SqlitePool, user_id: i64) -> Result<(), AuthError> {
    sqlx::query("DELETE FROM sessions WHERE user_id = ?1")
        .bind(user_id)
        .execute(db)
        .await?;
    Ok(())
}

/// Delete expired sessions; returns how many were removed.
pub async fn purge_expired(db: &SqlitePool) -> Result<u64, AuthError> {
    let now = chrono::Utc::now().timestamp();
    let res = sqlx::query("DELETE FROM sessions WHERE expires_at < ?1")
        .bind(now)
        .execute(db)
        .await?;
    Ok(res.rows_affected())
}

/// Extract the session token from a `Cookie` header set.
pub fn parse_session_cookie(headers: &HeaderMap) -> Option<String> {
    for value in headers.get_all(axum::http::header::COOKIE) {
        let raw = value.to_str().ok()?;
        for pair in raw.split(';') {
            let pair = pair.trim();
            if let Some(rest) = pair.strip_prefix(COOKIE_NAME) {
                let rest = rest.strip_prefix('=')?.trim_matches('"');
                if !rest.is_empty() {
                    return Some(rest.to_string());
                }
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::user::{Role, create_user};
    use crate::db::connect_in_memory;
    use crate::migrate;
    use std::time::Duration;

    async fn setup() -> SqlitePool {
        let db = connect_in_memory().await.unwrap();
        migrate::run_all(&db).await.unwrap();
        db
    }

    #[tokio::test]
    async fn create_and_resolve() {
        let db = setup().await;
        let u = create_user(&db, "dave", "super-secret-1", Role::User)
            .await
            .unwrap();
        let token = create_session(&db, u.id, Duration::from_secs(3600))
            .await
            .unwrap();
        let auth = resolve_session(&db, &token).await.unwrap().unwrap();
        assert_eq!(auth.username, "dave");
        assert_eq!(auth.role, Role::User);
    }

    #[tokio::test]
    async fn expired_session_is_rejected() {
        let db = setup().await;
        let u = create_user(&db, "erin", "super-secret-1", Role::User)
            .await
            .unwrap();
        let token = create_session(&db, u.id, Duration::from_secs(0))
            .await
            .unwrap();
        tokio::time::sleep(Duration::from_millis(1100)).await;
        assert!(resolve_session(&db, &token).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn revoke_and_user_revocation() {
        let db = setup().await;
        let u = create_user(&db, "frank", "super-secret-1", Role::User)
            .await
            .unwrap();
        let t1 = create_session(&db, u.id, Duration::from_secs(3600))
            .await
            .unwrap();
        let t2 = create_session(&db, u.id, Duration::from_secs(3600))
            .await
            .unwrap();
        revoke_session(&db, &t1).await.unwrap();
        assert!(resolve_session(&db, &t1).await.unwrap().is_none());
        assert!(resolve_session(&db, &t2).await.unwrap().is_some());
        revoke_user_sessions(&db, u.id).await.unwrap();
        assert!(resolve_session(&db, &t2).await.unwrap().is_none());
    }

    #[test]
    fn cookie_parsing() {
        let mut headers = HeaderMap::new();
        headers.insert(
            axum::http::header::COOKIE,
            "other=1; tpt_session=abc123; x=y".parse().unwrap(),
        );
        assert_eq!(parse_session_cookie(&headers).as_deref(), Some("abc123"));

        let mut none = HeaderMap::new();
        assert!(parse_session_cookie(&none).is_none());
        none.insert(axum::http::header::COOKIE, "a=b".parse().unwrap());
        assert!(parse_session_cookie(&none).is_none());
    }
}
