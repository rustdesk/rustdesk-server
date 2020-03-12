use hbb_common::{
    bytes::BytesMut, log, protobuf::parse_from_bytes, rendezvous_proto::*, udp::FramedSocket, tcp::new_listener,
    AddrMangle, ResultType,
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
        // used to test if udp/tcp share the same NAT port, yes in my test.
        // also be used to help client to get local ip.
        let addr = addr.to_string();
        hbb_common::tokio::spawn(async {
            let mut l = new_listener(addr, true).await.unwrap();
            while let Ok((_, addr)) = l.accept().await {
                log::debug!("Tcp connection from {:?}", addr);
            }
        });
        while let Some(Ok((bytes, addr))) = socket.next().await {
            rs.handle_msg(&bytes, addr, &mut socket).await?;
        }
        Ok(())
    }

    pub async fn handle_msg(
        &mut self,
        bytes: &BytesMut,
        addr: SocketAddr,
        socket: &mut FramedSocket,
    ) -> ResultType<()> {
        if let Ok(msg_in) = parse_from_bytes::<RendezvousMessage>(&bytes) {
            match msg_in.union {
                Some(RendezvousMessage_oneof_union::register_peer(rp)) => {
                    // B registered
                    if rp.hbb_addr.len() > 0 {
                        log::debug!("New peer registered: {:?} {:?}", &rp.hbb_addr, &addr);
                        self.peer_map
                            .insert(rp.hbb_addr, Peer { socket_addr: addr });
                    }
                }
                Some(RendezvousMessage_oneof_union::punch_hole_request(ph)) => {
                    // punch hole request from A, forward to B,
                    // check if in same intranet first,
                    // fetch local addrs if in same intranet.
                    // because punch hole won't work if in the same intranet,
                    // all routers will drop such self-connections.
                    if let Some(peer) = self.peer_map.get(&ph.hbb_addr) {
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
                                "Fetch local addrs {:?} {:?} request from {:?}",
                                &ph.hbb_addr,
                                &peer.socket_addr,
                                &addr
                            );
                            msg_out.set_fetch_local_addrs(FetchLocalAddrs {
                                socket_addr,
                                ..Default::default()
                            });
                        } else {
                            log::debug!(
                                "Punch hole {:?} {:?} request from {:?}",
                                &ph.hbb_addr,
                                &peer.socket_addr,
                                &addr
                            );
                            msg_out.set_punch_hole(PunchHole {
                                socket_addr,
                                ..Default::default()
                            });
                        }
                        socket.send(&msg_out, peer.socket_addr).await?;
                    }
                }
                Some(RendezvousMessage_oneof_union::punch_hole_sent(phs)) => {
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
                Some(RendezvousMessage_oneof_union::local_addrs(la)) => {
                    // forward local addrs of B to A
                    let addr_a = AddrMangle::decode(&la.socket_addr);
                    log::debug!("Local addrs response to {:?} from {:?}", &addr_a, &addr);
                    let mut msg_out = RendezvousMessage::new();
                    msg_out.set_local_addrs_response(LocalAddrsResponse {
                        socket_addrs: la.local_addrs,
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

#[cfg(test)]
mod tests {
    use super::*;
    use hbb_common::{new_error, tokio};

    #[allow(unused_must_use)]
    #[tokio::main]
    async fn test_rs_async() {
        let mut port_server: u16 = 0;
        let socket = FramedSocket::new("127.0.0.1:0").await.unwrap();
        if let SocketAddr::V4(addr) = socket.get_ref().local_addr().unwrap() {
            port_server = addr.port();
        }
        drop(socket);
        let addr_server = format!("127.0.0.1:{}", port_server);
        let f1 = RendezvousServer::start(&addr_server);
        let addr_server = addr_server.parse().unwrap();
        let f2 = punch_hole(addr_server);
        tokio::try_join!(f1, f2);
    }

    async fn punch_hole(addr_server: SocketAddr) -> ResultType<()> {
        // B register it to server
        let mut socket_b = FramedSocket::new("127.0.0.1:0").await?;
        let local_addr_b = socket_b.get_ref().local_addr().unwrap();
        let mut msg_out = RendezvousMessage::new();
        msg_out.set_register_peer(RegisterPeer {
            hbb_addr: "123".to_string(),
            ..Default::default()
        });
        socket_b.send(&msg_out, addr_server).await?;

        // A send punch request to server
        let mut socket_a = FramedSocket::new("127.0.0.1:0").await?;
        let local_addr_a = socket_a.get_ref().local_addr().unwrap();
        msg_out.set_punch_hole_request(PunchHoleRequest {
            hbb_addr: "123".to_string(),
            ..Default::default()
        });
        socket_a.send(&msg_out, addr_server).await?;

        println!(
            "A {:?} request punch hole to B {:?} via server {:?}",
            local_addr_a, local_addr_b, addr_server,
        );

        // on B side, responsed to A's punch request forwarded from server
        if let Some(Ok((bytes, addr))) = socket_b.next_timeout(1000).await {
            assert_eq!(addr_server, addr);
            let msg_in = parse_from_bytes::<RendezvousMessage>(&bytes)?;
            let remote_addr_a = AddrMangle::decode(&msg_in.get_punch_hole().socket_addr);
            assert_eq!(local_addr_a, remote_addr_a);

            // B punch A
            socket_b
                .get_mut()
                .send_to(&b"SYN"[..], &remote_addr_a)
                .await?;

            msg_out.set_punch_hole_sent(PunchHoleSent {
                socket_addr: AddrMangle::encode(&remote_addr_a),
                ..Default::default()
            });
            socket_b.send(&msg_out, addr_server).await?;
        } else {
            panic!("failed");
        }

        // on A side
        socket_a.next().await; // skip "SYN"
        if let Some(Ok((bytes, addr))) = socket_a.next_timeout(1000).await {
            assert_eq!(addr_server, addr);
            let msg_in = parse_from_bytes::<RendezvousMessage>(&bytes)?;
            let remote_addr_b = AddrMangle::decode(&msg_in.get_punch_hole_response().socket_addr);
            assert_eq!(local_addr_b, remote_addr_b);
        } else {
            panic!("failed");
        }

        Err(new_error("done"))
    }

    #[test]
    fn test_rs() {
        self::test_rs_async();
    }
}
