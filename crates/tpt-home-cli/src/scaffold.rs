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

/// Register `name` as a path dependency of the CLI, so its registration
/// statics are linked into the binary. Without this the blueprint is a
/// workspace member that never reaches the server.
fn add_cli_dependency(manifest: &str, name: &str) -> anyhow::Result<String> {
    if manifest.contains(&format!("tpt-home-bp-{name} =")) {
        return Ok(manifest.to_string()); // already linked
    }
    let line = format!(
        "tpt-home-bp-{name} = {{ path = \"../../blueprints/tpt-home-bp-{name}\", version = \"0.1.0\" }}"
    );
    // Split on '\n' and keep any '\r', so CRLF files keep their endings.
    let cr = line_ending_suffix(manifest);
    let mut lines: Vec<&str> = manifest.split('\n').collect();
    let Some(anchor) = lines.iter().rposition(|l| l.contains("../../blueprints/")) else {
        anyhow::bail!(
            "tpt-home-cli Cargo.toml has no blueprint dependencies to place the new one after"
        );
    };
    let entry = format!("{line}{cr}");
    lines.insert(anchor + 1, &entry);
    Ok(lines.join("\n"))
}

/// Add the new crate's `__link_anchor` to the CLI's blueprint anchor list.
fn add_link_anchor(main_src: &str, name: &str) -> anyhow::Result<String> {
    let entry = format!("    tpt_home_bp_{name}::__link_anchor,");
    if main_src.contains(&entry) {
        return Ok(main_src.to_string()); // already linked
    }
    let cr = line_ending_suffix(main_src);
    let mut lines: Vec<&str> = main_src.split('\n').collect();
    let Some(anchor) = lines.iter().position(|l| {
        l.trim_start()
            .starts_with("tpt_home_plugins::__link_anchor")
    }) else {
        anyhow::bail!(
            "tpt-home-cli main.rs has no BLUEPRINT_LINK_ANCHORS entries to place the new one before"
        );
    };
    let line = format!("{entry}{cr}");
    lines.insert(anchor, &line);
    Ok(lines.join("\n"))
}

/// `"\r"` when the text uses CRLF line endings, otherwise empty.
fn line_ending_suffix(text: &str) -> &'static str {
    if text.contains("\r\n") { "\r" } else { "" }
}

/// Starter code for `tpt-home new --template <kind>`. The sources are
/// embedded in the binary, so scaffolding works offline and always matches
/// the platform version it ships with. Each template is a small crate: its
/// files live under `templates/<kind>/` and get the name and title filled in.
#[derive(Clone, Copy, Debug, PartialEq, Eq, clap::ValueEnum)]
pub enum Template {
    /// One table and one endpoint pair, with a placeholder page.
    Basic,
    /// A list-and-form app: add, view and delete records.
    Records,
    /// A dated log of numbers and notes, with totals and a summary.
    Logbook,
    /// Upload, list, download and delete files.
    FileInbox,
    /// A background job that writes a periodic snapshot.
    ScheduledJob,
    /// Metric cards built from grouped queries.
    Dashboard,
    /// A job that fetches a URL on a schedule and keeps the responses.
    ScheduledFetch,
    /// Signed sensor events in, alerts out: quiet-hours openings and silent sensors.
    SensorAlerts,
}

/// One file of a template: its path inside the new crate and its source.
type TemplateFile = (&'static str, &'static str);

impl Template {
    fn files(self) -> &'static [TemplateFile] {
        match self {
            Template::Basic => &[("src/lib.rs", include_str!("../templates/basic/src/lib.rs"))],
            Template::Records => &[(
                "src/lib.rs",
                include_str!("../templates/records/src/lib.rs"),
            )],
            Template::Logbook => &[
                (
                    "src/lib.rs",
                    include_str!("../templates/logbook/src/lib.rs"),
                ),
                (
                    "src/api.rs",
                    include_str!("../templates/logbook/src/api.rs"),
                ),
                (
                    "src/page.rs",
                    include_str!("../templates/logbook/src/page.rs"),
                ),
            ],
            Template::FileInbox => &[
                (
                    "src/lib.rs",
                    include_str!("../templates/file-inbox/src/lib.rs"),
                ),
                (
                    "src/api.rs",
                    include_str!("../templates/file-inbox/src/api.rs"),
                ),
                (
                    "src/page.rs",
                    include_str!("../templates/file-inbox/src/page.rs"),
                ),
            ],
            Template::ScheduledJob => &[
                (
                    "src/lib.rs",
                    include_str!("../templates/scheduled-job/src/lib.rs"),
                ),
                (
                    "src/api.rs",
                    include_str!("../templates/scheduled-job/src/api.rs"),
                ),
                (
                    "src/page.rs",
                    include_str!("../templates/scheduled-job/src/page.rs"),
                ),
            ],
            Template::Dashboard => &[
                (
                    "src/lib.rs",
                    include_str!("../templates/dashboard/src/lib.rs"),
                ),
                (
                    "src/api.rs",
                    include_str!("../templates/dashboard/src/api.rs"),
                ),
                (
                    "src/page.rs",
                    include_str!("../templates/dashboard/src/page.rs"),
                ),
            ],
            Template::SensorAlerts => &[
                (
                    "src/lib.rs",
                    include_str!("../templates/sensor-alerts/src/lib.rs"),
                ),
                (
                    "src/api.rs",
                    include_str!("../templates/sensor-alerts/src/api.rs"),
                ),
                (
                    "src/page.rs",
                    include_str!("../templates/sensor-alerts/src/page.rs"),
                ),
            ],
            Template::ScheduledFetch => &[
                (
                    "src/lib.rs",
                    include_str!("../templates/scheduled-fetch/src/lib.rs"),
                ),
                (
                    "src/api.rs",
                    include_str!("../templates/scheduled-fetch/src/api.rs"),
                ),
                (
                    "src/page.rs",
                    include_str!("../templates/scheduled-fetch/src/page.rs"),
                ),
            ],
        }
    }

    /// Dependencies beyond the platform set, as `[dependencies]` lines.
    fn extra_deps(self) -> &'static str {
        match self {
            Template::FileInbox => {
                "tokio = { workspace = true }\ntokio-util = { workspace = true }\n"
            }
            Template::ScheduledFetch => {
                "reqwest = { version = \"0.12\", default-features = false, features = [\"rustls-tls\"] }\n"
            }
            Template::SensorAlerts => "chrono = { workspace = true }\n",
            _ => "",
        }
    }
}

fn cargo_toml_template(name: &str, extra_deps: &str) -> String {
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
{extra_deps}"#
    )
}

/// Replace the `<NAME>`, `<TITLE>` and `<PASCAL>` placeholders in a template file.
fn fill_placeholders(source: &str, name: &str, title: &str, pascal: &str) -> String {
    source
        .replace("<NAME>", name)
        .replace("<TITLE>", title)
        .replace("<PASCAL>", pascal)
}

/// `tpt-home new <name>`: create `blueprints/tpt-home-bp-<name>` and add it
/// to the workspace members. Recompiling activates the blueprint.
pub fn new_blueprint(name: &str, template: Template) -> anyhow::Result<()> {
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
    std::fs::create_dir_all(&dir)?;

    for (path, source) in template.files() {
        let target = dir.join(path);
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let text = fill_placeholders(source, name, &title, &pascal);
        std::fs::write(&target, text)?;
    }
    std::fs::write(
        dir.join("Cargo.toml"),
        cargo_toml_template(name, template.extra_deps()),
    )?;

    let manifest_path = root.join("Cargo.toml");
    let manifest = std::fs::read_to_string(&manifest_path).context("read root Cargo.toml")?;
    let updated = add_member(&manifest, &format!("blueprints/tpt-home-bp-{name}"))?;
    std::fs::write(&manifest_path, updated)?;

    // Link the crate into the binary: a CLI dependency plus a link anchor.
    let cli_dir = root.join("crates").join("tpt-home-cli");
    let cli_manifest_path = cli_dir.join("Cargo.toml");
    let cli_manifest =
        std::fs::read_to_string(&cli_manifest_path).context("read tpt-home-cli Cargo.toml")?;
    std::fs::write(&cli_manifest_path, add_cli_dependency(&cli_manifest, name)?)?;
    let main_path = cli_dir.join("src").join("main.rs");
    let main_src = std::fs::read_to_string(&main_path).context("read tpt-home-cli main.rs")?;
    std::fs::write(&main_path, add_link_anchor(&main_src, name)?)?;

    println!("created {}", dir.display());
    println!(
        "registered in {} and linked into tpt-home-cli",
        manifest_path.display()
    );
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
    fn every_template_fills_all_placeholders() {
        use clap::ValueEnum;
        for template in Template::value_variants() {
            for (path, source) in template.files() {
                let out = fill_placeholders(source, "demo", "Demo", "Demo");
                for placeholder in ["<NAME>", "<TITLE>", "<PASCAL>"] {
                    assert!(
                        !out.contains(placeholder),
                        "{template:?} {path} left {placeholder}"
                    );
                }
            }
            let lib = fill_placeholders(template.files()[0].1, "demo", "Demo", "Demo");
            assert!(
                lib.contains("#[blueprint("),
                "{template:?} lib.rs has no blueprint"
            );
            assert!(
                lib.contains("__link_anchor"),
                "{template:?} lib.rs has no link anchor"
            );
        }
    }

    #[test]
    fn cli_dependency_is_added_after_existing_blueprints() {
        let manifest = "[dependencies]\n\
            tpt-home-bp-track = { path = \"../../blueprints/tpt-home-bp-track\", version = \"0.1.0\" }\n\
            tpt-home-plugins = { path = \"../tpt-home-plugins\", version = \"0.1.0\" }\n";
        let out = add_cli_dependency(manifest, "demo").unwrap();
        let track = out.find("tpt-home-bp-track").unwrap();
        let demo = out
            .find("tpt-home-bp-demo = { path = \"../../blueprints/tpt-home-bp-demo\"")
            .unwrap();
        let plugins = out.find("tpt-home-plugins").unwrap();
        assert!(track < demo && demo < plugins, "{out}");

        // Idempotent.
        assert_eq!(add_cli_dependency(&out, "demo").unwrap(), out);
    }

    #[test]
    fn link_anchor_is_added_before_plugins() {
        let src = "static X: &[fn() -> &'static str] = &[\n    tpt_home_bp_track::__link_anchor,\n    tpt_home_plugins::__link_anchor,\n];\n";
        let out = add_link_anchor(src, "demo").unwrap();
        assert!(
            out.contains(
                "    tpt_home_bp_demo::__link_anchor,\n    tpt_home_plugins::__link_anchor,"
            ),
            "{out}"
        );
        assert_eq!(add_link_anchor(&out, "demo").unwrap(), out);
    }

    #[test]
    fn crlf_files_keep_their_line_endings() {
        let manifest =
            "[dependencies]\r\ntpt-home-bp-track = { path = \"../../blueprints/x\" }\r\n";
        let out = add_cli_dependency(manifest, "demo").unwrap();
        assert_eq!(out.matches("\r\n").count(), 3, "{out:?}");
        assert!(!out.replace("\r\n", "").contains('\n'), "{out:?}");
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
