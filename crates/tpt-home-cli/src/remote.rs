// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 TPT Solutions

//! `tpt-home remote status|setup`: the checks behind "connect from anywhere".
//! It finds the official clients on this machine, prints the steps only a
//! person can do (signing in, creating a tunnel), and changes `config.toml`
//! only after confirmation. The server still binds to 127.0.0.1; the clients
//! reach it through `tailscale serve` or `cloudflared`.
//!
//! Each client is found on PATH by default. `TPT_HOME_TAILSCALE_BIN`,
//! `TPT_HOME_CLOUDFLARED_BIN` and `TPT_HOME_NETBIRD_BIN` replace the program,
//! for clients installed outside PATH and for tests.

use std::ffi::OsString;
use std::path::Path;
use std::process::{Command, Output};

use anyhow::Context;
use clap::ValueEnum;

use tpt_home_core::config::Config;

#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
pub enum Provider {
    /// Private to your tailnet; no domain or router changes needed.
    Tailscale,
    /// A public hostname behind a login, through a Cloudflare tunnel.
    Cloudflare,
}

fn program(name: &str, env: &str) -> OsString {
    std::env::var_os(env).unwrap_or_else(|| OsString::from(name))
}

fn tailscale_bin() -> OsString {
    program("tailscale", "TPT_HOME_TAILSCALE_BIN")
}

fn cloudflared_bin() -> OsString {
    program("cloudflared", "TPT_HOME_CLOUDFLARED_BIN")
}

fn netbird_bin() -> OsString {
    program("netbird", "TPT_HOME_NETBIRD_BIN")
}

/// Run a client. `None` means it is not installed (it could not be started).
fn spawn(bin: &OsString, args: &[&str]) -> Option<Output> {
    Command::new(bin).args(args).output().ok()
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct TailscaleState {
    pub installed: bool,
    /// Signed in and connected (`BackendState` is `Running`).
    pub running: bool,
    /// MagicDNS name of this machine, without the trailing dot.
    pub dns_name: Option<String>,
    pub ips: Vec<String>,
}

/// Read the output of `tailscale status --json`.
pub fn parse_tailscale_status(json: &str) -> Option<TailscaleState> {
    let v: serde_json::Value = serde_json::from_str(json).ok()?;
    let me = &v["Self"];
    let dns_name = me["DNSName"]
        .as_str()
        .map(|s| s.trim_end_matches('.').to_string())
        .filter(|s| !s.is_empty());
    let ips = me["TailscaleIPs"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|x| x.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default();
    Some(TailscaleState {
        installed: true,
        running: v["BackendState"].as_str() == Some("Running"),
        dns_name,
        ips,
    })
}

fn tailscale_state(bin: &OsString) -> TailscaleState {
    match spawn(bin, &["status", "--json"]) {
        None => TailscaleState::default(),
        Some(out) => {
            let mut state =
                parse_tailscale_status(&String::from_utf8_lossy(&out.stdout)).unwrap_or_default();
            state.installed = true;
            state
        }
    }
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

    let ts = tailscale_state(&tailscale_bin());
    println!("tailscale: {}", describe_tailscale(&ts));

    let cf = spawn(&cloudflared_bin(), &["--version"]).is_some();
    println!(
        "cloudflared: {}",
        if cf { "installed" } else { "not found" }
    );

    let nb = spawn(&netbird_bin(), &["version"]).is_some();
    println!("netbird: {}", if nb { "installed" } else { "not found" });
    Ok(())
}

fn describe_tailscale(ts: &TailscaleState) -> String {
    match (ts.installed, ts.running, &ts.dns_name) {
        (false, _, _) => "not found".to_string(),
        (true, false, _) => "installed, not signed in".to_string(),
        (true, true, Some(name)) => format!("signed in as https://{name}"),
        (true, true, None) => "signed in, no MagicDNS name".to_string(),
    }
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
    let bin = tailscale_bin();
    let ts = tailscale_state(&bin);
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
    let bin = cloudflared_bin();
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
    fn parses_a_signed_in_tailscale_status() {
        let json = r#"{"BackendState":"Running","Self":{"DNSName":"laptop.tail1234.ts.net.","TailscaleIPs":["100.64.0.5","fd7a::5"]}}"#;
        let st = parse_tailscale_status(json).unwrap();
        assert!(st.installed && st.running);
        assert_eq!(st.dns_name.as_deref(), Some("laptop.tail1234.ts.net"));
        assert_eq!(st.ips, ["100.64.0.5", "fd7a::5"]);
    }

    #[test]
    fn logged_out_tailscale_is_not_running() {
        let json = r#"{"BackendState":"NeedsLogin","Self":{}}"#;
        let st = parse_tailscale_status(json).unwrap();
        assert!(st.installed && !st.running);
        assert_eq!(st.dns_name, None);
        assert!(parse_tailscale_status("not json").is_none());
    }

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

    #[test]
    fn describes_each_tailscale_state() {
        assert_eq!(describe_tailscale(&TailscaleState::default()), "not found");
        let off = TailscaleState {
            installed: true,
            ..Default::default()
        };
        assert_eq!(describe_tailscale(&off), "installed, not signed in");
    }
}
