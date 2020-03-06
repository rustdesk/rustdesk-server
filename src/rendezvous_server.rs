use super::message_proto::*;
use bytes::Bytes;
use futures::SinkExt;
use protobuf::{parse_from_bytes, Message as _};
use std::{
    collections::HashMap,
    error::Error,
    net::{Ipv4Addr, SocketAddr, SocketAddrV4},
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tokio::{
    net::UdpSocket,
    stream::StreamExt,
    time::{self, delay_for},
};
use tokio_util::{codec::BytesCodec, udp::UdpFramed};

/// Certain router and firewalls scan the packet and if they
/// find an IP address belonging to their pool that they use to do the NAT mapping/translation, so here we mangle the ip address

pub struct V4AddrMangle();

impl V4AddrMangle {
    pub fn encode(addr: &SocketAddrV4) -> Vec<u8> {
        let tm = (SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_micros() as u32) as u128;
        let ip = u32::from_ne_bytes(addr.ip().octets()) as u128;
        let port = addr.port() as u128;
        let v = ((ip + tm) << 49) | (tm << 17) | (port + (tm & 0xFFFF));
        let bytes = v.to_ne_bytes();
        let mut n_padding = 0;
        for i in bytes.iter().rev() {
            if i == &0u8 {
                n_padding += 1;
            } else {
                break;
            }
        }
        bytes[..(16 - n_padding)].to_vec()
    }

    pub fn decode(bytes: &[u8]) -> SocketAddrV4 {
        let mut padded = [0u8; 16];
        padded[..bytes.len()].copy_from_slice(&bytes);
        let number = u128::from_ne_bytes(padded);
        let tm = (number >> 17) & (u32::max_value() as u128);
        let ip = (((number >> 49) - tm) as u32).to_ne_bytes();
        let port = (number & 0xFFFFFF) - (tm & 0xFFFF);
        SocketAddrV4::new(Ipv4Addr::new(ip[0], ip[1], ip[2], ip[3]), port as u16)
    }
}

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
            if let SocketAddr::V4(addr_v4) = addr {
                if let Ok(msg_in) = parse_from_bytes::<Message>(&bytes) {
                    match msg_in.union {
                        Some(Message_oneof_union::register_peer(rp)) => {
                            if rp.hbb_addr.len() > 0 {
                                rs.peer_map.insert(
                                    rp.hbb_addr,
                                    Peer {
                                        socket_addr: addr_v4,
                                    },
                                );
                            }
                        }
                        Some(Message_oneof_union::peek_peer(pp)) => {
                            rs.handle_peek_peer(&pp, addr, &mut socket).await?;
                        }
                        _ => {}
                    }
                }
            }
        }
        Ok(())
    }

    pub async fn handle_peek_peer(
        &self,
        pp: &PeekPeer,
        addr: SocketAddr,
        socket: &mut FramedSocket,
    ) -> ResultType {
        if let Some(peer) = self.peer_map.get(&pp.hbb_addr) {
            let mut msg_out = Message::new();
            msg_out.set_peek_peer_response(PeekPeerResponse {
                socket_addr: V4AddrMangle::encode(&peer.socket_addr),
                ..Default::default()
            });
            send_to(&msg_out, addr, socket).await?;
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
    #[test]
    fn test_mangle() {
        let addr = SocketAddrV4::new(Ipv4Addr::new(192, 168, 16, 32), 21116);
        assert_eq!(addr, V4AddrMangle::decode(&V4AddrMangle::encode(&addr)[..]));
    }

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
                time::timeout(Duration::from_millis(1), socket.next()).await
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
