use clap::App;
use hbb_common::{
    allow_err, anyhow::{Context, Result}, get_version_number, log, tokio, ResultType
};
use ini::Ini;
use sodiumoxide::crypto::sign;
use std::{
    io::prelude::*,
    io::Read,
    net::SocketAddr,
    time::{Instant, SystemTime},
};

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
#[derive(Default, Debug, PartialEq)]
struct DisabledSignals {
    hup: bool,
    term: bool,
    int: bool,
    quit: bool,
}

// opt-in via the DISABLE_SIGNALS env var, e.g. "hup", "term,int" or "all".
// a disabled signal is caught and swallowed so the process ignores it; the
// main use case is DISABLE_SIGNALS=hup to survive an ssh logout without nohup.
#[cfg(unix)]
fn parse_disabled_signals(s: &str) -> DisabledSignals {
    let mut d = DisabledSignals::default();
    for tok in s.split(',') {
        let t = tok.trim().to_lowercase();
        match t.strip_prefix("sig").unwrap_or(&t) {
            "" => {}
            "all" => d = DisabledSignals { hup: true, term: true, int: true, quit: true },
            "hup" => d.hup = true,
            "term" => d.term = true,
            "int" => d.int = true,
            "quit" => d.quit = true,
            other => log::warn!("ignoring unknown signal in DISABLE_SIGNALS: {other}"),
        }
    }
    d
}

#[cfg(unix)]
async fn recv(s: &mut Option<hbb_common::tokio::signal::unix::Signal>) {
    match s {
        Some(sig) => {
            sig.recv().await;
        }
        None => std::future::pending::<()>().await,
    }
}

#[cfg(unix)]
pub async fn listen_signal() -> Result<()> {
    use hbb_common::tokio;
    use hbb_common::tokio::signal::unix::{signal, SignalKind};

    let disabled = parse_disabled_signals(&std::env::var("DISABLE_SIGNALS").unwrap_or_default());

    tokio::spawn(async move {
        // SIGHUP is only ever registered when disabled, so by default it keeps
        // its terminate disposition and nothing here changes.
        let ignore = |kind: SignalKind, name: &'static str| match signal(kind) {
            Ok(mut s) => {
                log::info!("ignoring signal {name}");
                tokio::spawn(async move { while s.recv().await.is_some() {} });
            }
            Err(e) => log::warn!("failed to register ignore handler for signal {name}: {e}"),
        };
        if disabled.hup {
            ignore(SignalKind::hangup(), "hup");
        }
        if disabled.term {
            ignore(SignalKind::terminate(), "term");
        }
        if disabled.int {
            ignore(SignalKind::interrupt(), "int");
        }
        if disabled.quit {
            ignore(SignalKind::quit(), "quit");
        }

        let mut term = if disabled.term { None } else { Some(signal(SignalKind::terminate())?) };
        let mut int = if disabled.int { None } else { Some(signal(SignalKind::interrupt())?) };
        let mut quit = if disabled.quit { None } else { Some(signal(SignalKind::quit())?) };

        // with every graceful signal disabled all three branches pend forever,
        // so the server only stops on SIGKILL.
        tokio::select! {
            _ = recv(&mut term) => log::info!("signal terminate"),
            _ = recv(&mut int) => log::info!("signal interrupt"),
            _ = recv(&mut quit) => log::info!("signal quit"),
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

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    #[test]
    fn parse_signals() {
        assert_eq!(parse_disabled_signals(""), DisabledSignals::default());
        assert_eq!(
            parse_disabled_signals("hup"),
            DisabledSignals { hup: true, ..Default::default() }
        );
        assert_eq!(
            parse_disabled_signals(" SIGTERM , int "),
            DisabledSignals { term: true, int: true, ..Default::default() }
        );
        assert_eq!(
            parse_disabled_signals("all"),
            DisabledSignals { hup: true, term: true, int: true, quit: true }
        );
        assert_eq!(
            parse_disabled_signals("bogus,quit"),
            DisabledSignals { quit: true, ..Default::default() }
        );
    }
}