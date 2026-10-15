// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 TPT Solutions

//! `tpt-home remote status|setup`: the checks behind "connect from anywhere".
//! It finds the official clients on this machine, prints the steps only a
//! person can do (signing in, creating a tunnel), and changes `config.toml`
//! only after confirmation. The server still binds to 127.0.0.1; the clients
//! reach it through `tailscale serve` or `cloudflared`. Detection lives in
//! `tpt_home_core::remote`, shared with the admin Remote access page.

use std::path::Path;

use anyhow::Context;
use clap::ValueEnum;

use tpt_home_core::config::Config;
use tpt_home_core::remote::{self, describe_tailscale, spawn};

#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
pub enum Provider {
    /// Private to your tailnet; no domain or router changes needed.
    Tailscale,
    /// A public hostname behind a login, through a Cloudflare tunnel.
    Cloudflare,
}

/// Set keys in `config.toml`, keeping every other key. Creates the file if it
/// does not exist. Comments in the file are not kept.
pub fn set_config(path: &Path, values: &[(&str, toml::Value)]) -> anyhow::Result<()> {
    let mut table: toml::Table = if path.is_file() {
        let text = std::fs::read_to_string(path)?;
        toml::from_str(&text).with_context(|| format!("parse {}", path.display()))?
    } else {
        toml::Table::new()
    };
    for (key, value) in values {
        table.insert((*key).to_string(), value.clone());
    }
    std::fs::write(path, toml::to_string_pretty(&table)?)
        .with_context(|| format!("write {}", path.display()))
}

fn confirm(question: &str, yes: bool) -> anyhow::Result<bool> {
    if yes {
        return Ok(true);
    }
    println!("{question} [y/N]");
    let mut line = String::new();
    std::io::stdin().read_line(&mut line)?;
    Ok(matches!(
        line.trim().to_ascii_lowercase().as_str(),
        "y" | "yes"
    ))
}

/// `tpt-home remote status`: what is set up on this machine.
pub fn status(cfg: &Config) -> anyhow::Result<()> {
    println!(
        "config: host {}, port {}, secure_cookies {}, trust_proxy {}",
        cfg.host, cfg.port, cfg.secure_cookies, cfg.trust_proxy
    );
    let found = remote::detect();
    println!("tailscale: {}", describe_tailscale(&found.tailscale));
    println!(
        "cloudflared: {}",
        if found.cloudflared {
            "installed"
        } else {
            "not found"
        }
    );
    println!(
        "netbird: {}",
        if found.netbird {
            "installed"
        } else {
            "not found"
        }
    );
    Ok(())
}

/// `tpt-home remote setup --provider <p>`: check the client, explain, and
/// make the config change after confirmation. `yes` skips the prompt.
pub fn setup(cfg: &Config, provider: Provider, yes: bool) -> anyhow::Result<()> {
    let config_path = cfg.data_dir.join("config.toml");
    match provider {
        Provider::Tailscale => setup_tailscale(cfg, &config_path, yes),
        Provider::Cloudflare => setup_cloudflare(cfg, &config_path, yes),
    }
}

fn setup_tailscale(cfg: &Config, config_path: &Path, yes: bool) -> anyhow::Result<()> {
    let bin = remote::tailscale_bin();
    let ts = remote::tailscale_state(&bin);
    if !ts.installed {
        anyhow::bail!(
            "Tailscale is not installed. Install the official client from https://tailscale.com/download, sign in, then run this again."
        );
    }
    if !ts.running {
        anyhow::bail!(
            "Tailscale is installed but not signed in. Run `tailscale up`, sign in, then run this again."
        );
    }
    let Some(name) = ts.dns_name.clone() else {
        anyhow::bail!(
            "this machine has no MagicDNS name. Turn on MagicDNS in the Tailscale admin console, then run this again."
        );
    };

    println!("This will:");
    println!(
        "  1. run `tailscale serve --bg {}` (HTTPS at https://{name}, visible only to your tailnet)",
        cfg.port
    );
    println!(
        "  2. set secure_cookies = true in {}",
        config_path.display()
    );
    println!("The server keeps listening on {}.", cfg.host);
    if !confirm("Continue?", yes)? {
        println!("No changes made.");
        return Ok(());
    }

    let port = cfg.port.to_string();
    let out = spawn(&bin, &["serve", "--bg", &port]).context("run tailscale serve")?;
    if !out.status.success() {
        anyhow::bail!(
            "tailscale serve failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    set_config(
        config_path,
        &[("secure_cookies", toml::Value::Boolean(true))],
    )?;
    println!("Done. Open https://{name} on any device signed in to your tailnet.");
    println!("Restart the server so the new cookie setting takes effect.");
    Ok(())
}

fn setup_cloudflare(cfg: &Config, config_path: &Path, yes: bool) -> anyhow::Result<()> {
    let bin = remote::cloudflared_bin();
    if spawn(&bin, &["--version"]).is_none() {
        anyhow::bail!(
            "cloudflared is not installed. Install it from https://developers.cloudflare.com/cloudflare-one/connections/connect-networks/downloads/, then run this again."
        );
    }

    println!("Steps that need your Cloudflare account (this command cannot do them):");
    println!("  1. Add a domain to Cloudflare.");
    println!("  2. In Zero Trust, create a tunnel and copy its token.");
    println!(
        "  3. Add a public hostname to the tunnel, pointing at http://127.0.0.1:{}.",
        cfg.port
    );
    println!("  4. Protect that hostname with Cloudflare Access (a login in front of the app).");
    println!("  5. Run cloudflared with the token, as a service.");
    println!();
    println!("This will set in {}:", config_path.display());
    println!("  secure_cookies = true");
    println!("  trust_proxy = true");
    println!("trust_proxy makes the server believe X-Forwarded-For. Only set it when");
    println!("cloudflared is the only way to reach the server; otherwise clients can");
    println!("spoof their address and get around the login rate limit.");
    if !confirm("Continue?", yes)? {
        println!("No changes made.");
        return Ok(());
    }
    set_config(
        config_path,
        &[
            ("secure_cookies", toml::Value::Boolean(true)),
            ("trust_proxy", toml::Value::Boolean(true)),
        ],
    )?;
    println!("Done. Restart the server, then finish the Cloudflare steps above.");
    println!("Until Access is in place, the hostname is open to the internet.");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn set_config_keeps_other_keys_and_creates_the_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(&path, "host = \"127.0.0.1\"\nport = 9000\n").unwrap();
        set_config(&path, &[("secure_cookies", toml::Value::Boolean(true))]).unwrap();
        let table: toml::Table = toml::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(table["host"].as_str(), Some("127.0.0.1"));
        assert_eq!(table["port"].as_integer(), Some(9000));
        assert_eq!(table["secure_cookies"].as_bool(), Some(true));

        let fresh = dir.path().join("new.toml");
        set_config(&fresh, &[("trust_proxy", toml::Value::Boolean(true))]).unwrap();
        assert!(fresh.is_file());
    }
}
