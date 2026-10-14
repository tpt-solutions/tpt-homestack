// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 TPT Solutions

//! Admin "Remote access" page data: live checks of the remote-access clients
//! and the steps still to do. The page never changes the machine. Config
//! changes stay in `tpt-home remote setup`, which asks before it acts, and the
//! page shows that command instead.

use std::sync::Arc;

use axum::Json;
use axum::extract::Extension;
use serde::Serialize;
use serde_json::json;

use tpt_home_core::config::Config;
use tpt_home_core::error::ApiError;
use tpt_home_core::remote::{Detected, describe_tailscale};

/// Where a provider stands: finished, waiting for a person, or blocked on an
/// install that has to happen first.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Stage {
    Done,
    Todo,
    Blocked,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Step {
    pub provider: &'static str,
    pub stage: Stage,
    pub text: String,
    /// The command to run on the server, when there is one.
    pub command: Option<&'static str>,
}

/// The steps for each provider, from what the checks found and the config.
pub fn plan(cfg: &Config, found: &Detected) -> Vec<Step> {
    let ts = &found.tailscale;
    let mut steps = Vec::new();

    let tailscale = if !ts.installed {
        Step {
            provider: "tailscale",
            stage: Stage::Blocked,
            text: "Install the official Tailscale client from tailscale.com/download.".into(),
            command: None,
        }
    } else if !ts.running {
        Step {
            provider: "tailscale",
            stage: Stage::Todo,
            text: "Sign in: run `tailscale up` on this computer, then check again.".into(),
            command: Some("tailscale up"),
        }
    } else if ts.dns_name.is_none() {
        Step {
            provider: "tailscale",
            stage: Stage::Todo,
            text: "Turn on MagicDNS in the Tailscale admin console, then check again.".into(),
            command: None,
        }
    } else if cfg.secure_cookies {
        Step {
            provider: "tailscale",
            stage: Stage::Done,
            text: format!(
                "Set up: open https://{} from another device on your tailnet.",
                ts.dns_name.as_deref().unwrap_or_default()
            ),
            command: None,
        }
    } else {
        Step {
            provider: "tailscale",
            stage: Stage::Todo,
            text: format!(
                "Publish the server on https://{} (it asks before changing anything).",
                ts.dns_name.as_deref().unwrap_or_default()
            ),
            command: Some("tpt-home remote setup --provider tailscale"),
        }
    };
    steps.push(tailscale);

    steps.push(if !found.cloudflared {
        Step {
            provider: "cloudflare",
            stage: Stage::Blocked,
            text: "Install cloudflared from Cloudflare's downloads page first.".into(),
            command: None,
        }
    } else if cfg.trust_proxy && cfg.secure_cookies {
        Step {
            provider: "cloudflare",
            stage: Stage::Done,
            text: "Tunnel settings are in place. Check that Cloudflare Access protects the hostname.".into(),
            command: None,
        }
    } else {
        Step {
            provider: "cloudflare",
            stage: Stage::Todo,
            text: "Lists the dashboard steps (domain, tunnel, Access) and asks before it changes the config.".into(),
            command: Some("tpt-home remote setup --provider cloudflare"),
        }
    });

    steps.push(Step {
        provider: "netbird",
        stage: if found.netbird {
            Stage::Todo
        } else {
            Stage::Blocked
        },
        text: if found.netbird {
            "Netbird is installed. Its setup is in docs/REMOTE_ADVANCED.md; the server listens on the Netbird address only.".into()
        } else {
            "Optional: install Netbird for a private network (see docs/REMOTE_ADVANCED.md).".into()
        },
        command: None,
    });
    steps
}

/// The one-line status of Tailscale, for the page header.
pub fn tailscale_line(found: &Detected) -> String {
    describe_tailscale(&found.tailscale)
}

/// GET /api/admin/remote: the config that matters for remote access, what the
/// checks found, and the steps.
pub async fn status(
    Extension(cfg): Extension<Arc<Config>>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let found = tokio::task::spawn_blocking(tpt_home_core::remote::detect)
        .await
        .map_err(|e| ApiError::internal(format!("remote check: {e}")))?;
    let steps = plan(&cfg, &found);
    Ok(Json(json!({
        "config": {
            "host": cfg.host,
            "port": cfg.port,
            "secure_cookies": cfg.secure_cookies,
            "trust_proxy": cfg.trust_proxy,
            "exposed": cfg.exposed(),
        },
        "detected": found,
        "tailscale": tailscale_line(&found),
        "steps": steps,
    })))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tpt_home_core::remote::TailscaleState;

    fn found(ts: TailscaleState, cloudflared: bool) -> Detected {
        Detected {
            tailscale: ts,
            cloudflared,
            netbird: false,
        }
    }

    fn tailscale_step(steps: &[Step]) -> &Step {
        steps.iter().find(|s| s.provider == "tailscale").unwrap()
    }

    #[test]
    fn a_missing_client_blocks_the_first_step() {
        let steps = plan(&Config::default(), &found(TailscaleState::default(), false));
        assert_eq!(tailscale_step(&steps).stage, Stage::Blocked);
        let cf = steps.iter().find(|s| s.provider == "cloudflare").unwrap();
        assert_eq!(cf.stage, Stage::Blocked);
    }

    #[test]
    fn a_signed_out_client_asks_for_sign_in() {
        let ts = TailscaleState {
            installed: true,
            ..Default::default()
        };
        let steps = plan(&Config::default(), &found(ts, true));
        assert_eq!(tailscale_step(&steps).command, Some("tailscale up"));
    }

    #[test]
    fn a_signed_in_client_offers_the_setup_command_until_it_is_done() {
        let ts = TailscaleState {
            installed: true,
            running: true,
            dns_name: Some("laptop.tail1.ts.net".into()),
            ips: vec![],
        };
        let mut cfg = Config::default();
        let todo = plan(&cfg, &found(ts.clone(), true));
        assert_eq!(tailscale_step(&todo).stage, Stage::Todo);
        assert_eq!(
            tailscale_step(&todo).command,
            Some("tpt-home remote setup --provider tailscale")
        );
        cfg.secure_cookies = true;
        let done = plan(&cfg, &found(ts, true));
        assert_eq!(tailscale_step(&done).stage, Stage::Done);
        assert!(
            tailscale_step(&done)
                .text
                .contains("https://laptop.tail1.ts.net")
        );
    }

    #[test]
    fn cloudflare_is_done_only_with_proxy_and_secure_cookies() {
        let mut cfg = Config {
            trust_proxy: true,
            ..Config::default()
        };
        let steps = plan(&cfg, &found(TailscaleState::default(), true));
        let cf = steps.iter().find(|s| s.provider == "cloudflare").unwrap();
        assert_eq!(cf.stage, Stage::Todo);
        cfg.secure_cookies = true;
        let steps = plan(&cfg, &found(TailscaleState::default(), true));
        let cf = steps.iter().find(|s| s.provider == "cloudflare").unwrap();
        assert_eq!(cf.stage, Stage::Done);
    }
}
