use bytes::{Bytes, BytesMut};
use futures::SinkExt;
use hbb_common::{
    message_proto::*,
    protobuf::{parse_from_bytes, Message as _},
    V4AddrMangle,
};
use std::{
    collections::HashMap,
    error::Error,
    net::{SocketAddr, SocketAddrV4},
    time::Duration,
};
use tokio::{net::UdpSocket, stream::StreamExt, time::delay_for};
use tokio_util::{codec::BytesCodec, udp::UdpFramed};

pub struct Peer {
    socket_addr: SocketAddrV4,
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
            if let SocketAddr::V4(addr_v4) = addr {
                match msg_in.union {
                    Some(Message_oneof_union::register_peer(rp)) => {
                        if rp.hbb_addr.len() > 0 {
                            self.peer_map.insert(
                                rp.hbb_addr,
                                Peer {
                                    socket_addr: addr_v4,
                                },
                            );
                        }
                    }
                    Some(Message_oneof_union::peek_peer(pp)) => {
                        if let Some(peer) = self.peer_map.get(&pp.hbb_addr) {
                            let mut msg_out = Message::new();
                            msg_out.set_peek_peer_response(PeekPeerResponse {
                                socket_addr: V4AddrMangle::encode(&peer.socket_addr),
                                ..Default::default()
                            });
                            send_to(&msg_out, addr, socket).await?;
                        }
                    }
                    _ => {}
                }
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

pub async fn sleep(sec: f32) {
    delay_for(Duration::from_secs_f32(sec)).await;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[allow(unused_must_use)]
    #[tokio::main]
    async fn test_rs_async() {
        let server_addr = "0.0.0.0:21116";
        let f1 = RendezvousServer::start(server_addr);
        let to_addr = server_addr.parse().unwrap();
        let f2 = async {
            let socket = UdpSocket::bind("127.0.0.1:0").await.unwrap();
            let local_addr = socket.local_addr().unwrap();
            let mut socket = UdpFramed::new(socket, BytesCodec::new());
            let mut msg_out = Message::new();
            msg_out.set_register_peer(RegisterPeer {
                hbb_addr: "123".to_string(),
                ..Default::default()
            });
            send_to(&msg_out, to_addr, &mut socket).await;
            msg_out.set_peek_peer(PeekPeer {
                hbb_addr: "123".to_string(),
                ..Default::default()
            });
            send_to(&msg_out, to_addr, &mut socket).await;
            if let Ok(Some(Ok((bytes, _)))) =
                tokio::time::timeout(Duration::from_millis(1), socket.next()).await
            {
                if let Ok(msg_in) = parse_from_bytes::<Message>(&bytes) {
                    assert_eq!(
                        local_addr,
                        SocketAddr::V4(V4AddrMangle::decode(
                            &msg_in.get_peek_peer_response().socket_addr[..]
                        ))
                    );
                }
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
