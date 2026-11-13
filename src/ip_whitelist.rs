// Allows a connection to start only when an external HTTP service approves
// the initiator's IP.
//
// IP_WHITELIST_URL         GET endpoint answering {"allowed": bool}. `{ip}` in
//                          the URL is replaced by the IP; without it, `ip=<IP>`
//                          is appended as a query parameter. Empty disables the
//                          check.
// IP_WHITELIST_TIMEOUT_MS  Request timeout (default 3000).
// IP_WHITELIST_FAIL_OPEN   "Y" lets connections through when the service cannot
//                          be reached or answers badly (default: denied).

use crate::common::get_arg;
use hbb_common::log;
use once_cell::sync::Lazy;
use serde_derive::Deserialize;
use std::{net::IpAddr, time::Duration};

const DEFAULT_TIMEOUT_MS: u64 = 3_000;

struct Config {
    url: String,
    fail_open: bool,
    client: Option<reqwest::Client>,
}

#[derive(Deserialize)]
struct Answer {
    allowed: bool,
}

static CONFIG: Lazy<Config> = Lazy::new(|| {
    let url = get_arg("IP_WHITELIST_URL").trim().to_owned();
    let timeout = get_arg("IP_WHITELIST_TIMEOUT_MS")
        .parse::<u64>()
        .ok()
        .filter(|ms| *ms > 0)
        .unwrap_or(DEFAULT_TIMEOUT_MS);
    let fail_open = get_arg("IP_WHITELIST_FAIL_OPEN").to_uppercase() == "Y";
    let client = if url.is_empty() {
        None
    } else {
        match reqwest::Client::builder()
            .timeout(Duration::from_millis(timeout))
            .build()
        {
            Ok(client) => Some(client),
            Err(err) => {
                log::error!("IP whitelist: failed to build HTTP client: {err}");
                None
            }
        }
    };
    Config {
        url,
        fail_open,
        client,
    }
});

pub fn log_config() {
    let cfg = &*CONFIG;
    if cfg.url.is_empty() {
        log::info!("IP_WHITELIST_URL not set, initiator IP check disabled");
    } else {
        log::info!(
            "IP_WHITELIST_URL={} fail_open={}",
            cfg.url,
            cfg.fail_open
        );
    }
}

fn build_url(base: &str, ip: &IpAddr) -> String {
    let ip = ip.to_string();
    if base.contains("{ip}") {
        return base.replace("{ip}", &ip);
    }
    let sep = if base.contains('?') { '&' } else { '?' };
    // IPv6 colons are valid in a query string, so the IP needs no escaping.
    format!("{base}{sep}ip={ip}")
}

async fn query(client: &reqwest::Client, url: &str) -> Result<bool, String> {
    let res = client.get(url).send().await.map_err(|e| e.to_string())?;
    let status = res.status();
    if !status.is_success() {
        return Err(format!("HTTP {status}"));
    }
    let bytes = res.bytes().await.map_err(|e| e.to_string())?;
    let answer: Answer = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
    Ok(answer.allowed)
}

/// Whether `ip` may start a connection. Always true when no URL is configured.
pub async fn is_allowed(ip: IpAddr) -> bool {
    let cfg = &*CONFIG;
    if cfg.url.is_empty() {
        return true;
    }
    let Some(client) = cfg.client.as_ref() else {
        return cfg.fail_open;
    };
    let url = build_url(&cfg.url, &ip);
    match query(client, &url).await {
        Ok(allowed) => {
            if !allowed {
                log::info!("IP whitelist denied {ip}");
            }
            allowed
        }
        Err(err) => {
            log::warn!(
                "IP whitelist check for {ip} failed: {err}, {}",
                if cfg.fail_open { "allowing" } else { "denying" }
            );
            cfg.fail_open
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn url_placeholder_and_query() {
        let v4: IpAddr = "1.2.3.4".parse().unwrap();
        let v6: IpAddr = "::1".parse().unwrap();
        assert_eq!(
            build_url("http://x/check/{ip}", &v4),
            "http://x/check/1.2.3.4"
        );
        assert_eq!(build_url("http://x/check", &v4), "http://x/check?ip=1.2.3.4");
        assert_eq!(build_url("http://x/c?k=1", &v6), "http://x/c?k=1&ip=::1");
    }
}
