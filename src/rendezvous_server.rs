use bytes::{Bytes, BytesMut};
use futures::SinkExt;
use hbb_common::{
    message_proto::*,
    protobuf::{parse_from_bytes, Message as _},
    AddrMangle,
};
use std::{collections::HashMap, error::Error, net::SocketAddr, time::Duration};
use tokio::{net::UdpSocket, stream::StreamExt, time::delay_for};
use tokio_util::{codec::BytesCodec, udp::UdpFramed};

pub struct Peer {
    socket_addr: SocketAddr,
}

type PeerMap = HashMap<String, Peer>;

pub struct RendezvousServer {
    peer_map: PeerMap,
}

type FramedSocket = UdpFramed<BytesCodec>;
type ResultType = Result<(), Box<dyn Error>>;

impl RendezvousServer {
    pub async fn start(addr: &str) -> ResultType {
        let socket = UdpSocket::bind(addr).await?;
        let mut socket = UdpFramed::new(socket, BytesCodec::new());

        let mut rs = Self {
            peer_map: PeerMap::new(),
        };
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
    ) -> ResultType {
        if let Ok(msg_in) = parse_from_bytes::<Message>(&bytes) {
            match msg_in.union {
                Some(Message_oneof_union::register_peer(rp)) => {
                    if rp.hbb_addr.len() > 0 {
                        self.peer_map
                            .insert(rp.hbb_addr, Peer { socket_addr: addr });
                    }
                }
                Some(Message_oneof_union::punch_hole_request(ph)) => {
                    // punch hole request from A, forward to B
                    if let Some(peer) = self.peer_map.get(&ph.hbb_addr) {
                        let mut msg_out = Message::new();
                        msg_out.set_punch_hole(PunchHole {
                            socket_addr: AddrMangle::encode(&addr),
                            ..Default::default()
                        });
                        send_to(&msg_out, peer.socket_addr, socket).await?;
                    }
                }
                Some(Message_oneof_union::punch_hole_sent(phs)) => {
                    // punch hole sent from B, tell A that B ready
                    let addr_a = AddrMangle::decode(&phs.socket_addr);
                    let mut msg_out = Message::new();
                    msg_out.set_punch_hole_response(PunchHoleResponse {
                        socket_addr: AddrMangle::encode(&addr),
                        ..Default::default()
                    });
                    send_to(&msg_out, addr_a, socket).await?;
                }
                _ => {}
            }
        }
        Ok(())
    }
}

#[inline]
pub async fn send_to(msg: &Message, addr: SocketAddr, socket: &mut FramedSocket) -> ResultType {
    socket
        .send((Bytes::from(msg.write_to_bytes().unwrap()), addr))
        .await?;
    Ok(())
}

#[inline]
pub async fn sleep(sec: f32) {
    delay_for(Duration::from_secs_f32(sec)).await;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[allow(unused_must_use)]
    #[tokio::main]
    async fn test_rs_async() {
        let mut port_server: u16 = 0;
        let socket = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        if let SocketAddr::V4(addr) = socket.local_addr().unwrap() {
            port_server = addr.port();
        }
        drop(socket);
        let addr_server = format!("127.0.0.1:{}", port_server);
        let f1 = RendezvousServer::start(&addr_server);
        let addr_server = addr_server.parse().unwrap();
        let f2 = async {
            // B register it to server
            let socket_b = UdpSocket::bind("127.0.0.1:0").await.unwrap();
            let local_addr_b = socket_b.local_addr().unwrap();
            let mut socket_b = UdpFramed::new(socket_b, BytesCodec::new());
            let mut msg_out = Message::new();
            msg_out.set_register_peer(RegisterPeer {
                hbb_addr: "123".to_string(),
                ..Default::default()
            });
            send_to(&msg_out, addr_server, &mut socket_b).await;

            // A send punch request to server
            let socket_a = UdpSocket::bind("127.0.0.1:0").await.unwrap();
            let local_addr_a = socket_a.local_addr().unwrap();
            let mut socket_a = UdpFramed::new(socket_a, BytesCodec::new());
            msg_out.set_punch_hole_request(PunchHoleRequest {
                hbb_addr: "123".to_string(),
                ..Default::default()
            });
            send_to(&msg_out, addr_server, &mut socket_a).await;

            println!(
                "A {:?} request punch hole to B {:?} via server {:?}",
                local_addr_a, local_addr_b, addr_server,
            );

            // on B side, responsed to A's punch request forwarded from server
            if let Ok(Some(Ok((bytes, addr)))) =
                tokio::time::timeout(Duration::from_millis(1000), socket_b.next()).await
            {
                assert_eq!(addr_server, addr);
                let msg_in = parse_from_bytes::<Message>(&bytes).unwrap();
                let remote_addr_a = AddrMangle::decode(&msg_in.get_punch_hole().socket_addr[..]);
                assert_eq!(local_addr_a, remote_addr_a);

                // B punch A
                socket_b
                    .get_mut()
                    .send_to(&b"SYN"[..], &remote_addr_a)
                    .await;

                msg_out.set_punch_hole_sent(PunchHoleSent {
                    socket_addr: AddrMangle::encode(&remote_addr_a),
                    ..Default::default()
                });
                send_to(&msg_out, addr_server, &mut socket_b).await;
            }

            // on A side
            socket_a.next().await; // skip "SYN"
            if let Ok(Some(Ok((bytes, addr)))) =
                tokio::time::timeout(Duration::from_millis(1000), socket_a.next()).await
            {
                assert_eq!(addr_server, addr);
                let msg_in = parse_from_bytes::<Message>(&bytes).unwrap();
                let remote_addr_b =
                    AddrMangle::decode(&msg_in.get_punch_hole_response().socket_addr[..]);
                println!("{:?}", msg_in);
                assert_eq!(local_addr_b, remote_addr_b);
            }

            if true {
                Err(Box::new(simple_error::SimpleError::new("done")))
            } else {
                Ok(())
            }
        };
        tokio::try_join!(f1, f2);
    }

    #[test]
    fn test_rs() {
        self::test_rs_async();
    }
}
