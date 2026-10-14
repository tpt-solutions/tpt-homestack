// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 TPT Solutions

//! The one place the server fetches files from the internet: the update
//! manifest and the plugin catalog. Only HTTPS is allowed (plain HTTP only to
//! loopback, for tests), redirects are not followed, and every response has a
//! size cap and a timeout. Blocking: call it from `spawn_blocking`.

use std::time::Duration;

/// Refuse a URL that is not HTTPS, unless it points at loopback.
pub fn check_url(url: &str) -> Result<(), String> {
    let secure = url.starts_with("https://")
        || url.starts_with("http://127.0.0.1")
        || url.starts_with("http://localhost");
    if secure {
        Ok(())
    } else {
        Err("the address must use https".into())
    }
}

/// GET `url` and return the body, at most `max_bytes` long. Anything over the
/// cap, any status other than 200, and any redirect is an error.
pub fn https_get(url: &str, max_bytes: u64) -> Result<Vec<u8>, String> {
    check_url(url)?;
    let config = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(15)))
        .max_redirects(0)
        .http_status_as_error(false)
        .build();
    let agent = ureq::Agent::new_with_config(config);
    let mut response = agent
        .get(url)
        .call()
        .map_err(|e| format!("could not reach {url}: {e}"))?;
    let status = response.status().as_u16();
    if status != 200 {
        return Err(format!("the server answered {status} for {url}"));
    }
    let bytes = response
        .body_mut()
        .with_config()
        .limit(max_bytes.saturating_add(1))
        .read_to_vec()
        .map_err(|e| format!("could not read {url}: {e}"))?;
    if bytes.len() as u64 > max_bytes {
        return Err(format!("{url} is larger than the {max_bytes}-byte limit"));
    }
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_https_or_loopback_is_fetched() {
        assert!(check_url("https://example.org/x").is_ok());
        assert!(check_url("http://127.0.0.1:9/x").is_ok());
        assert!(check_url("http://example.org/x").is_err());
        assert!(check_url("file:///etc/passwd").is_err());
    }
}
