// Shared helpers for the protocol integration tests: start a real hbbs on free
// ports and talk to it like the RustDesk client 1.5.0 (and older) does.
#![allow(dead_code)]

use hbb_common::{
    bytes::{Bytes, BytesMut},
    protobuf::Message as _,
    rendezvous_proto::*,
    tcp::{FramedStream, KxTranscript, KX_PARAMS_DOMAIN},
    tokio::{
        net::UdpSocket,
        time::{sleep, timeout, Duration, Instant},
    },
    AddrMangle,
};
use sodiumoxide::crypto::{box_, secretbox, sign};
use std::{
    net::{IpAddr, Ipv4Addr, SocketAddr, TcpListener},
    process::{Child, Command, Stdio},
};

pub const CONTROLLER_IP: IpAddr = IpAddr::V4(Ipv4Addr::new(127, 0, 0, 2));
pub const PEER_IP: IpAddr = IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1));

pub struct Hbbs {
    child: Child,
    _port: PortLock,
    pub dir: tempfile::TempDir,
    pub port: u16,
    // base64 public key, what clients configure as `key`
    pub key: String,
    pub pk: sign::PublicKey,
}

// hbbs binds [::] dual-stack: TCP port-1, port, port+2 and UDP port
fn port_set_free(port: u16) -> bool {
    let tcp = |p: u16| TcpListener::bind(("::", p)).is_ok();
    let udp = std::net::UdpSocket::bind(("::", port)).is_ok();
    tcp(port - 1) && tcp(port) && tcp(port + 2) && udp
}

// hbbs binds with SO_REUSEPORT, so two test processes (or two tests) that pick
// the same port would share it. A lock file per port keeps the pick exclusive.
// hbbs also binds port - 1 and port + 2, so ports are picked in aligned blocks
// of four: a neighbor that is not listening yet cannot take one of them.
struct PortLock(std::path::PathBuf);

impl Drop for PortLock {
    fn drop(&mut self) {
        std::fs::remove_file(&self.0).ok();
    }
}

fn free_port() -> (u16, PortLock) {
    let dir = std::env::temp_dir().join("hbbs-test-ports");
    if let Err(err) = std::fs::create_dir_all(&dir) {
        panic!("cannot create {}: {}", dir.display(), err);
    }
    for _ in 0..1000 {
        let port = 20_001 + 4 * (rand_u32() % 10_000) as u16;
        let path = dir.join(port.to_string());
        let stale = std::fs::metadata(&path)
            .and_then(|m| m.modified())
            .map(|t| t.elapsed().map(|e| e.as_secs() > 600).unwrap_or(false))
            .unwrap_or(false);
        if stale {
            std::fs::remove_file(&path).ok();
        }
        if std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .is_err()
        {
            continue;
        }
        let lock = PortLock(path);
        if port_set_free(port) {
            return (port, lock);
        }
    }
    panic!(
        "no free port block in 1000 tries; the tests need TCP and UDP on [::] \
         and lock files in {}",
        dir.display()
    );
}

fn rand_u32() -> u32 {
    let mut b = [0u8; 4];
    sodiumoxide::randombytes::randombytes_into(&mut b);
    u32::from_le_bytes(b)
}

impl Hbbs {
    pub async fn start(envs: &[(&str, &str)], args: &[&str]) -> Hbbs {
        Self::start_bin(env!("CARGO_BIN_EXE_hbbs"), envs, args).await
    }

    /// Starts another hbbs binary, e.g. the original one for differential tests.
    pub async fn start_bin(bin: &str, envs: &[(&str, &str)], args: &[&str]) -> Hbbs {
        sodiumoxide::init().ok();
        for _ in 0..20 {
            let (port, port_lock) = free_port();
            let dir = tempfile::tempdir().unwrap();
            let log = std::fs::File::create(dir.path().join("hbbs.log")).unwrap();
            let mut cmd = Command::new(bin);
            cmd.current_dir(dir.path())
                .arg("-p")
                .arg(port.to_string())
                .args(args)
                .env("TEST_HBBS", "no")
                .env("DB_URL", dir.path().join("db.sqlite3"))
                .env("RUST_LOG", "debug")
                .stdin(Stdio::null())
                .stdout(log.try_clone().unwrap())
                .stderr(log);
            for k in [
                "SECURE_TCP",
                "IPV6_PUNCH",
                "WEBRTC",
                "UDP_PUNCH",
                "ALWAYS_USE_RELAY",
            ] {
                cmd.env_remove(k);
            }
            for (k, v) in envs {
                cmd.env(k, v);
            }
            let mut child = cmd.spawn().unwrap();
            let deadline = Instant::now() + Duration::from_secs(20);
            let mut up = false;
            while Instant::now() < deadline {
                if let Ok(Some(_)) = child.try_wait() {
                    break;
                }
                // with -k <public key> hbbs has no key pair
                let keyed = args.contains(&"-k") || dir.path().join("id_ed25519.pub").exists();
                if std::net::TcpStream::connect(("127.0.0.1", port)).is_ok() && keyed {
                    up = true;
                    break;
                }
                sleep(Duration::from_millis(50)).await;
            }
            if !up {
                child.kill().ok();
                child.wait().ok();
                continue;
            }
            let key = match args.iter().position(|a| *a == "-k") {
                Some(i) => args[i + 1].to_owned(),
                None => std::fs::read_to_string(dir.path().join("id_ed25519.pub"))
                    .unwrap()
                    .trim()
                    .to_owned(),
            };
            let pk = base64::decode(&key)
                .ok()
                .and_then(|raw| sign::PublicKey::from_slice(&raw))
                .unwrap_or(sign::PublicKey([0u8; 32]));
            return Hbbs {
                child,
                _port: port_lock,
                dir,
                port,
                key,
                pk,
            };
        }
        panic!("hbbs did not start");
    }

    pub fn tcp_addr(&self) -> SocketAddr {
        SocketAddr::new(PEER_IP, self.port)
    }

    pub fn udp_addr(&self) -> SocketAddr {
        SocketAddr::new(PEER_IP, self.port)
    }

    pub fn nat_addr(&self) -> SocketAddr {
        SocketAddr::new(PEER_IP, self.port - 1)
    }

    pub fn ws_addr(&self) -> SocketAddr {
        SocketAddr::new(PEER_IP, self.port + 2)
    }

    pub fn log(&self) -> String {
        std::fs::read_to_string(self.dir.path().join("hbbs.log")).unwrap_or_default()
    }

    pub fn pid(&self) -> u32 {
        self.child.id()
    }

    /// Open file descriptors of the hbbs process (Linux).
    pub fn open_fds(&self) -> usize {
        std::fs::read_dir(format!("/proc/{}/fd", self.pid()))
            .map(|d| d.count())
            .unwrap_or(0)
    }

    pub fn is_running(&mut self) -> bool {
        matches!(self.child.try_wait(), Ok(None))
    }
}

impl Drop for Hbbs {
    fn drop(&mut self) {
        self.child.kill().ok();
        self.child.wait().ok();
        if std::env::var("HBBS_TEST_KEEP_LOG").is_ok() {
            eprintln!("---- hbbs log ----\n{}", self.log());
        }
    }
}

pub fn msg(f: impl FnOnce(&mut RendezvousMessage)) -> RendezvousMessage {
    let mut m = RendezvousMessage::new();
    f(&mut m);
    m
}

pub fn parse(bytes: &[u8]) -> Option<RendezvousMessage> {
    RendezvousMessage::parse_from_bytes(bytes).ok()
}

/// The controlled side: registered over UDP like `RendezvousMediator::start_udp`.
pub struct Peer {
    pub id: String,
    pub sock: UdpSocket,
    pub server: SocketAddr,
    pub sk: sign::SecretKey,
    pub pk: sign::PublicKey,
}

impl Peer {
    pub async fn register(hbbs: &Hbbs, id: &str) -> Peer {
        Self::register_from(hbbs, id, PEER_IP).await
    }

    pub async fn register_from(hbbs: &Hbbs, id: &str, ip: IpAddr) -> Peer {
        let sock = UdpSocket::bind(SocketAddr::new(ip, 0)).await.unwrap();
        let (pk, sk) = sign::gen_keypair();
        let peer = Peer {
            id: id.to_owned(),
            sock,
            server: hbbs.udp_addr(),
            sk,
            pk,
        };
        let uuid: Bytes = format!("uuid-{}", id).into_bytes().into();
        for _ in 0..5 {
            peer.send(&msg(|m| {
                m.set_register_pk(RegisterPk {
                    id: id.to_owned(),
                    uuid: uuid.clone(),
                    pk: peer.pk.0.to_vec().into(),
                    ..Default::default()
                })
            }))
            .await;
            let deadline = Instant::now() + Duration::from_millis(1000);
            while let Some(m) = peer.recv_until(deadline).await {
                if let Some(rendezvous_message::Union::RegisterPkResponse(r)) = m.union {
                    assert_eq!(r.result.enum_value(), Ok(register_pk_response::Result::OK));
                    return peer;
                }
            }
        }
        panic!("peer {} could not register", id);
    }

    pub async fn send(&self, m: &RendezvousMessage) {
        self.sock
            .send_to(&m.write_to_bytes().unwrap(), self.server)
            .await
            .unwrap();
    }

    pub async fn recv_until(&self, deadline: Instant) -> Option<RendezvousMessage> {
        let mut buf = vec![0u8; 65536];
        loop {
            let left = deadline.checked_duration_since(Instant::now())?;
            match timeout(left, self.sock.recv_from(&mut buf)).await {
                Ok(Ok((n, _))) => {
                    if let Some(m) = parse(&buf[..n]) {
                        return Some(m);
                    }
                }
                _ => return None,
            }
        }
    }

    pub async fn recv(&self, ms: u64) -> Option<RendezvousMessage> {
        self.recv_until(Instant::now() + Duration::from_millis(ms))
            .await
    }

    /// Waits for the next message matching `pick`, skipping others.
    pub async fn expect<T>(
        &self,
        ms: u64,
        mut pick: impl FnMut(rendezvous_message::Union) -> Option<T>,
    ) -> Option<T> {
        let deadline = Instant::now() + Duration::from_millis(ms);
        while let Some(m) = self.recv_until(deadline).await {
            if let Some(u) = m.union {
                if let Some(t) = pick(u) {
                    return Some(t);
                }
            }
        }
        None
    }

    /// Re-registers so the server keeps the peer online.
    pub async fn keep_alive(&self) {
        self.send(&msg(|m| {
            m.set_register_peer(RegisterPeer {
                id: self.id.clone(),
                ..Default::default()
            })
        }))
        .await;
        self.expect(1000, |u| match u {
            rendezvous_message::Union::RegisterPeerResponse(r) => Some(r),
            _ => None,
        })
        .await;
    }
}

/// How a client handles the server's KeyExchange.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Kx {
    /// no `secure_tcp` call: plaintext, an unsolicited KeyExchange is skipped
    None,
    /// clients before 1.5.0: version 0, ignores `version`/`signed_params`
    Legacy,
    /// client 1.5.0: version 1 with signed parameters
    V1,
}

pub struct Tcp {
    pub conn: FramedStream,
    pub secured: bool,
}

impl Tcp {
    pub async fn connect(hbbs: &Hbbs, ip: IpAddr, kx: Kx) -> Tcp {
        Self::connect_to(hbbs.tcp_addr(), &hbbs.pk, ip, kx).await
    }

    pub async fn connect_to(
        addr: SocketAddr,
        server_pk: &sign::PublicKey,
        ip: IpAddr,
        kx: Kx,
    ) -> Tcp {
        let conn = FramedStream::new(addr, Some(SocketAddr::new(ip, 0)), 3000)
            .await
            .unwrap();
        let mut tcp = Tcp {
            conn,
            secured: false,
        };
        if kx != Kx::None {
            tcp.secured = tcp.secure(server_pk, kx).await.unwrap();
        }
        tcp
    }

    pub fn local_addr(&self) -> SocketAddr {
        self.conn.local_addr()
    }

    /// The client's `key_exchange()` (common.rs at 1.5.0; 1.4.x for `Kx::Legacy`).
    pub async fn secure(&mut self, server_pk: &sign::PublicKey, kx: Kx) -> Result<bool, String> {
        let Some(Ok(bytes)) = self.conn.next_timeout(3000).await else {
            return Err("no first frame".to_owned());
        };
        let Some(RendezvousMessage {
            union: Some(rendezvous_message::Union::KeyExchange(ex)),
            ..
        }) = parse(&bytes)
        else {
            return Ok(false);
        };
        if ex.keys.len() != 1 {
            return Err("invalid key exchange message".to_owned());
        }
        let their_pk = sign::verify(&ex.keys[0], server_pk)
            .map_err(|_| "Signature mismatch in key exchange".to_owned())?;
        if their_pk.len() != box_::PUBLICKEYBYTES {
            return Err("wrong pk length".to_owned());
        }
        let mut pk = [0u8; box_::PUBLICKEYBYTES];
        pk.copy_from_slice(&their_pk);
        let mut advertised = 0;
        if kx == Kx::V1 {
            if pk[31] & 0x80 != 0 || !ex.signed_params.is_empty() {
                let params = sign::verify(&ex.signed_params, server_pk)
                    .ok()
                    .and_then(|s| {
                        let p = s.strip_prefix(KX_PARAMS_DOMAIN)?.to_vec();
                        KxParams::parse_from_bytes(&p).ok()
                    })
                    .ok_or("Missing or invalid signed key exchange parameters")?;
                if params.pk[..] != pk[..] || params.version != ex.version {
                    return Err("version or key does not match its signature".to_owned());
                }
            }
            advertised = ex.version;
        }
        let picked = advertised.min(hbb_common::tcp::KX_VERSION_LATEST);
        let (our_pk, our_sk) = box_::gen_keypair();
        let key = secretbox::gen_key();
        let nonce = box_::Nonce([0u8; box_::NONCEBYTES]);
        let sealed = box_::seal(&key.0, &nonce, &box_::PublicKey(pk), &our_sk);
        let reply = msg(|m| {
            m.set_key_exchange(KeyExchange {
                keys: vec![our_pk.0.to_vec().into(), sealed.into()],
                version: picked,
                ..Default::default()
            })
        });
        self.conn.send(&reply).await.map_err(|e| e.to_string())?;
        if picked == 0 {
            self.conn.set_key(key);
        } else {
            self.conn
                .set_key_split(
                    key,
                    true,
                    &KxTranscript {
                        initiator_pk: &our_pk.0,
                        responder_pk: &pk,
                        advertised,
                        picked,
                    },
                )
                .map_err(|e| e.to_string())?;
        }
        Ok(true)
    }

    pub async fn send(&mut self, m: &RendezvousMessage) {
        self.conn.send(m).await.unwrap();
    }

    pub async fn send_raw(&mut self, bytes: Vec<u8>) {
        self.conn.send_raw(bytes).await.unwrap();
    }

    /// The client's `get_next_nonkeyexchange_msg`: skips at most one KeyExchange.
    pub async fn recv(&mut self, ms: u64) -> Option<RendezvousMessage> {
        for _ in 0..2 {
            match self.conn.next_timeout(ms).await {
                Some(Ok(bytes)) => {
                    let m = parse(&bytes)?;
                    if let Some(rendezvous_message::Union::KeyExchange(_)) = m.union {
                        continue;
                    }
                    return Some(m);
                }
                _ => return None,
            }
        }
        None
    }

    /// Next message matching `pick`, skipping others, until `ms` passed.
    pub async fn expect<T>(
        &mut self,
        ms: u64,
        mut pick: impl FnMut(rendezvous_message::Union) -> Option<T>,
    ) -> Option<T> {
        let deadline = Instant::now() + Duration::from_millis(ms);
        loop {
            let left = deadline.checked_duration_since(Instant::now())?;
            let m = self.recv(left.as_millis() as u64 + 1).await?;
            if let Some(u) = m.union {
                if let Some(t) = pick(u) {
                    return Some(t);
                }
            }
        }
    }

    /// True if the server closed the connection within `ms`.
    pub async fn closed_within(&mut self, ms: u64) -> bool {
        let deadline = Instant::now() + Duration::from_millis(ms);
        loop {
            let Some(left) = deadline.checked_duration_since(Instant::now()) else {
                return false;
            };
            match timeout(left, self.conn.next()).await {
                Ok(None) | Ok(Some(Err(_))) => return true,
                Ok(Some(Ok(_))) => continue,
                Err(_) => return false,
            }
        }
    }
}

pub fn punch_hole_request(id: &str, key: &str) -> PunchHoleRequest {
    PunchHoleRequest {
        id: id.to_owned(),
        nat_type: NatType::ASYMMETRIC.into(),
        licence_key: key.to_owned(),
        conn_type: ConnType::DEFAULT_CONN.into(),
        version: "1.5.0".to_owned(),
        ..Default::default()
    }
}

pub fn mangle(addr: SocketAddr) -> Bytes {
    AddrMangle::encode(addr).into()
}

pub fn demangle(bytes: &[u8]) -> SocketAddr {
    AddrMangle::decode(bytes)
}

pub fn pick_punch_hole(u: rendezvous_message::Union) -> Option<PunchHole> {
    match u {
        rendezvous_message::Union::PunchHole(p) => Some(p),
        _ => None,
    }
}

pub fn pick_fetch_local_addr(u: rendezvous_message::Union) -> Option<FetchLocalAddr> {
    match u {
        rendezvous_message::Union::FetchLocalAddr(p) => Some(p),
        _ => None,
    }
}

pub fn pick_punch_hole_response(u: rendezvous_message::Union) -> Option<PunchHoleResponse> {
    match u {
        rendezvous_message::Union::PunchHoleResponse(p) => Some(p),
        _ => None,
    }
}

pub fn pick_relay_response(u: rendezvous_message::Union) -> Option<RelayResponse> {
    match u {
        rendezvous_message::Union::RelayResponse(p) => Some(p),
        _ => None,
    }
}

pub fn pick_ice(u: rendezvous_message::Union) -> Option<IceCandidate> {
    match u {
        rendezvous_message::Union::IceCandidate(p) => Some(p),
        _ => None,
    }
}

/// Raw bytes of an unencrypted frame, as BytesCodec would read them.
pub fn frame_payload(b: BytesMut) -> Vec<u8> {
    b.to_vec()
}
