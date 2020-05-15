use hbb_common::{
    allow_err,
    bytes::{Bytes, BytesMut},
    bytes_codec::BytesCodec,
    futures_util::{
        sink::SinkExt,
        stream::{SplitSink, StreamExt},
    },
    log,
    protobuf::{parse_from_bytes, Message as _},
    rendezvous_proto::*,
    tcp::new_listener,
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

#[derive(Clone)]
struct Peer {
    socket_addr: SocketAddr,
    last_reg_time: Instant,
}

impl Default for Peer {
    fn default() -> Self {
        Self {
            socket_addr: "0.0.0.0:0".parse().unwrap(),
            last_reg_time: Instant::now()
                .checked_sub(std::time::Duration::from_secs(3600))
                .unwrap(),
        }
    }
}

#[derive(Debug, Serialize, Deserialize, Default)]
struct PeerSerde {
    #[serde(default)]
    ip: String,
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
    fn insert(&mut self, key: String, peer: Peer) {
        let ip = peer.socket_addr.ip();
        if self
            .map
            .write()
            .unwrap()
            .insert(key.clone(), peer)
            .is_none()
        {
            let ip = ip.to_string();
            self.db.insert(key, PeerSerde { ip });
        }
    }

    #[inline]
    async fn get(&mut self, key: String) -> Option<Peer> {
        let p = self.map.read().unwrap().get(&key).map(|x| x.clone());
        if p.is_some() {
            return p;
        } else {
            if let Some(_) = self.db.get(key).await {
                return Some(Peer::default());
            }
        }
        None
    }

    #[inline]
    fn is_in_memory(&self, key: &str) -> bool {
        self.map.read().unwrap().contains_key(key)
    }
}

const REG_TIMEOUT: i32 = 30_000;
type Sink = SplitSink<Framed<TcpStream, BytesCodec>, Bytes>;

#[derive(Clone)]
pub struct RendezvousServer {
    tcp_punch: Arc<Mutex<HashMap<SocketAddr, Sink>>>,
    pm: PeerMap,
    tx: mpsc::UnboundedSender<(RendezvousMessage, SocketAddr)>,
}

impl RendezvousServer {
    pub async fn start(addr: &str) -> ResultType<()> {
        let mut socket = FramedSocket::new(addr).await?;
        let (tx, mut rx) = mpsc::unbounded_channel::<(RendezvousMessage, SocketAddr)>();
        let mut rs = Self {
            tcp_punch: Arc::new(Mutex::new(HashMap::new())),
            pm: PeerMap::new()?,
            tx: tx.clone(),
        };
        let mut listener = new_listener(addr, true).await?;
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
                    tcp_punch.lock().unwrap().insert(addr, a);
                    let mut rs = rs.clone();
                    tokio::spawn(async move {
                        while let Some(Ok(bytes)) = b.next().await {
                            if let Ok(msg_in) = parse_from_bytes::<RendezvousMessage>(&bytes) {
                                match msg_in.union {
                                    Some(rendezvous_message::Union::punch_hole_request(ph)) => {
                                        allow_err!(rs.handle_tcp_punch_hole_request(addr, ph.id).await);
                                    }
                                    Some(rendezvous_message::Union::punch_hole_sent(phs)) => {
                                        allow_err!(rs.handle_hole_sent(&phs, addr, None).await);
                                        break;
                                    }
                                    Some(rendezvous_message::Union::local_addr(la)) => {
                                        allow_err!(rs.handle_local_addr(&la, addr, None).await);
                                        break;
                                    }
                                    _ => {}
                                }
                            }
                        }
                        rs.tcp_punch.lock().unwrap().remove(&addr);
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
        if let Ok(msg_in) = parse_from_bytes::<RendezvousMessage>(&bytes) {
            match msg_in.union {
                Some(rendezvous_message::Union::register_peer(rp)) => {
                    // B registered
                    if rp.id.len() > 0 {
                        log::debug!("New peer registered: {:?} {:?}", &rp.id, &addr);
                        self.pm.insert(
                            rp.id,
                            Peer {
                                socket_addr: addr,
                                last_reg_time: Instant::now(),
                            },
                        );
                        let mut msg_out = RendezvousMessage::new();
                        msg_out.set_register_peer_response(RegisterPeerResponse::default());
                        socket.send(&msg_out, addr).await?
                    }
                }
                Some(rendezvous_message::Union::punch_hole_request(ph)) => {
                    let id = ph.id;
                    if self.pm.is_in_memory(&id) {
                        self.handle_udp_punch_hole_request(addr, id).await?;
                    } else {
                        let mut me = self.clone();
                        tokio::spawn(async move {
                            allow_err!(me.handle_udp_punch_hole_request(addr, id).await);
                        });
                    }
                }
                Some(rendezvous_message::Union::punch_hole_sent(phs)) => {
                    self.handle_hole_sent(&phs, addr, Some(socket)).await?;
                }
                Some(rendezvous_message::Union::local_addr(la)) => {
                    self.handle_local_addr(&la, addr, Some(socket)).await?;
                }
                _ => {}
            }
        }
        Ok(())
    }

    #[inline]
    async fn handle_hole_sent<'a>(
        &mut self,
        phs: &PunchHoleSent,
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
        msg_out.set_punch_hole_response(PunchHoleResponse {
            socket_addr: AddrMangle::encode(addr),
            ..Default::default()
        });
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
        la: &LocalAddr,
        addr: SocketAddr,
        socket: Option<&'a mut FramedSocket>,
    ) -> ResultType<()> {
        // forward local addrs of B to A
        let addr_a = AddrMangle::decode(&la.socket_addr);
        log::debug!(
            "{} local addrs response to {:?} from {:?}",
            if socket.is_none() { "TCP" } else { "UDP" },
            &addr_a,
            &addr
        );
        let mut msg_out = RendezvousMessage::new();
        msg_out.set_punch_hole_response(PunchHoleResponse {
            socket_addr: la.local_addr.clone(),
            ..Default::default()
        });
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
        id: String,
    ) -> ResultType<(RendezvousMessage, Option<SocketAddr>)> {
        // punch hole request from A, forward to B,
        // check if in same intranet first,
        // fetch local addrs if in same intranet.
        // because punch hole won't work if in the same intranet,
        // all routers will drop such self-connections.
        if let Some(peer) = self.pm.get(id.clone()).await {
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
        id: String,
    ) -> ResultType<()> {
        let (msg, to_addr) = self.handle_punch_hole_request(addr, id).await?;
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
        id: String,
    ) -> ResultType<()> {
        let (msg, to_addr) = self.handle_punch_hole_request(addr, id).await?;
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
