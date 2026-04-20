use crate::common::*;
use crate::peer::*;
use hbb_common::{
    allow_err, bail,
    bytes::{Bytes, BytesMut},
    bytes_codec::BytesCodec,
    config,
    futures::future::join_all,
    futures_util::{
        sink::SinkExt,
        stream::{SplitSink, StreamExt},
    },
    log,
    protobuf::{Message as _, MessageField},
    rendezvous_proto::{
        register_pk_response::Result::{
            INVALID_ID_FORMAT, LICENSE_MISMATCH, TOO_FREQUENT, UUID_MISMATCH,
        },
        *,
    },
    tcp::{listen_any, FramedStream},
    timeout,
    tokio::{
        self,
        io::{AsyncReadExt, AsyncWriteExt},
        net::{TcpListener, TcpStream},
        sync::{mpsc, Mutex},
        time::{interval, Duration},
    },
    tokio_util::codec::Framed,
    try_into_v4,
    udp::FramedSocket,
    AddrMangle, ResultType,
};
use ipnetwork::Ipv4Network;
use sodiumoxide::crypto::sign;
use std::{
    collections::{HashMap, HashSet},
    net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr},
    sync::atomic::{AtomicBool, AtomicUsize, Ordering},
    sync::Arc,
    time::Instant,
};

#[derive(Clone, Debug)]
enum Data {
    Msg(Box<RendezvousMessage>, SocketAddr),
    RelayServers0(String),
    RelayServers(RelayServers),
}

const REG_TIMEOUT: i32 = 30_000;
const MIN_REGISTRATION_ID_LEN: usize = 6;
const MAX_REGISTRATION_ID_LEN: usize = 100;
const MAX_ONLINE_REQUEST_PEERS_ENV: &str = "MAX_ONLINE_REQUEST_PEERS";
const FANOUT_WINDOW_SECONDS_ENV: &str = "FANOUT_WINDOW_SECONDS";
const MAX_FANOUT_TRACKED_SOURCES_ENV: &str = "MAX_FANOUT_TRACKED_SOURCES";
const MAX_PUNCH_TARGETS_PER_IP_PER_WINDOW_ENV: &str = "MAX_PUNCH_TARGETS_PER_IP_PER_WINDOW";
const MAX_RELAY_TARGETS_PER_IP_PER_WINDOW_ENV: &str = "MAX_RELAY_TARGETS_PER_IP_PER_WINDOW";
const TCP_PUNCH_ENTRY_TTL_SECS_ENV: &str = "TCP_PUNCH_ENTRY_TTL_SECS";
const MAX_TCP_PUNCH_ENTRIES_ENV: &str = "MAX_TCP_PUNCH_ENTRIES";
const DEFAULT_MAX_ONLINE_REQUEST_PEERS: usize = 4_096;
const DEFAULT_FANOUT_WINDOW_SECONDS: usize = 60;
const DEFAULT_MAX_FANOUT_TRACKED_SOURCES: usize = 8_192;
const DEFAULT_MAX_PUNCH_TARGETS_PER_IP_PER_WINDOW: usize = 256;
const DEFAULT_MAX_RELAY_TARGETS_PER_IP_PER_WINDOW: usize = 256;
const DEFAULT_TCP_PUNCH_ENTRY_TTL_SECS: usize = 30;
const DEFAULT_MAX_TCP_PUNCH_ENTRIES: usize = 4_096;
type TcpStreamSink = SplitSink<Framed<TcpStream, BytesCodec>, Bytes>;
type WsSink = SplitSink<tokio_tungstenite::WebSocketStream<TcpStream>, tungstenite::Message>;
enum Sink {
    TcpStream(TcpStreamSink),
    Ws(WsSink),
}
struct TcpPunchEntry {
    sink: Sink,
    created_at: Instant,
}
type Sender = mpsc::UnboundedSender<Data>;
type Receiver = mpsc::UnboundedReceiver<Data>;
static ROTATION_RELAY_SERVER: AtomicUsize = AtomicUsize::new(0);
type RelayServers = Vec<String>;
const CHECK_RELAY_TIMEOUT: u64 = 3_000;
static ALWAYS_USE_RELAY: AtomicBool = AtomicBool::new(false);

// Store punch hole requests
use once_cell::sync::Lazy;
use tokio::sync::Mutex as TokioMutex; // differentiate if needed
#[derive(Clone)]
struct PunchReqEntry {
    tm: Instant,
    from_ip: String,
    to_ip: String,
    to_id: String,
}
static PUNCH_REQS: Lazy<TokioMutex<Vec<PunchReqEntry>>> = Lazy::new(|| TokioMutex::new(Vec::new()));
const PUNCH_REQ_DEDUPE_SEC: u64 = 60;
const PUNCH_REQ_RETENTION_SECS: u64 = 600;
const MAX_PUNCH_REQS: usize = 8192;

struct FanoutEntry {
    window_started_at: Instant,
    last_seen_at: Instant,
    targets: HashSet<String>,
}

type FanoutMap = HashMap<String, FanoutEntry>;
static PUNCH_FANOUT: Lazy<TokioMutex<FanoutMap>> = Lazy::new(|| TokioMutex::new(HashMap::new()));
static RELAY_FANOUT: Lazy<TokioMutex<FanoutMap>> = Lazy::new(|| TokioMutex::new(HashMap::new()));

#[derive(Clone)]
struct Inner {
    serial: i32,
    version: String,
    software_url: String,
    mask: Option<Ipv4Network>,
    local_ip: String,
    sk: Option<sign::SecretKey>,
}

#[derive(Clone)]
pub struct RendezvousServer {
    tcp_punch: Arc<Mutex<HashMap<SocketAddr, TcpPunchEntry>>>,
    pm: PeerMap,
    tx: Sender,
    relay_servers: Arc<RelayServers>,
    relay_servers0: Arc<RelayServers>,
    rendezvous_servers: Arc<Vec<String>>,
    inner: Arc<Inner>,
}

enum LoopFailure {
    UdpSocket,
    Listener3,
    Listener2,
    Listener,
}

impl RendezvousServer {
    #[tokio::main(flavor = "multi_thread")]
    pub async fn start(port: i32, serial: i32, key: &str, rmem: usize) -> ResultType<()> {
        let (key, sk) = Self::get_server_sk(key);
        let nat_port = port - 1;
        let ws_port = port + 2;
        let pm = PeerMap::new().await?;
        log::info!("serial={}", serial);
        let rendezvous_servers = get_servers(&get_arg("rendezvous-servers"), "rendezvous-servers");
        log::info!("Listening on tcp/udp :{}", port);
        log::info!("Listening on tcp :{}, extra port for NAT test", nat_port);
        log::info!("Listening on websocket :{}", ws_port);
        let mut socket = create_udp_listener(port, rmem).await?;
        let (tx, mut rx) = mpsc::unbounded_channel::<Data>();
        let software_url = get_arg("software-url");
        let version = hbb_common::get_version_from_url(&software_url);
        if !version.is_empty() {
            log::info!("software_url: {}, version: {}", software_url, version);
        }
        let mask = get_arg("mask").parse().ok();
        let local_ip = if mask.is_none() {
            "".to_owned()
        } else {
            get_arg_or(
                "local-ip",
                local_ip_address::local_ip()
                    .map(|x| x.to_string())
                    .unwrap_or_default(),
            )
        };
        let mut rs = Self {
            tcp_punch: Arc::new(Mutex::new(HashMap::new())),
            pm,
            tx: tx.clone(),
            relay_servers: Default::default(),
            relay_servers0: Default::default(),
            rendezvous_servers: Arc::new(rendezvous_servers),
            inner: Arc::new(Inner {
                serial,
                version,
                software_url,
                sk,
                mask,
                local_ip,
            }),
        };
        log::info!("mask: {:?}", rs.inner.mask);
        log::info!("local-ip: {:?}", rs.inner.local_ip);
        std::env::set_var("PORT_FOR_API", port.to_string());
        rs.parse_relay_servers(&get_arg("relay-servers"));
        let mut listener = create_tcp_listener(port).await?;
        let mut listener2 = create_tcp_listener(nat_port).await?;
        let mut listener3 = create_tcp_listener(ws_port).await?;
        let test_addr = std::env::var("TEST_HBBS").unwrap_or_default();
        if std::env::var("ALWAYS_USE_RELAY")
            .unwrap_or_default()
            .to_uppercase()
            == "Y"
        {
            ALWAYS_USE_RELAY.store(true, Ordering::SeqCst);
        }
        log::info!(
            "ALWAYS_USE_RELAY={}",
            if ALWAYS_USE_RELAY.load(Ordering::SeqCst) {
                "Y"
            } else {
                "N"
            }
        );
        if test_addr.to_lowercase() != "no" {
            let test_addr = if test_addr.is_empty() {
                listener.local_addr()?
            } else {
                test_addr.parse()?
            };
            let test_key = key.to_owned();
            tokio::spawn(async move {
                if let Err(err) = test_hbbs(test_addr, test_key.clone()).await {
                    if test_addr.is_ipv6() && test_addr.ip().is_unspecified() {
                        let mut test_addr = test_addr;
                        test_addr.set_ip(IpAddr::V4(Ipv4Addr::UNSPECIFIED));
                        if let Err(err) = test_hbbs(test_addr, test_key).await {
                            log::error!("Failed to run hbbs test with {test_addr}: {err}");
                            std::process::exit(1);
                        }
                    } else {
                        log::error!("Failed to run hbbs test with {test_addr}: {err}");
                        std::process::exit(1);
                    }
                }
            });
        };
        let main_task = async move {
            loop {
                log::info!("Start");
                match rs
                    .io_loop(
                        &mut rx,
                        &mut listener,
                        &mut listener2,
                        &mut listener3,
                        &mut socket,
                        &key,
                    )
                    .await
                {
                    LoopFailure::UdpSocket => {
                        drop(socket);
                        socket = create_udp_listener(port, rmem).await?;
                    }
                    LoopFailure::Listener => {
                        drop(listener);
                        listener = create_tcp_listener(port).await?;
                    }
                    LoopFailure::Listener2 => {
                        drop(listener2);
                        listener2 = create_tcp_listener(nat_port).await?;
                    }
                    LoopFailure::Listener3 => {
                        drop(listener3);
                        listener3 = create_tcp_listener(ws_port).await?;
                    }
                }
            }
        };
        let listen_signal = listen_signal();
        tokio::select!(
            res = main_task => res,
            res = listen_signal => res,
        )
    }

    async fn io_loop(
        &mut self,
        rx: &mut Receiver,
        listener: &mut TcpListener,
        listener2: &mut TcpListener,
        listener3: &mut TcpListener,
        socket: &mut FramedSocket,
        key: &str,
    ) -> LoopFailure {
        let mut timer_check_relay = interval(Duration::from_millis(CHECK_RELAY_TIMEOUT));
        loop {
            tokio::select! {
                _ = timer_check_relay.tick() => {
                    if self.relay_servers0.len() > 1 {
                        let rs = self.relay_servers0.clone();
                        let tx = self.tx.clone();
                        tokio::spawn(async move {
                            check_relay_servers(rs, tx).await;
                        });
                    }
                }
                Some(data) = rx.recv() => {
                    match data {
                        Data::Msg(msg, addr) => { allow_err!(socket.send(msg.as_ref(), addr).await); }
                        Data::RelayServers0(rs) => { self.parse_relay_servers(&rs); }
                        Data::RelayServers(rs) => { self.relay_servers = Arc::new(rs); }
                    }
                }
                res = socket.next() => {
                    match res {
                        Some(Ok((bytes, addr))) => {
                            let addr: SocketAddr = addr.into();
                            if !crate::common::allow_udp_packet_from_ip("hbbs-udp", addr) {
                                log::warn!("Rate limit exceeded for hbbs-udp from {}", addr.ip());
                                continue;
                            }
                            if let Err(err) = self.handle_udp(&bytes, addr, socket, key).await {
                                log::error!("udp failure: {}", err);
                                return LoopFailure::UdpSocket;
                            }
                        }
                        Some(Err(err)) => {
                            log::error!("udp failure: {}", err);
                            return LoopFailure::UdpSocket;
                        }
                        None => {
                            // unreachable!() ?
                        }
                    }
                }
                res = listener2.accept() => {
                    match res {
                        Ok((stream, addr))  => {
                            if !crate::common::allow_connection_from_ip("hbbs-nat", addr) {
                                log::warn!("Rate limit exceeded for hbbs-nat from {}", addr.ip());
                                continue;
                            }
                            stream.set_nodelay(true).ok();
                            self.handle_listener2(stream, addr, key).await;
                        }
                        Err(err) => {
                           log::error!("listener2.accept failed: {}", err);
                           return LoopFailure::Listener2;
                        }
                    }
                }
                res = listener3.accept() => {
                    match res {
                        Ok((stream, addr))  => {
                            if !crate::common::allow_connection_from_ip("hbbs-ws", addr) {
                                log::warn!("Rate limit exceeded for hbbs-ws from {}", addr.ip());
                                continue;
                            }
                            stream.set_nodelay(true).ok();
                            self.handle_listener(stream, addr, key, true).await;
                        }
                        Err(err) => {
                           log::error!("listener3.accept failed: {}", err);
                           return LoopFailure::Listener3;
                        }
                    }
                }
                res = listener.accept() => {
                    match res {
                        Ok((stream, addr)) => {
                            if !crate::common::allow_connection_from_ip("hbbs-main", addr) {
                                log::warn!("Rate limit exceeded for hbbs-main from {}", addr.ip());
                                continue;
                            }
                            stream.set_nodelay(true).ok();
                            self.handle_listener(stream, addr, key, false).await;
                        }
                       Err(err) => {
                           log::error!("listener.accept failed: {}", err);
                           return LoopFailure::Listener;
                       }
                    }
                }
            }
        }
    }

    #[inline]
    async fn handle_udp(
        &mut self,
        bytes: &BytesMut,
        addr: SocketAddr,
        socket: &mut FramedSocket,
        key: &str,
    ) -> ResultType<()> {
        if let Ok(msg_in) = RendezvousMessage::parse_from_bytes(bytes) {
            match msg_in.union {
                Some(rendezvous_message::Union::RegisterPeer(rp)) => {
                    // B registered
                    if !rp.id.is_empty() {
                        if !is_valid_server_key(key, &rp.licence_key) {
                            log::warn!(
                                "Authentication failed from {} for peer {} - invalid key",
                                addr,
                                rp.id
                            );
                            return Ok(());
                        }
                        if !is_valid_registration_id(&rp.id) {
                            log::warn!("Invalid peer registration id from {}: {:?}", addr, rp.id);
                            return Ok(());
                        }
                        let ip = addr.ip().to_string();
                        if !self.check_ip_blocker(&ip, &rp.id).await {
                            log::warn!(
                                "Peer registration rate-limited from {} for id {}",
                                addr,
                                rp.id
                            );
                            return Ok(());
                        }
                        log::trace!("New peer registered: {:?} {:?}", &rp.id, &addr);
                        self.update_addr(rp.id, addr, socket).await?;
                        if self.inner.serial > rp.serial {
                            let mut msg_out = RendezvousMessage::new();
                            msg_out.set_configure_update(ConfigUpdate {
                                serial: self.inner.serial,
                                rendezvous_servers: (*self.rendezvous_servers).clone(),
                                ..Default::default()
                            });
                            socket.send(&msg_out, addr).await?;
                        }
                    }
                }
                Some(rendezvous_message::Union::RegisterPk(rk)) => {
                    if rk.uuid.is_empty() || rk.pk.is_empty() {
                        return Ok(());
                    }
                    if !is_valid_server_key(key, &rk.licence_key) {
                        log::warn!(
                            "Authentication failed from {} for peer {} - invalid key",
                            addr,
                            rk.id
                        );
                        return send_rk_res(socket, addr, LICENSE_MISMATCH).await;
                    }
                    let id = rk.id;
                    let ip = addr.ip().to_string();
                    if !is_valid_registration_id(&id) {
                        return send_rk_res(socket, addr, INVALID_ID_FORMAT).await;
                    } else if !self.check_ip_blocker(&ip, &id).await {
                        return send_rk_res(socket, addr, TOO_FREQUENT).await;
                    }
                    let Some(peer) = self.pm.get_or_for_registration(&id, &ip).await else {
                        log::warn!(
                            "Pending registration cache limit reached from {} for id {}",
                            addr,
                            id
                        );
                        return send_rk_res(socket, addr, TOO_FREQUENT).await;
                    };
                    let (changed, ip_changed) = {
                        let peer = peer.read().await;
                        if peer.uuid.is_empty() {
                            (true, false)
                        } else {
                            if peer.uuid == rk.uuid {
                                if peer.info.ip != ip && peer.pk != rk.pk {
                                    log::warn!(
                                        "Peer {} ip/pk mismatch: {}/{:?} vs {}/{:?}",
                                        id,
                                        ip,
                                        rk.pk,
                                        peer.info.ip,
                                        peer.pk,
                                    );
                                    drop(peer);
                                    return send_rk_res(socket, addr, UUID_MISMATCH).await;
                                }
                            } else {
                                log::warn!(
                                    "Peer {} uuid mismatch: {:?} vs {:?}",
                                    id,
                                    rk.uuid,
                                    peer.uuid
                                );
                                drop(peer);
                                return send_rk_res(socket, addr, UUID_MISMATCH).await;
                            }
                            let ip_changed = peer.info.ip != ip;
                            (
                                peer.uuid != rk.uuid || peer.pk != rk.pk || ip_changed,
                                ip_changed,
                            )
                        }
                    };
                    let mut req_pk = peer.read().await.reg_pk;
                    if req_pk.1.elapsed().as_secs() > 6 {
                        req_pk.0 = 0;
                    } else if req_pk.0 > 2 {
                        return send_rk_res(socket, addr, TOO_FREQUENT).await;
                    }
                    req_pk.0 += 1;
                    req_pk.1 = Instant::now();
                    peer.write().await.reg_pk = req_pk;
                    if ip_changed {
                        let mut lock = IP_CHANGES.lock().await;
                        track_ip_change(&mut lock, &id, &ip);
                    }
                    let result = if changed {
                        self.pm.update_pk(id, peer, addr, rk.uuid, rk.pk, ip).await
                    } else {
                        register_pk_response::Result::OK
                    };
                    let mut msg_out = RendezvousMessage::new();
                    msg_out.set_register_pk_response(RegisterPkResponse {
                        result: result.into(),
                        ..Default::default()
                    });
                    socket.send(&msg_out, addr).await?
                }
                Some(rendezvous_message::Union::PunchHoleRequest(ph)) => {
                    if self.pm.is_in_memory(&ph.id).await {
                        self.handle_udp_punch_hole_request(addr, ph, key).await?;
                    } else {
                        // not in memory, fetch from db with spawn in case blocking me
                        let mut me = self.clone();
                        let key = key.to_owned();
                        tokio::spawn(async move {
                            allow_err!(me.handle_udp_punch_hole_request(addr, ph, &key).await);
                        });
                    }
                }
                Some(rendezvous_message::Union::PunchHoleSent(phs)) => {
                    self.handle_hole_sent(phs, addr, Some(socket)).await?;
                }
                Some(rendezvous_message::Union::LocalAddr(la)) => {
                    self.handle_local_addr(la, addr, Some(socket)).await?;
                }
                Some(rendezvous_message::Union::ConfigureUpdate(mut cu)) => {
                    if try_into_v4(addr).ip().is_loopback() && cu.serial > self.inner.serial {
                        let mut inner: Inner = (*self.inner).clone();
                        inner.serial = cu.serial;
                        self.inner = Arc::new(inner);
                        self.rendezvous_servers = Arc::new(
                            cu.rendezvous_servers
                                .drain(..)
                                .filter(|x| {
                                    !x.is_empty()
                                        && test_if_valid_server(x, "rendezvous-server").is_ok()
                                })
                                .collect(),
                        );
                        log::info!(
                            "configure updated: serial={} rendezvous-servers={:?}",
                            self.inner.serial,
                            self.rendezvous_servers
                        );
                    }
                }
                Some(rendezvous_message::Union::SoftwareUpdate(su)) => {
                    if !self.inner.version.is_empty() && su.url != self.inner.version {
                        let mut msg_out = RendezvousMessage::new();
                        msg_out.set_software_update(SoftwareUpdate {
                            url: self.inner.software_url.clone(),
                            ..Default::default()
                        });
                        socket.send(&msg_out, addr).await?;
                    }
                }
                _ => {}
            }
        }
        Ok(())
    }

    #[inline]
    async fn handle_tcp(
        &mut self,
        bytes: &[u8],
        sink: &mut Option<Sink>,
        addr: SocketAddr,
        key: &str,
        ws: bool,
    ) -> bool {
        if let Ok(msg_in) = RendezvousMessage::parse_from_bytes(bytes) {
            let should_rate_limit = matches!(
                msg_in.union.as_ref(),
                Some(rendezvous_message::Union::PunchHoleRequest(_))
                    | Some(rendezvous_message::Union::RequestRelay(_))
                    | Some(rendezvous_message::Union::RelayResponse(_))
                    | Some(rendezvous_message::Union::PunchHoleSent(_))
                    | Some(rendezvous_message::Union::LocalAddr(_))
                    | Some(rendezvous_message::Union::TestNatRequest(_))
            );
            if should_rate_limit
                && !crate::common::allow_control_message_from_ip("hbbs-control", addr)
            {
                log::warn!("Control message rate limit exceeded from {}", addr.ip());
                return false;
            }
            match msg_in.union {
                Some(rendezvous_message::Union::PunchHoleRequest(ph)) => {
                    // there maybe several attempt, so sink can be none
                    if let Some(sink) = sink.take() {
                        let rejected_sink = {
                            let mut lock = self.tcp_punch.lock().await;
                            insert_tcp_punch_entry(&mut lock, try_into_v4(addr), sink)
                        };
                        if let Some(rejected_sink) = rejected_sink {
                            crate::common::record_protection_event("tcp_punch_entry_limit_hits");
                            log::warn!("tcp_punch entry limit exceeded for {}", addr);
                            let mut msg_out = RendezvousMessage::new();
                            msg_out.set_punch_hole_response(PunchHoleResponse {
                                other_failure: "Too many pending TCP punch sessions".to_owned(),
                                ..Default::default()
                            });
                            Self::send_to_sink(&mut Some(rejected_sink), msg_out).await;
                            return false;
                        }
                    }
                    allow_err!(self.handle_tcp_punch_hole_request(addr, ph, key, ws).await);
                    return true;
                }
                Some(rendezvous_message::Union::RequestRelay(mut rf)) => {
                    let source_ip = try_into_v4(addr).ip().to_string();
                    let target_id = rf.id.clone();
                    let allowed = {
                        let mut lock = RELAY_FANOUT.lock().await;
                        allow_target_fanout(
                            &mut lock,
                            &source_ip,
                            &target_id,
                            max_relay_targets_per_ip_per_window(),
                            "relay_fanout_entries_evicted",
                            "relay_fanout_entries_rejected",
                        )
                    };
                    if !allowed {
                        crate::common::record_protection_event("relay_target_fanout_limit_hits");
                        log::warn!(
                            "Relay target fan-out limit exceeded from {} toward {}",
                            addr,
                            target_id
                        );
                        let mut msg_out = RendezvousMessage::new();
                        msg_out.set_relay_response(RelayResponse {
                            uuid: rf.uuid.clone(),
                            refuse_reason: "Too many distinct relay targets".to_owned(),
                            ..Default::default()
                        });
                        if sink.is_some() {
                            Self::send_to_sink(sink, msg_out).await;
                        } else {
                            allow_err!(self.send_to_tcp_sync(msg_out, addr).await);
                        }
                        return true;
                    }
                    // there maybe several attempt, so sink can be none
                    if let Some(sink) = sink.take() {
                        let rejected_sink = {
                            let mut lock = self.tcp_punch.lock().await;
                            insert_tcp_punch_entry(&mut lock, try_into_v4(addr), sink)
                        };
                        if let Some(rejected_sink) = rejected_sink {
                            crate::common::record_protection_event("tcp_punch_entry_limit_hits");
                            log::warn!("tcp_punch entry limit exceeded for {}", addr);
                            let mut msg_out = RendezvousMessage::new();
                            msg_out.set_relay_response(RelayResponse {
                                uuid: rf.uuid.clone(),
                                refuse_reason: "Too many pending TCP punch sessions".to_owned(),
                                ..Default::default()
                            });
                            Self::send_to_sink(&mut Some(rejected_sink), msg_out).await;
                            return true;
                        }
                    }
                    if let Some(peer) = self.pm.get_in_memory(&rf.id).await {
                        let mut msg_out = RendezvousMessage::new();
                        rf.socket_addr = AddrMangle::encode(addr).into();
                        msg_out.set_request_relay(rf);
                        let peer_addr = peer.read().await.socket_addr;
                        self.tx.send(Data::Msg(msg_out.into(), peer_addr)).ok();
                    }
                    return true;
                }
                Some(rendezvous_message::Union::RelayResponse(mut rr)) => {
                    let addr_b = AddrMangle::decode(&rr.socket_addr);
                    rr.socket_addr = Default::default();
                    let id = rr.id();
                    if !id.is_empty() {
                        let pk = self.get_pk(&rr.version, id.to_owned()).await;
                        rr.set_pk(pk);
                    }
                    let mut msg_out = RendezvousMessage::new();
                    if !rr.relay_server.is_empty() {
                        if self.is_lan(addr_b) {
                            // https://github.com/rustdesk/rustdesk-server/issues/24
                            rr.relay_server = self.inner.local_ip.clone();
                        } else if rr.relay_server == self.inner.local_ip {
                            rr.relay_server = self.get_relay_server(addr.ip(), addr_b.ip());
                        }
                    }
                    msg_out.set_relay_response(rr);
                    allow_err!(self.send_to_tcp_sync(msg_out, addr_b).await);
                }
                Some(rendezvous_message::Union::PunchHoleSent(phs)) => {
                    allow_err!(self.handle_hole_sent(phs, addr, None).await);
                }
                Some(rendezvous_message::Union::LocalAddr(la)) => {
                    allow_err!(self.handle_local_addr(la, addr, None).await);
                }
                Some(rendezvous_message::Union::TestNatRequest(tar)) => {
                    if !is_valid_server_key(key, &tar.licence_key) {
                        log::warn!("Authentication failed from {} for nat probe", addr);
                        return true;
                    }
                    let mut msg_out = RendezvousMessage::new();
                    let mut res = TestNatResponse {
                        port: addr.port() as _,
                        ..Default::default()
                    };
                    if self.inner.serial > tar.serial {
                        let mut cu = ConfigUpdate::new();
                        cu.serial = self.inner.serial;
                        cu.rendezvous_servers = (*self.rendezvous_servers).clone();
                        res.cu = MessageField::from_option(Some(cu));
                    }
                    msg_out.set_test_nat_response(res);
                    Self::send_to_sink(sink, msg_out).await;
                }
                Some(rendezvous_message::Union::RegisterPk(_)) => {
                    let res = register_pk_response::Result::NOT_SUPPORT;
                    let mut msg_out = RendezvousMessage::new();
                    msg_out.set_register_pk_response(RegisterPkResponse {
                        result: res.into(),
                        ..Default::default()
                    });
                    Self::send_to_sink(sink, msg_out).await;
                }
                _ => {}
            }
        }
        false
    }

    #[inline]
    async fn update_addr(
        &mut self,
        id: String,
        socket_addr: SocketAddr,
        socket: &mut FramedSocket,
    ) -> ResultType<()> {
        let (request_pk, ip_change) = if let Some(old) = self.pm.get_in_memory(&id).await {
            let mut old = old.write().await;
            let ip = socket_addr.ip();
            let ip_change = if old.socket_addr.port() != 0 {
                ip != old.socket_addr.ip()
            } else {
                ip.to_string() != old.info.ip
            } && !ip.is_loopback();
            let request_pk = old.pk.is_empty() || ip_change;
            if !request_pk {
                old.socket_addr = socket_addr;
                old.last_reg_time = Instant::now();
            }
            let ip_change = if ip_change && old.reg_pk.0 <= 2 {
                Some(if old.socket_addr.port() == 0 {
                    old.info.ip.clone()
                } else {
                    old.socket_addr.to_string()
                })
            } else {
                None
            };
            (request_pk, ip_change)
        } else {
            (true, None)
        };
        if let Some(old) = ip_change {
            log::info!("IP change of {} from {} to {}", id, old, socket_addr);
        }
        let mut msg_out = RendezvousMessage::new();
        msg_out.set_register_peer_response(RegisterPeerResponse {
            request_pk,
            ..Default::default()
        });
        socket.send(&msg_out, socket_addr).await
    }

    #[inline]
    async fn handle_hole_sent<'a>(
        &mut self,
        phs: PunchHoleSent,
        addr: SocketAddr,
        socket: Option<&'a mut FramedSocket>,
    ) -> ResultType<()> {
        // punch hole sent from B, tell A that B is ready to be connected
        let addr_a = AddrMangle::decode(&phs.socket_addr);
        log::debug!(
            "{} punch hole response to {:?} from {:?}",
            if socket.is_none() { "TCP" } else { "UDP" },
            &addr_a,
            &addr
        );
        let mut msg_out = RendezvousMessage::new();
        let mut p = PunchHoleResponse {
            socket_addr: AddrMangle::encode(addr).into(),
            pk: self.get_pk(&phs.version, phs.id).await,
            relay_server: phs.relay_server.clone(),
            ..Default::default()
        };
        if let Ok(t) = phs.nat_type.enum_value() {
            p.set_nat_type(t);
        }
        msg_out.set_punch_hole_response(p);
        if let Some(socket) = socket {
            socket.send(&msg_out, addr_a).await?;
        } else {
            self.send_to_tcp(msg_out, addr_a).await;
        }
        Ok(())
    }

    #[inline]
    async fn handle_local_addr<'a>(
        &mut self,
        la: LocalAddr,
        addr: SocketAddr,
        socket: Option<&'a mut FramedSocket>,
    ) -> ResultType<()> {
        // relay local addrs of B to A
        let addr_a = AddrMangle::decode(&la.socket_addr);
        log::debug!(
            "{} local addrs response to {:?} from {:?}",
            if socket.is_none() { "TCP" } else { "UDP" },
            &addr_a,
            &addr
        );
        let mut msg_out = RendezvousMessage::new();
        let mut p = PunchHoleResponse {
            socket_addr: la.local_addr.clone(),
            pk: self.get_pk(&la.version, la.id).await,
            relay_server: la.relay_server,
            ..Default::default()
        };
        p.set_is_local(true);
        msg_out.set_punch_hole_response(p);
        if let Some(socket) = socket {
            socket.send(&msg_out, addr_a).await?;
        } else {
            self.send_to_tcp(msg_out, addr_a).await;
        }
        Ok(())
    }

    #[inline]
    async fn handle_punch_hole_request(
        &mut self,
        addr: SocketAddr,
        ph: PunchHoleRequest,
        key: &str,
        ws: bool,
    ) -> ResultType<(RendezvousMessage, Option<SocketAddr>)> {
        let mut ph = ph;
        if !key.is_empty() && ph.licence_key != key {
            log::warn!(
                "Authentication failed from {} for peer {} - invalid key",
                addr,
                ph.id
            );
            let mut msg_out = RendezvousMessage::new();
            msg_out.set_punch_hole_response(PunchHoleResponse {
                failure: punch_hole_response::Failure::LICENSE_MISMATCH.into(),
                ..Default::default()
            });
            return Ok((msg_out, None));
        }
        let id = ph.id;
        let source_ip = try_into_v4(addr).ip().to_string();
        let allowed = {
            let mut lock = PUNCH_FANOUT.lock().await;
            allow_target_fanout(
                &mut lock,
                &source_ip,
                &id,
                max_punch_targets_per_ip_per_window(),
                "punch_fanout_entries_evicted",
                "punch_fanout_entries_rejected",
            )
        };
        if !allowed {
            crate::common::record_protection_event("punch_target_fanout_limit_hits");
            log::warn!(
                "Punch target fan-out limit exceeded from {} toward {}",
                addr,
                id
            );
            let mut msg_out = RendezvousMessage::new();
            msg_out.set_punch_hole_response(PunchHoleResponse {
                other_failure: "Too many distinct punch targets".to_owned(),
                ..Default::default()
            });
            return Ok((msg_out, None));
        }
        // punch hole request from A, relay to B,
        // check if in same intranet first,
        // fetch local addrs if in same intranet.
        // because punch hole won't work if in the same intranet,
        // all routers will drop such self-connections.
        if let Some(peer) = self.pm.get(&id).await {
            let (elapsed, peer_addr) = {
                let r = peer.read().await;
                (r.last_reg_time.elapsed().as_millis() as i32, r.socket_addr)
            };
            if elapsed >= REG_TIMEOUT {
                let mut msg_out = RendezvousMessage::new();
                msg_out.set_punch_hole_response(PunchHoleResponse {
                    failure: punch_hole_response::Failure::OFFLINE.into(),
                    ..Default::default()
                });
                return Ok((msg_out, None));
            }

            // record punch hole request (from addr -> peer id/peer_addr)
            {
                let from_ip = try_into_v4(addr).ip().to_string();
                let to_ip = try_into_v4(peer_addr).ip().to_string();
                let to_id_clone = id.clone();
                let mut lock = PUNCH_REQS.lock().await;
                prune_punch_requests(&mut lock);
                let mut dup = false;
                for e in lock.iter().rev().take(30) {
                    // only check recent tail subset for speed
                    if e.from_ip == from_ip && e.to_id == to_id_clone {
                        if e.tm.elapsed().as_secs() < PUNCH_REQ_DEDUPE_SEC {
                            dup = true;
                        }
                        break;
                    }
                }
                if !dup {
                    lock.push(PunchReqEntry {
                        tm: Instant::now(),
                        from_ip,
                        to_ip,
                        to_id: to_id_clone,
                    });
                    prune_punch_requests(&mut lock);
                }
            }

            let mut msg_out = RendezvousMessage::new();
            let peer_is_lan = self.is_lan(peer_addr);
            let is_lan = self.is_lan(addr);
            let mut relay_server = self.get_relay_server(addr.ip(), peer_addr.ip());
            if ALWAYS_USE_RELAY.load(Ordering::SeqCst) || (peer_is_lan ^ is_lan) {
                if peer_is_lan {
                    // https://github.com/rustdesk/rustdesk-server/issues/24
                    relay_server = self.inner.local_ip.clone()
                }
                ph.nat_type = NatType::SYMMETRIC.into(); // will force relay
            }
            let same_intranet: bool = !ws
                && (peer_is_lan && is_lan || {
                    match (peer_addr, addr) {
                        (SocketAddr::V4(a), SocketAddr::V4(b)) => a.ip() == b.ip(),
                        (SocketAddr::V6(a), SocketAddr::V6(b)) => a.ip() == b.ip(),
                        _ => false,
                    }
                });
            let socket_addr = AddrMangle::encode(addr).into();
            if same_intranet {
                log::debug!(
                    "Fetch local addr {:?} {:?} request from {:?}",
                    id,
                    peer_addr,
                    addr
                );
                msg_out.set_fetch_local_addr(FetchLocalAddr {
                    socket_addr,
                    relay_server,
                    ..Default::default()
                });
            } else {
                log::debug!(
                    "Punch hole {:?} {:?} request from {:?}",
                    id,
                    peer_addr,
                    addr
                );
                msg_out.set_punch_hole(PunchHole {
                    socket_addr,
                    nat_type: ph.nat_type,
                    relay_server,
                    ..Default::default()
                });
            }
            Ok((msg_out, Some(peer_addr)))
        } else {
            let mut msg_out = RendezvousMessage::new();
            msg_out.set_punch_hole_response(PunchHoleResponse {
                failure: punch_hole_response::Failure::ID_NOT_EXIST.into(),
                ..Default::default()
            });
            Ok((msg_out, None))
        }
    }

    #[inline]
    async fn handle_online_request(
        &mut self,
        stream: &mut FramedStream,
        peers: Vec<String>,
    ) -> ResultType<()> {
        let peer_lookup_limit = clamped_online_request_peer_count(peers.len());
        if peer_lookup_limit < peers.len() {
            crate::common::record_protection_event("online_request_peer_limit_hits");
            log::warn!(
                "Capping online request lookup from {} to {} peers",
                peers.len(),
                peer_lookup_limit
            );
        }
        let mut states = BytesMut::zeroed((peers.len() + 7) / 8);
        for (i, peer_id) in peers.iter().take(peer_lookup_limit).enumerate() {
            if let Some(peer) = self.pm.get_in_memory(peer_id).await {
                let elapsed = peer.read().await.last_reg_time.elapsed().as_millis() as i32;
                // bytes index from left to right
                let states_idx = i / 8;
                let bit_idx = 7 - i % 8;
                if elapsed < REG_TIMEOUT {
                    states[states_idx] |= 0x01 << bit_idx;
                }
            }
        }

        let mut msg_out = RendezvousMessage::new();
        msg_out.set_online_response(OnlineResponse {
            states: states.into(),
            ..Default::default()
        });
        stream.send(&msg_out).await?;

        Ok(())
    }

    #[inline]
    async fn send_to_tcp(&mut self, msg: RendezvousMessage, addr: SocketAddr) {
        let mut tcp = {
            let mut lock = self.tcp_punch.lock().await;
            prune_tcp_punch_entries(&mut lock);
            lock.remove(&try_into_v4(addr)).map(|entry| entry.sink)
        };
        tokio::spawn(async move {
            Self::send_to_sink(&mut tcp, msg).await;
        });
    }

    #[inline]
    async fn send_to_sink(sink: &mut Option<Sink>, msg: RendezvousMessage) {
        if let Some(sink) = sink.as_mut() {
            if let Ok(bytes) = msg.write_to_bytes() {
                match sink {
                    Sink::TcpStream(s) => {
                        allow_err!(s.send(Bytes::from(bytes)).await);
                    }
                    Sink::Ws(ws) => {
                        allow_err!(ws.send(tungstenite::Message::Binary(bytes)).await);
                    }
                }
            }
        }
    }

    #[inline]
    async fn send_to_tcp_sync(
        &mut self,
        msg: RendezvousMessage,
        addr: SocketAddr,
    ) -> ResultType<()> {
        let mut sink = {
            let mut lock = self.tcp_punch.lock().await;
            prune_tcp_punch_entries(&mut lock);
            lock.remove(&try_into_v4(addr)).map(|entry| entry.sink)
        };
        Self::send_to_sink(&mut sink, msg).await;
        Ok(())
    }

    #[inline]
    async fn handle_tcp_punch_hole_request(
        &mut self,
        addr: SocketAddr,
        ph: PunchHoleRequest,
        key: &str,
        ws: bool,
    ) -> ResultType<()> {
        let (msg, to_addr) = self.handle_punch_hole_request(addr, ph, key, ws).await?;
        if let Some(addr) = to_addr {
            self.tx.send(Data::Msg(msg.into(), addr))?;
        } else {
            self.send_to_tcp_sync(msg, addr).await?;
        }
        Ok(())
    }

    #[inline]
    async fn handle_udp_punch_hole_request(
        &mut self,
        addr: SocketAddr,
        ph: PunchHoleRequest,
        key: &str,
    ) -> ResultType<()> {
        let (msg, to_addr) = self.handle_punch_hole_request(addr, ph, key, false).await?;
        self.tx.send(Data::Msg(
            msg.into(),
            match to_addr {
                Some(addr) => addr,
                None => addr,
            },
        ))?;
        Ok(())
    }

    async fn check_ip_blocker(&self, ip: &str, id: &str) -> bool {
        let mut lock = IP_BLOCKER.lock().await;
        allow_ip_registration_attempt(&mut lock, ip, id)
    }

    fn parse_relay_servers(&mut self, relay_servers: &str) {
        let rs = get_servers(relay_servers, "relay-servers");
        self.relay_servers0 = Arc::new(rs);
        self.relay_servers = self.relay_servers0.clone();
    }

    fn get_relay_server(&self, _pa: IpAddr, _pb: IpAddr) -> String {
        if self.relay_servers.is_empty() {
            return "".to_owned();
        } else if self.relay_servers.len() == 1 {
            return self.relay_servers[0].clone();
        }
        let i = ROTATION_RELAY_SERVER.fetch_add(1, Ordering::SeqCst) % self.relay_servers.len();
        self.relay_servers[i].clone()
    }

    async fn check_cmd(&self, cmd: &str) -> String {
        use std::fmt::Write as _;

        let mut res = "".to_owned();
        let mut fds = cmd.trim().split(' ');
        match fds.next() {
            Some("h") => {
                res = format!(
                    "{}\n{}\n{}\n{}\n{}\n{}\n{}\n{}\n",
                    "relay-servers(rs) <separated by ,>",
                    "reload-geo(rg)",
                    "ip-blocker(ib) [<ip>|<number>] [-]",
                    "ip-changes(ic) [<id>|<number>] [-]",
                    "punch-requests(pr) [<number>] [-]",
                    "always-use-relay(aur)",
                    "protection-stats(ps)",
                    "test-geo(tg) <ip1> <ip2>"
                )
            }
            Some("relay-servers" | "rs") => {
                if let Some(rs) = fds.next() {
                    self.tx.send(Data::RelayServers0(rs.to_owned())).ok();
                } else {
                    for ip in self.relay_servers.iter() {
                        let _ = writeln!(res, "{ip}");
                    }
                }
            }
            Some("ip-blocker" | "ib") => {
                let mut lock = IP_BLOCKER.lock().await;
                prune_ip_blocker_entries(&mut lock);
                res = format!("{}\n", lock.len());
                let ip = fds.next();
                let mut start = ip.map(|x| x.parse::<i32>().unwrap_or(-1)).unwrap_or(-1);
                if start < 0 {
                    if let Some(ip) = ip {
                        if let Some((a, b)) = lock.get(ip) {
                            let _ = writeln!(
                                res,
                                "{}/{}s {}/{}s",
                                a.0,
                                a.1.elapsed().as_secs(),
                                b.0.len(),
                                b.1.elapsed().as_secs()
                            );
                        }
                        if fds.next() == Some("-") {
                            lock.remove(ip);
                        }
                    } else {
                        start = 0;
                    }
                }
                if start >= 0 {
                    let mut it = lock.iter();
                    for i in 0..(start + 10) {
                        let x = it.next();
                        if x.is_none() {
                            break;
                        }
                        if i < start {
                            continue;
                        }
                        if let Some((ip, (a, b))) = x {
                            let _ = writeln!(
                                res,
                                "{}: {}/{}s {}/{}s",
                                ip,
                                a.0,
                                a.1.elapsed().as_secs(),
                                b.0.len(),
                                b.1.elapsed().as_secs()
                            );
                        }
                    }
                }
            }
            Some("ip-changes" | "ic") => {
                let mut lock = IP_CHANGES.lock().await;
                prune_ip_change_entries(&mut lock);
                res = format!("{}\n", lock.len());
                let id = fds.next();
                let mut start = id.map(|x| x.parse::<i32>().unwrap_or(-1)).unwrap_or(-1);
                if !(0..=10_000_000).contains(&start) {
                    if let Some(id) = id {
                        if let Some((tm, ips)) = lock.get(id) {
                            let _ = writeln!(res, "{}s {:?}", tm.elapsed().as_secs(), ips);
                        }
                        if fds.next() == Some("-") {
                            lock.remove(id);
                        }
                    } else {
                        start = 0;
                    }
                }
                if start >= 0 {
                    let mut it = lock.iter();
                    for i in 0..(start + 10) {
                        let x = it.next();
                        if x.is_none() {
                            break;
                        }
                        if i < start {
                            continue;
                        }
                        if let Some((id, (tm, ips))) = x {
                            let _ = writeln!(res, "{}: {}s {:?}", id, tm.elapsed().as_secs(), ips,);
                        }
                    }
                }
            }
            Some("punch-requests" | "pr") => {
                use std::fmt::Write as _;
                let mut lock = PUNCH_REQS.lock().await;
                let arg = fds.next();
                if let Some("-") = arg {
                    lock.clear();
                } else {
                    prune_punch_requests(&mut lock);
                    let start = arg.and_then(|x| x.parse::<usize>().ok()).unwrap_or(0);
                    let mut page_size = fds
                        .next()
                        .and_then(|x| x.parse::<usize>().ok())
                        .unwrap_or(10);
                    if page_size == 0 {
                        page_size = 10;
                    }
                    for (_, e) in lock.iter().enumerate().skip(start).take(page_size) {
                        let age = e.tm.elapsed();
                        let event_system = std::time::SystemTime::now() - age;
                        let event_iso = chrono::DateTime::<chrono::Utc>::from(event_system)
                            .to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
                        let _ = writeln!(
                            res,
                            "{} {} -> {}@{}",
                            event_iso, e.from_ip, e.to_id, e.to_ip
                        );
                    }
                }
            }
            Some("always-use-relay" | "aur") => {
                if let Some(rs) = fds.next() {
                    if rs.to_uppercase() == "Y" {
                        ALWAYS_USE_RELAY.store(true, Ordering::SeqCst);
                    } else {
                        ALWAYS_USE_RELAY.store(false, Ordering::SeqCst);
                    }
                    self.tx.send(Data::RelayServers0(rs.to_owned())).ok();
                } else {
                    let _ = writeln!(
                        res,
                        "ALWAYS_USE_RELAY: {:?}",
                        ALWAYS_USE_RELAY.load(Ordering::SeqCst)
                    );
                }
            }
            Some("protection-stats" | "ps") => {
                for line in crate::common::protection_limits_summary() {
                    let _ = writeln!(res, "{line}");
                }
                let tcp_punch_entries = {
                    let mut lock = self.tcp_punch.lock().await;
                    prune_tcp_punch_entries(&mut lock);
                    lock.len()
                };
                let _ = writeln!(
                    res,
                    "tcp_punch_entry_ttl_secs={}",
                    tcp_punch_entry_ttl_secs()
                );
                let _ = writeln!(res, "max_tcp_punch_entries={}", max_tcp_punch_entries());
                let _ = writeln!(res, "tcp_punch_entries={tcp_punch_entries}");
                let _ = writeln!(res, "fanout_window_seconds={}", fanout_window_seconds());
                let _ = writeln!(
                    res,
                    "max_fanout_tracked_sources={}",
                    max_fanout_tracked_sources()
                );
                let _ = writeln!(
                    res,
                    "max_punch_targets_per_ip_per_window={}",
                    max_punch_targets_per_ip_per_window()
                );
                let _ = writeln!(
                    res,
                    "max_relay_targets_per_ip_per_window={}",
                    max_relay_targets_per_ip_per_window()
                );
                let _ = writeln!(
                    res,
                    "max_online_request_peers={}",
                    max_online_request_peers()
                );
                for (name, value) in crate::common::protection_stats_snapshot() {
                    let _ = writeln!(res, "{name}={value}");
                }
            }
            Some("test-geo" | "tg") => {
                if let Some(rs) = fds.next() {
                    if let Ok(a) = rs.parse::<IpAddr>() {
                        if let Some(rs) = fds.next() {
                            if let Ok(b) = rs.parse::<IpAddr>() {
                                res = format!("{:?}", self.get_relay_server(a, b));
                            }
                        } else {
                            res = format!("{:?}", self.get_relay_server(a, a));
                        }
                    }
                }
            }
            _ => {}
        }
        res
    }

    async fn handle_listener2(&self, stream: TcpStream, addr: SocketAddr, key: &str) {
        let mut rs = self.clone();
        let key = key.to_owned();
        let ip = try_into_v4(addr).ip();
        if ip.is_loopback() {
            tokio::spawn(async move {
                let mut stream = stream;
                let mut buffer = [0; 1024];
                if let Ok(Ok(n)) = timeout(1000, stream.read(&mut buffer[..])).await {
                    if let Ok(data) = std::str::from_utf8(&buffer[..n]) {
                        let res = rs.check_cmd(data).await;
                        stream.write(res.as_bytes()).await.ok();
                    }
                }
            });
            return;
        }
        let stream = FramedStream::from(stream, addr);
        tokio::spawn(async move {
            let mut stream = stream;
            if let Some(Ok(bytes)) = stream.next_timeout(30_000).await {
                if let Ok(msg_in) = RendezvousMessage::parse_from_bytes(&bytes) {
                    match msg_in.union {
                        Some(rendezvous_message::Union::TestNatRequest(tar)) => {
                            if !is_valid_server_key(&key, &tar.licence_key) {
                                log::warn!("Authentication failed from {} for nat probe", addr);
                                return;
                            }
                            let mut msg_out = RendezvousMessage::new();
                            msg_out.set_test_nat_response(TestNatResponse {
                                port: addr.port() as _,
                                ..Default::default()
                            });
                            stream.send(&msg_out).await.ok();
                        }
                        Some(rendezvous_message::Union::OnlineRequest(or)) => {
                            if !is_valid_server_key(&key, &or.licence_key) {
                                log::warn!("Authentication failed from {} for online query", addr);
                                return;
                            }
                            allow_err!(rs.handle_online_request(&mut stream, or.peers).await);
                        }
                        _ => {}
                    }
                }
            }
        });
    }

    async fn handle_listener(&self, stream: TcpStream, addr: SocketAddr, key: &str, ws: bool) {
        log::debug!("Tcp connection from {:?}, ws: {}", addr, ws);
        let mut rs = self.clone();
        let key = key.to_owned();
        tokio::spawn(async move {
            allow_err!(rs.handle_listener_inner(stream, addr, &key, ws).await);
        });
    }

    #[inline]
    async fn handle_listener_inner(
        &mut self,
        stream: TcpStream,
        mut addr: SocketAddr,
        key: &str,
        ws: bool,
    ) -> ResultType<()> {
        let mut sink;
        if ws {
            let peer_addr = addr;
            use tokio_tungstenite::tungstenite::handshake::server::{Request, Response};
            let callback = |req: &Request, response: Response| {
                addr = crate::common::apply_trusted_proxy_addr(addr, req.headers());
                Ok(response)
            };
            let ws_stream = tokio_tungstenite::accept_hdr_async(stream, callback).await?;
            if addr.ip() != peer_addr.ip()
                && !crate::common::allow_connection_from_ip("hbbs-ws-forwarded", addr)
            {
                log::warn!(
                    "Rate limit exceeded for hbbs-ws-forwarded from {}",
                    addr.ip()
                );
                return Ok(());
            }
            let (a, mut b) = ws_stream.split();
            sink = Some(Sink::Ws(a));
            while let Ok(Some(Ok(msg))) = timeout(30_000, b.next()).await {
                if let tungstenite::Message::Binary(bytes) = msg {
                    if !self.handle_tcp(&bytes, &mut sink, addr, key, ws).await {
                        break;
                    }
                }
            }
        } else {
            let (a, mut b) = Framed::new(stream, BytesCodec::new()).split();
            sink = Some(Sink::TcpStream(a));
            while let Ok(Some(Ok(bytes))) = timeout(30_000, b.next()).await {
                if !self.handle_tcp(&bytes, &mut sink, addr, key, ws).await {
                    break;
                }
            }
        }
        if sink.is_none() {
            self.tcp_punch.lock().await.remove(&try_into_v4(addr));
        }
        log::debug!("Tcp connection from {:?} closed", addr);
        Ok(())
    }

    #[inline]
    async fn get_pk(&mut self, version: &str, id: String) -> Bytes {
        if version.is_empty() || self.inner.sk.is_none() {
            Bytes::new()
        } else {
            match self.pm.get(&id).await {
                Some(peer) => {
                    let pk = peer.read().await.pk.clone();
                    sign::sign(
                        &hbb_common::message_proto::IdPk {
                            id,
                            pk,
                            ..Default::default()
                        }
                        .write_to_bytes()
                        .unwrap_or_default(),
                        self.inner.sk.as_ref().unwrap(),
                    )
                    .into()
                }
                _ => Bytes::new(),
            }
        }
    }

    #[inline]
    fn get_server_sk(key: &str) -> (String, Option<sign::SecretKey>) {
        let mut out_sk = None;
        let mut key = key.to_owned();
        if let Ok(sk) = base64::decode(&key) {
            if sk.len() == sign::SECRETKEYBYTES {
                log::info!("The key is a crypto private key");
                key = base64::encode(&sk[(sign::SECRETKEYBYTES / 2)..]);
                let mut tmp = [0u8; sign::SECRETKEYBYTES];
                tmp[..].copy_from_slice(&sk);
                out_sk = Some(sign::SecretKey(tmp));
            }
        }

        if key.is_empty() || key == "-" || key == "_" {
            let (pk, sk) = crate::common::gen_sk(0);
            out_sk = sk;
            if !key.is_empty() {
                key = pk;
            }
        }

        if !key.is_empty() {
            log::info!("Key: {}", key);
        }
        (key, out_sk)
    }

    #[inline]
    fn is_lan(&self, addr: SocketAddr) -> bool {
        if let Some(network) = &self.inner.mask {
            match addr {
                SocketAddr::V4(v4_socket_addr) => {
                    return network.contains(*v4_socket_addr.ip());
                }

                SocketAddr::V6(v6_socket_addr) => {
                    if let Some(v4_addr) = v6_socket_addr.ip().to_ipv4() {
                        return network.contains(v4_addr);
                    }
                }
            }
        }
        false
    }
}

async fn check_relay_servers(rs0: Arc<RelayServers>, tx: Sender) {
    let mut futs = Vec::new();
    let rs = Arc::new(Mutex::new(Vec::new()));
    for x in rs0.iter() {
        let mut host = x.to_owned();
        if !host.contains(':') {
            host = format!("{}:{}", host, config::RELAY_PORT);
        }
        let rs = rs.clone();
        let x = x.clone();
        futs.push(tokio::spawn(async move {
            if FramedStream::new(&host, None, CHECK_RELAY_TIMEOUT)
                .await
                .is_ok()
            {
                rs.lock().await.push(x);
            }
        }));
    }
    join_all(futs).await;
    log::debug!("check_relay_servers");
    let rs = std::mem::take(&mut *rs.lock().await);
    if !rs.is_empty() {
        tx.send(Data::RelayServers(rs)).ok();
    }
}

// temp solution to solve udp socket failure
async fn test_hbbs(addr: SocketAddr, key: String) -> ResultType<()> {
    let mut addr = addr;
    if addr.ip().is_unspecified() {
        addr.set_ip(if addr.is_ipv4() {
            IpAddr::V4(Ipv4Addr::LOCALHOST)
        } else {
            IpAddr::V6(Ipv6Addr::LOCALHOST)
        });
    }

    let mut socket = FramedSocket::new(config::Config::get_any_listen_addr(addr.is_ipv4())).await?;
    let mut msg_out = RendezvousMessage::new();
    msg_out.set_register_peer(RegisterPeer {
        id: "(:test_hbbs:)".to_owned(),
        licence_key: key,
        ..Default::default()
    });
    let mut last_time_recv = Instant::now();

    let mut timer = interval(Duration::from_secs(1));
    loop {
        tokio::select! {
          _ = timer.tick() => {
              if last_time_recv.elapsed().as_secs() > 12 {
                  bail!("Timeout of test_hbbs");
              }
              socket.send(&msg_out, addr).await?;
          }
          Some(Ok((bytes, _))) = socket.next() => {
              if let Ok(msg_in) = RendezvousMessage::parse_from_bytes(&bytes) {
                 log::trace!("Recv {:?} of test_hbbs", msg_in);
                 last_time_recv = Instant::now();
              }
          }
        }
    }
}

fn prune_punch_requests(entries: &mut Vec<PunchReqEntry>) {
    let before = entries.len();
    entries.retain(|entry| entry.tm.elapsed().as_secs() < PUNCH_REQ_RETENTION_SECS);
    if entries.len() > MAX_PUNCH_REQS {
        let excess = entries.len() - MAX_PUNCH_REQS;
        entries.drain(0..excess);
    }
    if before > entries.len() {
        crate::common::record_protection_event("punch_requests_pruned");
    }
}

fn fanout_window_seconds() -> usize {
    std::env::var(FANOUT_WINDOW_SECONDS_ENV)
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(DEFAULT_FANOUT_WINDOW_SECONDS)
}

fn max_fanout_tracked_sources() -> usize {
    std::env::var(MAX_FANOUT_TRACKED_SOURCES_ENV)
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(DEFAULT_MAX_FANOUT_TRACKED_SOURCES)
}

fn max_punch_targets_per_ip_per_window() -> usize {
    std::env::var(MAX_PUNCH_TARGETS_PER_IP_PER_WINDOW_ENV)
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(DEFAULT_MAX_PUNCH_TARGETS_PER_IP_PER_WINDOW)
}

fn max_relay_targets_per_ip_per_window() -> usize {
    std::env::var(MAX_RELAY_TARGETS_PER_IP_PER_WINDOW_ENV)
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(DEFAULT_MAX_RELAY_TARGETS_PER_IP_PER_WINDOW)
}

fn tcp_punch_entry_ttl_secs() -> usize {
    std::env::var(TCP_PUNCH_ENTRY_TTL_SECS_ENV)
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(DEFAULT_TCP_PUNCH_ENTRY_TTL_SECS)
}

fn max_tcp_punch_entries() -> usize {
    std::env::var(MAX_TCP_PUNCH_ENTRIES_ENV)
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(DEFAULT_MAX_TCP_PUNCH_ENTRIES)
}

fn prune_tcp_punch_entries(entries: &mut HashMap<SocketAddr, TcpPunchEntry>) {
    let before = entries.len();
    let ttl_secs = tcp_punch_entry_ttl_secs() as u64;
    entries.retain(|_, entry| entry.created_at.elapsed().as_secs() < ttl_secs);
    if before > entries.len() {
        crate::common::record_protection_event("tcp_punch_entries_pruned");
    }
}

fn evict_oldest_tcp_punch_entry(entries: &mut HashMap<SocketAddr, TcpPunchEntry>) -> bool {
    let oldest_addr = entries
        .iter()
        .min_by_key(|(_, entry)| entry.created_at)
        .map(|(addr, _)| *addr);
    if let Some(addr) = oldest_addr {
        entries.remove(&addr);
        return true;
    }
    false
}

fn insert_tcp_punch_entry(
    entries: &mut HashMap<SocketAddr, TcpPunchEntry>,
    addr: SocketAddr,
    sink: Sink,
) -> Option<Sink> {
    prune_tcp_punch_entries(entries);
    if !entries.contains_key(&addr) {
        let max_entries = max_tcp_punch_entries();
        if max_entries > 0 && entries.len() >= max_entries && evict_oldest_tcp_punch_entry(entries)
        {
            crate::common::record_protection_event("tcp_punch_entries_evicted");
        }
        if max_entries > 0 && entries.len() >= max_entries {
            crate::common::record_protection_event("tcp_punch_entries_rejected");
            return Some(sink);
        }
    }
    entries.insert(
        addr,
        TcpPunchEntry {
            sink,
            created_at: Instant::now(),
        },
    );
    None
}

fn prune_target_fanout(entries: &mut FanoutMap) {
    let window_secs = fanout_window_seconds() as u64;
    entries
        .retain(|_, entry| entry.last_seen_at.elapsed().as_secs() < window_secs.saturating_mul(2));
}

fn evict_oldest_target_fanout(entries: &mut FanoutMap) -> bool {
    let oldest_ip = entries
        .iter()
        .min_by_key(|(_, entry)| entry.last_seen_at)
        .map(|(ip, _)| ip.clone());
    if let Some(ip) = oldest_ip {
        entries.remove(&ip);
        return true;
    }
    false
}

fn allow_target_fanout(
    entries: &mut FanoutMap,
    source_ip: &str,
    target_id: &str,
    max_targets_per_window: usize,
    evicted_event: &'static str,
    rejected_event: &'static str,
) -> bool {
    let now = Instant::now();
    let window_secs = fanout_window_seconds() as u64;
    prune_target_fanout(entries);
    if let Some(entry) = entries.get_mut(source_ip) {
        if now.duration_since(entry.window_started_at).as_secs() >= window_secs {
            entry.window_started_at = now;
            entry.targets.clear();
        }
        entry.last_seen_at = now;
        if entry.targets.contains(target_id) {
            return true;
        }
        if max_targets_per_window > 0 && entry.targets.len() >= max_targets_per_window {
            return false;
        }
        entry.targets.insert(target_id.to_owned());
        return true;
    }

    let max_entries = max_fanout_tracked_sources();
    if max_entries > 0 && entries.len() >= max_entries && evict_oldest_target_fanout(entries) {
        crate::common::record_protection_event(evicted_event);
    }
    if max_entries > 0 && entries.len() >= max_entries {
        crate::common::record_protection_event(rejected_event);
        return false;
    }

    entries.insert(
        source_ip.to_owned(),
        FanoutEntry {
            window_started_at: now,
            last_seen_at: now,
            targets: HashSet::from([target_id.to_owned()]),
        },
    );
    true
}

fn max_online_request_peers() -> usize {
    std::env::var(MAX_ONLINE_REQUEST_PEERS_ENV)
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(DEFAULT_MAX_ONLINE_REQUEST_PEERS)
}

fn clamped_online_request_peer_count(total_peers: usize) -> usize {
    total_peers.min(max_online_request_peers())
}

#[inline]
fn is_valid_registration_id(id: &str) -> bool {
    let len = id.chars().count();
    (MIN_REGISTRATION_ID_LEN..=MAX_REGISTRATION_ID_LEN).contains(&len)
}

#[inline]
fn is_valid_server_key(configured_key: &str, supplied_key: &str) -> bool {
    configured_key.is_empty() || supplied_key == configured_key
}

#[cfg(test)]
mod tests {
    use super::{
        allow_target_fanout, clamped_online_request_peer_count, is_valid_registration_id,
        is_valid_server_key, prune_punch_requests, FanoutMap, PunchReqEntry,
        DEFAULT_MAX_ONLINE_REQUEST_PEERS, FANOUT_WINDOW_SECONDS_ENV,
        MAX_FANOUT_TRACKED_SOURCES_ENV, MAX_ONLINE_REQUEST_PEERS_ENV, MAX_PUNCH_REQS,
        PUNCH_REQ_RETENTION_SECS,
    };
    use std::{
        collections::HashMap,
        time::{Duration, Instant},
    };

    #[test]
    fn server_key_validation_accepts_open_server_or_matching_key() {
        assert!(is_valid_server_key("", ""));
        assert!(is_valid_server_key("", "anything"));
        assert!(is_valid_server_key("shared-secret", "shared-secret"));
    }

    #[test]
    fn server_key_validation_rejects_mismatched_key() {
        assert!(!is_valid_server_key("shared-secret", ""));
        assert!(!is_valid_server_key("shared-secret", "wrong"));
    }

    #[test]
    fn registration_id_validation_enforces_basic_length_bounds() {
        assert!(!is_valid_registration_id(""));
        assert!(!is_valid_registration_id("12345"));
        assert!(is_valid_registration_id("123456"));
        assert!(is_valid_registration_id(&"a".repeat(100)));
        assert!(!is_valid_registration_id(&"a".repeat(101)));
    }

    #[test]
    fn prune_punch_requests_removes_old_entries_and_caps_growth() {
        let now = Instant::now();
        let old = now
            .checked_sub(Duration::from_secs(PUNCH_REQ_RETENTION_SECS + 1))
            .unwrap_or(now);
        let mut entries = vec![PunchReqEntry {
            tm: old,
            from_ip: "192.0.2.10".to_owned(),
            to_ip: "198.51.100.10".to_owned(),
            to_id: "old".to_owned(),
        }];
        for i in 0..(MAX_PUNCH_REQS + 5) {
            entries.push(PunchReqEntry {
                tm: now,
                from_ip: format!("192.0.2.{i}"),
                to_ip: "198.51.100.10".to_owned(),
                to_id: format!("peer-{i}"),
            });
        }

        prune_punch_requests(&mut entries);

        assert_eq!(entries.len(), MAX_PUNCH_REQS);
        assert!(entries
            .iter()
            .all(|entry| entry.tm.elapsed().as_secs() < PUNCH_REQ_RETENTION_SECS));
        assert_eq!(
            entries.first().map(|entry| entry.to_id.as_str()),
            Some("peer-5")
        );
    }

    #[test]
    fn online_request_peer_lookup_count_is_bounded() {
        std::env::remove_var(MAX_ONLINE_REQUEST_PEERS_ENV);
        assert_eq!(
            clamped_online_request_peer_count(DEFAULT_MAX_ONLINE_REQUEST_PEERS + 1),
            DEFAULT_MAX_ONLINE_REQUEST_PEERS
        );

        std::env::set_var(MAX_ONLINE_REQUEST_PEERS_ENV, "3");
        assert_eq!(clamped_online_request_peer_count(2), 2);
        assert_eq!(clamped_online_request_peer_count(3), 3);
        assert_eq!(clamped_online_request_peer_count(4), 3);
        std::env::remove_var(MAX_ONLINE_REQUEST_PEERS_ENV);
    }

    #[test]
    fn target_fanout_caps_distinct_targets_but_allows_repeats() {
        std::env::set_var(FANOUT_WINDOW_SECONDS_ENV, "60");
        let mut entries: FanoutMap = HashMap::new();

        assert!(allow_target_fanout(
            &mut entries,
            "198.51.100.10",
            "peer-a",
            2,
            "fanout_evicted",
            "fanout_rejected"
        ));
        assert!(allow_target_fanout(
            &mut entries,
            "198.51.100.10",
            "peer-a",
            2,
            "fanout_evicted",
            "fanout_rejected"
        ));
        assert!(allow_target_fanout(
            &mut entries,
            "198.51.100.10",
            "peer-b",
            2,
            "fanout_evicted",
            "fanout_rejected"
        ));
        assert!(!allow_target_fanout(
            &mut entries,
            "198.51.100.10",
            "peer-c",
            2,
            "fanout_evicted",
            "fanout_rejected"
        ));

        std::env::remove_var(FANOUT_WINDOW_SECONDS_ENV);
    }

    #[test]
    fn target_fanout_tracked_sources_are_bounded() {
        std::env::set_var(FANOUT_WINDOW_SECONDS_ENV, "60");
        std::env::set_var(MAX_FANOUT_TRACKED_SOURCES_ENV, "2");
        let now = Instant::now();
        let older = now.checked_sub(Duration::from_secs(10)).unwrap_or(now);
        let newer = now.checked_sub(Duration::from_secs(1)).unwrap_or(now);
        let mut entries: FanoutMap = HashMap::from([
            (
                "198.51.100.1".to_owned(),
                super::FanoutEntry {
                    window_started_at: older,
                    last_seen_at: older,
                    targets: std::collections::HashSet::from(["peer-a".to_owned()]),
                },
            ),
            (
                "198.51.100.2".to_owned(),
                super::FanoutEntry {
                    window_started_at: newer,
                    last_seen_at: newer,
                    targets: std::collections::HashSet::from(["peer-b".to_owned()]),
                },
            ),
        ]);

        assert!(allow_target_fanout(
            &mut entries,
            "198.51.100.3",
            "peer-c",
            2,
            "fanout_evicted",
            "fanout_rejected"
        ));
        assert_eq!(entries.len(), 2);
        assert!(!entries.contains_key("198.51.100.1"));
        assert!(entries.contains_key("198.51.100.2"));
        assert!(entries.contains_key("198.51.100.3"));

        std::env::remove_var(FANOUT_WINDOW_SECONDS_ENV);
        std::env::remove_var(MAX_FANOUT_TRACKED_SOURCES_ENV);
    }
}

#[inline]
async fn send_rk_res(
    socket: &mut FramedSocket,
    addr: SocketAddr,
    res: register_pk_response::Result,
) -> ResultType<()> {
    let mut msg_out = RendezvousMessage::new();
    msg_out.set_register_pk_response(RegisterPkResponse {
        result: res.into(),
        ..Default::default()
    });
    socket.send(&msg_out, addr).await
}

async fn create_udp_listener(port: i32, rmem: usize) -> ResultType<FramedSocket> {
    let addr = SocketAddr::new(IpAddr::V6(Ipv6Addr::UNSPECIFIED), port as _);
    if let Ok(s) = FramedSocket::new_reuse(&addr, true, rmem).await {
        log::debug!("listen on udp {:?}", s.local_addr());
        return Ok(s);
    }
    let addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::UNSPECIFIED), port as _);
    let s = FramedSocket::new_reuse(&addr, true, rmem).await?;
    log::debug!("listen on udp {:?}", s.local_addr());
    Ok(s)
}

#[inline]
async fn create_tcp_listener(port: i32) -> ResultType<TcpListener> {
    let s = listen_any(port as _).await?;
    log::debug!("listen on tcp {:?}", s.local_addr());
    Ok(s)
}
