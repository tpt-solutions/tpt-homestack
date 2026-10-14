// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 TPT Solutions

//! Detection of the remote-access clients on this machine: Tailscale,
//! cloudflared and Netbird. Shared by `tpt-home remote` and the admin
//! "Remote access" page. Nothing here changes the machine; it only runs each
//! client's status command.
//!
//! Each client is found on PATH by default. `TPT_HOME_TAILSCALE_BIN`,
//! `TPT_HOME_CLOUDFLARED_BIN` and `TPT_HOME_NETBIRD_BIN` replace the program,
//! for clients installed outside PATH and for tests.

use std::ffi::OsString;
use std::process::{Command, Output};

use serde::Serialize;

fn program(name: &str, env: &str) -> OsString {
    std::env::var_os(env).unwrap_or_else(|| OsString::from(name))
}

pub fn tailscale_bin() -> OsString {
    program("tailscale", "TPT_HOME_TAILSCALE_BIN")
}

pub fn cloudflared_bin() -> OsString {
    program("cloudflared", "TPT_HOME_CLOUDFLARED_BIN")
}

pub fn netbird_bin() -> OsString {
    program("netbird", "TPT_HOME_NETBIRD_BIN")
}

/// Run a client. `None` means it is not installed (it could not be started).
pub fn spawn(bin: &OsString, args: &[&str]) -> Option<Output> {
    Command::new(bin).args(args).output().ok()
}

#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize)]
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

/// Ask the Tailscale client for this machine's state.
pub fn tailscale_state(bin: &OsString) -> TailscaleState {
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

/// One line for people: "not found", "signed in as https://…" and so on.
pub fn describe_tailscale(ts: &TailscaleState) -> String {
    match (ts.installed, ts.running, &ts.dns_name) {
        (false, _, _) => "not found".to_string(),
        (true, false, _) => "installed, not signed in".to_string(),
        (true, true, Some(name)) => format!("signed in as https://{name}"),
        (true, true, None) => "signed in, no MagicDNS name".to_string(),
    }
}

/// Everything `detect` found on this machine.
#[derive(Debug, Serialize)]
pub struct Detected {
    pub tailscale: TailscaleState,
    pub cloudflared: bool,
    pub netbird: bool,
}

/// Check all three clients. Runs each client's status command, so call it
/// from a blocking context (e.g. `tokio::task::spawn_blocking`).
pub fn detect() -> Detected {
    Detected {
        tailscale: tailscale_state(&tailscale_bin()),
        cloudflared: spawn(&cloudflared_bin(), &["--version"]).is_some(),
        netbird: spawn(&netbird_bin(), &["version"]).is_some(),
    }
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
    fn describes_each_tailscale_state() {
        assert_eq!(describe_tailscale(&TailscaleState::default()), "not found");
        let off = TailscaleState {
            installed: true,
            ..Default::default()
        };
        assert_eq!(describe_tailscale(&off), "installed, not signed in");
    }

    #[test]
    fn a_missing_client_is_reported_as_not_installed() {
        let missing = OsString::from("tpt-home-no-such-client-binary");
        assert_eq!(tailscale_state(&missing), TailscaleState::default());
        assert!(spawn(&missing, &["--version"]).is_none());
    }
}
