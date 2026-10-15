// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 TPT Solutions

//! Email alerts. The admin configures one SMTP account (singleton settings
//! row). Saving it also creates the notification channel `email`, so a plugin
//! that declares `notify_channels = ["email"]` and is approved for `notify`
//! sends mail with `tpt.notify("email", text)`. The channel's URL is a marker,
//! not a real address, so the channels API cannot point it elsewhere.
//!
//! The SMTP password is stored in the database in plain text, the same as hook
//! and channel secrets (the server must send it, so it cannot be hashed). The
//! settings API never returns it.

use std::time::Duration;

use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use lettre::message::Mailbox;
use lettre::transport::smtp::authentication::Credentials;
use lettre::{Message, SmtpTransport, Transport};
use serde::Deserialize;
use serde_json::json;
use tpt_home_core::db::DbPool;
use tpt_home_core::error::ApiError;

/// Channel name that routes to SMTP.
pub const EMAIL_CHANNEL: &str = "email";
/// URL stored on the `email` channel. Not an address: it marks the channel.
pub const EMAIL_CHANNEL_URL: &str = "mailto:configured";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Security {
    /// Implicit TLS (usually port 465).
    Tls,
    /// STARTTLS upgrade (usually port 587).
    Starttls,
    /// No encryption. Only for a relay on a network you trust.
    None,
}

impl Security {
    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "tls" => Some(Self::Tls),
            "starttls" => Some(Self::Starttls),
            "none" => Some(Self::None),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Tls => "tls",
            Self::Starttls => "starttls",
            Self::None => "none",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EmailSettings {
    pub host: String,
    pub port: u16,
    pub security: Security,
    pub username: String,
    pub password: String,
    pub from: String,
    pub to: String,
}

/// Check the settings before they are stored. Returns a reason on failure.
pub fn validate(s: &EmailSettings) -> Result<(), String> {
    if s.host.is_empty()
        || s.host.len() > 253
        || s.host.chars().any(|c| c.is_whitespace() || c == '/')
    {
        return Err("host must be a host name or address".into());
    }
    if s.port == 0 {
        return Err("port must be between 1 and 65535".into());
    }
    if s.username.len() > 256 || s.password.len() > 1024 {
        return Err("username or password is too long".into());
    }
    s.from
        .parse::<Mailbox>()
        .map_err(|_| "from must be an email address".to_string())?;
    s.to.parse::<Mailbox>()
        .map_err(|_| "to must be an email address".to_string())?;
    Ok(())
}

/// Send one plain-text message. Blocking: call from a blocking context.
pub fn send(s: &EmailSettings, subject: &str, text: &str) -> Result<(), String> {
    let from: Mailbox = s.from.parse().map_err(|e| format!("from: {e}"))?;
    let to: Mailbox = s.to.parse().map_err(|e| format!("to: {e}"))?;
    let message = Message::builder()
        .from(from)
        .to(to)
        .subject(subject)
        .body(text.to_string())
        .map_err(|e| format!("message: {e}"))?;

    let builder = match s.security {
        Security::Tls => SmtpTransport::relay(&s.host).map_err(|e| e.to_string())?,
        Security::Starttls => SmtpTransport::starttls_relay(&s.host).map_err(|e| e.to_string())?,
        Security::None => SmtpTransport::builder_dangerous(&s.host),
    };
    let mut builder = builder.port(s.port).timeout(Some(Duration::from_secs(20)));
    if !s.username.is_empty() {
        builder = builder.credentials(Credentials::new(s.username.clone(), s.password.clone()));
    }
    builder
        .build()
        .send(&message)
        .map(|_| ())
        .map_err(|e| format!("send failed: {e}"))
}

/// Load the stored settings, if any. Uses the blocking connection the plugin
/// host uses.
pub fn load_sync(conn: &rusqlite::Connection) -> Result<Option<EmailSettings>, String> {
    use rusqlite::OptionalExtension;
    let row = conn
        .query_row(
            "SELECT host, port, security, username, password, from_addr, to_addr
             FROM email_settings WHERE id = 1",
            [],
            |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, i64>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, String>(3)?,
                    r.get::<_, String>(4)?,
                    r.get::<_, String>(5)?,
                    r.get::<_, String>(6)?,
                ))
            },
        )
        .optional()
        .map_err(|e| e.to_string())?;
    let Some((host, port, security, username, password, from, to)) = row else {
        return Ok(None);
    };
    Ok(Some(EmailSettings {
        host,
        port: u16::try_from(port).map_err(|_| "stored port is out of range".to_string())?,
        security: Security::parse(&security).ok_or("stored security is unknown")?,
        username,
        password,
        from,
        to,
    }))
}

inventory::submit! {
    tpt_home_core::RouteEntry {
        blueprint: "tpt-home-plugins",
        method: "GET",
        path: "/api/notify/email",
        role: Some("admin"),
        register: |router| {
            router.route(
                "/api/notify/email",
                axum::routing::get(get_settings)
                    .put(save_settings)
                    .delete(delete_settings)
                    .layer(tpt_home_core::admin_only!()),
            )
        },
    }
}

inventory::submit! {
    tpt_home_core::RouteEntry {
        blueprint: "tpt-home-plugins",
        method: "POST",
        path: "/api/notify/email/test",
        role: Some("admin"),
        register: |router| {
            router.route("/api/notify/email/test", axum::routing::post(test_email).layer(tpt_home_core::admin_only!()))
        },
    }
}

#[derive(Debug, Deserialize)]
pub struct SettingsBody {
    pub host: String,
    pub port: u16,
    pub security: String,
    #[serde(default)]
    pub username: String,
    /// Empty or absent keeps the stored password.
    #[serde(default)]
    pub password: String,
    pub from: String,
    pub to: String,
}

/// `GET /api/notify/email` (admin): the settings, without the password.
pub async fn get_settings(State(db): State<DbPool>) -> Result<impl IntoResponse, ApiError> {
    let row: Option<(String, i64, String, String, String, String, bool)> = sqlx::query_as(
        "SELECT host, port, security, username, from_addr, to_addr, password != ''
         FROM email_settings WHERE id = 1",
    )
    .fetch_optional(&db)
    .await?;
    let Some((host, port, security, username, from, to, password_set)) = row else {
        return Ok(Json(json!({"configured": false})));
    };
    Ok(Json(json!({
        "configured": true,
        "host": host,
        "port": port,
        "security": security,
        "username": username,
        "from": from,
        "to": to,
        "password_set": password_set,
    })))
}

/// `PUT /api/notify/email` (admin): save the account and its channel.
pub async fn save_settings(
    State(db): State<DbPool>,
    Json(body): Json<SettingsBody>,
) -> Result<impl IntoResponse, ApiError> {
    let security = Security::parse(&body.security)
        .ok_or_else(|| ApiError::bad_request("security must be tls, starttls or none"))?;
    let existing: Option<(String,)> =
        sqlx::query_as("SELECT password FROM email_settings WHERE id = 1")
            .fetch_optional(&db)
            .await?;
    let password = if body.password.is_empty() {
        existing.map(|(p,)| p).unwrap_or_default()
    } else {
        body.password.clone()
    };
    let settings = EmailSettings {
        host: body.host.trim().to_string(),
        port: body.port,
        security,
        username: body.username.trim().to_string(),
        password,
        from: body.from.trim().to_string(),
        to: body.to.trim().to_string(),
    };
    validate(&settings).map_err(ApiError::bad_request)?;

    let now = unix_now();
    let mut tx = db.begin().await?;
    sqlx::query(
        "INSERT INTO email_settings (id, host, port, security, username, password, from_addr, to_addr, updated_at)
         VALUES (1, ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
         ON CONFLICT (id) DO UPDATE SET host = excluded.host, port = excluded.port,
             security = excluded.security, username = excluded.username,
             password = excluded.password, from_addr = excluded.from_addr,
             to_addr = excluded.to_addr, updated_at = excluded.updated_at",
    )
    .bind(&settings.host)
    .bind(i64::from(settings.port))
    .bind(settings.security.as_str())
    .bind(&settings.username)
    .bind(&settings.password)
    .bind(&settings.from)
    .bind(&settings.to)
    .bind(now)
    .execute(&mut *tx)
    .await?;
    sqlx::query(
        "INSERT INTO notification_channels (name, url, created_at) VALUES (?1, ?2, ?3)
         ON CONFLICT (name) DO UPDATE SET url = excluded.url, last_error = NULL",
    )
    .bind(EMAIL_CHANNEL)
    .bind(EMAIL_CHANNEL_URL)
    .bind(now)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok((StatusCode::OK, Json(json!({"ok": true}))))
}

/// `DELETE /api/notify/email` (admin): remove the account and its channel.
pub async fn delete_settings(State(db): State<DbPool>) -> Result<impl IntoResponse, ApiError> {
    let mut tx = db.begin().await?;
    sqlx::query("DELETE FROM email_settings WHERE id = 1")
        .execute(&mut *tx)
        .await?;
    sqlx::query("DELETE FROM notification_channels WHERE name = ?1")
        .bind(EMAIL_CHANNEL)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(Json(json!({"ok": true})))
}

/// `POST /api/notify/email/test` (admin): send a test message now. The outcome
/// is recorded on the channel and returned to the caller.
pub async fn test_email(State(db): State<DbPool>) -> Result<impl IntoResponse, ApiError> {
    let row: Option<(String, i64, String, String, String, String, String)> = sqlx::query_as(
        "SELECT host, port, security, username, password, from_addr, to_addr
         FROM email_settings WHERE id = 1",
    )
    .fetch_optional(&db)
    .await?;
    let Some((host, port, security, username, password, from, to)) = row else {
        return Err(ApiError::not_found("no email account is set"));
    };
    let settings = EmailSettings {
        host,
        port: u16::try_from(port).map_err(|_| ApiError::internal("stored port is out of range"))?,
        security: Security::parse(&security)
            .ok_or_else(|| ApiError::internal("stored security is unknown"))?,
        username,
        password,
        from,
        to,
    };
    let outcome = tokio::task::spawn_blocking(move || {
        send(
            &settings,
            "tpt-homestack test",
            "This is a test message from tpt-homestack. If you can read it, email alerts work.",
        )
    })
    .await
    .map_err(|e| ApiError::internal(format!("send task: {e}")))?;
    let now = unix_now();
    match outcome {
        Ok(()) => {
            sqlx::query(
                "UPDATE notification_channels SET last_sent_at = ?2, last_error = NULL WHERE name = ?1",
            )
            .bind(EMAIL_CHANNEL)
            .bind(now)
            .execute(&db)
            .await?;
            Ok(Json(json!({"ok": true})))
        }
        Err(reason) => {
            sqlx::query("UPDATE notification_channels SET last_error = ?2 WHERE name = ?1")
                .bind(EMAIL_CHANNEL)
                .bind(&reason)
                .execute(&db)
                .await?;
            Err(ApiError::bad_request(reason))
        }
    }
}

fn unix_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn settings() -> EmailSettings {
        EmailSettings {
            host: "smtp.example.org".into(),
            port: 587,
            security: Security::Starttls,
            username: String::new(),
            password: String::new(),
            from: "homestack@example.org".into(),
            to: "Family <family@example.org>".into(),
        }
    }

    #[test]
    fn validation_refuses_bad_addresses_and_ports() {
        assert!(validate(&settings()).is_ok());
        let mut s = settings();
        s.from = "not an address".into();
        assert!(validate(&s).unwrap_err().contains("from"));
        let mut s = settings();
        s.port = 0;
        assert!(validate(&s).unwrap_err().contains("port"));
        let mut s = settings();
        s.host = "smtp.example.org/evil".into();
        assert!(validate(&s).unwrap_err().contains("host"));
    }

    #[test]
    fn security_names_round_trip() {
        for name in ["tls", "starttls", "none"] {
            assert_eq!(Security::parse(name).unwrap().as_str(), name);
        }
        assert!(Security::parse("plain").is_none());
    }
}
