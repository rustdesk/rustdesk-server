use hbb_common::{
    allow_err, bytes::BytesMut, log, protobuf::parse_from_bytes, rendezvous_proto::*,
    tcp::new_listener, tokio, udp::FramedSocket, AddrMangle, ResultType,
};
use std::{collections::HashMap, net::SocketAddr};

pub struct Peer {
    socket_addr: SocketAddr,
}

type PeerMap = HashMap<String, Peer>;

pub struct RendezvousServer {
    peer_map: PeerMap,
}

impl RendezvousServer {
    pub async fn start(addr: &str) -> ResultType<()> {
        let mut socket = FramedSocket::new(addr).await?;
        let mut rs = Self {
            peer_map: PeerMap::new(),
        };
        // tcp listener used to test if udp/tcp share the same NAT port, yes in my test.
        // also be used to help client to get local ip.
        let mut listener = new_listener(addr, true).await.unwrap();
        loop {
            tokio::select! {
                Some(Ok((bytes, addr))) = socket.next() => {
                    allow_err!(rs.handle_msg(&bytes, addr, &mut socket).await);
                }
                Ok((_, addr)) = listener.accept() => {
                    log::debug!("Tcp connection from {:?}", addr);
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
                        self.peer_map.insert(rp.id, Peer { socket_addr: addr });
                        let mut msg_out = RendezvousMessage::new();
                        msg_out.set_register_peer_response(RegisterPeerResponse::default());
                        socket.send(&msg_out, addr).await?
                    }
                }
                Some(rendezvous_message::Union::punch_hole_request(ph)) => {
                    // punch hole request from A, forward to B,
                    // check if in same intranet first,
                    // fetch local addrs if in same intranet.
                    // because punch hole won't work if in the same intranet,
                    // all routers will drop such self-connections.
                    if let Some(peer) = self.peer_map.get(&ph.id) {
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
                        let socket_addr = AddrMangle::encode(&addr);
                        if same_intranet {
                            log::debug!(
                                "Fetch local addr {:?} {:?} request from {:?}",
                                &ph.id,
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
                                &ph.id,
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
                }
                Some(rendezvous_message::Union::punch_hole_sent(phs)) => {
                    // punch hole sent from B, tell A that B is ready to be connected
                    let addr_a = AddrMangle::decode(&phs.socket_addr);
                    log::debug!("Punch hole response to {:?} from {:?}", &addr_a, &addr);
                    let mut msg_out = RendezvousMessage::new();
                    msg_out.set_punch_hole_response(PunchHoleResponse {
                        socket_addr: AddrMangle::encode(&addr),
                        ..Default::default()
                    });
                    socket.send(&msg_out, addr_a).await?;
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
                }
                _ => {}
            }
        }
        Ok(())
    }
}