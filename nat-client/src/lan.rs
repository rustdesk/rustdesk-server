//! 局域网对等节点发现模块（参照 RustDesk lan.rs）
//!
//! 功能：
//! 1. 监听 UDP 广播，响应来自其他 nat-client 的 ping 探测
//! 2. 主动发送 broadcast ping，收集局域网内其他节点的 pong 响应
//! 3. 将发现的节点缓存到全局 `DISCOVERED_PEERS`
//!
//! 线路协议与 hbbs 一致：`rendezvous_codec` 自动识别入站 proto3/capnp；
//! 主动广播 ping 使用配置 `rendezvous_wire_protocol`，pong 与入站 ping 同格式。

use crate::config::ClientConfig;
use core_common::{
    allow_err,
    config::RENDEZVOUS_PORT,
    log,
    protobuf::Message as _,
    rendezvous_codec::{self, Protocol},
    rendezvous_proto::{rendezvous_message, PeerDiscovery, RendezvousMessage},
    ResultType,
};
use once_cell::sync::Lazy;
use serde_derive::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    net::{IpAddr, Ipv4Addr, SocketAddr, UdpSocket},
    sync::{Arc, Mutex},
    time::Instant,
};

/// 序列化 LAN rendezvous 帧（proto3 / capnp）
#[inline]
fn serialize_lan_message(msg: &RendezvousMessage, wire: Protocol) -> Vec<u8> {
    rendezvous_codec::serialize(msg, wire)
        .map(|b| b.to_vec())
        .unwrap_or_else(|| msg.write_to_bytes().unwrap_or_default())
}

// ──────────────────────────────────────────────────────────────────────────────
// 数据结构
// ──────────────────────────────────────────────────────────────────────────────

/// 已发现的局域网对等节点
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DiscoveredPeer {
    /// 对端 Peer ID
    pub id: String,
    /// IP 地址
    pub ip: String,
    /// MAC 地址（可能为空）
    pub mac: String,
    /// 主机名
    pub hostname: String,
    /// 用户名
    pub username: String,
    /// 操作系统平台
    pub platform: String,
    /// 是否当前在线（最近 30 秒内有响应）
    pub online: bool,
    /// 最后一次响应时间
    #[serde(skip)]
    pub last_seen: Option<Instant>,
}

// ──────────────────────────────────────────────────────────────────────────────
// 全局发现缓存
// ──────────────────────────────────────────────────────────────────────────────

/// 全局已发现节点列表（key = peer_id）
pub static DISCOVERED_PEERS: Lazy<Arc<Mutex<HashMap<String, DiscoveredPeer>>>> =
    Lazy::new(|| Arc::new(Mutex::new(HashMap::new())));

/// 将发现的节点写入缓存
fn upsert_peer(peer: DiscoveredPeer) {
    if let Ok(mut map) = DISCOVERED_PEERS.lock() {
        let entry = map.entry(peer.id.clone()).or_insert_with(|| peer.clone());
        entry.ip = peer.ip;
        entry.mac = peer.mac;
        entry.hostname = peer.hostname;
        entry.username = peer.username;
        entry.platform = peer.platform;
        entry.online = true;
        entry.last_seen = Some(Instant::now());
    }
}

/// 返回所有已发现节点的快照
pub fn get_peers() -> Vec<DiscoveredPeer> {
    let map = DISCOVERED_PEERS.lock().unwrap();
    let mut peers: Vec<DiscoveredPeer> = map.values().cloned().collect();
    // 超过 60 秒未响应的节点标记为离线
    let now = Instant::now();
    for p in &mut peers {
        if let Some(t) = p.last_seen {
            if now.duration_since(t).as_secs() > 60 {
                p.online = false;
            }
        }
    }
    peers.sort_by(|a, b| a.id.cmp(&b.id));
    peers
}

// ──────────────────────────────────────────────────────────────────────────────
// 广播端口
// ──────────────────────────────────────────────────────────────────────────────

/// LAN 发现使用的 UDP 广播端口（= RENDEZVOUS_PORT + 3 = 21119）
#[inline]
fn broadcast_port() -> u16 {
    (RENDEZVOUS_PORT + 3) as u16
}

// ──────────────────────────────────────────────────────────────────────────────
// 监听端（响应来自其他节点的 ping）
// ──────────────────────────────────────────────────────────────────────────────

/// 在后台线程中监听 LAN 广播 ping，并回应 pong（阻塞，需在独立线程运行）
pub fn start_listening() -> ResultType<()> {
    let addr = SocketAddr::from(([0, 0, 0, 0], broadcast_port()));
    let socket = UdpSocket::bind(addr)?;
    socket.set_read_timeout(Some(std::time::Duration::from_millis(1000)))?;
    log::info!("[lan] 局域网发现监听器已启动，端口 {}", broadcast_port());

    loop {
        let mut buf = [0u8; 2048];
        let (len, from_addr) = match socket.recv_from(&mut buf) {
            Ok(v) => v,
            Err(_) => continue, // 超时或暂时错误，继续
        };

        let reply_proto = rendezvous_codec::detect(&buf[..len]);
        let msg = match rendezvous_codec::parse(&buf[..len]) {
            Some(m) => m,
            None => continue,
        };

        if let Some(rendezvous_message::Union::PeerDiscovery(p)) = msg.union {
            if p.cmd != "ping" {
                continue;
            }
            let my_id = ClientConfig::get_id();
            // 忽略自己发出的 ping
            if p.id == my_id {
                continue;
            }

            log::debug!("[lan] 收到 ping from {} (peer={})", from_addr, p.id);

            // 构造 pong 响应
            let hostname = whoami::fallible::hostname().unwrap_or_else(|_| "unknown".to_owned());
            let hostname = if hostname == "localhost" {
                "unknown".to_owned()
            } else {
                hostname
            };

            let pong = PeerDiscovery {
                cmd: "pong".to_owned(),
                id: my_id,
                hostname,
                username: whoami::username(),
                platform: format!("{}", whoami::platform()),
                mac: get_local_mac(&from_addr),
                ..Default::default()
            };
            let mut msg_out = RendezvousMessage::new();
            msg_out.set_peer_discovery(pong);
            let bytes = serialize_lan_message(&msg_out, reply_proto);
            if !bytes.is_empty() {
                allow_err!(socket.send_to(&bytes, from_addr));
                log::debug!("[lan] pong 已发送至 {}", from_addr);
            }
        }
    }
}

// ──────────────────────────────────────────────────────────────────────────────
// 主动发现（发送 ping，收集 pong）
// ──────────────────────────────────────────────────────────────────────────────

/// 主动扫描局域网：发送 UDP 广播 ping，等待 3 秒收集响应
pub fn discover() -> ResultType<Vec<DiscoveredPeer>> {
    let sockets = create_broadcast_sockets();
    if sockets.is_empty() {
        log::warn!("[lan] 未找到可绑定的网络接口");
        return Ok(get_peers());
    }

    let wire = ClientConfig::get_rendezvous_wire_protocol();

    // 构造 ping 消息
    let my_id = ClientConfig::get_id();
    let ping = PeerDiscovery {
        cmd: "ping".to_owned(),
        id: my_id.clone(),
        ..Default::default()
    };
    let mut msg_out = RendezvousMessage::new();
    msg_out.set_peer_discovery(ping);
    let out = serialize_lan_message(&msg_out, wire);

    // 向 255.255.255.255:broadcast_port 广播
    let bcast_addr = SocketAddr::from(([255, 255, 255, 255], broadcast_port()));
    for socket in &sockets {
        allow_err!(socket.send_to(&out, bcast_addr));
    }
    log::info!("[lan] 已发送发现 ping");

    // 在多个线程中等待响应
    let results: Arc<Mutex<Vec<DiscoveredPeer>>> = Arc::new(Mutex::new(Vec::new()));
    let mut threads = Vec::new();

    for socket in sockets {
        let results_clone = Arc::clone(&results);
        let my_id_clone = my_id.clone();
        threads.push(std::thread::spawn(move || {
            allow_err!(collect_responses(socket, my_id_clone, results_clone));
        }));
    }

    for t in threads {
        t.join().ok();
    }

    // 将本次发现的结果写入全局缓存
    let fresh: Vec<DiscoveredPeer> = results.lock().unwrap().drain(..).collect();
    for peer in fresh {
        upsert_peer(peer);
    }

    log::info!("[lan] 发现完成，共 {} 个节点", get_peers().len());
    Ok(get_peers())
}

// ──────────────────────────────────────────────────────────────────────────────
// 内部工具
// ──────────────────────────────────────────────────────────────────────────────

/// 在给定 socket 上等待最多 3 秒的 pong 响应
fn collect_responses(
    socket: UdpSocket,
    my_id: String,
    out: Arc<Mutex<Vec<DiscoveredPeer>>>,
) -> ResultType<()> {
    socket.set_read_timeout(Some(std::time::Duration::from_millis(50)))?;
    let deadline = Instant::now() + std::time::Duration::from_secs(3);

    loop {
        if Instant::now() >= deadline {
            break;
        }
        let mut buf = [0u8; 2048];
        let (len, from_addr) = match socket.recv_from(&mut buf) {
            Ok(v) => v,
            Err(_) => continue,
        };

        let msg = match rendezvous_codec::parse(&buf[..len]) {
            Some(m) => m,
            None => continue,
        };

        if let Some(rendezvous_message::Union::PeerDiscovery(p)) = msg.union {
            if p.cmd != "pong" || p.id == my_id {
                continue;
            }
            log::info!("[lan] 发现节点 id={} ip={}", p.id, from_addr.ip());
            let peer = DiscoveredPeer {
                id: p.id,
                ip: from_addr.ip().to_string(),
                mac: p.mac,
                hostname: p.hostname,
                username: p.username,
                platform: p.platform,
                online: true,
                last_seen: Some(Instant::now()),
            };
            if let Ok(mut v) = out.lock() {
                v.push(peer);
            }
        }
    }
    Ok(())
}

/// 创建用于广播的 UDP socket 列表（每个 IPv4 接口一个）
fn create_broadcast_sockets() -> Vec<UdpSocket> {
    let mut sockets = Vec::new();

    // 绑定所有接口的 0 号端口（系统自动分配）
    let candidates: Vec<Ipv4Addr> = {
        let mut v = Vec::new();
        #[cfg(not(any(target_os = "android", target_os = "ios")))]
        {
            // 尝试获取本机所有 IPv4 接口
            if let Ok(addrs) = local_ipv4_addrs() {
                for ip in addrs {
                    v.push(ip);
                }
            }
        }
        v.push(Ipv4Addr::UNSPECIFIED); // 0.0.0.0 作为兜底
        v
    };

    for ip in candidates {
        let bind_addr = SocketAddr::from((ip, 0u16));
        if let Ok(s) = UdpSocket::bind(bind_addr) {
            if s.set_broadcast(true).is_ok() {
                sockets.push(s);
            }
        }
    }
    sockets
}

/// 获取本机所有 IPv4 地址
#[cfg(not(any(target_os = "android", target_os = "ios")))]
fn local_ipv4_addrs() -> ResultType<Vec<Ipv4Addr>> {
    use std::net::ToSocketAddrs;
    let hostname = whoami::fallible::hostname().unwrap_or_else(|_| "localhost".to_owned());
    let mut addrs = Vec::new();
    for addr in (hostname.as_str(), 0u16).to_socket_addrs()? {
        if let IpAddr::V4(v4) = addr.ip() {
            addrs.push(v4);
        }
    }
    Ok(addrs)
}

/// 尝试获取与目标地址对应的本机 MAC 地址
fn get_local_mac(peer: &SocketAddr) -> String {
    // 通过连接对端 UDP 套接字获取本机接口地址，再查 MAC
    let probe = UdpSocket::bind("0.0.0.0:0").ok();
    if let Some(s) = probe {
        if s.connect(peer).is_ok() {
            if let Ok(local) = s.local_addr() {
                return format!("{}", local.ip()); // 简化：返回 IP（MAC 查询跨平台复杂）
            }
        }
    }
    String::new()
}
