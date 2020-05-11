use hbb_common::{
    allow_err,
    bytes::Bytes,
    bytes::BytesMut,
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
use std::{
    collections::HashMap,
    net::SocketAddr,
    sync::{Arc, Mutex},
    time::Instant,
};

pub struct Peer {
    socket_addr: SocketAddr,
    last_reg_time: Instant,
}

type PeerMap = HashMap<String, Peer>;
const REG_TIMEOUT: i32 = 30_000;

pub struct RendezvousServer {
    peer_map: PeerMap,
    tcp_punch: Arc<Mutex<HashMap<SocketAddr, SplitSink<Framed<TcpStream, BytesCodec>, Bytes>>>>,
}

impl RendezvousServer {
    pub async fn start(addr: &str) -> ResultType<()> {
        let mut socket = FramedSocket::new(addr).await?;
        let mut rs = Self {
            peer_map: PeerMap::new(),
            tcp_punch: Arc::new(Mutex::new(HashMap::new())),
        };
        let (tx, mut rx) = mpsc::unbounded_channel::<(SocketAddr, String)>();
        let mut listener = new_listener(addr, true).await.unwrap();
        loop {
            tokio::select! {
                Some((addr, id)) = rx.recv() => {
                    allow_err!(rs.handle_punch_hole_request(addr, &id, &mut socket).await);
                }
                Some(Ok((bytes, addr))) = socket.next() => {
                    allow_err!(rs.handle_msg(&bytes, addr, &mut socket).await);
                }
                Ok((stream, addr)) = listener.accept() => {
                    log::debug!("Tcp connection from {:?}", addr);
                    let (a, mut b) = Framed::new(stream, BytesCodec::new()).split();
                    let tcp_punch = rs.tcp_punch.clone();
                    tcp_punch.lock().unwrap().insert(addr, a);
                    let tx = tx.clone();
                    tokio::spawn(async move {
                        while let Some(Ok(bytes)) = b.next().await {
                            if let Ok(msg_in) = parse_from_bytes::<RendezvousMessage>(&bytes) {
                                match msg_in.union {
                                    Some(rendezvous_message::Union::punch_hole_request(ph)) => {
                                        tx.send((addr, ph.id)).ok();
                                    }
                                    _ => {}
                                }
                            }
                        }
                        tcp_punch.lock().unwrap().remove(&addr);
                        log::debug!("Tcp connection from {:?} closed", addr);
                    });
                }
            }
        }
    }

    pub async fn handle_msg(
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
                        self.peer_map.insert(
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
                    self.handle_punch_hole_request(addr, &ph.id, socket).await?;
                }
                Some(rendezvous_message::Union::punch_hole_sent(phs)) => {
                    // punch hole sent from B, tell A that B is ready to be connected
                    let addr_a = AddrMangle::decode(&phs.socket_addr);
                    log::debug!("Punch hole response to {:?} from {:?}", &addr_a, &addr);
                    let mut msg_out = RendezvousMessage::new();
                    msg_out.set_punch_hole_response(PunchHoleResponse {
                        socket_addr: AddrMangle::encode(addr),
                        ..Default::default()
                    });
                    socket.send(&msg_out, addr_a).await?;
                    self.send_to_tcp(&msg_out, addr_a).await?;
                }
                Some(rendezvous_message::Union::local_addr(la)) => {
                    // forward local addrs of B to A
                    let addr_a = AddrMangle::decode(&la.socket_addr);
                    log::debug!("Local addrs response to {:?} from {:?}", &addr_a, &addr);
                    let mut msg_out = RendezvousMessage::new();
                    msg_out.set_punch_hole_response(PunchHoleResponse {
                        socket_addr: la.local_addr,
                        ..Default::default()
                    });
                    socket.send(&msg_out, addr_a).await?;
                    self.send_to_tcp(&msg_out, addr_a).await?;
                }
                _ => {}
            }
        }
        Ok(())
    }

    async fn handle_punch_hole_request(
        &mut self,
        addr: SocketAddr,
        id: &str,
        socket: &mut FramedSocket,
    ) -> ResultType<()> {
        // punch hole request from A, forward to B,
        // check if in same intranet first,
        // fetch local addrs if in same intranet.
        // because punch hole won't work if in the same intranet,
        // all routers will drop such self-connections.
        if let Some(peer) = self.peer_map.get(id) {
            if peer.last_reg_time.elapsed().as_millis() as i32 >= REG_TIMEOUT {
                let mut msg_out = RendezvousMessage::new();
                msg_out.set_punch_hole_response(PunchHoleResponse {
                    failure: punch_hole_response::Failure::OFFLINE.into(),
                    ..Default::default()
                });
                return socket.send(&msg_out, addr).await;
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
            socket.send(&msg_out, peer.socket_addr).await?;
        } else {
            let mut msg_out = RendezvousMessage::new();
            msg_out.set_punch_hole_response(PunchHoleResponse {
                failure: punch_hole_response::Failure::ID_NOT_EXIST.into(),
                ..Default::default()
            });
            socket.send(&msg_out, addr).await?
        }
        Ok(())
    }

    async fn send_to_tcp(&mut self, msg: &RendezvousMessage, addr: SocketAddr) -> ResultType<()> {
        let tcp = self.tcp_punch.lock().unwrap().remove(&addr);
        if let Some(mut tcp) = tcp {
            if let Ok(bytes) = msg.write_to_bytes() {
                tokio::spawn(async move {
                    allow_err!(tcp.send(Bytes::from(bytes)).await);
                    log::debug!("Send punch hole to {} via tcp", addr);
                });
            }
        }
        Ok(())
    }
}
