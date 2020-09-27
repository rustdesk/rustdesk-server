use hbb_common::{
    allow_err,
    bytes::{Bytes, BytesMut},
    bytes_codec::BytesCodec,
    futures_util::{
        sink::SinkExt,
        stream::{SplitSink, StreamExt},
    },
    log,
    protobuf::Message as _,
    rendezvous_proto::*,
    tcp::new_listener,
    timeout,
    tokio::{self, net::TcpStream, sync::mpsc},
    tokio_util::codec::Framed,
    udp::FramedSocket,
    AddrMangle, ResultType,
};
use serde_derive::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    net::SocketAddr,
    sync::{Arc, Mutex, RwLock},
    time::Instant,
};

#[derive(Clone, Debug)]
struct Peer {
    socket_addr: SocketAddr,
    last_reg_time: Instant,
    uuid: Vec<u8>,
    pk: Vec<u8>,
}

impl Default for Peer {
    fn default() -> Self {
        Self {
            socket_addr: "0.0.0.0:0".parse().unwrap(),
            last_reg_time: Instant::now()
                .checked_sub(std::time::Duration::from_secs(3600))
                .unwrap(),
            uuid: Vec::new(),
            pk: Vec::new(),
        }
    }
}

#[derive(Debug, Serialize, Deserialize, Default)]
struct PeerSerde {
    #[serde(default)]
    ip: String,
    #[serde(default)]
    uuid: Vec<u8>,
    #[serde(default)]
    pk: Vec<u8>,
}

#[derive(Clone)]
struct PeerMap {
    map: Arc<RwLock<HashMap<String, Peer>>>,
    db: super::SledAsync,
}

impl PeerMap {
    fn new() -> ResultType<Self> {
        Ok(Self {
            map: Default::default(),
            db: super::SledAsync::new("./sled.db", true)?,
        })
    }

    #[inline]
    fn update_pk(&mut self, id: String, socket_addr: SocketAddr, uuid: Vec<u8>, pk: Vec<u8>) {
        log::info!("update_pk {} {:?} {:?} {:?}", id, socket_addr, uuid, pk);
        let mut lock = self.map.write().unwrap();
        lock.insert(
            id.clone(),
            Peer {
                socket_addr,
                last_reg_time: Instant::now(),
                uuid: uuid.clone(),
                pk: pk.clone(),
            },
        );
        drop(lock);
        let ip = socket_addr.ip().to_string();
        self.db.insert(id, PeerSerde { ip, uuid, pk });
    }

    #[inline]
    async fn get(&mut self, id: &str) -> Option<Peer> {
        let p = self.map.read().unwrap().get(id).map(|x| x.clone());
        if p.is_some() {
            return p;
        } else {
            let id = id.to_owned();
            let v = self.db.get(id.clone()).await;
            if let Some(v) = super::SledAsync::deserialize::<PeerSerde>(&v) {
                self.map.write().unwrap().insert(
                    id,
                    Peer {
                        uuid: v.uuid,
                        pk: v.pk,
                        ..Default::default()
                    },
                );
                return Some(Peer::default());
            }
        }
        None
    }

    #[inline]
    fn is_in_memory(&self, id: &str) -> bool {
        self.map.read().unwrap().contains_key(id)
    }
}

const REG_TIMEOUT: i32 = 30_000;
type Sink = SplitSink<Framed<TcpStream, BytesCodec>, Bytes>;
type Sender = mpsc::UnboundedSender<(RendezvousMessage, SocketAddr)>;

#[derive(Clone)]
pub struct RendezvousServer {
    tcp_punch: Arc<Mutex<HashMap<SocketAddr, Sink>>>,
    pm: PeerMap,
    tx: Sender,
    relay_server: String,
    serial: i32,
    rendezvous_servers: Vec<String>,
    version: String,
    software_url: String,
}

impl RendezvousServer {
    pub async fn start(
        addr: &str,
        relay_server: String,
        serial: i32,
        rendezvous_servers: Vec<String>,
        software_url: String,
    ) -> ResultType<()> {
        let mut socket = FramedSocket::new(addr).await?;
        let (tx, mut rx) = mpsc::unbounded_channel::<(RendezvousMessage, SocketAddr)>();
        let version = hbb_common::get_version_from_url(&software_url);
        if !version.is_empty() {
            log::info!("software_url: {}, version: {}", software_url, version);
        }
        let mut rs = Self {
            tcp_punch: Arc::new(Mutex::new(HashMap::new())),
            pm: PeerMap::new()?,
            tx: tx.clone(),
            relay_server,
            serial,
            rendezvous_servers,
            version,
            software_url,
        };
        let mut listener = new_listener(addr, false).await?;
        loop {
            tokio::select! {
                Some((msg, addr)) = rx.recv() => {
                    allow_err!(socket.send(&msg, addr).await);
                }
                Some(Ok((bytes, addr))) = socket.next() => {
                    allow_err!(rs.handle_msg(&bytes, addr, &mut socket).await);
                }
                Ok((stream, addr)) = listener.accept() => {
                    log::debug!("Tcp connection from {:?}", addr);
                    let (a, mut b) = Framed::new(stream, BytesCodec::new()).split();
                    let tcp_punch = rs.tcp_punch.clone();
                    let mut rs = rs.clone();
                    tokio::spawn(async move {
                        let mut sender = Some(a);
                        while let Ok(Some(Ok(bytes))) = timeout(30_000, b.next()).await {
                            if let Ok(msg_in) = RendezvousMessage::parse_from_bytes(&bytes) {
                                match msg_in.union {
                                    Some(rendezvous_message::Union::punch_hole_request(ph)) => {
                                        if let Some(sender) = sender.take() {
                                            tcp_punch.lock().unwrap().insert(addr, sender);
                                        } else {
                                            break;
                                        }
                                        allow_err!(rs.handle_tcp_punch_hole_request(addr, ph).await);
                                    }
                                    Some(rendezvous_message::Union::request_relay(mut rf)) => {
                                        if let Some(sender) = sender.take() {
                                            tcp_punch.lock().unwrap().insert(addr, sender);
                                        } else {
                                            break;
                                        }
                                        if let Some(peer) = rs.pm.map.read().unwrap().get(&rf.id).map(|x| x.clone()) {
                                            let mut msg_out = RendezvousMessage::new();
                                            rf.socket_addr = AddrMangle::encode(addr);
                                            msg_out.set_request_relay(rf);
                                            rs.tx.send((msg_out, peer.socket_addr)).ok();
                                        }
                                    }
                                    Some(rendezvous_message::Union::relay_response(mut rr)) => {
                                        let addr_b = AddrMangle::decode(&rr.socket_addr);
                                        rr.socket_addr = Default::default();
                                        let id = rr.get_id();
                                        if !id.is_empty() {
                                            if let Some(peer) = rs.pm.get(&id).await {
                                                rr.set_pk(peer.pk.clone());
                                            }
                                        }
                                        let mut msg_out = RendezvousMessage::new();
                                        msg_out.set_relay_response(rr);
                                        allow_err!(rs.send_to_tcp_sync(&msg_out, addr_b).await);
                                        break;
                                    }
                                    Some(rendezvous_message::Union::punch_hole_sent(phs)) => {
                                        allow_err!(rs.handle_hole_sent(phs, addr, None).await);
                                        break;
                                    }
                                    Some(rendezvous_message::Union::local_addr(la)) => {
                                        allow_err!(rs.handle_local_addr(la, addr, None).await);
                                        break;
                                    }
                                    Some(rendezvous_message::Union::test_nat_request(_)) => {
                                        let mut msg_out = RendezvousMessage::new();
                                        msg_out.set_test_nat_response(TestNatResponse {
                                            port: addr.port() as _,
                                            ..Default::default()
                                        });
                                        if let Some(tcp) = sender.as_mut() {
                                            if let Ok(bytes) = msg_out.write_to_bytes() {
                                                allow_err!(tcp.send(Bytes::from(bytes)).await);
                                            }
                                        }
                                        break;
                                    }
                                    _ => {
                                        break;
                                    }
                                }
                            } else {
                                break;
                            }
                        }
                        if sender.is_none() {
                            rs.tcp_punch.lock().unwrap().remove(&addr);
                        }
                        log::debug!("Tcp connection from {:?} closed", addr);
                    });
                }
            }
        }
    }

    #[inline]
    async fn handle_msg(
        &mut self,
        bytes: &BytesMut,
        addr: SocketAddr,
        socket: &mut FramedSocket,
    ) -> ResultType<()> {
        if let Ok(msg_in) = RendezvousMessage::parse_from_bytes(&bytes) {
            match msg_in.union {
                Some(rendezvous_message::Union::register_peer(rp)) => {
                    // B registered
                    if rp.id.len() > 0 {
                        log::debug!("New peer registered: {:?} {:?}", &rp.id, &addr);
                        self.update_addr(rp.id, addr, socket).await?;
                        if self.serial > rp.serial {
                            let mut msg_out = RendezvousMessage::new();
                            msg_out.set_configure_update(ConfigUpdate {
                                serial: self.serial,
                                rendezvous_servers: self.rendezvous_servers.clone(),
                                ..Default::default()
                            });
                            socket.send(&msg_out, addr).await?;
                        }
                    }
                }
                Some(rendezvous_message::Union::register_pk(rk)) => {
                    if rk.uuid.is_empty() {
                        return Ok(());
                    }
                    let id = rk.id;
                    let mut res = register_pk_response::Result::OK;
                    if let Some(peer) = self.pm.get(&id).await {
                        if !peer.uuid.is_empty() && peer.uuid != rk.uuid {
                            log::warn!(
                                "Peer {} uuid mismatch: {:?} vs {:?}",
                                id,
                                rk.uuid,
                                peer.uuid
                            );
                            res = register_pk_response::Result::UUID_MISMATCH;
                        } else if peer.uuid.is_empty() || peer.pk != rk.pk {
                            self.pm.update_pk(id, addr, rk.uuid, rk.pk);
                        }
                    } else {
                        self.pm.update_pk(id, addr, rk.uuid, rk.pk);
                    }
                    let mut msg_out = RendezvousMessage::new();
                    msg_out.set_register_pk_response(RegisterPkResponse {
                        result: res.into(),
                        ..Default::default()
                    });
                    socket.send(&msg_out, addr).await?
                }
                Some(rendezvous_message::Union::punch_hole_request(ph)) => {
                    if self.pm.is_in_memory(&ph.id) {
                        self.handle_udp_punch_hole_request(addr, ph).await?;
                    } else {
                        // not in memory, fetch from db with spawn in case blocking me
                        let mut me = self.clone();
                        tokio::spawn(async move {
                            allow_err!(me.handle_udp_punch_hole_request(addr, ph).await);
                        });
                    }
                }
                Some(rendezvous_message::Union::punch_hole_sent(phs)) => {
                    self.handle_hole_sent(phs, addr, Some(socket)).await?;
                }
                Some(rendezvous_message::Union::local_addr(la)) => {
                    self.handle_local_addr(la, addr, Some(socket)).await?;
                }
                Some(rendezvous_message::Union::configure_update(mut cu)) => {
                    if addr.ip() == std::net::IpAddr::V4(std::net::Ipv4Addr::new(127, 0, 0, 1))
                        && cu.serial > self.serial
                    {
                        self.serial = cu.serial;
                        self.rendezvous_servers = cu
                            .rendezvous_servers
                            .drain(..)
                            .filter(|x| test_if_valid_server(x).is_ok())
                            .collect();
                        log::info!(
                            "configure updated: serial={} rendezvous-servers={:?}",
                            self.serial,
                            self.rendezvous_servers
                        );
                    }
                }
                Some(rendezvous_message::Union::software_update(su)) => {
                    if !self.version.is_empty() && su.url != self.version {
                        let mut msg_out = RendezvousMessage::new();
                        msg_out.set_software_update(SoftwareUpdate {
                            url: self.software_url.clone(),
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
    async fn update_addr(
        &mut self,
        id: String,
        socket_addr: SocketAddr,
        socket: &mut FramedSocket,
    ) -> ResultType<()> {
        let mut lock = self.pm.map.write().unwrap();
        let last_reg_time = Instant::now();
        if let Some(old) = lock.get_mut(&id) {
            old.socket_addr = socket_addr;
            old.last_reg_time = last_reg_time;
            let request_pk = old.pk.is_empty();
            drop(lock);
            let mut msg_out = RendezvousMessage::new();
            msg_out.set_register_peer_response(RegisterPeerResponse {
                request_pk,
                ..Default::default()
            });
            socket.send(&msg_out, socket_addr).await?;
        } else {
            drop(lock);
            let mut pm = self.pm.clone();
            let tx = self.tx.clone();
            tokio::spawn(async move {
                let v = pm.db.get(id.clone()).await;
                let (uuid, pk) = {
                    if let Some(v) = super::SledAsync::deserialize::<PeerSerde>(&v) {
                        (v.uuid, v.pk)
                    } else {
                        (Vec::new(), Vec::new())
                    }
                };
                let mut msg_out = RendezvousMessage::new();
                msg_out.set_register_peer_response(RegisterPeerResponse {
                    request_pk: pk.is_empty(),
                    ..Default::default()
                });
                tx.send((msg_out, socket_addr)).ok();
                pm.map.write().unwrap().insert(
                    id,
                    Peer {
                        socket_addr,
                        last_reg_time,
                        uuid,
                        pk,
                    },
                );
            });
        }
        Ok(())
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
        let pk = match self.pm.get(&phs.id).await {
            Some(peer) => peer.pk,
            _ => Vec::new(),
        };
        let mut p = PunchHoleResponse {
            socket_addr: AddrMangle::encode(addr),
            pk,
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
            self.send_to_tcp(&msg_out, addr_a).await;
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
            relay_server: la.relay_server,
            ..Default::default()
        };
        p.set_is_local(true);
        msg_out.set_punch_hole_response(p);
        if let Some(socket) = socket {
            socket.send(&msg_out, addr_a).await?;
        } else {
            self.send_to_tcp(&msg_out, addr_a).await;
        }
        Ok(())
    }

    #[inline]
    async fn handle_punch_hole_request(
        &mut self,
        addr: SocketAddr,
        ph: PunchHoleRequest,
    ) -> ResultType<(RendezvousMessage, Option<SocketAddr>)> {
        let id = ph.id;
        // punch hole request from A, relay to B,
        // check if in same intranet first,
        // fetch local addrs if in same intranet.
        // because punch hole won't work if in the same intranet,
        // all routers will drop such self-connections.
        if let Some(peer) = self.pm.get(&id).await {
            if peer.last_reg_time.elapsed().as_millis() as i32 >= REG_TIMEOUT {
                let mut msg_out = RendezvousMessage::new();
                msg_out.set_punch_hole_response(PunchHoleResponse {
                    failure: punch_hole_response::Failure::OFFLINE.into(),
                    ..Default::default()
                });
                return Ok((msg_out, None));
            }
            let mut msg_out = RendezvousMessage::new();
            let same_intranet = match peer.socket_addr {
                SocketAddr::V4(a) => match addr {
                    SocketAddr::V4(b) => a.ip() == b.ip(),
                    _ => false,
                },
                SocketAddr::V6(a) => match addr {
                    SocketAddr::V6(b) => a.ip() == b.ip(),
                    _ => false,
                },
            };
            let socket_addr = AddrMangle::encode(addr);
            if same_intranet {
                log::debug!(
                    "Fetch local addr {:?} {:?} request from {:?}",
                    id,
                    &peer.socket_addr,
                    &addr
                );
                msg_out.set_fetch_local_addr(FetchLocalAddr {
                    socket_addr,
                    relay_server: self.relay_server.clone(),
                    ..Default::default()
                });
            } else {
                log::debug!(
                    "Punch hole {:?} {:?} request from {:?}",
                    id,
                    &peer.socket_addr,
                    &addr
                );
                msg_out.set_punch_hole(PunchHole {
                    socket_addr,
                    nat_type: ph.nat_type,
                    relay_server: self.relay_server.clone(),
                    ..Default::default()
                });
            }
            return Ok((msg_out, Some(peer.socket_addr)));
        } else {
            let mut msg_out = RendezvousMessage::new();
            msg_out.set_punch_hole_response(PunchHoleResponse {
                failure: punch_hole_response::Failure::ID_NOT_EXIST.into(),
                ..Default::default()
            });
            return Ok((msg_out, None));
        }
    }

    #[inline]
    async fn send_to_tcp(&mut self, msg: &RendezvousMessage, addr: SocketAddr) {
        let tcp = self.tcp_punch.lock().unwrap().remove(&addr);
        if let Some(mut tcp) = tcp {
            if let Ok(bytes) = msg.write_to_bytes() {
                tokio::spawn(async move {
                    allow_err!(tcp.send(Bytes::from(bytes)).await);
                });
            }
        }
    }

    #[inline]
    async fn send_to_tcp_sync(
        &mut self,
        msg: &RendezvousMessage,
        addr: SocketAddr,
    ) -> ResultType<()> {
        let tcp = self.tcp_punch.lock().unwrap().remove(&addr);
        if let Some(mut tcp) = tcp {
            if let Ok(bytes) = msg.write_to_bytes() {
                tcp.send(Bytes::from(bytes)).await?;
            }
        }
        Ok(())
    }

    #[inline]
    async fn handle_tcp_punch_hole_request(
        &mut self,
        addr: SocketAddr,
        ph: PunchHoleRequest,
    ) -> ResultType<()> {
        let (msg, to_addr) = self.handle_punch_hole_request(addr, ph).await?;
        if let Some(addr) = to_addr {
            self.tx.send((msg, addr))?;
        } else {
            self.send_to_tcp_sync(&msg, addr).await?;
        }
        Ok(())
    }

    #[inline]
    async fn handle_udp_punch_hole_request(
        &mut self,
        addr: SocketAddr,
        ph: PunchHoleRequest,
    ) -> ResultType<()> {
        let (msg, to_addr) = self.handle_punch_hole_request(addr, ph).await?;
        self.tx.send((
            msg,
            match to_addr {
                Some(addr) => addr,
                None => addr,
            },
        ))?;
        Ok(())
    }
}

pub fn test_if_valid_server(host: &str) -> ResultType<SocketAddr> {
    if host.contains(":") {
        hbb_common::to_socket_addr(host)
    } else {
        hbb_common::to_socket_addr(&format!("{}:{}", host, 0))
    }
}
