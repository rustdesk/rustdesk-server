use clap::App;
use hbb_common::{
    allow_err, anyhow::{Context, Result}, get_version_number, log, tokio, ResultType
};
use ini::Ini;
use sodiumoxide::crypto::sign;
use std::{
    fs,
    io::prelude::*,
    io::Read,
    net::{IpAddr, Ipv4Addr, SocketAddr},
    path::{Path, PathBuf},
    time::{Instant, SystemTime},
};

pub fn parse_bind_address(value: &str) -> Result<Option<IpAddr>> {
    let value = value.trim();
    if value.is_empty() {
        Ok(None)
    } else {
        value
            .parse()
            .with_context(|| format!("Invalid bind address: {value}"))
            .map(Some)
    }
}

pub async fn listen_tcp(
    bind_addr: Option<IpAddr>,
    port: u16,
) -> ResultType<hbb_common::tokio::net::TcpListener> {
    if let Some(bind_addr) = bind_addr {
        hbb_common::tcp::new_listener(SocketAddr::new(bind_addr, port), true).await
    } else {
        hbb_common::tcp::listen_any(port).await
    }
}

pub fn console_addr(bind_addr: Option<IpAddr>, port: u16) -> Option<SocketAddr> {
    let bind_addr = bind_addr?;
    if bind_addr.is_unspecified() || bind_addr == IpAddr::V4(Ipv4Addr::LOCALHOST) {
        return None;
    }
    Some(SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), port))
}

// The runtime console (check_cmd) is reached via 127.0.0.1, so when the bind
// address does not already accept connections to 127.0.0.1 (it is neither the
// any-address nor 127.0.0.1 itself), the console gets a dedicated listener
// there; it is never bound to the external bind address.
pub async fn listen_console(
    bind_addr: Option<IpAddr>,
    port: u16,
) -> ResultType<Option<hbb_common::tokio::net::TcpListener>> {
    match console_addr(bind_addr, port) {
        Some(addr) => {
            let listener = hbb_common::tcp::new_listener(addr, true).await?;
            log::info!("Listening on tcp {} for the console", addr);
            Ok(Some(listener))
        }
        None => Ok(None),
    }
}

pub async fn accept_or_pending(
    listener: Option<&hbb_common::tokio::net::TcpListener>,
) -> std::io::Result<(hbb_common::tokio::net::TcpStream, SocketAddr)> {
    match listener {
        Some(listener) => listener.accept().await,
        None => std::future::pending().await,
    }
}

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
#[inline]
pub fn set_arg(name: &str, value: &str) {
    std::env::set_var(arg_name(name), value);
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
                .for_each(|(k, v)| set_arg(k, v));
        }
    }
    if let Some(config) = matches.value_of("config") {
        if let Ok(v) = Ini::load_from_file(config) {
            if let Some(section) = v.section(None::<String>) {
                section
                    .iter()
                    .for_each(|(k, v)| set_arg(k, v));
            }
        }
    }
    for (k, v) in matches.args {
        if let Some(v) = v.vals.first() {
            set_arg(k, &v.to_string_lossy());
        }
    }
}

#[allow(dead_code)]
pub fn get_arg_opt(name: &str) -> Option<String> {
    let dashed = arg_name(name);
    let underscored = dashed.replace('-', "_");
    let lower_dashed = dashed.to_lowercase();
    let lower_underscored = underscored.to_lowercase();
    for alias in [&dashed, &underscored, &lower_dashed, &lower_underscored] {
        if let Ok(value) = std::env::var(alias) {
            return Some(value);
        }
    }
    let mut aliases = std::env::vars_os()
        .filter_map(|(key, value)| {
            let key = key.into_string().ok()?;
            if arg_name(&key) == dashed {
                Some((key, value.into_string().ok()?))
            } else {
                None
            }
        })
        .collect::<Vec<_>>();
    aliases.sort_by(|a, b| a.0.cmp(&b.0));
    aliases.into_iter().next().map(|(_, value)| value)
}

#[allow(dead_code)]
#[inline]
pub fn get_arg(name: &str) -> String {
    get_arg_or(name, "".to_owned())
}

#[allow(dead_code)]
#[inline]
pub fn get_arg_or(name: &str, default: String) -> String {
    get_arg_opt(name).unwrap_or(default)
}

#[allow(dead_code)]
#[inline]
pub fn now() -> u64 {
    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|x| x.as_secs())
        .unwrap_or_default()
}

#[cfg(unix)]
const PRIVATE_KEY_FILE_MODE: u32 = 0o600;
const MAX_KEY_SYMLINKS: usize = 40;

fn resolve_key_path(path: &Path) -> ResultType<PathBuf> {
    let mut path = path.to_path_buf();
    // Bound traversal even if symlinks change while they are being resolved.
    for _ in 0..MAX_KEY_SYMLINKS {
        match fs::symlink_metadata(&path) {
            Ok(metadata) if metadata.file_type().is_symlink() => {}
            Ok(_) => return Ok(path),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(path),
            Err(err) => return Err(err.into()),
        }
        let target = fs::read_link(&path)?;
        path.pop();
        path.push(target);
    }
    hbb_common::anyhow::bail!(
        "Too many key symlinks (limit: {MAX_KEY_SYMLINKS}): {}",
        path.display()
    );
}

/// On Unix, restricts access from creation so another user cannot retain a readable descriptor.
/// Follows dangling symlinks for configured key paths, but never overwrites an existing key.
/// Writes after verifying Unix permissions; failures close and try to remove the new file.
fn create_private_key_file(path: &str, contents: &[u8]) -> ResultType<()> {
    #[cfg(unix)]
    use std::os::unix::fs::OpenOptionsExt;

    let path = resolve_key_path(Path::new(path))?;
    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    options.mode(PRIVATE_KEY_FILE_MODE);
    let mut file = options.open(&path)?;
    let result = (|| {
        #[cfg(unix)]
        set_private_key_permissions(&file, true)?;
        file.write_all(contents)
            .context("Failed to write private key")
    })();
    if let Err(err) = result {
        drop(file);
        fs::remove_file(&path)
            .with_context(|| format!("Failed to remove {} after {err:#}", path.display()))?;
        return Err(err);
    }
    Ok(())
}

#[cfg(unix)]
fn set_private_key_permissions(file: &fs::File, newly_created: bool) -> ResultType<()> {
    use std::os::unix::fs::PermissionsExt;

    const PERMISSION_BITS: u32 = 0o7777;
    let mode = file.metadata()?.permissions().mode() & PERMISSION_BITS;
    if !newly_created && mode & !PRIVATE_KEY_FILE_MODE == 0 {
        return Ok(());
    }
    hbb_common::anyhow::ensure!(
        !newly_created || mode & !PRIVATE_KEY_FILE_MODE == 0,
        "Unsafe initial private key permissions: {mode:04o}"
    );
    if mode != PRIVATE_KEY_FILE_MODE {
        file.set_permissions(fs::Permissions::from_mode(PRIVATE_KEY_FILE_MODE))
            .context("Failed to set private key permissions to 0600")?;
        let mode = file.metadata()?.permissions().mode() & PERMISSION_BITS;
        hbb_common::anyhow::ensure!(
            mode == PRIVATE_KEY_FILE_MODE,
            "Private key permissions are {mode:04o}, expected 0600"
        );
    }
    Ok(())
}

fn missing_public_key_path(path: &Path, pk: &str) -> ResultType<Option<PathBuf>> {
    let path = resolve_key_path(path).context("Failed to resolve public key path")?;
    match fs::read_to_string(&path) {
        Ok(contents) if contents.trim() == pk => Ok(None),
        Ok(_) => {
            let err = hbb_common::anyhow::anyhow!(
                "Public key in {} does not match private key in id_ed25519",
                path.display()
            );
            log::error!("{err}");
            Err(err)
        }
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(Some(path)),
        Err(err) => Err(err).context("Failed to read public key file"),
    }
}

fn publish_public_key(file: tempfile::TempPath, path: &Path, pk: &str) -> ResultType<()> {
    match file.persist_noclobber(path) {
        Ok(_) => Ok(()),
        Err(err) if err.error.kind() == std::io::ErrorKind::AlreadyExists => {
            hbb_common::anyhow::ensure!(
                missing_public_key_path(path, pk)?.is_none(),
                "Public key disappeared during publication"
            );
            Ok(())
        }
        Err(err) => Err(err.error).context("Failed to publish public key file"),
    }
}

/// Publishes a fully written temporary file so interrupted writes cannot expose partial public keys.
/// Preserves existing files: matching public keys are accepted, while mismatches return errors.
/// With `required = false`, temp-file creation failure only warns to support read-only provisioning.
fn write_public_key(path: &str, pk: &str, required: bool) -> ResultType<()> {
    let Some(path) = missing_public_key_path(Path::new(path), pk)? else {
        return Ok(());
    };
    let temporary = path.with_file_name(format!(".id_ed25519.pub.{}", uuid::Uuid::new_v4()));
    let mut file = match fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temporary)
    {
        Ok(file) => file,
        // Existing private keys may be provisioned in a read-only directory.
        Err(err) if !required => {
            log::warn!(
                "Failed to create {}: {err}; using the existing private key",
                path.display()
            );
            return Ok(());
        }
        Err(err) => return Err(err).context("Failed to create public key file"),
    };
    let temporary = tempfile::TempPath::from_path(temporary);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        const OWNER_READ_WRITE: u32 = 0o600;
        let mode = file.metadata()?.permissions().mode();
        if mode & OWNER_READ_WRITE != OWNER_READ_WRITE {
            file.set_permissions(fs::Permissions::from_mode(mode | OWNER_READ_WRITE))
                .context("Failed to set public key owner permissions")?;
        }
    }
    file.write_all(pk.as_bytes())
        .context("Failed to write public key")?;
    file.sync_all().context("Failed to sync public key")?;
    drop(file);
    publish_public_key(temporary, &path, pk)
}

pub fn gen_sk(wait: u64) -> ResultType<(String, Option<sign::SecretKey>)> {
    let sk_file = "id_ed25519";
    let pub_file = format!("{sk_file}.pub");
    if wait > 0 && !std::path::Path::new(sk_file).exists() {
        std::thread::sleep(std::time::Duration::from_millis(wait));
    }
    match fs::File::open(sk_file) {
        Ok(mut file) => {
            #[cfg(unix)]
            set_private_key_permissions(&file, false)?;
            let mut contents = String::new();
            file.read_to_string(&mut contents)
                .context("Failed to read private key")?;
            let sk = base64::decode(contents.trim()).context("Malformed private key")?;
            let sk = sign::SecretKey::from_slice(&sk).context("Malformed private key")?;
            let pk = base64::encode(sk.public_key());
            write_public_key(&pub_file, &pk, false)?;
            log::info!("Private key comes from {}", sk_file);
            return Ok((pk, Some(sk)));
        }
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
        Err(err) => return Err(err).context("Failed to open private key"),
    }
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
    missing_public_key_path(Path::new(&pub_file), &pk)?;
    create_private_key_file(sk_file, base64::encode(&sk).as_bytes())
        .context("Failed to create private key file")?;
    write_public_key(&pub_file, &pk, true)?;
    log::info!("Private/public key written to {}/{}", sk_file, pub_file);
    log::debug!("Public key: {}", pk);
    Ok((pk, Some(sk)))
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
    use super::*;
    use std::net::{Ipv4Addr, Ipv6Addr};

    #[test]
    fn argument_names_ignore_case_and_separator() {
        let aliases = [
            "RUSTDESK-CONFIG-ALIAS-TEST",
            "RUSTDESK_CONFIG_ALIAS_TEST",
            "rustdesk-config-alias-test",
            "rustdesk_config_alias_test",
            "RustDesk_Config-Alias_Test",
        ];
        for alias in aliases {
            std::env::remove_var(alias);
        }
        for alias in aliases {
            std::env::set_var(alias, alias);
            assert_eq!(get_arg("RUSTDESK_CONFIG_ALIAS_TEST"), alias);
            std::env::remove_var(alias);
        }
        set_arg("rustdesk_config_alias_test", "normalized");
        assert_eq!(
            std::env::var("RUSTDESK-CONFIG-ALIAS-TEST").unwrap(),
            "normalized"
        );
        std::env::set_var("RUSTDESK_CONFIG_ALIAS_TEST", "inherited");
        set_arg("rustdesk-config-alias-test", "higher-priority");
        assert_eq!(get_arg("rustdesk_config_alias_test"), "higher-priority");
        std::env::remove_var("RUSTDESK-CONFIG-ALIAS-TEST");
        std::env::remove_var("RUSTDESK_CONFIG_ALIAS_TEST");
    }

    #[test]
    fn parses_bind_address() {
        assert_eq!(parse_bind_address("").unwrap(), None);
        assert_eq!(
            parse_bind_address("127.0.0.1").unwrap(),
            Some(IpAddr::V4(Ipv4Addr::LOCALHOST))
        );
        assert_eq!(
            parse_bind_address("::1").unwrap(),
            Some(IpAddr::V6(Ipv6Addr::LOCALHOST))
        );
        assert!(parse_bind_address("not-an-ip").is_err());
    }

    #[hbb_common::tokio::test]
    async fn tcp_listener_uses_bind_address() {
        let bind_addr = IpAddr::V4(Ipv4Addr::LOCALHOST);
        let listener = listen_tcp(Some(bind_addr), 0).await.unwrap();
        assert_eq!(listener.local_addr().unwrap().ip(), bind_addr);
    }

    #[test]
    fn console_addr_only_when_bind_does_not_cover_ipv4_localhost() {
        for bind_addr in [
            None,
            Some(IpAddr::V4(Ipv4Addr::UNSPECIFIED)),
            Some(IpAddr::V6(Ipv6Addr::UNSPECIFIED)),
            Some(IpAddr::V4(Ipv4Addr::LOCALHOST)),
        ] {
            assert_eq!(console_addr(bind_addr, 21117), None);
        }
        for bind_addr in [
            Some(IpAddr::V4(Ipv4Addr::new(192, 0, 2, 1))),
            Some("2001:db8::1".parse().unwrap()),
            Some(IpAddr::V6(Ipv6Addr::LOCALHOST)),
        ] {
            assert_eq!(
                console_addr(bind_addr, 21117),
                Some(SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 21117))
            );
        }
    }

    #[hbb_common::tokio::test]
    async fn console_listener_binds_ipv4_localhost() {
        let listener = listen_console(Some(IpAddr::V4(Ipv4Addr::new(192, 0, 2, 1))), 0)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            listener.local_addr().unwrap().ip(),
            IpAddr::V4(Ipv4Addr::LOCALHOST)
        );
        assert!(listen_console(None, 0).await.unwrap().is_none());
        assert!(listen_console(Some(IpAddr::V4(Ipv4Addr::LOCALHOST)), 0)
            .await
            .unwrap()
            .is_none());
    }

    #[cfg(unix)]
    #[test]
    fn key_path_symlinks_are_bounded() {
        use std::os::unix::fs::symlink;

        let directory = tempfile::tempdir().unwrap();
        let target = directory.path().join("target");
        let mut path = target.clone();
        for index in 1..MAX_KEY_SYMLINKS {
            let link = directory.path().join(format!("link-{index}"));
            symlink(&path, &link).unwrap();
            path = link;
        }
        assert_eq!(resolve_key_path(&path).unwrap(), target);
        let extra = directory.path().join("extra");
        symlink(&path, &extra).unwrap();
        let cycle = directory.path().join("cycle");
        symlink(&cycle, &cycle).unwrap();
        for path in [&extra, &cycle] {
            let error = resolve_key_path(path).unwrap_err();
            assert!(error.to_string().starts_with("Too many key symlinks"));
        }
    }

    #[cfg(unix)]
    #[test]
    fn private_key_file_permissions() {
        use std::process::Command;

        const CHILD_ENV: &str = "RUSTDESK_PRIVATE_KEY_TEST_CHILD";
        const CHILD_COMPLETED: &str = "private_key_test_child_completed";
        const TEST_NAME: &str = "common::tests::private_key_file_permissions";
        if std::env::var_os(CHILD_ENV).is_some() {
            const TEST_UMASK: hbb_common::libc::mode_t = 0o022;
            // The child process isolates the umask from other tests.
            unsafe { hbb_common::libc::umask(TEST_UMASK) };
            assert_private_key_file_permissions();
            println!("{CHILD_COMPLETED}");
            return;
        }

        let output = Command::new(std::env::current_exe().unwrap())
            .args(["--exact", TEST_NAME, "--nocapture"])
            .env(CHILD_ENV, "1")
            .output()
            .unwrap();
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(
            output.status.success() && stdout.contains(CHILD_COMPLETED),
            "key-file test failed ({}):\n{stdout}\n{}",
            output.status,
            String::from_utf8_lossy(&output.stderr)
        );
    }

    #[cfg(target_os = "linux")]
    fn with_file_size_limit<T>(limit: hbb_common::libc::rlim_t, run: impl FnOnce() -> T) -> T {
        use hbb_common::libc;

        unsafe {
            let mut limits = std::mem::zeroed();
            assert_eq!(libc::getrlimit(libc::RLIMIT_FSIZE, &mut limits), 0);
            let handler = libc::signal(libc::SIGXFSZ, libc::SIG_IGN);
            assert_ne!(handler, libc::SIG_ERR);
            let restricted = libc::rlimit {
                rlim_cur: limit,
                ..limits
            };
            assert_eq!(libc::setrlimit(libc::RLIMIT_FSIZE, &restricted), 0);
            let result = run();
            assert_eq!(libc::setrlimit(libc::RLIMIT_FSIZE, &limits), 0);
            assert_ne!(libc::signal(libc::SIGXFSZ, handler), libc::SIG_ERR);
            result
        }
    }

    #[cfg(target_os = "linux")]
    fn assert_public_key_write_failure(public_key: &str) {
        const WRITE_LIMIT: hbb_common::libc::rlim_t = 16;
        let pub_file = "id_ed25519.pub";
        fs::remove_file(pub_file).unwrap();
        assert!(with_file_size_limit(WRITE_LIMIT, || gen_sk(0)).is_err());
        assert!(!Path::new(pub_file).exists());
        assert_eq!(gen_sk(0).unwrap().0, public_key);
        assert_eq!(fs::read_to_string(pub_file).unwrap(), public_key);
    }

    #[cfg(unix)]
    fn assert_public_key_recovery() -> String {
        use std::os::unix::fs::MetadataExt;

        let sk_file = "id_ed25519";
        let pub_file = "id_ed25519.pub";
        let mismatched_key = base64::encode(sign::gen_keypair().0);
        fs::write(pub_file, &mismatched_key).unwrap();
        assert!(gen_sk(0).is_err());
        assert!(!Path::new(sk_file).exists());
        assert_eq!(fs::read_to_string(pub_file).unwrap(), mismatched_key);
        fs::remove_file(pub_file).unwrap();
        let public_key = gen_sk(0).unwrap().0;
        let private_key = fs::read(sk_file).unwrap();
        fs::remove_file(pub_file).unwrap();
        fs::create_dir(pub_file).unwrap();
        assert!(gen_sk(0).is_err());
        fs::remove_dir(pub_file).unwrap();
        assert_eq!(gen_sk(0).unwrap().0, public_key);
        assert_eq!(fs::read_to_string(pub_file).unwrap(), public_key);
        #[cfg(target_os = "linux")]
        assert_public_key_write_failure(&public_key);
        let path = Path::new(pub_file);
        let prepare = |pk: &str| {
            let mut file = tempfile::NamedTempFile::new_in(".").unwrap();
            file.write_all(pk.as_bytes()).unwrap();
            file.into_temp_path()
        };
        fs::remove_file(path).unwrap();
        let pending = prepare(&public_key);
        gen_sk(0).unwrap();
        let inode = fs::metadata(path).unwrap().ino();
        publish_public_key(pending, path, &public_key).unwrap();
        assert!(publish_public_key(prepare(&mismatched_key), path, &mismatched_key).is_err());
        assert_eq!(fs::metadata(path).unwrap().ino(), inode);
        assert_eq!(fs::read_to_string(path).unwrap(), public_key);
        fs::write(pub_file, &mismatched_key).unwrap();
        assert!(gen_sk(0).is_err());
        assert!(fs::read(sk_file).unwrap() == private_key);
        assert_eq!(fs::read_to_string(pub_file).unwrap(), mismatched_key);
        fs::write(pub_file, &public_key).unwrap();
        public_key
    }

    #[cfg(unix)]
    fn assert_private_key_file_permissions() {
        use std::os::unix::fs::{symlink, PermissionsExt};

        const PERMISSION_BITS: u32 = 0o777;
        const NEW_FILE_MODE: u32 = 0o600;
        const EXISTING_MODE: u32 = 0o644;
        const READ_ONLY_MODE: u32 = 0o400;
        const RESTRICTIVE_UMASK: hbb_common::libc::mode_t = 0o777;
        let directory = tempfile::tempdir().unwrap();
        std::env::set_current_dir(directory.path()).unwrap();
        let path = Path::new("id_ed25519");
        let mode = || fs::metadata(path).unwrap().permissions().mode() & PERMISSION_BITS;
        symlink("key-alias", path).unwrap();
        symlink("key-target", "key-alias").unwrap();
        #[cfg(target_os = "linux")]
        {
            const PARTIAL_WRITE_LIMIT: hbb_common::libc::rlim_t = 16;
            for limit in [0, PARTIAL_WRITE_LIMIT] {
                assert!(with_file_size_limit(limit, || gen_sk(0)).is_err());
                assert!(path.is_symlink() && Path::new("key-alias").is_symlink());
                assert!(!Path::new("key-target").exists());
                assert!(!Path::new("id_ed25519.pub").exists());
            }
        }
        let public_key = assert_public_key_recovery();
        assert_eq!(mode(), NEW_FILE_MODE);
        let contents = fs::read(path).unwrap();
        fs::set_permissions(path, fs::Permissions::from_mode(EXISTING_MODE)).unwrap();
        let file = fs::File::open(path).unwrap();
        assert!(set_private_key_permissions(&file, true).is_err());
        assert_eq!(mode(), EXISTING_MODE);
        gen_sk(0).unwrap();
        assert_eq!(mode(), NEW_FILE_MODE);
        assert_eq!(fs::read(path).unwrap(), contents);
        assert!(create_private_key_file("id_ed25519", b"do not overwrite").is_err());
        assert_eq!(fs::read(path).unwrap(), contents);
        fs::set_permissions(path, fs::Permissions::from_mode(READ_ONLY_MODE)).unwrap();
        fs::set_permissions("id_ed25519.pub", fs::Permissions::from_mode(READ_ONLY_MODE)).unwrap();
        assert_eq!(gen_sk(0).unwrap().0, public_key);
        assert_eq!(mode(), READ_ONLY_MODE);

        fs::remove_file(path).unwrap();
        fs::remove_file("id_ed25519.pub").unwrap();
        unsafe { hbb_common::libc::umask(RESTRICTIVE_UMASK) };
        gen_sk(0).unwrap();
        assert_eq!(mode(), NEW_FILE_MODE);
        let contents = fs::read(path).unwrap();
        gen_sk(0).unwrap();
        assert_eq!(fs::read(path).unwrap(), contents);
        std::env::set_current_dir(directory.path().parent().unwrap()).unwrap();
    }
}
