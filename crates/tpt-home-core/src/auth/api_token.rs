// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 TPT Solutions

//! API tokens: long-lived bearer credentials for scripts, agents and external
//! plugin processes.
//!
//! A token is `tpt_` followed by 256 bits of OS randomness. The database stores
//! only its SHA-256 hash, and the plaintext is shown once, at creation.
//!
//! Each token carries scopes that limit where it may be used:
//! - `api`: every `/api` route the owning user may call.
//! - `plugin:<id>`: only that plugin's routes (`/api/<id>/…`) and its host
//!   services (`/api/plugin-host/<id>/…`).

use std::time::Duration;

use sha2::{Digest, Sha256};
use sqlx::SqlitePool;

use super::AuthError;
use super::user::{AuthUser, Role};

/// Prefix on every token, so leaked tokens are recognizable to secret scanners.
pub const TOKEN_PREFIX: &str = "tpt_";

/// A token that authenticated a request.
#[derive(Debug, Clone)]
pub struct ResolvedToken {
    pub user: AuthUser,
    pub scopes: Vec<String>,
}

/// The scopes of the credential that authenticated a request, attached as a
/// request extension. Cookie sessions carry `["api"]`.
#[derive(Debug, Clone)]
pub struct TokenScopes(pub Vec<String>);

/// Token metadata for listings. The secret and its hash are never included.
#[derive(Debug, Clone)]
pub struct TokenInfo {
    pub id: i64,
    pub user_id: i64,
    pub username: String,
    pub name: String,
    pub scopes: Vec<String>,
    pub created_at: i64,
    pub expires_at: Option<i64>,
    pub last_used_at: Option<i64>,
}

fn token_hash(token: &str) -> String {
    Sha256::digest(token.as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// Validate a scope string: `api`, or `plugin:<id>` with a plugin id.
pub fn validate_scope(scope: &str) -> Result<(), AuthError> {
    if scope == "api" {
        return Ok(());
    }
    if let Some(id) = scope.strip_prefix("plugin:") {
        let ok = (1..=48).contains(&id.len())
            && id
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
            && !id.starts_with(|c: char| c.is_ascii_digit());
        if ok {
            return Ok(());
        }
    }
    Err(AuthError::Invalid(format!(
        "invalid token scope '{scope}': use 'api' or 'plugin:<id>'"
    )))
}

/// Issue a token for `user_id`. Returns the new row id and the plaintext
/// token, which the caller must show to the user now and never again.
pub async fn create_api_token(
    db: &SqlitePool,
    user_id: i64,
    name: &str,
    scopes: &[String],
    ttl: Option<Duration>,
) -> Result<(i64, String), AuthError> {
    if name.trim().is_empty() || name.len() > 64 {
        return Err(AuthError::Invalid(
            "token name must be 1-64 characters".into(),
        ));
    }
    if scopes.is_empty() {
        return Err(AuthError::Invalid(
            "a token needs at least one scope".into(),
        ));
    }
    for scope in scopes {
        validate_scope(scope)?;
    }

    let token = format!(
        "{TOKEN_PREFIX}{}{}",
        uuid::Uuid::new_v4().simple(),
        uuid::Uuid::new_v4().simple()
    );
    let now = chrono::Utc::now().timestamp();
    let expires = ttl.map(|t| now + t.as_secs() as i64);
    let result = sqlx::query(
        "INSERT INTO api_tokens (user_id, name, token_hash, scopes, created_at, expires_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
    )
    .bind(user_id)
    .bind(name)
    .bind(token_hash(&token))
    .bind(scopes.join(" "))
    .bind(now)
    .bind(expires)
    .execute(db)
    .await?;
    Ok((result.last_insert_rowid(), token))
}

/// Resolve a presented token to its owner and scopes. Returns `None` for an
/// unknown, expired, revoked, or disabled-user token. Updates `last_used_at`.
pub async fn resolve_api_token(
    db: &SqlitePool,
    token: &str,
) -> Result<Option<ResolvedToken>, AuthError> {
    if !token.starts_with(TOKEN_PREFIX) {
        return Ok(None);
    }
    let now = chrono::Utc::now().timestamp();
    let row = sqlx::query_as::<_, (i64, i64, String, String, String, bool, Option<i64>)>(
        "SELECT t.id, u.id, u.username, u.role, t.scopes, u.disabled, t.expires_at
         FROM api_tokens t JOIN users u ON u.id = t.user_id
         WHERE t.token_hash = ?1",
    )
    .bind(token_hash(token))
    .fetch_optional(db)
    .await?;

    let Some((token_id, user_id, username, role, scopes, disabled, expires_at)) = row else {
        return Ok(None);
    };
    if disabled || expires_at.is_some_and(|e| e <= now) {
        return Ok(None);
    }
    sqlx::query("UPDATE api_tokens SET last_used_at = ?1 WHERE id = ?2")
        .bind(now)
        .bind(token_id)
        .execute(db)
        .await?;
    Ok(Some(ResolvedToken {
        user: AuthUser {
            id: user_id,
            username,
            role: Role::parse(&role).unwrap_or(Role::User),
        },
        scopes: scopes.split_whitespace().map(str::to_string).collect(),
    }))
}

/// Whether `scopes` permit a request to `path` (an absolute `/api/...` path).
pub fn scope_allows(scopes: &[String], path: &str) -> bool {
    if scopes.iter().any(|s| s == "api") {
        return true;
    }
    let Some(rest) = path.strip_prefix("/api/") else {
        return false;
    };
    let mut parts = rest.split('/');
    let first = parts.next().unwrap_or("");
    let plugin_id = if first == "plugin-host" {
        parts.next().unwrap_or("")
    } else {
        first
    };
    !plugin_id.is_empty()
        && scopes
            .iter()
            .any(|s| s.strip_prefix("plugin:") == Some(plugin_id))
}

/// List tokens, optionally for one user.
pub async fn list_api_tokens(
    db: &SqlitePool,
    user_id: Option<i64>,
) -> Result<Vec<TokenInfo>, AuthError> {
    let rows = sqlx::query_as::<_, (i64, i64, String, String, String, i64, Option<i64>, Option<i64>)>(
        "SELECT t.id, t.user_id, u.username, t.name, t.scopes, t.created_at, t.expires_at, t.last_used_at
         FROM api_tokens t JOIN users u ON u.id = t.user_id
         WHERE (?1 IS NULL OR t.user_id = ?1)
         ORDER BY t.id",
    )
    .bind(user_id)
    .fetch_all(db)
    .await?;
    Ok(rows
        .into_iter()
        .map(
            |(id, user_id, username, name, scopes, created_at, expires_at, last_used_at)| {
                TokenInfo {
                    id,
                    user_id,
                    username,
                    name,
                    scopes: scopes.split_whitespace().map(str::to_string).collect(),
                    created_at,
                    expires_at,
                    last_used_at,
                }
            },
        )
        .collect())
}

/// Revoke a token by id. Returns whether a row was removed.
pub async fn revoke_api_token(db: &SqlitePool, id: i64) -> Result<bool, AuthError> {
    let result = sqlx::query("DELETE FROM api_tokens WHERE id = ?1")
        .bind(id)
        .execute(db)
        .await?;
    Ok(result.rows_affected() > 0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::user::create_user;
    use crate::db::connect_in_memory;
    use crate::migrate;

    async fn setup() -> (SqlitePool, i64) {
        let db = connect_in_memory().await.unwrap();
        migrate::run_all(&db).await.unwrap();
        let user = create_user(&db, "alice", "correct horse battery", Role::User)
            .await
            .unwrap();
        (db, user.id)
    }

    fn scopes(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    #[tokio::test]
    async fn token_round_trip_and_hash_only_storage() {
        let (db, uid) = setup().await;
        let (id, token) = create_api_token(&db, uid, "laptop", &scopes(&["api"]), None)
            .await
            .unwrap();
        assert!(token.starts_with(TOKEN_PREFIX));

        let resolved = resolve_api_token(&db, &token)
            .await
            .unwrap()
            .expect("valid");
        assert_eq!(resolved.user.username, "alice");
        assert_eq!(resolved.scopes, vec!["api".to_string()]);

        // The plaintext is never stored.
        let stored: Vec<(String,)> =
            sqlx::query_as("SELECT token_hash FROM api_tokens WHERE id = ?1")
                .bind(id)
                .fetch_all(&db)
                .await
                .unwrap();
        assert_ne!(stored[0].0, token);
        assert_eq!(stored[0].0.len(), 64);
    }

    #[tokio::test]
    async fn unknown_revoked_and_expired_tokens_fail() {
        let (db, uid) = setup().await;
        assert!(resolve_api_token(&db, "tpt_nope").await.unwrap().is_none());
        assert!(
            resolve_api_token(&db, "not-a-token")
                .await
                .unwrap()
                .is_none()
        );

        let (id, token) = create_api_token(&db, uid, "gone", &scopes(&["api"]), None)
            .await
            .unwrap();
        assert!(revoke_api_token(&db, id).await.unwrap());
        assert!(resolve_api_token(&db, &token).await.unwrap().is_none());

        let (_, stale) = create_api_token(
            &db,
            uid,
            "stale",
            &scopes(&["api"]),
            Some(Duration::from_secs(0)),
        )
        .await
        .unwrap();
        assert!(resolve_api_token(&db, &stale).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn disabled_user_loses_access() {
        let (db, uid) = setup().await;
        let (_, token) = create_api_token(&db, uid, "x", &scopes(&["api"]), None)
            .await
            .unwrap();
        sqlx::query("UPDATE users SET disabled = 1 WHERE id = ?1")
            .bind(uid)
            .execute(&db)
            .await
            .unwrap();
        assert!(resolve_api_token(&db, &token).await.unwrap().is_none());
    }

    #[test]
    fn scope_rules() {
        let api = scopes(&["api"]);
        assert!(scope_allows(&api, "/api/notes/list"));
        assert!(scope_allows(&api, "/api/plugin-host/calendar/db/query"));

        let plugin = scopes(&["plugin:calendar"]);
        assert!(scope_allows(&plugin, "/api/calendar/events"));
        assert!(scope_allows(&plugin, "/api/plugin-host/calendar/kv/get"));
        assert!(!scope_allows(&plugin, "/api/notes/list"));
        assert!(!scope_allows(&plugin, "/api/plugin-host/notes/kv/get"));
        assert!(!scope_allows(&plugin, "/api/"));
        assert!(!scope_allows(&plugin, "/login"));
    }

    #[test]
    fn scope_validation() {
        assert!(validate_scope("api").is_ok());
        assert!(validate_scope("plugin:calendar").is_ok());
        assert!(validate_scope("plugin:Bad Id").is_err());
        assert!(validate_scope("admin").is_err());
        assert!(validate_scope("plugin:").is_err());
    }

    #[tokio::test]
    async fn creation_rejects_empty_scopes_and_bad_names() {
        let (db, uid) = setup().await;
        assert!(create_api_token(&db, uid, "x", &[], None).await.is_err());
        assert!(
            create_api_token(&db, uid, "  ", &scopes(&["api"]), None)
                .await
                .is_err()
        );
        assert!(
            create_api_token(&db, uid, "x", &scopes(&["root"]), None)
                .await
                .is_err()
        );
    }
}
