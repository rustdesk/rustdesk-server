use clap::App;
use hbb_common::{
    allow_err, anyhow::{Context, Result}, get_version_number, log, tokio, ResultType
};
use http::HeaderMap;
use ini::Ini;
use once_cell::sync::Lazy;
use sodiumoxide::crypto::sign;
use std::{
    collections::HashMap,
    io::prelude::*,
    io::Read,
    net::{IpAddr, SocketAddr},
    sync::Mutex,
    time::{Instant, SystemTime},
};

const TRUST_PROXY_HEADERS_ENV: &str = "TRUST_PROXY_HEADERS";
const CONN_RATE_WINDOW_SECONDS_ENV: &str = "CONNECTION_RATE_WINDOW_SECONDS";
const MAX_CONN_PER_IP_PER_WINDOW_ENV: &str = "MAX_CONNECTIONS_PER_IP_PER_WINDOW";
const UDP_RATE_WINDOW_SECONDS_ENV: &str = "UDP_RATE_WINDOW_SECONDS";
const MAX_UDP_PACKETS_PER_IP_PER_WINDOW_ENV: &str = "MAX_UDP_PACKETS_PER_IP_PER_WINDOW";
const DEFAULT_CONN_RATE_WINDOW_SECONDS: usize = 60;
const DEFAULT_MAX_CONN_PER_IP_PER_WINDOW: usize = 120;
const DEFAULT_UDP_RATE_WINDOW_SECONDS: usize = 60;
const DEFAULT_MAX_UDP_PACKETS_PER_IP_PER_WINDOW: usize = 240;

#[derive(Clone, Copy)]
struct ConnectionRateEntry {
    window_started_at: Instant,
    last_seen_at: Instant,
    count: usize,
}

static CONNECTION_RATE_LIMITS: Lazy<Mutex<HashMap<String, ConnectionRateEntry>>> =
    Lazy::new(|| Mutex::new(HashMap::new()));
static PROTECTION_STATS: Lazy<Mutex<HashMap<&'static str, u64>>> =
    Lazy::new(|| Mutex::new(HashMap::new()));

#[allow(dead_code)]
pub(crate) fn get_expired_time() -> Instant {
    let now = Instant::now();
    now.checked_sub(std::time::Duration::from_secs(3600))
        .unwrap_or(now)
}

#[allow(dead_code)]
pub(crate) fn test_if_valid_server(host: &str, name: &str) -> ResultType<SocketAddr> {
    use std::net::ToSocketAddrs;
    let res = if host.contains(':') {
        host.to_socket_addrs()?.next().context("")
    } else {
        format!("{}:{}", host, 0)
            .to_socket_addrs()?
            .next()
            .context("")
    };
    if res.is_err() {
        log::error!("Invalid {} {}: {:?}", name, host, res);
    }
    res
}

#[allow(dead_code)]
pub(crate) fn get_servers(s: &str, tag: &str) -> Vec<String> {
    let servers: Vec<String> = s
        .split(',')
        .filter(|x| !x.is_empty() && test_if_valid_server(x, tag).is_ok())
        .map(|x| x.to_owned())
        .collect();
    log::info!("{}={:?}", tag, servers);
    servers
}

#[allow(dead_code)]
#[inline]
fn arg_name(name: &str) -> String {
    name.to_uppercase().replace('_', "-")
}

#[allow(dead_code)]
pub fn init_args(args: &str, name: &str, about: &str) {
    let matches = App::new(name)
        .version(crate::version::VERSION)
        .author("Purslane Ltd. <info@rustdesk.com>")
        .about(about)
        .args_from_usage(args)
        .get_matches();
    if let Ok(v) = Ini::load_from_file(".env") {
        if let Some(section) = v.section(None::<String>) {
            section
                .iter()
                .for_each(|(k, v)| std::env::set_var(arg_name(k), v));
        }
    }
    if let Some(config) = matches.value_of("config") {
        if let Ok(v) = Ini::load_from_file(config) {
            if let Some(section) = v.section(None::<String>) {
                section
                    .iter()
                    .for_each(|(k, v)| std::env::set_var(arg_name(k), v));
            }
        }
    }
    for (k, v) in matches.args {
        if let Some(v) = v.vals.first() {
            std::env::set_var(arg_name(k), v.to_string_lossy().to_string());
        }
    }
}

#[allow(dead_code)]
#[inline]
pub fn get_arg(name: &str) -> String {
    get_arg_or(name, "".to_owned())
}

#[allow(dead_code)]
#[inline]
pub fn get_arg_or(name: &str, default: String) -> String {
    std::env::var(arg_name(name)).unwrap_or(default)
}

#[allow(dead_code)]
pub fn trust_proxy_headers() -> bool {
    matches!(
        std::env::var(TRUST_PROXY_HEADERS_ENV)
            .unwrap_or_default()
            .trim()
            .to_ascii_lowercase()
            .as_str(),
        "y" | "yes" | "true" | "1"
    )
}

fn env_usize_or(name: &str, default: usize) -> usize {
    std::env::var(name)
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(default)
}

fn conn_rate_window_seconds() -> usize {
    env_usize_or(
        CONN_RATE_WINDOW_SECONDS_ENV,
        DEFAULT_CONN_RATE_WINDOW_SECONDS,
    )
}

fn max_conn_per_ip_per_window() -> usize {
    env_usize_or(
        MAX_CONN_PER_IP_PER_WINDOW_ENV,
        DEFAULT_MAX_CONN_PER_IP_PER_WINDOW,
    )
}

fn udp_rate_window_seconds() -> usize {
    env_usize_or(
        UDP_RATE_WINDOW_SECONDS_ENV,
        DEFAULT_UDP_RATE_WINDOW_SECONDS,
    )
}

fn max_udp_packets_per_ip_per_window() -> usize {
    env_usize_or(
        MAX_UDP_PACKETS_PER_IP_PER_WINDOW_ENV,
        DEFAULT_MAX_UDP_PACKETS_PER_IP_PER_WINDOW,
    )
}

fn prune_connection_rate_limits(
    entries: &mut HashMap<String, ConnectionRateEntry>,
    now: Instant,
    window_secs: usize,
) {
    entries.retain(|_, entry| {
        now.duration_since(entry.last_seen_at).as_secs() < (window_secs * 2) as u64
    });
}

#[allow(dead_code)]
fn allow_ip_activity(scope: &str, addr: SocketAddr, window_secs: usize, max_events: usize) -> bool {
    if addr.ip().is_loopback() {
        return true;
    }
    let now = Instant::now();
    let mut lock = CONNECTION_RATE_LIMITS.lock().unwrap();
    prune_connection_rate_limits(&mut lock, now, window_secs);
    let key = format!("{scope}|{}", addr.ip());
    let entry = lock.entry(key).or_insert(ConnectionRateEntry {
        window_started_at: now,
        last_seen_at: now,
        count: 0,
    });
    if now.duration_since(entry.window_started_at).as_secs() >= window_secs as u64 {
        entry.window_started_at = now;
        entry.count = 0;
    }
    entry.last_seen_at = now;
    if entry.count >= max_events {
        return false;
    }
    entry.count += 1;
    true
}

#[allow(dead_code)]
pub fn allow_connection_from_ip(scope: &str, addr: SocketAddr) -> bool {
    let allowed = allow_ip_activity(
        scope,
        addr,
        conn_rate_window_seconds(),
        max_conn_per_ip_per_window(),
    );
    if !allowed {
        record_protection_event("connection_rate_limit_hits");
    }
    allowed
}

#[allow(dead_code)]
pub fn allow_udp_packet_from_ip(scope: &str, addr: SocketAddr) -> bool {
    let allowed = allow_ip_activity(
        scope,
        addr,
        udp_rate_window_seconds(),
        max_udp_packets_per_ip_per_window(),
    );
    if !allowed {
        record_protection_event("udp_rate_limit_hits");
    }
    allowed
}

#[allow(dead_code)]
pub fn record_protection_event(name: &'static str) {
    let mut lock = PROTECTION_STATS.lock().unwrap();
    *lock.entry(name).or_insert(0) += 1;
}

#[allow(dead_code)]
pub fn protection_stats_snapshot() -> Vec<(String, u64)> {
    let mut entries: Vec<(String, u64)> = PROTECTION_STATS
        .lock()
        .unwrap()
        .iter()
        .map(|(name, value)| ((*name).to_owned(), *value))
        .collect();
    entries.sort_by(|a, b| a.0.cmp(&b.0));
    entries
}

#[allow(dead_code)]
pub fn protection_limits_summary() -> Vec<String> {
    vec![
        format!(
            "connections_per_ip_per_window={}/{}s",
            max_conn_per_ip_per_window(),
            conn_rate_window_seconds()
        ),
        format!(
            "udp_packets_per_ip_per_window={}/{}s",
            max_udp_packets_per_ip_per_window(),
            udp_rate_window_seconds()
        ),
        format!("trust_proxy_headers={}", trust_proxy_headers()),
    ]
}

#[allow(dead_code)]
pub fn apply_trusted_proxy_addr(addr: SocketAddr, headers: &HeaderMap) -> SocketAddr {
    if !trust_proxy_headers() {
        return addr;
    }
    let forwarded_ip = headers
        .get("X-Real-IP")
        .or_else(|| headers.get("X-Forwarded-For"))
        .and_then(|header_value| header_value.to_str().ok())
        .and_then(parse_forwarded_ip);
    forwarded_ip.map(|ip| SocketAddr::new(ip, 0)).unwrap_or(addr)
}

fn parse_forwarded_ip(value: &str) -> Option<IpAddr> {
    value
        .split(',')
        .next()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .and_then(|value| value.parse::<IpAddr>().ok())
}

#[allow(dead_code)]
#[inline]
pub fn now() -> u64 {
    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|x| x.as_secs())
        .unwrap_or_default()
}

pub fn gen_sk(wait: u64) -> (String, Option<sign::SecretKey>) {
    let sk_file = "id_ed25519";
    if wait > 0 && !std::path::Path::new(sk_file).exists() {
        std::thread::sleep(std::time::Duration::from_millis(wait));
    }
    if let Ok(mut file) = std::fs::File::open(sk_file) {
        let mut contents = String::new();
        if file.read_to_string(&mut contents).is_ok() {
            let contents = contents.trim();
            let sk = base64::decode(contents).unwrap_or_default();
            if sk.len() == sign::SECRETKEYBYTES {
                let mut tmp = [0u8; sign::SECRETKEYBYTES];
                tmp[..].copy_from_slice(&sk);
                let pk = base64::encode(&tmp[sign::SECRETKEYBYTES / 2..]);
                log::info!("Private key comes from {}", sk_file);
                return (pk, Some(sign::SecretKey(tmp)));
            } else {
                // don't use log here, since it is async
                println!("Fatal error: malformed private key in {sk_file}.");
                std::process::exit(1);
            }
        }
    } else {
        let gen_func = || {
            let (tmp, sk) = sign::gen_keypair();
            (base64::encode(tmp), sk)
        };
        let (mut pk, mut sk) = gen_func();
        for _ in 0..300 {
            if !pk.contains('/') && !pk.contains(':') {
                break;
            }
            (pk, sk) = gen_func();
        }
        let pub_file = format!("{sk_file}.pub");
        if let Ok(mut f) = std::fs::File::create(&pub_file) {
            f.write_all(pk.as_bytes()).ok();
            if let Ok(mut f) = std::fs::File::create(sk_file) {
                let s = base64::encode(&sk);
                if f.write_all(s.as_bytes()).is_ok() {
                    log::info!("Private/public key written to {}/{}", sk_file, pub_file);
                    log::debug!("Public key: {}", pk);
                    return (pk, Some(sk));
                }
            }
        }
    }
    ("".to_owned(), None)
}

#[cfg(unix)]
pub async fn listen_signal() -> Result<()> {
    use hbb_common::tokio;
    use hbb_common::tokio::signal::unix::{signal, SignalKind};

    tokio::spawn(async {
        let mut s = signal(SignalKind::terminate())?;
        let terminate = s.recv();
        let mut s = signal(SignalKind::interrupt())?;
        let interrupt = s.recv();
        let mut s = signal(SignalKind::quit())?;
        let quit = s.recv();

        tokio::select! {
            _ = terminate => {
                log::info!("signal terminate");
            }
            _ = interrupt => {
                log::info!("signal interrupt");
            }
            _ = quit => {
                log::info!("signal quit");
            }
        }
        Ok(())
    })
    .await?
}

#[cfg(not(unix))]
pub async fn listen_signal() -> Result<()> {
    let () = std::future::pending().await;
    unreachable!();
}


pub fn check_software_update() {
    const ONE_DAY_IN_SECONDS: u64 = 60 * 60 * 24;
    std::thread::spawn(move || loop {
        std::thread::spawn(move || allow_err!(check_software_update_()));
        std::thread::sleep(std::time::Duration::from_secs(ONE_DAY_IN_SECONDS));
    });
}

#[tokio::main(flavor = "current_thread")]
async fn check_software_update_() -> hbb_common::ResultType<()> {
    let (request, url) = hbb_common::version_check_request(hbb_common::VER_TYPE_RUSTDESK_SERVER.to_string());
    let latest_release_response = reqwest::Client::builder().build()?
        .post(url)
        .json(&request)
        .send()
        .await?;

    let bytes = latest_release_response.bytes().await?;
    let resp: hbb_common::VersionCheckResponse = serde_json::from_slice(&bytes)?;
    let response_url = resp.url;
    let latest_release_version = response_url.rsplit('/').next().unwrap_or_default();
    if get_version_number(&latest_release_version) > get_version_number(crate::version::VERSION) {
       log::info!("new version is available: {}", latest_release_version);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{
        allow_connection_from_ip, allow_udp_packet_from_ip, apply_trusted_proxy_addr,
        conn_rate_window_seconds, max_conn_per_ip_per_window, max_udp_packets_per_ip_per_window,
        protection_limits_summary, protection_stats_snapshot, record_protection_event,
        trust_proxy_headers, udp_rate_window_seconds, CONNECTION_RATE_LIMITS,
        CONN_RATE_WINDOW_SECONDS_ENV, MAX_CONN_PER_IP_PER_WINDOW_ENV,
        MAX_UDP_PACKETS_PER_IP_PER_WINDOW_ENV, PROTECTION_STATS, TRUST_PROXY_HEADERS_ENV,
        UDP_RATE_WINDOW_SECONDS_ENV,
    };
    use http::HeaderMap;
    use std::{
        net::{IpAddr, Ipv4Addr, SocketAddr},
        sync::Mutex,
    };

    static TEST_PROXY_HEADERS_LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn trusted_proxy_headers_are_disabled_by_default() {
        let _guard = TEST_PROXY_HEADERS_LOCK.lock().unwrap();
        std::env::remove_var(TRUST_PROXY_HEADERS_ENV);
        assert!(!trust_proxy_headers());
    }

    #[test]
    fn apply_trusted_proxy_addr_only_changes_addr_when_enabled() {
        let _guard = TEST_PROXY_HEADERS_LOCK.lock().unwrap();
        let original = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(10, 0, 0, 4)), 21117);
        let mut headers = HeaderMap::new();
        headers.insert("X-Forwarded-For", "198.51.100.10, 10.0.0.1".parse().unwrap());

        std::env::remove_var(TRUST_PROXY_HEADERS_ENV);
        assert_eq!(apply_trusted_proxy_addr(original, &headers), original);

        std::env::set_var(TRUST_PROXY_HEADERS_ENV, "Y");
        assert_eq!(
            apply_trusted_proxy_addr(original, &headers),
            SocketAddr::new(IpAddr::V4(Ipv4Addr::new(198, 51, 100, 10)), 0)
        );

        std::env::remove_var(TRUST_PROXY_HEADERS_ENV);
    }

    #[test]
    fn connection_rate_limiter_enforces_per_ip_window_and_exempts_loopback() {
        let _guard = TEST_PROXY_HEADERS_LOCK.lock().unwrap();
        CONNECTION_RATE_LIMITS.lock().unwrap().clear();
        PROTECTION_STATS.lock().unwrap().clear();
        std::env::set_var(MAX_CONN_PER_IP_PER_WINDOW_ENV, "2");
        std::env::set_var(CONN_RATE_WINDOW_SECONDS_ENV, "60");
        std::env::set_var(MAX_UDP_PACKETS_PER_IP_PER_WINDOW_ENV, "3");
        std::env::set_var(UDP_RATE_WINDOW_SECONDS_ENV, "60");

        let remote = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(198, 51, 100, 10)), 21117);
        assert!(allow_connection_from_ip("hbbs-main", remote));
        assert!(allow_connection_from_ip("hbbs-main", remote));
        assert!(!allow_connection_from_ip("hbbs-main", remote));
        assert!(allow_udp_packet_from_ip("hbbs-udp", remote));
        assert!(allow_udp_packet_from_ip("hbbs-udp", remote));
        assert!(allow_udp_packet_from_ip("hbbs-udp", remote));
        assert!(!allow_udp_packet_from_ip("hbbs-udp", remote));

        let loopback = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 21117);
        assert!(allow_connection_from_ip("hbbs-main", loopback));
        assert!(allow_connection_from_ip("hbbs-main", loopback));
        assert!(allow_connection_from_ip("hbbs-main", loopback));
        assert_eq!(
            protection_stats_snapshot(),
            vec![
                ("connection_rate_limit_hits".to_owned(), 1),
                ("udp_rate_limit_hits".to_owned(), 1),
            ]
        );
        record_protection_event("peer_records_pruned");
        assert_eq!(
            protection_stats_snapshot(),
            vec![
                ("connection_rate_limit_hits".to_owned(), 1),
                ("peer_records_pruned".to_owned(), 1),
                ("udp_rate_limit_hits".to_owned(), 1),
            ]
        );

        std::env::remove_var(MAX_CONN_PER_IP_PER_WINDOW_ENV);
        std::env::remove_var(CONN_RATE_WINDOW_SECONDS_ENV);
        std::env::remove_var(MAX_UDP_PACKETS_PER_IP_PER_WINDOW_ENV);
        std::env::remove_var(UDP_RATE_WINDOW_SECONDS_ENV);
        CONNECTION_RATE_LIMITS.lock().unwrap().clear();
        PROTECTION_STATS.lock().unwrap().clear();
        assert_eq!(max_conn_per_ip_per_window(), super::DEFAULT_MAX_CONN_PER_IP_PER_WINDOW);
        assert_eq!(conn_rate_window_seconds(), super::DEFAULT_CONN_RATE_WINDOW_SECONDS);
        assert_eq!(
            max_udp_packets_per_ip_per_window(),
            super::DEFAULT_MAX_UDP_PACKETS_PER_IP_PER_WINDOW
        );
        assert_eq!(
            udp_rate_window_seconds(),
            super::DEFAULT_UDP_RATE_WINDOW_SECONDS
        );
        assert_eq!(
            protection_limits_summary(),
            vec![
                "connections_per_ip_per_window=120/60s".to_owned(),
                "udp_packets_per_ip_per_window=240/60s".to_owned(),
                "trust_proxy_headers=false".to_owned(),
            ]
        );
    }
}
