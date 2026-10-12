// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 TPT Solutions

//! Authentication: Argon2id password hashing, users/roles and DB-backed
//! sessions.

pub mod api_token;
pub mod password;
pub mod session;
pub mod user;

use crate::error::ApiError;

/// Errors surfaced by the auth layer; `?`-friendly with `ApiError`.
#[derive(Debug, thiserror::Error)]
pub enum AuthError {
    #[error("invalid username or password")]
    InvalidCredentials,
    #[error("{0}")]
    WeakPassword(String),
    #[error("{0}")]
    InvalidUsername(String),
    #[error("{0}")]
    Invalid(String),
    #[error("{0}")]
    NotFound(String),
    #[error("session expired")]
    SessionExpired,
    #[error(transparent)]
    Internal(#[from] anyhow::Error),
}

impl From<sqlx::Error> for AuthError {
    fn from(e: sqlx::Error) -> Self {
        AuthError::Internal(e.into())
    }
}

impl From<AuthError> for ApiError {
    fn from(e: AuthError) -> Self {
        match e {
            AuthError::InvalidCredentials | AuthError::SessionExpired => ApiError::Unauthorized,
            AuthError::WeakPassword(m) | AuthError::InvalidUsername(m) | AuthError::Invalid(m) => {
                ApiError::BadRequest(m)
            }
            AuthError::NotFound(m) => ApiError::NotFound(m),
            AuthError::Internal(e) => ApiError::Internal(e),
        }
    }
}
