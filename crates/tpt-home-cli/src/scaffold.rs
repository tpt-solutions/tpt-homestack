// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 TPT Solutions

//! `tpt-home new <name>` — scaffold a blueprint crate and register it in the
//! workspace, so "create a custom app" is one command plus your business
//! logic.

use anyhow::Context;

/// Validate a blueprint id: lowercase letters/digits/underscore, no leading
/// digit, 1-48 chars.
pub fn validate_name(name: &str) -> anyhow::Result<()> {
    let ok = !name.is_empty()
        && name.len() <= 48
        && name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
        && !name.starts_with(|c: char| c.is_ascii_digit());
    if ok {
        Ok(())
    } else {
        anyhow::bail!(
            "invalid blueprint id '{name}': use 1-48 chars of lowercase letters, digits or '_', not starting with a digit"
        )
    }
}

/// Insert a member line into the workspace `members = [...]` array.
fn add_member(manifest: &str, member: &str) -> anyhow::Result<String> {
    if manifest.contains(&format!("\"{member}\"")) {
        return Ok(manifest.to_string()); // already registered
    }
    let Some(start) = manifest.find("members") else {
        anyhow::bail!("root Cargo.toml has no [workspace] members array");
    };
    let Some(bracket) = manifest[start..].find('[') else {
        anyhow::bail!("root Cargo.toml members is not an array");
    };
    let open = start + bracket;
    let Some(close_rel) = manifest[open..].find(']') else {
        anyhow::bail!("root Cargo.toml members array is unterminated");
    };
    let close = open + close_rel;

    let entry = format!("    \"{member}\",\n");
    let mut out = String::with_capacity(manifest.len() + entry.len());
    out.push_str(&manifest[..=open]);
    out.push('\n');
    out.push_str(&entry);
    out.push_str(&manifest[open + 1..close]);
    out.push_str(&manifest[close..]);
    Ok(out)
}

const LIB_TEMPLATE: &str = r##"// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 TPT Solutions

//! <NAME> — a tpt-homestack blueprint.
//!
//! This is the whole platform integration surface: one metadata macro, one
//! SQL migration, routes, and a UI page. Delete what you don't need and
//! write your business logic; the macros do the wiring.

use tpt_home_core::prelude::*;
use tpt_home_macros::{blueprint, bp_migration, bp_route, bp_ui_route};

#[blueprint(id = "<NAME>", name = "<TITLE>")]
pub struct <PASCAL>Blueprint;

#[bp_migration]
const SCHEMA: &str = r#"
    CREATE TABLE IF NOT EXISTS <NAME>_items (
        id         INTEGER PRIMARY KEY,
        title      TEXT    NOT NULL,
        created_at INTEGER NOT NULL
    );
"#;

#[bp_route(get, "/api/<NAME>/items")]
pub async fn list_items(
    State(db): State<DbPool>,
) -> Result<impl IntoResponse, ApiError> {
    let items: Vec<(i64, String)> =
        sqlx::query_as("SELECT id, title FROM <NAME>_items ORDER BY id DESC")
            .fetch_all(&db)
            .await?;
    Ok(Json(items))
}

#[bp_route(post, "/api/<NAME>/items")]
pub async fn create_item(
    State(db): State<DbPool>,
    Json(body): Json<CreateItem>,
) -> Result<impl IntoResponse, ApiError> {
    let (id,): (i64,) = sqlx::query_as(
        "INSERT INTO <NAME>_items (title, created_at) VALUES (?1, ?2) RETURNING id",
    )
    .bind(body.title.trim())
    .bind(unix_now())
    .fetch_one(&db)
    .await?;
    Ok((axum::http::StatusCode::CREATED, Json(serde_json::json!({ "id": id }))))
}

#[derive(serde::Deserialize)]
pub struct CreateItem {
    pub title: String,
}

#[bp_ui_route("/<NAME>", name = "<TITLE>")]
pub async fn <NAME>_page(
    axum::Extension(user): axum::Extension<AuthUser>,
) -> impl IntoResponse {
    let view = tpt_home_ui::UserView {
        username: user.username.clone(),
        role: user.role.as_str().to_string(),
    };
    tpt_home_ui::page(
        "<TITLE>",
        &registry::nav_entries(),
        "/<NAME>",
        &view,
        "<h2>Welcome to <TITLE></h2><p class=\"muted\">Replace this page with your UI.</p>",
    )
}

fn unix_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}
"##;

fn cargo_toml_template(name: &str) -> String {
    format!(
        r#"[package]
name = "tpt-home-bp-{name}"
description = "tpt-homestack blueprint: {name}"
version.workspace = true
edition.workspace = true
rust-version.workspace = true
license.workspace = true
authors.workspace = true
repository.workspace = true

[dependencies]
tpt-home-core = {{ workspace = true }}
tpt-home-macros = {{ workspace = true }}
tpt-home-ui = {{ workspace = true }}
axum = {{ workspace = true }}
serde = {{ workspace = true }}
serde_json = {{ workspace = true }}
sqlx = {{ workspace = true }}
"#
    )
}

/// `tpt-home new <name>`: create `blueprints/tpt-home-bp-<name>` and add it
/// to the workspace members. Recompiling activates the blueprint.
pub fn new_blueprint(name: &str) -> anyhow::Result<()> {
    validate_name(name)?;
    let title = {
        let mut s = name.split('_').map(str::to_owned).collect::<Vec<_>>();
        for part in &mut s {
            if let Some(first) = part.chars().next() {
                *part = first.to_uppercase().to_string() + &part[first.len_utf8()..];
            }
        }
        s.join(" ")
    };
    let pascal: String = title
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .collect();

    let root = find_workspace_root()?;
    let dir = root.join("blueprints").join(format!("tpt-home-bp-{name}"));
    if dir.exists() {
        anyhow::bail!("{} already exists", dir.display());
    }
    let src = dir.join("src");
    std::fs::create_dir_all(&src)?;

    let lib = LIB_TEMPLATE
        .replace("<NAME>", name)
        .replace("<TITLE>", &title)
        .replace("<PASCAL>", &pascal);
    std::fs::write(dir.join("Cargo.toml"), cargo_toml_template(name))?;
    std::fs::write(src.join("lib.rs"), lib)?;

    let manifest_path = root.join("Cargo.toml");
    let manifest = std::fs::read_to_string(&manifest_path).context("read root Cargo.toml")?;
    let updated = add_member(&manifest, &format!("blueprints/tpt-home-bp-{name}"))?;
    std::fs::write(&manifest_path, updated)?;

    println!("created {}", dir.display());
    println!("registered in {}", manifest_path.display());
    println!();
    println!("Next steps:");
    println!("  1. cargo build");
    println!("  2. Add your tables, routes and UI to the crate.");
    println!("  3. tpt-home run  — '{}' appears in the sidebar.", title);
    Ok(())
}

/// Walk up from the CWD to the directory containing the workspace root
/// Cargo.toml (the one with a `[workspace]` table).
fn find_workspace_root() -> anyhow::Result<std::path::PathBuf> {
    let cwd = std::env::current_dir().context("read current directory")?;
    let mut dir = Some(cwd);
    while let Some(d) = dir {
        let manifest = d.join("Cargo.toml");
        if let Ok(text) = std::fs::read_to_string(&manifest)
            && text.contains("[workspace]")
        {
            return Ok(d);
        }
        dir = d.parent().map(std::path::Path::to_path_buf);
    }
    anyhow::bail!(
        "not inside a tpt-homestack workspace (no [workspace] Cargo.toml found); run this from a checkout of the repo"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_are_validated() {
        assert!(validate_name("my_app").is_ok());
        assert!(validate_name("app2").is_ok());
        assert!(validate_name("").is_err());
        assert!(validate_name("MyApp").is_err());
        assert!(validate_name("2app").is_err());
        assert!(validate_name("bad name").is_err());
        assert!(validate_name(&"x".repeat(49)).is_err());
    }

    #[test]
    fn member_insertion() {
        let manifest = "[workspace]\nmembers = [\n    \"crates/tpt-home-core\",\n]\n";
        let out = add_member(manifest, "blueprints/tpt-home-bp-demo").unwrap();
        assert!(out.contains("\"crates/tpt-home-core\","));
        assert!(out.contains("\"blueprints/tpt-home-bp-demo\","));

        // Idempotent.
        let again = add_member(&out, "blueprints/tpt-home-bp-demo").unwrap();
        assert_eq!(out, again);

        // Single-line array form.
        let single = "[workspace]\nmembers = [\"a\"]\n";
        let out2 = add_member(single, "b").unwrap();
        assert!(out2.contains("\"b\","));
    }
}
