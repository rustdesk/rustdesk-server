// NAT打洞穿透服务器实现
// 实现了完整的NAT穿透协议，包括UDP打洞、TCP打洞、中继服务器等功能

use crate::common::*;
use crate::peer::*;
use core_common::{
    allow_err, bail,
    bytes::{Bytes, BytesMut},
    bytes_codec::BytesCodec,
    config,
    futures::future::join_all,
    futures_util::{
        sink::SinkExt,
        stream::{SplitSink, StreamExt},
    },
    log,
    protobuf::{Message as _, MessageField},
    rendezvous_proto::{
        register_pk_response::Result::{TOO_FREQUENT, UUID_MISMATCH},
        *,
    },
    tcp::{listen_any, FramedStream},
    timeout,
    tokio::{
        self,
        io::{AsyncReadExt, AsyncWriteExt},
        net::{TcpListener, TcpStream},
        sync::{mpsc, Mutex},
        time::{interval, Duration},
    },
    tokio_util::codec::Framed,
    try_into_v4,
    udp::FramedSocket,
    AddrMangle, ResultType,
};
use ipnetwork::Ipv4Network;
use sodiumoxide::crypto::sign;
use std::{
    collections::HashMap,
    net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr},
    sync::atomic::{AtomicBool, AtomicUsize, Ordering},
    sync::Arc,
    time::Instant,
};

// 数据类型枚举，用于处理不同类型的消息
#[derive(Clone, Debug)]
enum Data {
    Msg(Box<RendezvousMessage>, SocketAddr),  // 消息和来源地址
    RelayServers0(String),                 // 中继服务器列表（字符串格式）
    RelayServers(RelayServers),             // 中继服务器列表（结构化格式）
}

// 注册超时时间（毫秒）
const REG_TIMEOUT: i32 = 30_000;

// 检查中继服务器超时时间
const CHECK_RELAY_TIMEOUT: u64 = 30_000;

// IP地址变化持续时间（秒）
const IP_CHANGE_DUR: u64 = 60;

// 打洞请求去重时间窗口（秒）
const PUNCH_REQ_DEDUPE_SEC: u64 = 5;

// 是否总是使用中继服务器
static ALWAYS_USE_RELAY: AtomicBool = AtomicBool::new(false);

// Rendezvous服务器主结构体
#[derive(Clone)]
pub struct RendezvousServer {
    // TCP打洞请求映射表：存储发起打洞的客户端信息
    tcp_punch: Arc<Mutex<HashMap<SocketAddr, Sink>>>,
    // Peer管理器：处理所有连接的peer
    pm: PeerMap,
    // 消息发送通道
    tx: Sender<Data>,
    // 中继服务器列表
    relay_servers: Arc<RelayServers>,
    // 中继服务器列表（字符串格式）
    relay_servers0: Arc<Vec<String>>,
    // Rendezvous服务器列表
    rendezvous_servers: Arc<Vec<String>>,
    // 内部状态
    inner: Arc<Inner>,
}

// 内部状态结构体
#[derive(Clone)]
struct Inner {
    // 序列号
    serial: i32,
    // 版本号
    version: String,
    // 软件更新URL
    software_url: String,
    // 服务器私钥
    sk: Vec<u8>,
    // 网络掩码
    mask: Option<Ipv4Network>,
    // 本地IP地址
    local_ip: String,
}

// NAT类型枚举
#[derive(Clone, Copy, Debug)]
enum NatType {
    UNKNOWN = 0,    // 未知类型
    SYMMETRIC = 1, // 对称NAT
    RESTRICTED = 2, // 限制性NAT
    PORT_RESTRICTED = 3, // 端口限制性NAT
    FULL_CONE = 4,  // 完全锥形NAT
    UDP_BLOCKED = 5, // UDP被阻止
    SYMMETRIC_UDP_BLOCKED = 6, // 对称NAT且UDP被阻止
}

// 主服务器实现
impl RendezvousServer {
    // 启动服务器的主函数
    #[tokio::main(flavor = "multi_thread")]
    pub async fn start(port: i32, serial: i32, key: &str, rmem: usize) -> ResultType<()> {
        // 获取服务器密钥对
        let (key, sk) = Self::get_server_sk(key);
        // NAT测试端口（主端口-1）
        let nat_port = port - 1;
        // WebSocket端口（主端口+2）
        let ws_port = port + 2;
        // 初始化Peer管理器
        let pm = PeerMap::new().await?;
        log::info!("serial={}", serial);
        // 获取Rendezvous服务器列表
        let rendezvous_servers = get_servers(&get_arg("rendezvous-servers"), "rendezvous-servers");
        // 日志输出监听端口信息
        log::info!("Listening on tcp/udp :{}", port);
        log::info!("Listening on tcp :{}, extra port for NAT test", nat_port);
        log::info!("Listening on websocket :{}", ws_port);
        // 创建UDP监听器
        let mut socket = create_udp_listener(port, rmem).await?;
        // 创建消息通道
        let (tx, mut rx) = mpsc::unbounded_channel::<Data>();
        // 获取软件更新URL
        let software_url = get_arg("software-url");
        // 获取版本信息
        let version = core_common::get_version_from_url(&software_url);
        if !version.is_empty() {
            log::info!("software_url: {}, version: {}", software_url, version);
        }
        // 获取网络掩码
        let mask = get_arg("mask").parse().ok();
        // 获取本地IP地址
        let local_ip = if mask.is_none() {
            "".to_owned()
        } else {
            get_arg_or(
                "local-ip",
                local_ip_address::local_ip()
                    .map(|x| x.to_string())
                    .unwrap_or_default(),
            )
        };
        // 初始化服务器结构体
        let mut rs = Self {
            tcp_punch: Arc::new(Mutex::new(HashMap::new())),
            pm,
            tx: tx.clone(),
            relay_servers: Default::default(),
            relay_servers0: Default::default(),
            rendezvous_servers: Arc::new(rendezvous_servers),
            inner: Arc::new(Inner {
                serial,
                version,
                software_url,
                sk,
                mask,
                local_ip,
            }),
        };
        // 输出配置信息
        log::info!("mask: {:?}", rs.inner.mask);
        log::info!("local-ip: {:?}", rs.inner.local_ip);
        // 设置API端口环境变量
        std::env::set_var("PORT_FOR_API", port.to_string());
        // 解析中继服务器
        rs.parse_relay_servers(&get_arg("relay-servers"));
        // 创建TCP监听器
        let mut listener = create_tcp_listener(port).await?;
        let mut listener2 = create_tcp_listener(nat_port).await?;
        let mut listener3 = create_tcp_listener(ws_port).await?;
        // 获取测试地址
        let test_addr = std::env::var("TEST_HBBS").unwrap_or_default();
        // 检查是否总是使用中继
        if std::env::var("ALWAYS_USE_RELAY")
            .unwrap_or_default()
            .to_uppercase()
            == "Y"
        {
            ALWAYS_USE_RELAY.store(true, Ordering::SeqCst);
        }
        // 输出中继使用状态
        log::info!(
            "ALWAYS_USE_RELAY={}",
            if ALWAYS_USE_RELAY.load(Ordering::SeqCst) {
                "Y"
            } else {
                "N"
            }
        );
        // 启动测试任务（如果需要）
        if test_addr.to_lowercase() != "no" {
            let test_addr = if test_addr.is_empty() {
                listener.local_addr()?
            } else {
                test_addr.parse()?
            };
            tokio::spawn(async move {
                if let Err(err) = test_hbbs(test_addr).await {
                    // IPv6测试失败时，尝试IPv4
                    if test_addr.is_ipv6() && test_addr.ip().is_unspecified() {
                        let mut test_addr = test_addr;
                        test_addr.set_ip(IpAddr::V4(Ipv4Addr::UNSPECIFIED));
                        if let Err(err) = test_hbbs(test_addr).await {
                            log::error!("Failed to run hbbs test with {test_addr}: {err}");
                            std::process::exit(1);
                        }
                    } else {
                        log::error!("Failed to run hbbs test with {test_addr}: {err}");
                        std::process::exit(1);
                    }
                }
            });
        };
        // 主事件循环任务
        let main_task = async move {
            loop {
                log::info!("Start");
                match rs
                    .io_loop(
                        &mut rx,
                        &mut listener,
                        &mut listener2,
                        &mut listener3,
                        &mut socket,
                        &key,
                    )
                    .await
                {
                    LoopFailure::UdpSocket => {
                        drop(socket);
                        socket = create_udp_listener(port, rmem).await?;
                    }
                    LoopFailure::Listener => {
                        drop(listener);
                        listener = create_tcp_listener(port).await?;
                    }
                    LoopFailure::Listener2 => {
                        drop(listener2);
                        listener2 = create_tcp_listener(nat_port).await?;
                    }
                    LoopFailure::Listener3 => {
                        drop(listener3);
                        listener3 = create_tcp_listener(ws_port).await?;
                    }
                }
            }
        };
        // 监听系统信号
        let listen_signal = listen_signal();
        // 等待主任务或信号完成
        tokio::select!(
            res = main_task => res,
            res = listen_signal => res,
        )
    }

    // 主I/O循环：处理所有网络事件
    async fn io_loop(
        &mut self,
        rx: &mut Receiver,
        listener: &mut TcpListener,
        listener2: &mut TcpListener,
        listener3: &mut TcpListener,
        socket: &mut FramedSocket,
        key: &str,
    ) -> LoopFailure {
        // 创建中继服务器检查定时器
        let mut timer_check_relay = interval(Duration::from_millis(CHECK_RELAY_TIMEOUT));
        loop {
            tokio::select! {
                // 定时检查中继服务器状态
                _ = timer_check_relay.tick() => {
                    if self.relay_servers0.len() > 1 {
                        let rs = self.relay_servers0.clone();
                        let tx = self.tx.clone();
                        tokio::spawn(async move {
                            check_relay_servers(rs, tx).await;
                        });
                    }
                }
                // 处理内部消息
                Some(data) = rx.recv() => {
                    match data {
                        Data::Msg(msg, addr) => { 
                            // 发送UDP消息
                            allow_err!(socket.send(msg.as_ref(), addr).await); 
                        }
                        Data::RelayServers0(rs) => { 
                            // 解析中继服务器列表（字符串格式）
                            self.parse_relay_servers(&rs); 
                        }
                        Data::RelayServers(rs) => { 
                            // 设置中继服务器列表（结构化格式）
                            self.relay_servers = Arc::new(rs); 
                        }
                    }
                }
                // 处理UDP消息
                res = socket.next() => {
                    match res {
                        Some(Ok((bytes, addr))) => {
                            // 处理接收到的UDP消息
                            if let Err(err) = self.handle_udp(&bytes, addr.into(), socket, key).await {
                                log::error!("udp failure: {}", err);
                                return LoopFailure::UdpSocket;
                            }
                        }
                        Some(Err(err)) => {
                            log::error!("udp failure: {}", err);
                            return LoopFailure::UdpSocket;
                        }
                        None => {
                            // 理论上不应该到达这里
                        }
                    }
                }
                // 处理NAT测试连接（TCP）
                res = listener2.accept() => {
                    match res {
                        Ok((stream, addr))  => {
                            // 设置无延迟模式
                            stream.set_nodelay(true).ok();
                            // 处理NAT测试连接
                            self.handle_listener2(stream, addr).await;
                        }
                        Err(err) => {
                           log::error!("listener2.accept failed: {}", err);
                           return LoopFailure::Listener2;
                        }
                    }
                }
                // 处理WebSocket连接（TCP）
                res = listener3.accept() => {
                    match res {
                        Ok((stream, addr))  => {
                            // 设置无延迟模式
                            stream.set_nodelay(true).ok();
                            // 处理WebSocket连接
                            self.handle_listener(stream, addr, key, true).await;
                        }
                        Err(err) => {
                           log::error!("listener3.accept failed: {}", err);
                           return LoopFailure::Listener3;
                        }
                    }
                }
            }
        }
    }

    // 处理接收到的UDP消息
    async fn handle_udp(
        &mut self,
        bytes: &[u8],
        addr: SocketAddr,
        socket: &mut FramedSocket,
        key: &str,
    ) -> ResultType<()> {
        // 解析协议消息
        if let Ok(msg_in) = RendezvousMessage::parse_from_bytes(bytes) {
            match msg_in.union {
                // 处理公钥注册请求
                Some(rendezvous_message::Union::RegisterPk(rp)) => {
                    self.handle_register_pk(rp, addr, socket).await?;
                }
                // 处理peer注册请求
                Some(rendezvous_message::Union::RegisterPeer(mut rp)) => {
                    self.handle_register_peer(&mut rp, addr, socket).await?;
                }
                // 处理打洞请求
                Some(rendezvous_message::Union::PunchHoleRequest(ph)) => {
                    // 检查peer是否在内存中
                    if self.pm.is_in_memory(&ph.id).await {
                        // 直接处理（在内存中）
                        self.handle_udp_punch_hole_request(addr, ph, key).await?;
                    } else {
                        // 从数据库加载（避免阻塞主线程）
                        let mut me = self.clone();
                        let key = key.to_owned();
                        tokio::spawn(async move {
                            allow_err!(me.handle_udp_punch_hole_request(addr, ph, &key).await);
                        });
                    }
                }
                // 处理打洞完成通知
                Some(rendezvous_message::Union::PunchHoleSent(phs)) => {
                    self.handle_hole_sent(phs, addr, Some(socket)).await?;
                }
                // 处理本地地址通知
                Some(rendezvous_message::Union::LocalAddr(la)) => {
                    self.handle_local_addr(la, addr, Some(socket)).await?;
                }
                // 处理配置更新
                Some(rendezvous_message::Union::ConfigureUpdate(mut cu)) => {
                    // 只允许来自环回地址的配置更新
                    if try_into_v4(addr).ip().is_loopback() && cu.serial > self.inner.serial {
                        let mut inner: Inner = (*self.inner).clone();
                        inner.serial = cu.serial;
                        self.inner = Arc::new(inner);
                        // 更新Rendezvous服务器列表
                        self.rendezvous_servers = Arc::new(
                            cu.rendezvous_servers
                                .drain(..)
                                .filter(|x| {
                                    !x.is_empty()
                                        && test_if_valid_server(x, "rendezvous-server").is_ok()
                                })
                                .collect(),
                        );
                        log::info!(
                            "configure updated: serial={} rendezvous-servers={:?}",
                            self.inner.serial,
                            self.rendezvous_servers
                        );
                    }
                }
                // 处理软件更新通知
                Some(rendezvous_message::Union::SoftwareUpdate(su)) => {
                    if !self.inner.version.is_empty() && su.url != self.inner.version {
                        let mut msg_out = RendezvousMessage::new();
                        msg_out.set_software_update(SoftwareUpdate {
                            url: self.inner.software_url.clone(),
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

    // 处理TCP连接（包括WebSocket和NAT测试）
    #[inline]
    async fn handle_tcp(
        &mut self,
        bytes: &[u8],
        sink: &mut Option<Sink>,
        addr: SocketAddr,
        key: &str,
        ws: bool,  // 是否为WebSocket连接
    ) -> bool {
        // 解析协议消息
        if let Ok(msg_in) = RendezvousMessage::parse_from_bytes(bytes) {
            match msg_in.union {
                // 处理打洞请求
                Some(rendezvous_message::Union::PunchHoleRequest(ph)) => {
                    // 保存TCP打洞请求信息
                    if let Some(sink) = sink.take() {
                        self.tcp_punch.lock().await.insert(try_into_v4(addr), sink);
                    }
                    // 处理打洞请求
                    allow_err!(self.handle_tcp_punch_hole_request(addr, ph, key, ws).await);
                    return true;
                }
                // 处理在线请求
                Some(rendezvous_message::Union::OnlineRequest(or)) => {
                    allow_err!(self.handle_online_request(addr, or, sink).await);
                    return true;
                }
                _ => {}
            }
        }
        false
    }

    // 处理公钥注册请求
    async fn handle_register_pk(
        &mut self,
        rp: RegisterPk,
        addr: SocketAddr,
        socket: &mut FramedSocket,
    ) -> ResultType<()> {
        // 检查注册频率限制
        if let Err(e) = self.pm.check_register_freq(&addr).await {
            let mut msg_out = RendezvousMessage::new();
            msg_out.set_register_pk_response(RegisterPkResponse {
                result: TOO_FREQUENT.into(),
                ..Default::default()
            });
            socket.send(&msg_out, addr).await?;
            return Ok(());
        }
        
        // 获取peer信息
        let peer = match self.pm.get(&rp.id).await {
            Some(peer) => peer,
            None => {
                let mut msg_out = RendezvousMessage::new();
                msg_out.set_register_pk_response(RegisterPkResponse {
                    result: UUID_MISMATCH.into(),
                    ..Default::default()
                });
                socket.send(&msg_out, addr).await?;
                return Ok(());
            }
        };
        
        // 验证公钥
        if peer.pk != rp.pk {
            let mut msg_out = RendezvousMessage::new();
            msg_out.set_register_pk_response(RegisterPkResponse {
                result: UUID_MISMATCH.into(),
                ..Default::default()
            });
            socket.send(&msg_out, addr).await?;
            return Ok(());
        }
        
        // 更新peer的公钥信息
        let id = rp.id.clone();
        let addr = try_into_v4(addr);
        let ip = addr.ip().to_string();
        let (request_pk, ip_change) = {
            let r = peer.read().await;
            (r.request_pk, r.ip_change)
        };
        
        // 检查IP地址变化
        if let Some(old) = ip_change {
            log::info!("IP change of {} from {} to {}", id, old, socket_addr);
        }
        
        // 构建响应消息
        let mut msg_out = RendezvousMessage::new();
        msg_out.set_register_peer_response(RegisterPeerResponse {
            request_pk,
            ..Default::default()
        });
        socket.send(&msg_out, socket_addr).await
    }

    // 处理peer注册请求
    async fn handle_register_peer(
        &mut self,
        rp: &mut RegisterPeer,
        addr: SocketAddr,
        socket: &mut FramedSocket,
    ) -> ResultType<()> {
        // 获取peer信息
        let peer = match self.pm.get(&rp.id).await {
            Some(peer) => peer,
            None => {
                let mut msg_out = RendezvousMessage::new();
                msg_out.set_register_peer_response(RegisterPeerResponse {
                    failure: register_peer_response::Failure::ID_NOT_EXIST.into(),
                    ..Default::default()
                });
                socket.send(&msg_out, addr).await?;
                return Ok(());
            }
        };
        
        // 获取IP变化信息
        let (request_pk, ip_change) = {
            let r = peer.read().await;
            (r.request_pk, r.ip_change)
        };
        
        // 检查IP地址变化
        if let Some(old) = ip_change {
            log::info!("IP change of {} from {} to {}", rp.id, old, socket_addr);
        }
        
        // 构建响应消息
        let mut msg_out = RendezvousMessage::new();
        msg_out.set_register_peer_response(RegisterPeerResponse {
            request_pk,
            ..Default::default()
        });
        socket.send(&msg_out, addr).await
    }

    // 处理打洞完成通知
    #[inline]
    async fn handle_hole_sent<'a>(
        &mut self,
        phs: PunchHoleSent,
        addr: SocketAddr,
        socket: Option<&'a mut FramedSocket>,
    ) -> ResultType<()> {
        // 解码目标地址（从B发送的打洞完成通知）
        let addr_a = AddrMangle::decode(&phs.socket_addr);
        log::debug!(
            "{} punch hole response to {:?} from {:?}",
            if socket.is_none() { "TCP" } else { "UDP" },
            &addr_a,
            &addr
        );
        
        // 构建响应消息
        let mut msg_out = RendezvousMessage::new();
        let mut p = PunchHoleResponse {
            // 编码本机地址
            socket_addr: AddrMangle::encode(addr).into(),
            // 获取peer的公钥
            pk: self.get_pk(&phs.version, phs.id).await,
            // 中继服务器信息
            relay_server: phs.relay_server.clone(),
            ..Default::default()
        };
        // 设置NAT类型
        if let Ok(t) = phs.nat_type.enum_value() {
            p.set_nat_type(t);
        }
        msg_out.set_punch_hole_response(p);
        
        // 发送响应
        if let Some(socket) = socket {
            socket.send(&msg_out, addr_a).await?;
        } else {
            self.send_to_tcp(msg_out, addr_a).await;
        }
        Ok(())
    }

    // 处理本地地址通知
    #[inline]
    async fn handle_local_addr<'a>(
        &mut self,
        la: LocalAddr,
        addr: SocketAddr,
        socket: Option<&'a mut FramedSocket>,
    ) -> ResultType<()> {
        // 解码目标地址（从B发送的本地地址通知）
        let addr_a = AddrMangle::decode(&la.socket_addr);
        log::debug!(
            "{} local addrs response to {:?} from {:?}",
            if socket.is_none() { "TCP" } else { "UDP" },
            &addr_a,
            &addr
        );
        
        // 构建响应消息
        let mut msg_out = RendezvousMessage::new();
        let mut p = PunchHoleResponse {
            // 本地地址信息
            socket_addr: la.local_addr.clone(),
            // 获取peer的公钥
            pk: self.get_pk(&la.version, la.id).await,
            // 中继服务器信息
            relay_server: la.relay_server,
            ..Default::default()
        };
        // 标记为本地地址响应
        p.set_is_local(true);
        msg_out.set_punch_hole_response(p);
        
        // 发送响应
        if let Some(socket) = socket {
            socket.send(&msg_out, addr_a).await?;
        } else {
            self.send_to_tcp(msg_out, addr_a).await;
        }
        Ok(())
    }

    // 处理UDP打洞请求
    #[inline]
    async fn handle_udp_punch_hole_request(
        &mut self,
        addr: SocketAddr,
        ph: PunchHoleRequest,
        key: &str,
    ) -> ResultType<(RendezvousMessage, Option<SocketAddr>)> {
        // 验证许可证密钥
        let mut ph = ph;
        if !key.is_empty() && ph.licence_key != key {
            log::warn!("Authentication failed from {} for peer {} - invalid key", addr, ph.id);
            let mut msg_out = RendezvousMessage::new();
            msg_out.set_punch_hole_response(PunchHoleResponse {
                failure: punch_hole_response::Failure::LICENSE_MISMATCH.into(),
                ..Default::default()
            });
            return Ok((msg_out, None));
        }
        
        // 获取peer ID
        let id = ph.id;
        
        // 打洞请求从A发送，中继给B
        // 首先检查是否在同一内网
        // 如果在同一内网，打洞不会工作，因为所有路由器都会丢弃这种自连接
        if let Some(peer) = self.pm.get(&id).await {
            // 获取peer的注册时间和地址
            let (elapsed, peer_addr) = {
                let r = peer.read().await;
                (r.last_reg_time.elapsed().as_millis() as i32, r.socket_addr)
            };
            
            // 检查是否超时
            if elapsed >= REG_TIMEOUT {
                let mut msg_out = RendezvousMessage::new();
                msg_out.set_punch_hole_response(PunchHoleResponse {
                    failure: punch_hole_response::Failure::OFFLINE.into(),
                    ..Default::default()
                });
                return Ok((msg_out, None));
            }
            
            // 记录打洞请求（从地址 -> peer ID/peer地址）
            {
                let from_ip = try_into_v4(addr).ip().to_string();
                let to_ip = try_into_v4(peer_addr).ip().to_string();
                let to_id_clone = id.clone();
                
                // 去重检查
                let mut lock = PUNCH_REQS.lock().await;
                let mut dup = false;
                for e in lock.iter().rev().take(30) { // 只检查最近的30个条目以提高性能
                    if e.from_ip == from_ip && e.to_id == to_id_clone {
                        if e.tm.elapsed().as_secs() < PUNCH_REQ_DEDUPE_SEC { 
                            dup = true; 
                        }
                        break;
                    }
                }
                if !dup { 
                    lock.push(PunchReqEntry { 
                        tm: Instant::now(), 
                        from_ip, 
                        to_ip, 
                        to_id: to_id_clone 
                    }); 
                }
            }

            // 获取peer的NAT类型
            let peer_is_lan = self.is_lan(peer_addr);
            let is_lan = self.is_lan(addr);
            
            // 选择中继服务器
            let mut relay_server = self.get_relay_server(addr.ip(), peer_addr.ip());
            
            // 检查是否需要使用中继
            if ALWAYS_USE_RELAY.load(Ordering::SeqCst) || (peer_is_lan ^ is_lan) {
                if peer_is_lan {
                    // 如果peer在LAN，使用本地IP作为中继
                    // https://github.com/rustdesk/rustdesk-server/issues/24
                    relay_server = self.inner.local_ip.clone()
                }
                // 强制使用对称NAT类型（将强制使用中继）
                ph.nat_type = NatType::SYMMETRIC.into();
            }
            
            // 检查是否在同一内网
            let same_intranet: bool = !ws
                && (peer_is_lan && is_lan || {
                    match (peer_addr, addr) {
                        (SocketAddr::V4(a), SocketAddr::V4(b)) => a.ip() == b.ip(),
                        (SocketAddr::V6(a), SocketAddr::V6(b)) => a.ip() == b.ip(),
                        _ => false,
                    }
                });
            
            // 编码socket地址
            let socket_addr = AddrMangle::encode(addr).into();
            
            if same_intranet {
                // 在同一内网，获取本地地址
                log::debug!(
                    "Fetch local addr {:?} {:?} request from {:?}",
                    id,
                    peer_addr,
                    addr
                );
                
                // 构建获取本地地址请求
                msg_out.set_fetch_local_addr(FetchLocalAddr {
                    socket_addr,
                    relay_server,
                    ..Default::default()
                });
            } else {
                // 不同内网，执行打洞
                log::debug!(
                    "Punch hole {:?} {:?} request from {:?}",
                    id,
                    peer_addr,
                    addr
                );
                
                // 构建打洞请求
                msg_out.set_punch_hole(PunchHole {
                    socket_addr,
                    nat_type: ph.nat_type,
                    relay_server,
                    ..Default::default()
                });
            }
            
            // 返回响应和目标peer地址
            Ok((msg_out, Some(peer_addr)))
        } else {
            // peer不存在
            let mut msg_out = RendezvousMessage::new();
            msg_out.set_punch_hole_response(PunchHoleResponse {
                failure: punch_hole_response::Failure::ID_NOT_EXIST.into(),
                ..Default::default()
            });
            Ok((msg_out, None))
        }
    }

    // 处理在线状态请求
    #[inline]
    async fn handle_online_request(
        &mut self,
        stream: &mut FramedStream,
        peers: Vec<String>,
    ) -> ResultType<()> {
        // 创建状态字节数组（每个peer 1字节 + 7字节头部）
        let mut states = BytesMut::zeroed((peers.len() + 7) / 8);
        for (i, peer_id) in peers.iter().enumerate() {
            // 获取peer信息
            match self.pm.get(peer_id).await {
                Some(peer) => {
                    let r = peer.read().await;
                    // 设置在线状态（1字节）
                    states[i] = if r.disconnected.is_some() {
                        0 // 离线
                    } else {
                        1 // 在线
                    };
                }
                }
                None => {
                    states[i] = 0; // 不存在，离线
                }
            }
        }
        
        // 发送响应
        allow_err!(stream.send(&states).await);
        Ok(())
    }

    // 处理TCP打洞请求
    #[inline]
    async fn handle_tcp_punch_hole_request(
        &mut self,
        addr: SocketAddr,
        ph: PunchHoleRequest,
        key: &str,
        ws: bool,
    ) -> ResultType<()> {
        // 验证许可证密钥
        let mut ph = ph;
        if !key.is_empty() && ph.licence_key != key {
            log::warn!("Authentication failed from {} for peer {} - invalid key", addr, ph.id);
            let mut msg_out = RendezvousMessage::new();
            msg_out.set_punch_hole_response(PunchHoleResponse {
                failure: punch_hole_response::Failure::LICENSE_MISMATCH.into(),
                ..Default::default()
            });
            self.send_to_tcp(msg_out, addr).await;
            return Ok(());
        }
        
        // 获取peer信息
        let id = ph.id;
        if let Some(peer) = self.pm.get(&id).await {
            // 获取peer的注册时间和地址
            let (elapsed, peer_addr) = {
                let r = peer.read().await;
                (r.last_reg_time.elapsed().as_millis() as i32, r.socket_addr)
            };
            
            // 检查是否超时
            if elapsed >= REG_TIMEOUT {
                let mut msg_out = RendezvousMessage::new();
                msg_out.set_punch_hole_response(PunchHoleResponse {
                    failure: punch_hole_response::Failure::OFFLINE.into(),
                    ..Default::default()
                });
                self.send_to_tcp(msg_out, addr).await;
                return Ok(());
            }
            
            // 记录TCP打洞请求
            {
                let from_ip = try_into_v4(addr).ip().to_string();
                let to_ip = try_into_v4(peer_addr).ip().to_string();
                let to_id_clone = id.clone();
                
                // 去重检查
                let mut lock = PUNCH_REQS.lock().await;
                let mut dup = false;
                for e in lock.iter().rev().take(30) { // 只检查最近的30个条目
                    if e.from_ip == from_ip && e.to_id == to_id_clone {
                        if e.tm.elapsed().as_secs() < PUNCH_REQ_DEDUPE_SEC { 
                            dup = true; 
                        }
                        break;
                    }
                }
                if !dup { 
                    lock.push(PunchReqEntry { 
                        tm: Instant::now(), 
                        from_ip, 
                        to_ip, 
                        to_id: to_id_clone 
                    }); 
                }
            }

            // 获取NAT类型和地址信息
            let peer_is_lan = self.is_lan(peer_addr);
            let is_lan = self.is_lan(addr);
            let mut relay_server = self.get_relay_server(addr.ip(), peer_addr.ip());
            
            // 检查是否需要使用中继
            if ALWAYS_USE_RELAY.load(Ordering::SeqCst) || (peer_is_lan ^ is_lan) {
                if peer_is_lan {
                    // 如果peer在LAN，使用本地IP作为中继
                    relay_server = self.inner.local_ip.clone()
                }
                // 强制使用对称NAT类型
                ph.nat_type = NatType::SYMMETRIC.into();
            }
            
            // 检查是否在同一内网
            let same_intranet: bool = !ws
                && (peer_is_lan && is_lan || {
                    match (peer_addr, addr) {
                        (SocketAddr::V4(a), SocketAddr::V4(b)) => a.ip() == b.ip(),
                        (SocketAddr::V6(a), SocketAddr::V6(b)) => a.ip() == b.ip(),
                        _ => false,
                    }
                });
            
            // 构建响应消息
            let mut msg_out = RendezvousMessage::new();
            if same_intranet {
                // 同一内网，获取本地地址
                msg_out.set_fetch_local_addr(FetchLocalAddr {
                    socket_addr: AddrMangle::encode(addr).into(),
                    relay_server,
                    ..Default::default()
                });
            } else {
                // 不同内网，执行打洞
                msg_out.set_punch_hole(PunchHole {
                    socket_addr: AddrMangle::encode(peer_addr).into(),
                    nat_type: ph.nat_type,
                    relay_server,
                    ..Default::default()
                });
            }
            
            // 发送响应
            self.send_to_tcp(msg_out, addr).await;
            Ok(())
        } else {
            // peer不存在
            let mut msg_out = RendezvousMessage::new();
            msg_out.set_punch_hole_response(PunchHoleResponse {
                failure: punch_hole_response::Failure::ID_NOT_EXIST.into(),
                ..Default::default()
            });
            self.send_to_tcp(msg_out, addr).await;
            Ok(())
        }
    }

    // 检查地址是否在LAN内
    #[inline]
    fn is_lan(&self, addr: SocketAddr) -> bool {
        if let Some(mask) = self.inner.mask {
            // 使用配置的网络掩码检查
            mask.contains(try_into_v4(addr).ip())
        } else {
            // 使用默认的私有网络范围检查
            try_into_v4(addr).ip().is_private()
        }
    }

    // 获取peer的公钥
    async fn get_pk(&self, version: i32, id: String) -> Vec<u8> {
        // 首先尝试从内存获取
        if let Some(peer) = self.pm.get(&id).await {
            let r = peer.read().await;
            if r.version == version {
                return r.pk;
            }
        }
        
        // 从数据库获取
        match self.pm.get_pk(&id).await {
            Ok(pk) => pk,
            Err(err) => {
                log::error!("Failed to get pk of {} from db: {}", id, err);
                vec![]
            }
        }
    }

    // 获取中继服务器
    fn get_relay_server(&self, ip1: IpAddr, ip2: IpAddr) -> String {
        // 检查是否有可用的中继服务器
        if self.relay_servers.servers.is_empty() {
            if !self.inner.local_ip.is_empty() {
                return self.inner.local_ip.clone();
            }
        }
        
        // 优先使用IPv4地址的中继服务器
        for rs in &self.relay_servers.servers {
            for r in &rs.relay_servers {
                if r.host.contains(&ip1.to_string()) || r.host.contains(&ip2.to_string()) {
                    return r.host.clone();
                }
            }
        }
        
        // 如果没有匹配的，使用第一个
        if let Some(rs) = self.relay_servers.servers.first() {
            if let Some(r) = rs.relay_servers.first() {
                return r.host.clone();
            }
        }
        
        // 最后使用本地IP
        self.inner.local_ip.clone()
    }

    // 解析中继服务器配置
    fn parse_relay_servers(&mut self, data: &str) {
        // 清除现有配置
        self.relay_servers0 = Arc::new(vec![]);
        if data.is_empty() {
            return;
        }
        
        // 解析JSON配置
        if let Ok(rs) = serde_json::from_str::<RelayServers>(data) {
            self.relay_servers = Arc::new(rs.clone());
            let mut hosts = vec![];
            for r in &rs.servers {
                for r in &r.relay_servers {
                    hosts.push(r.host.clone());
                }
            }
            self.relay_servers0 = Arc::new(hosts);
            log::info!("Relay servers from config: {:?}", hosts);
        } else {
            // 解析逗号分隔的列表
            let hosts: Vec<String> = data
                .split(',')
                .map(|x| x.trim().to_string())
                .collect();
            self.relay_servers0 = Arc::new(hosts.clone());
            self.relay_servers = Arc::new(RelayServers {
                servers: vec![RelayServer {
                    relay_servers: hosts.iter().map(|h| RelayServerInfo {
                        host: h.clone(),
                        ..Default::default()
                    }).collect(),
                }],
            });
            log::info!("Relay servers from string: {:?}", hosts);
        }
    }

    // 发送TCP消息
    async fn send_to_tcp(&self, msg: RendezvousMessage, addr: SocketAddr) {
        // 检查TCP打洞映射表
        let addr = try_into_v4(addr);
        if let Some(sink) = self.tcp_punch.lock().await.get(&addr) {
            allow_err!(sink.send(msg.as_ref()).await);
            return;
        }
        
        // 如果没有映射，直接发送
        let mut socket = create_tcp_socket();
        timeout(3000, socket.connect(addr)).await.ok();
        allow_err!(socket.send(msg.as_ref()).await);
    }

    // 获取服务器密钥对
    fn get_server_sk(key: &str) -> (String, Vec<u8>) {
        // 从环境变量或文件加载密钥
        if let Ok(data) = std::fs::read_to_string("id_ed25519") {
            let data = data.trim();
            if let Some(sk) = data.split(' ').nth(1) {
                return (data.to_owned(), sk.as_bytes().to_vec());
            }
        }
        
        // 使用默认密钥
        let sk = sign::generate_keypair();
        let pk = sk.public_key();
        let pk = base64::encode(pk.as_bytes());
        let sk_bytes = sk.as_bytes().to_vec();
        log::info!("Key: {}=", pk);
        (pk, sk_bytes)
    }
}
