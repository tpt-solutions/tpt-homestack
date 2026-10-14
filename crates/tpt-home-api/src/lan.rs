// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 TPT Solutions

//! Reach the server by name on the home network. When the server listens
//! beyond loopback it announces `<lan_name>.local` over mDNS, so a computer or
//! phone on the same network can open it without knowing the address.
//!
//! Name lookup is up to the client: Windows 10 and later, Apple devices and
//! most Linux desktops resolve `.local` names; some Android phones do not.
//! The settings page therefore also shows the server's address as a QR code.

use mdns_sd::{ServiceDaemon, ServiceInfo};

use tpt_home_core::config::Config;

/// The DNS-SD service type the server announces under.
pub const SERVICE_TYPE: &str = "_http._tcp.local.";

/// True when the configured bind address keeps the server on this machine.
pub fn is_loopback_host(host: &str) -> bool {
    matches!(host, "127.0.0.1" | "localhost" | "::1")
}

/// The address to type on another device, e.g. `http://homestack.local:8080`.
/// Port 80 is left out, as browsers do.
pub fn lan_url(cfg: &Config) -> String {
    if cfg.port == 80 {
        format!("http://{}.local", cfg.lan_name)
    } else {
        format!("http://{}.local:{}", cfg.lan_name, cfg.port)
    }
}

/// Keeps the mDNS announcement alive. Dropping it withdraws the name.
pub struct Announcement {
    daemon: ServiceDaemon,
    fullname: String,
}

impl Drop for Announcement {
    fn drop(&mut self) {
        let _ = self.daemon.unregister(&self.fullname);
        let _ = self.daemon.shutdown();
    }
}

/// Announce the server if it is configured to be reachable on the network.
/// Returns `Ok(None)` when announcing is off or the server is loopback-only.
pub fn announce(cfg: &Config) -> anyhow::Result<Option<Announcement>> {
    if !cfg.advertise || is_loopback_host(&cfg.host) {
        return Ok(None);
    }
    let daemon = ServiceDaemon::new().map_err(|e| anyhow::anyhow!("mDNS daemon: {e}"))?;
    let host_name = format!("{}.local.", cfg.lan_name);
    let properties: Vec<(&str, &str)> = vec![("path", "/")];
    // The addresses come from the host's interfaces (`()` plus auto update),
    // so a changed IP address is announced without a restart.
    let info = ServiceInfo::new(
        SERVICE_TYPE,
        &cfg.lan_name,
        &host_name,
        (),
        cfg.port,
        &properties[..],
    )
    .map_err(|e| anyhow::anyhow!("mDNS service: {e}"))?
    .enable_addr_auto();
    let fullname = info.get_fullname().to_string();
    daemon
        .register(info)
        .map_err(|e| anyhow::anyhow!("mDNS register: {e}"))?;
    Ok(Some(Announcement { daemon, fullname }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn url_leaves_out_the_default_http_port() {
        let mut cfg = Config {
            port: 80,
            ..Config::default()
        };
        assert_eq!(lan_url(&cfg), "http://homestack.local");
        cfg.port = 8080;
        assert_eq!(lan_url(&cfg), "http://homestack.local:8080");
        cfg.lan_name = "family".into();
        assert_eq!(lan_url(&cfg), "http://family.local:8080");
    }

    #[test]
    fn loopback_servers_announce_nothing() {
        let mut cfg = Config::default();
        assert!(is_loopback_host(&cfg.host));
        assert!(announce(&cfg).unwrap().is_none());
        cfg.host = "0.0.0.0".into();
        cfg.advertise = false;
        assert!(announce(&cfg).unwrap().is_none(), "switched off in config");
    }
}
