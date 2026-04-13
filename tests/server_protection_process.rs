use hbb_common::{
    anyhow::{bail, Context, Result},
    protobuf::Message,
    rendezvous_proto::{
        register_pk_response, rendezvous_message, OnlineRequest, RegisterPk, RegisterPkResponse,
        RendezvousMessage, RequestRelay,
    },
    tcp::FramedStream,
    udp::FramedSocket,
};
use sqlx::{Connection, Row, SqliteConnection};
use std::{
    fs,
    io::{Read, Write},
    net::{IpAddr, Ipv4Addr, SocketAddr, TcpListener, TcpStream, UdpSocket},
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    thread,
    time::{Duration, Instant, SystemTime},
};

struct ChildGuard {
    child: Child,
    temp_dir: PathBuf,
}

impl ChildGuard {
    fn try_wait(&mut self) -> Result<Option<std::process::ExitStatus>> {
        Ok(self.child.try_wait()?)
    }
}

impl Drop for ChildGuard {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = fs::remove_dir_all(&self.temp_dir);
    }
}

fn unique_temp_dir(prefix: &str) -> Result<PathBuf> {
    let mut path = std::env::temp_dir();
    let stamp = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    path.push(format!("{prefix}-{}-{stamp}", std::process::id()));
    fs::create_dir_all(&path)?;
    Ok(path)
}

fn reserve_hbbs_port() -> Result<u16> {
    for _ in 0..64 {
        let main = TcpListener::bind((Ipv4Addr::UNSPECIFIED, 0))?;
        let port = main.local_addr()?.port();
        if port <= 1024 || port >= (u16::MAX - 2) {
            continue;
        }
        if TcpListener::bind((Ipv4Addr::UNSPECIFIED, port - 1)).is_ok()
            && TcpListener::bind((Ipv4Addr::UNSPECIFIED, port + 2)).is_ok()
            && UdpSocket::bind((Ipv4Addr::UNSPECIFIED, port)).is_ok()
        {
            return Ok(port);
        }
    }
    bail!("failed to reserve hbbs port triplet");
}

fn reserve_hbbr_port() -> Result<u16> {
    for _ in 0..64 {
        let main = TcpListener::bind((Ipv4Addr::UNSPECIFIED, 0))?;
        let port = main.local_addr()?.port();
        if port >= (u16::MAX - 2) {
            continue;
        }
        if TcpListener::bind((Ipv4Addr::UNSPECIFIED, port + 2)).is_ok() {
            return Ok(port);
        }
    }
    bail!("failed to reserve hbbr port pair");
}

fn wait_for_tcp_ready(child: &mut ChildGuard, addr: SocketAddr) -> Result<()> {
    let deadline = Instant::now() + Duration::from_secs(10);
    while Instant::now() < deadline {
        match TcpStream::connect_timeout(&addr, Duration::from_millis(200)) {
            Ok(_) => return Ok(()),
            Err(_) => {
                if let Some(status) = child.try_wait()? {
                    bail!("server exited before becoming ready: {status}");
                }
                thread::sleep(Duration::from_millis(100));
            }
        }
    }
    bail!("timed out waiting for {addr} to accept connections");
}

fn find_bin_path(bin_env: &str, bin_name: &str) -> Result<PathBuf> {
    if let Some(path) = std::env::var_os(bin_env) {
        return Ok(PathBuf::from(path));
    }
    let mut path = std::env::current_exe()?;
    path.pop();
    path.pop();
    path.push(bin_name);
    #[cfg(windows)]
    path.set_extension("exe");
    if path.is_file() {
        return Ok(path);
    }
    bail!("missing cargo bin env and fallback binary path for {bin_name}");
}

fn spawn_server(
    bin_env: &str,
    bin_name: &str,
    args: &[String],
    current_dir: &Path,
    envs: &[(&str, &str)],
) -> Result<ChildGuard> {
    let bin = find_bin_path(bin_env, bin_name)?;
    let mut command = Command::new(bin);
    command
        .current_dir(current_dir)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    for (key, value) in envs {
        command.env(key, value);
    }
    let child = command.spawn()?;
    Ok(ChildGuard {
        child,
        temp_dir: current_dir.to_path_buf(),
    })
}

fn spawn_hbbs(port: u16, key: &str) -> Result<ChildGuard> {
    let temp_dir = unique_temp_dir("hbbs-process-test")?;
    spawn_hbbs_in_dir(port, key, temp_dir, &[])
}

fn spawn_hbbs_in_dir(
    port: u16,
    key: &str,
    temp_dir: PathBuf,
    envs: &[(&str, &str)],
) -> Result<ChildGuard> {
    let args = vec![
        "--port".to_owned(),
        port.to_string(),
        "--key".to_owned(),
        key.to_owned(),
    ];
    let mut child_envs = vec![("TEST_HBBS", "no")];
    child_envs.extend_from_slice(envs);
    let mut child = spawn_server(
        "CARGO_BIN_EXE_hbbs",
        "hbbs",
        &args,
        &temp_dir,
        &child_envs,
    )?;
    wait_for_tcp_ready(
        &mut child,
        SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), port - 1),
    )?;
    Ok(child)
}

fn spawn_hbbr(port: u16, key: &str) -> Result<ChildGuard> {
    let temp_dir = unique_temp_dir("hbbr-process-test")?;
    let args = vec![
        "--port".to_owned(),
        port.to_string(),
        "--key".to_owned(),
        key.to_owned(),
    ];
    let mut child = spawn_server("CARGO_BIN_EXE_hbbr", "hbbr", &args, &temp_dir, &[])?;
    wait_for_tcp_ready(
        &mut child,
        SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), port),
    )?;
    Ok(child)
}

fn admin_command(addr: SocketAddr, command: &str) -> Result<String> {
    let mut stream = TcpStream::connect_timeout(&addr, Duration::from_secs(2))?;
    stream.set_read_timeout(Some(Duration::from_secs(2)))?;
    stream.write_all(command.as_bytes())?;
    stream.shutdown(std::net::Shutdown::Write)?;
    let mut out = String::new();
    stream.read_to_string(&mut out)?;
    Ok(out)
}

fn non_loopback_local_ip() -> Option<IpAddr> {
    match local_ip_address::local_ip() {
        Ok(ip) if !ip.is_loopback() && !ip.is_unspecified() => Some(ip),
        _ => None,
    }
}

async fn send_online_request(addr: SocketAddr, key: &str) -> Result<Option<RendezvousMessage>> {
    let mut stream = FramedStream::new(addr, None, 1_500).await?;
    let mut message = RendezvousMessage::new();
    message.set_online_request(OnlineRequest {
        licence_key: key.to_owned(),
        ..Default::default()
    });
    stream.send(&message).await?;
    let response = match stream.next_timeout(1_000).await {
        Some(Ok(bytes)) => Some(RendezvousMessage::parse_from_bytes(&bytes)?),
        Some(Err(err)) => return Err(err.into()),
        None => None,
    };
    Ok(response)
}

async fn send_register_pk(addr: SocketAddr, id: &str, key: &str) -> Result<Option<RendezvousMessage>> {
    let mut socket = FramedSocket::new((Ipv4Addr::UNSPECIFIED, 0)).await?;
    let mut message = RendezvousMessage::new();
    message.set_register_pk(RegisterPk {
        id: id.to_owned(),
        uuid: vec![1, 2, 3, 4].into(),
        pk: vec![9, 8, 7, 6].into(),
        licence_key: key.to_owned(),
        ..Default::default()
    });
    socket.send(&message, addr).await?;
    let response = match socket.next_timeout(1_000).await {
        Some(Ok((bytes, _))) => Some(RendezvousMessage::parse_from_bytes(&bytes)?),
        Some(Err(err)) => return Err(err),
        None => None,
    };
    Ok(response)
}

async fn send_relay_request(addr: SocketAddr, key: &str) -> Result<Option<RendezvousMessage>> {
    let mut stream = FramedStream::new(addr, None, 1_500).await?;
    let mut message = RendezvousMessage::new();
    message.set_request_relay(RequestRelay {
        id: "peer-a".to_owned(),
        uuid: "relay-test-uuid".to_owned(),
        licence_key: key.to_owned(),
        ..Default::default()
    });
    stream.send(&message).await?;
    let response = match stream.next_timeout(1_000).await {
        Some(Ok(bytes)) => Some(RendezvousMessage::parse_from_bytes(&bytes)?),
        Some(Err(err)) => return Err(err.into()),
        None => None,
    };
    Ok(response)
}

fn runtime() -> Result<hbb_common::tokio::runtime::Runtime> {
    Ok(hbb_common::tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()?)
}

fn temp_db_path(dir: &Path) -> PathBuf {
    dir.join("db_v2.sqlite3")
}

async fn create_peer_schema_and_insert_stale_row(db_path: &Path, id: &str) -> Result<()> {
    if !db_path.exists() {
        fs::File::create(db_path)?;
    }
    let mut conn = SqliteConnection::connect(db_path.to_string_lossy().as_ref()).await?;
    sqlx::query(
        "
        create table if not exists peer (
            guid blob primary key not null,
            id varchar(100) not null,
            uuid blob not null,
            pk blob not null,
            created_at datetime not null default(current_timestamp),
            user blob,
            status tinyint,
            note varchar(300),
            info text not null
        ) without rowid;
        ",
    )
    .execute(&mut conn)
    .await?;
    sqlx::query("create unique index if not exists index_peer_id on peer (id);")
        .execute(&mut conn)
        .await?;
    sqlx::query("create index if not exists index_peer_created_at on peer (created_at);")
        .execute(&mut conn)
        .await?;
    sqlx::query(
        "insert into peer(guid, id, uuid, pk, created_at, info) values(?, ?, ?, ?, datetime('now', '-2 days'), ?)",
    )
    .bind(vec![0u8; 16])
    .bind(id)
    .bind(vec![1u8; 4])
    .bind(vec![2u8; 4])
    .bind("")
    .execute(&mut conn)
    .await?;
    Ok(())
}

async fn peer_exists(db_path: &Path, id: &str) -> Result<bool> {
    let mut conn = SqliteConnection::connect(db_path.to_string_lossy().as_ref()).await?;
    let row = sqlx::query("select count(*) from peer where id = ?")
        .bind(id)
        .fetch_one(&mut conn)
        .await?;
    let count: i64 = row.try_get(0)?;
    Ok(count > 0)
}

#[test]
fn hbbs_admin_protection_stats_reports_limits() -> Result<()> {
    let port = reserve_hbbs_port()?;
    let _child = spawn_hbbs(port, "server-key")?;
    let output = admin_command(
        SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), port - 1),
        "ps",
    )?;
    assert!(output.contains("connections_per_ip_per_window="));
    assert!(output.contains("udp_packets_per_ip_per_window="));
    assert!(output.contains("trust_proxy_headers="));
    Ok(())
}

#[test]
fn hbbr_admin_protection_stats_reports_limits() -> Result<()> {
    let port = reserve_hbbr_port()?;
    let _child = spawn_hbbr(port, "server-key")?;
    let output = admin_command(
        SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), port),
        "ps",
    )?;
    assert!(output.contains("connections_per_ip_per_window="));
    assert!(output.contains("udp_packets_per_ip_per_window="));
    assert!(output.contains("trust_proxy_headers="));
    Ok(())
}

#[test]
fn hbbs_online_request_requires_configured_key_process() -> Result<()> {
    let Some(ip) = non_loopback_local_ip() else {
        eprintln!("skipping non-loopback hbbs auth process test: no non-loopback local IP");
        return Ok(());
    };
    let port = reserve_hbbs_port()?;
    let _child = spawn_hbbs(port, "server-key")?;
    let addr = SocketAddr::new(ip, port - 1);

    let runtime = runtime()?;
    runtime.block_on(async move {
        let unauthorized = send_online_request(addr, "").await?;
        assert!(
            unauthorized.is_none(),
            "unauthorized online request unexpectedly received a response"
        );

        let authorized = send_online_request(addr, "server-key")
            .await?
            .context("authorized online request should receive a response")?;
        match authorized.union {
            Some(rendezvous_message::Union::OnlineResponse(response)) => {
                assert!(response.states.is_empty());
            }
            other => bail!("unexpected response to authorized online request: {other:?}"),
        }
        Ok::<(), hbb_common::anyhow::Error>(())
    })?;
    Ok(())
}

#[test]
fn hbbs_register_pk_requires_configured_key_process() -> Result<()> {
    let port = reserve_hbbs_port()?;
    let _child = spawn_hbbs(port, "server-key")?;
    let addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), port);
    let runtime = runtime()?;
    runtime.block_on(async move {
        let unauthorized = send_register_pk(addr, "peeruna1", "").await?
            .context("unauthorized register_pk should receive a response")?;
        match unauthorized.union {
            Some(rendezvous_message::Union::RegisterPkResponse(RegisterPkResponse { result, .. })) => {
                assert_eq!(result, register_pk_response::Result::LICENSE_MISMATCH.into());
            }
            other => bail!("unexpected response to unauthorized register_pk: {other:?}"),
        }

        let authorized = send_register_pk(addr, "peerauth1", "server-key").await?
            .context("authorized register_pk should receive a response")?;
        match authorized.union {
            Some(rendezvous_message::Union::RegisterPkResponse(RegisterPkResponse { result, .. })) => {
                assert_eq!(result, register_pk_response::Result::OK.into());
            }
            other => bail!("unexpected response to authorized register_pk: {other:?}"),
        }
        Ok::<(), hbb_common::anyhow::Error>(())
    })?;
    Ok(())
}

#[test]
fn hbbr_request_relay_reports_key_mismatch_process() -> Result<()> {
    let Some(ip) = non_loopback_local_ip() else {
        eprintln!("skipping non-loopback hbbr auth process test: no non-loopback local IP");
        return Ok(());
    };
    let port = reserve_hbbr_port()?;
    let _child = spawn_hbbr(port, "server-key")?;
    let addr = SocketAddr::new(ip, port);
    let runtime = runtime()?;
    runtime.block_on(async move {
        let response = send_relay_request(addr, "").await?
            .context("relay key mismatch should receive a refusal response")?;
        match response.union {
            Some(rendezvous_message::Union::RelayResponse(response)) => {
                assert_eq!(response.refuse_reason, "Key mismatch");
            }
            other => bail!("unexpected response to unauthorized relay request: {other:?}"),
        }
        Ok::<(), hbb_common::anyhow::Error>(())
    })?;
    Ok(())
}

#[test]
fn hbbs_register_pk_reports_peer_limit_reached_process() -> Result<()> {
    let port = reserve_hbbs_port()?;
    let temp_dir = unique_temp_dir("hbbs-peer-limit-process-test")?;
    let db_path = temp_db_path(&temp_dir);
    let db_url = db_path.to_string_lossy().to_string();
    let _child = spawn_hbbs_in_dir(
        port,
        "server-key",
        temp_dir,
        &[("DB_URL", db_url.as_str()), ("MAX_TOTAL_PEER_RECORDS", "1"), ("PEER_RECORD_RETENTION_DAYS", "3650")],
    )?;
    let addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), port);
    let runtime = runtime()?;
    runtime.block_on(async move {
        let first = send_register_pk(addr, "limitpeer1", "server-key").await?
            .context("first register_pk should receive a response")?;
        match first.union {
            Some(rendezvous_message::Union::RegisterPkResponse(RegisterPkResponse { result, .. })) => {
                assert_eq!(result, register_pk_response::Result::OK.into());
            }
            other => bail!("unexpected first register_pk response: {other:?}"),
        }

        let second = send_register_pk(addr, "limitpeer2", "server-key").await?
            .context("second register_pk should receive a response")?;
        match second.union {
            Some(rendezvous_message::Union::RegisterPkResponse(RegisterPkResponse { result, .. })) => {
                assert_eq!(result, register_pk_response::Result::PEER_LIMIT_REACHED.into());
            }
            other => bail!("unexpected second register_pk response: {other:?}"),
        }
        Ok::<(), hbb_common::anyhow::Error>(())
    })?;
    Ok(())
}

#[test]
fn hbbs_register_pk_prunes_stale_peer_before_enforcing_cap_process() -> Result<()> {
    let port = reserve_hbbs_port()?;
    let temp_dir = unique_temp_dir("hbbs-peer-retention-process-test")?;
    let db_path = temp_db_path(&temp_dir);
    let runtime = runtime()?;
    runtime.block_on(create_peer_schema_and_insert_stale_row(&db_path, "oldpeer1"))?;
    let db_url = db_path.to_string_lossy().to_string();
    let _child = spawn_hbbs_in_dir(
        port,
        "server-key",
        temp_dir,
        &[("DB_URL", db_url.as_str()), ("MAX_TOTAL_PEER_RECORDS", "1"), ("PEER_RECORD_RETENTION_DAYS", "1")],
    )?;
    let addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), port);
    runtime.block_on(async move {
        let response = send_register_pk(addr, "newpeer1", "server-key").await?
            .context("register_pk should receive a response")?;
        match response.union {
            Some(rendezvous_message::Union::RegisterPkResponse(RegisterPkResponse { result, .. })) => {
                assert_eq!(result, register_pk_response::Result::OK.into());
            }
            other => bail!("unexpected register_pk response after stale prune: {other:?}"),
        }
        Ok::<(), hbb_common::anyhow::Error>(())
    })?;
    assert!(!runtime.block_on(peer_exists(&db_path, "oldpeer1"))?);
    assert!(runtime.block_on(peer_exists(&db_path, "newpeer1"))?);
    Ok(())
}
