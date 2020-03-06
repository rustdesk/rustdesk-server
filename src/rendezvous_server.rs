use super::message_proto::Message;
use bytes::Bytes;
use futures::{FutureExt, SinkExt};
use protobuf::{parse_from_bytes, Message as _};
use std::{
    collections::HashMap,
    error::Error,
    net::{Ipv4Addr, SocketAddr, SocketAddrV4},
    time::{SystemTime, UNIX_EPOCH},
};
use tokio::net::UdpSocket;
use tokio::stream::StreamExt;
use tokio_util::{codec::BytesCodec, udp::UdpFramed};

/// Certain router and firewalls scan the packet and if they
/// find an IP address belonging to their pool that they use to do the NAT mapping/translation, so here we mangle the ip address

pub struct V4AddrMangle(Vec<u8>);

impl V4AddrMangle {
    pub fn encode(addr: SocketAddrV4) -> Self {
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
        Self(bytes[..(16 - n_padding)].to_vec())
    }

    pub fn decode(&self) -> SocketAddrV4 {
        let mut padded = [0u8; 16];
        padded[..self.0.len()].copy_from_slice(&self.0);
        let number = u128::from_ne_bytes(padded);
        let tm = (number >> 17) & (u32::max_value() as u128);
        let ip = (((number >> 49) - tm) as u32).to_ne_bytes();
        let port = (number & 0xFFFFFF) - (tm & 0xFFFF);
        SocketAddrV4::new(Ipv4Addr::new(ip[0], ip[1], ip[2], ip[3]), port as u16)
    }
}

pub struct Peer {
    socket_addr: SocketAddr,
}

type PeerMap = HashMap<String, Peer>;

pub struct RendezvousServer {
    peer_map: PeerMap,
}

impl RendezvousServer {
    pub async fn start(addr: &str) -> Result<Self, Box<dyn Error>> {
        let socket = UdpSocket::bind(addr).await?;
        let mut socket = UdpFramed::new(socket, BytesCodec::new());

        let rs = Self {
            peer_map: PeerMap::new(),
        };
        while let Some(Ok((bytes, addr))) = socket.next().await {
            if let SocketAddr::V4(addr_v4) = addr {
                if let Ok(msg_in) = parse_from_bytes::<Message>(&bytes) {
                    let msg_out = Message::new();
                    socket
                        .send((Bytes::from(msg_out.write_to_bytes().unwrap()), addr))
                        .await?;
                }
            }
        }
        Ok(rs)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn test_mangle() {
        let addr = SocketAddrV4::new(Ipv4Addr::new(192, 168, 16, 32), 21116);
        assert_eq!(addr, V4AddrMangle::encode(addr).decode());
        println!("{:?}", V4AddrMangle::encode(addr).0);
    }
}
