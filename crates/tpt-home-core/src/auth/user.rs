// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 TPT Solutions

//! Users and roles (admin / user). Roles are a fixed set for now; fine-grained
//! permissions can be layered later without schema churn because role checks
//! funnel through [`Role::allows`].

use sqlx::SqlitePool;

use super::AuthError;
use super::password::{hash_password, verify_password};

/// A role. Extend the enum + `allows` when fine-grained permissions land.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    Admin,
    User,
}

impl Role {
    pub fn as_str(self) -> &'static str {
        match self {
            Role::Admin => "admin",
            Role::User => "user",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "admin" => Some(Role::Admin),
            "user" => Some(Role::User),
            _ => None,
        }
    }

    /// Permission check: admins satisfy any requirement; users only their own.
    pub fn allows(self, required: Option<&str>) -> bool {
        match required {
            None => true,
            Some(_) => self == Role::Admin,
        }
    }
}

impl std::fmt::Display for Role {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A user row. `password_hash` is only exposed within this crate.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct User {
    pub id: i64,
    pub username: String,
    #[sqlx(rename = "password_hash")]
    pub password_hash: String,
    pub role: String,
    pub disabled: bool,
    pub created_at: i64,
}

impl User {
    pub fn role(&self) -> Role {
        Role::parse(&self.role).unwrap_or(Role::User)
    }
}

/// Authenticated identity attached to requests by the auth middleware.
#[derive(Debug, Clone)]
pub struct AuthUser {
    pub id: i64,
    pub username: String,
    pub role: Role,
}

impl AuthUser {
    pub fn is_admin(&self) -> bool {
        self.role == Role::Admin
    }
}

fn validate_username(username: &str) -> Result<(), AuthError> {
    let ok_len = (3..=32).contains(&username.chars().count());
    let ok_chars = username
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
    if ok_len && ok_chars {
        Ok(())
    } else {
        Err(AuthError::InvalidUsername(
            "username must be 3-32 chars: letters, digits, '-' or '_'".into(),
        ))
    }
}

/// Create a user. The first user of a fresh instance should be an admin.
pub async fn create_user(
    db: &SqlitePool,
    username: &str,
    password: &str,
    role: Role,
) -> Result<User, AuthError> {
    validate_username(username)?;
    let hash = hash_password(password)?;
    let now = chrono::Utc::now().timestamp();
    let rec = sqlx::query_as::<_, User>(
        "INSERT INTO users (username, password_hash, role, disabled, created_at)
         VALUES (?1, ?2, ?3, 0, ?4)
         RETURNING id, username, password_hash, role, disabled, created_at",
    )
    .bind(username)
    .bind(hash)
    .bind(role.as_str())
    .bind(now)
    .fetch_one(db)
    .await?;
    Ok(rec)
}

/// Number of user accounts (0 means the instance needs first-run setup).
pub async fn count_users(db: &SqlitePool) -> Result<i64, AuthError> {
    let (n,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM users")
        .fetch_one(db)
        .await?;
    Ok(n)
}

pub async fn get_user_by_name(db: &SqlitePool, username: &str) -> Result<User, AuthError> {
    sqlx::query_as::<_, User>(
        "SELECT id, username, password_hash, role, disabled, created_at
         FROM users WHERE username = ?1",
    )
    .bind(username)
    .fetch_optional(db)
    .await?
    .ok_or_else(|| AuthError::NotFound("user not found".into()))
}

pub async fn get_user_by_id(db: &SqlitePool, id: i64) -> Result<User, AuthError> {
    sqlx::query_as::<_, User>(
        "SELECT id, username, password_hash, role, disabled, created_at
         FROM users WHERE id = ?1",
    )
    .bind(id)
    .fetch_optional(db)
    .await?
    .ok_or_else(|| AuthError::NotFound("user not found".into()))
}

pub async fn list_users(db: &SqlitePool) -> Result<Vec<User>, AuthError> {
    let users = sqlx::query_as::<_, User>(
        "SELECT id, username, password_hash, role, disabled, created_at
         FROM users ORDER BY id",
    )
    .fetch_all(db)
    .await?;
    Ok(users)
}

/// Check credentials; returns the user when they exist, are enabled and the
/// password matches. Timing-flat for unknown users thanks to a dummy verify.
pub async fn authenticate(
    db: &SqlitePool,
    username: &str,
    password: &str,
) -> Result<User, AuthError> {
    let user = match get_user_by_name(db, username).await {
        Ok(u) => u,
        Err(_) => {
            // Burn comparable time so missing users are indistinguishable.
            super::password::verify_dummy(password);
            return Err(AuthError::InvalidCredentials);
        }
    };
    if user.disabled || !verify_password(password, &user.password_hash) {
        return Err(AuthError::InvalidCredentials);
    }
    Ok(user)
}

pub async fn set_password(db: &SqlitePool, id: i64, new_password: &str) -> Result<(), AuthError> {
    let hash = hash_password(new_password)?;
    let res = sqlx::query("UPDATE users SET password_hash = ?1 WHERE id = ?2")
        .bind(hash)
        .bind(id)
        .execute(db)
        .await?;
    if res.rows_affected() == 0 {
        return Err(AuthError::NotFound("user not found".into()));
    }
    // Any password reset invalidates existing sessions.
    super::session::revoke_user_sessions(db, id).await?;
    Ok(())
}

pub async fn set_role(db: &SqlitePool, id: i64, role: Role) -> Result<(), AuthError> {
    let res = sqlx::query("UPDATE users SET role = ?1 WHERE id = ?2")
        .bind(role.as_str())
        .bind(id)
        .execute(db)
        .await?;
    if res.rows_affected() == 0 {
        return Err(AuthError::NotFound("user not found".into()));
    }
    Ok(())
}

pub async fn set_disabled(db: &SqlitePool, id: i64, disabled: bool) -> Result<(), AuthError> {
    let res = sqlx::query("UPDATE users SET disabled = ?1 WHERE id = ?2")
        .bind(disabled)
        .bind(id)
        .execute(db)
        .await?;
    if res.rows_affected() == 0 {
        return Err(AuthError::NotFound("user not found".into()));
    }
    if disabled {
        super::session::revoke_user_sessions(db, id).await?;
    }
    Ok(())
}

pub async fn delete_user(db: &SqlitePool, id: i64) -> Result<(), AuthError> {
    let res = sqlx::query("DELETE FROM users WHERE id = ?1")
        .bind(id)
        .execute(db)
        .await?;
    if res.rows_affected() == 0 {
        return Err(AuthError::NotFound("user not found".into()));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::connect_in_memory;
    use crate::migrate;

    async fn db_with_users() -> SqlitePool {
        let db = connect_in_memory().await.unwrap();
        migrate::run_all(&db).await.unwrap();
        db
    }

    #[tokio::test]
    async fn create_authenticate_flow() {
        let db = db_with_users().await;
        create_user(&db, "alice", "super-secret-1", Role::Admin)
            .await
            .unwrap();

        let ok = authenticate(&db, "alice", "super-secret-1").await.unwrap();
        assert_eq!(ok.role(), Role::Admin);

        assert!(authenticate(&db, "alice", "wrong").await.is_err());
        assert!(authenticate(&db, "nobody", "super-secret-1").await.is_err());
    }

    #[tokio::test]
    async fn rejects_bad_usernames_and_short_passwords() {
        let db = db_with_users().await;
        assert!(
            create_user(&db, "ab", "long-enough-pw", Role::User)
                .await
                .is_err()
        );
        assert!(
            create_user(&db, "bad name!", "long-enough-pw", Role::User)
                .await
                .is_err()
        );
        assert!(
            create_user(&db, "alice", "short", Role::User)
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn disable_revokes_access() {
        let db = db_with_users().await;
        let u = create_user(&db, "bob", "super-secret-1", Role::User)
            .await
            .unwrap();
        set_disabled(&db, u.id, true).await.unwrap();
        assert!(authenticate(&db, "bob", "super-secret-1").await.is_err());
    }

    #[tokio::test]
    async fn unique_usernames_case_insensitive() {
        let db = db_with_users().await;
        create_user(&db, "carol", "super-secret-1", Role::User)
            .await
            .unwrap();
        assert!(
            create_user(&db, "CAROL", "super-secret-1", Role::User)
                .await
                .is_err()
        );
    }
}
