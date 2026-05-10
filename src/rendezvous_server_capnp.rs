// Cap'n Proto 版本的 Rendezvous Server 实现
// 替代原有的 proto3 实现

use crate::common::*;
use crate::peer::*;
use crate::capnp_serialization::{CapnpSerializer, CapnpDeserializer, CapnpError};
use crate::capnp_transport::{CapnpTransport, CapnpFramedTransport};
use hbb_common::{
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

// 导入capnp生成的模块
use hbb_common::protos::rendezvous_capnp;

// 数据类型枚举，用于处理不同类型的消息
#[derive(Clone, Debug)]
enum Data {
    Msg(Bytes, SocketAddr),  // 消息和来源地址
    RelayServers0(String),     // 中继服务器列表（字符串格式）
    RelayServers(String),       // 中继服务器列表（结构化格式）
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

// Cap'n Proto 版本的 RendezvousServer
#[derive(Clone)]
pub struct RendezvousServerCapnp {
    // TCP打洞请求映射表：存储发起打洞的客户端信息
    tcp_punch: Arc<Mutex<HashMap<SocketAddr, Sink>>>,
    // Peer管理器：处理所有连接的peer
    pm: PeerMap,
    // 消息发送通道
    tx: Sender<Data>,
    // 中继服务器列表
    relay_servers: Arc<Vec<String>>,
    // Rendezvous服务器列表
    rendezvous_servers: Arc<Vec<String>>,
    // 内部状态
    inner: Arc<Inner>,
}

impl RendezvousServerCapnp {
    // 启动服务器的主函数
    #[tokio::main(flavor = "multi_thread")]
    pub async fn start(port: i32, serial: i32, key: &str, rmem: usize) -> ResultType<()> {
        // 获取服务器密钥对
        let (key, sk) = Self::get_server_sk(key);
        let nat_port = port - 1;
        let ws_port = port + 2;
        let pm = PeerMap::new().await?;
        log::info!("serial={}", serial);
        let rendezvous_servers = get_servers(&get_arg("rendezvous-servers"), "rendezvous-servers");
        log::info!("Listening on tcp/udp :{}", port);
        log::info!("Listening on tcp :{}, extra port for NAT test", nat_port);
        log::info!("Listening on websocket :{}", ws_port);
        let mut socket = create_udp_listener(port, rmem).await?;
        let (tx, mut rx) = mpsc::unbounded_channel::<Data>();
        let software_url = get_arg("software-url");
        let version = hbb_common::get_version_from_url(&software_url);
        if !version.is_empty() {
            log::info!("software_url: {}, version: {}", software_url, version);
        }
        let mask = get_arg("mask").parse().ok();
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
        let mut rs = Self {
            tcp_punch: Arc::new(Mutex::new(HashMap::new())),
            pm,
            tx: tx.clone(),
            relay_servers: Arc::new(vec![]),
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
        log::info!("mask: {:?}", rs.inner.mask);
        log::info!("local-ip: {:?}", rs.inner.local_ip);
        std::env::set_var("PORT_FOR_API", port.to_string());
        rs.parse_relay_servers(&get_arg("relay-servers"));
        let mut listener = create_tcp_listener(port).await?;
        let mut listener2 = create_tcp_listener(nat_port).await?;
        let mut listener3 = create_tcp_listener(ws_port).await?;
        let test_addr = std::env::var("TEST_HBBS").unwrap_or_default();
        if std::env::var("ALWAYS_USE_RELAY")
            .unwrap_or_default()
            .to_uppercase()
            == "Y"
        {
            ALWAYS_USE_RELAY.store(true, Ordering::SeqCst);
        }
        log::info!(
            "ALWAYS_USE_RELAY={}",
            if ALWAYS_USE_RELAY.load(Ordering::SeqCst) {
                "Y"
            } else {
                "N"
            }
        );
        if test_addr.to_lowercase() != "no" {
            let test_addr = if test_addr.is_empty() {
                listener.local_addr()?
            } else {
                test_addr.parse()?
            };
            tokio::spawn(async move {
                if let Err(err) = test_hbbs(test_addr).await {
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
        let listen_signal = listen_signal();
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
                    if self.relay_servers.len() > 1 {
                        let rs = self.relay_servers.clone();
                        let tx = self.tx.clone();
                        tokio::spawn(async move {
                            check_relay_servers(rs, tx).await;
                        });
                    }
                }
                // 处理内部消息
                Some(data) = rx.recv() => {
                    match data {
                        Data::Msg(bytes, addr) => { 
                            // 发送UDP消息
                            allow_err!(socket.send_to(&bytes, &addr).await); 
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
                            stream.set_nodelay(true).ok();
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
                            stream.set_nodelay(true).ok();
                            self.handle_listener(stream, addr, key, true).await;
                        }
                        Err(err) => {
                           log::error!("listener3.accept failed: {}", err);
                           return LoopFailure::Listener3;
                        }
                    }
                }
                // 处理主TCP连接
                res = listener.accept() => {
                    match res {
                        Ok((stream, addr)) => {
                            stream.set_nodelay(true).ok();
                            self.handle_listener(stream, addr, key, false).await;
                        }
                        Err(err) => {
                           log::error!("listener.accept failed: {}", err);
                           return LoopFailure::Listener;
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
        // 使用Cap'n Proto反序列化消息
        match CapnpDeserializer::deserialize_message::<rendezvous_capnp::RendezvousMessage>(bytes) {
            Ok(message) => {
                // 处理根消息
                self.handle_rendezvous_message(message, addr, socket, key).await
            }
            Err(e) => {
                log::error!("Failed to deserialize Cap'n Proto message: {}", e);
                Ok(())
            }
        }
    }

    // 处理Rendezvous消息
    async fn handle_rendezvous_message(
        &mut self,
        message: rendezvous_capnp::RendezvousMessage,
        addr: SocketAddr,
        socket: &mut FramedSocket,
        key: &str,
    ) -> ResultType<()> {
        // 根据消息类型分发处理
        match message.which() {
            Ok(rendezvous_capnp::RendezvousMessage::RegisterPeer(register_peer)) => {
                self.handle_register_peer_capnp(register_peer, addr, socket).await?;
            }
            Ok(rendezvous_capnp::RendezvousMessage::RegisterPeerResponse(register_peer_response)) => {
                self.handle_register_peer_response_capnp(register_peer_response, addr, socket).await?;
            }
            Ok(rendezvous_capnp::RendezvousMessage::PunchHoleRequest(punch_hole_request)) => {
                self.handle_punch_hole_request_capnp(punch_hole_request, addr, socket, key).await?;
            }
            Ok(rendezvous_capnp::RendezvousMessage::PunchHole(punch_hole)) => {
                self.handle_punch_hole_capnp(punch_hole, addr, socket).await?;
            }
            Ok(rendezvous_capnp::RendezvousMessage::PunchHoleSent(punch_hole_sent)) => {
                self.handle_punch_hole_sent_capnp(punch_hole_sent, addr, socket).await?;
            }
            Ok(rendezvous_capnp::RendezvousMessage::PunchHoleResponse(punch_hole_response)) => {
                self.handle_punch_hole_response_capnp(punch_hole_response, addr, socket).await?;
            }
            Ok(rendezvous_capnp::RendezvousMessage::FetchLocalAddr(fetch_local_addr)) => {
                self.handle_fetch_local_addr_capnp(fetch_local_addr, addr, socket).await?;
            }
            Ok(rendezvous_capnp::RendezvousMessage::LocalAddr(local_addr)) => {
                self.handle_local_addr_capnp(local_addr, addr, socket).await?;
            }
            Ok(rendezvous_capnp::RendezvousMessage::ConfigureUpdate(configure_update)) => {
                self.handle_configure_update_capnp(configure_update, addr, socket).await?;
            }
            Ok(rendezvous_capnp::RendezvousMessage::SoftwareUpdate(software_update)) => {
                self.handle_software_update_capnp(software_update, addr, socket).await?;
            }
            Ok(rendezvous_capnp::RendezvousMessage::RegisterPk(register_pk)) => {
                self.handle_register_pk_capnp(register_pk, addr, socket).await?;
            }
            Ok(rendezvous_capnp::RendezvousMessage::RegisterPkResponse(register_pk_response)) => {
                self.handle_register_pk_response_capnp(register_pk_response, addr, socket).await?;
            }
            Ok(rendezvous_capnp::RendezvousMessage::RequestRelay(request_relay)) => {
                self.handle_request_relay_capnp(request_relay, addr, socket, key).await?;
            }
            Ok(rendezvous_capnp::RendezvousMessage::RelayResponse(relay_response)) => {
                self.handle_relay_response_capnp(relay_response, addr, socket).await?;
            }
            Ok(rendezvous_capnp::RendezvousMessage::TestNatRequest(test_nat_request)) => {
                self.handle_test_nat_request_capnp(test_nat_request, addr, socket).await?;
            }
            Ok(rendezvous_capnp::RendezvousMessage::TestNatResponse(test_nat_response)) => {
                self.handle_test_nat_response_capnp(test_nat_response, addr, socket).await?;
            }
            Ok(rendezvous_capnp::RendezvousMessage::PeerDiscovery(peer_discovery)) => {
                self.handle_peer_discovery_capnp(peer_discovery, addr, socket).await?;
            }
            Ok(rendezvous_capnp::RendezvousMessage::OnlineRequest(online_request)) => {
                self.handle_online_request_capnp(online_request, addr, socket).await?;
            }
            Ok(rendezvous_capnp::RendezvousMessage::OnlineResponse(online_response)) => {
                self.handle_online_response_capnp(online_response, addr, socket).await?;
            }
            Ok(rendezvous_capnp::RendezvousMessage::KeyExchange(key_exchange)) => {
                self.handle_key_exchange_capnp(key_exchange, addr, socket).await?;
            }
            Ok(rendezvous_capnp::RendezvousMessage::HealthCheck(health_check)) => {
                self.handle_health_check_capnp(health_check, addr, socket).await?;
            }
            _ => {
                log::warn!("Unknown message type received from {}", addr);
            }
        }
        Ok(())
    }

    // 处理RegisterPeer消息（Cap'n Proto版本）
    async fn handle_register_peer_capnp(
        &mut self,
        register_peer: rendezvous_capnp::RegisterPeer::Reader,
        addr: SocketAddr,
        socket: &mut FramedSocket,
    ) -> ResultType<()> {
        // 读取字段
        let id = register_peer.get_id()?;
        let serial = register_peer.get_serial();
        
        log::debug!("Register peer request from {}: id={}, serial={}", addr, id, serial);
        
        // 检查注册频率限制
        if let Err(e) = self.pm.check_register_freq(&addr).await {
            let response = self.create_register_peer_response_error(rendezvous_capnp::RegisterResult::TooFrequent);
            let serialized = CapnpSerializer::serialize_message(&response)?;
            socket.send_to(&serialized, &addr).await?;
            return Ok(());
        }
        
        // 获取或创建peer信息
        let peer = match self.pm.get(&id).await {
            Some(peer) => peer,
            None => {
                let response = self.create_register_peer_response_error(rendezvous_capnp::RegisterResult::IdExists);
                let serialized = CapnpSerializer::serialize_message(&response)?;
                socket.send_to(&serialized, &addr).await?;
                return Ok(());
            }
        };
        
        // 更新peer信息
        self.pm.update_peer_info(&id, serial, addr).await;
        
        // 构建成功响应
        let mut response = self.create_register_peer_response_success();
        response.get_request_pk().set(true);
        
        // 序列化并发送响应
        let serialized = CapnpSerializer::serialize_message(&response)?;
        socket.send_to(&serialized, &addr).await?;
        
        Ok(())
    }

    // 创建RegisterPeerResponse错误响应
    fn create_register_peer_response_error(&self, error: rendezvous_capnp::RegisterResult) -> rendezvous_capnp::RegisterPeerResponse::Builder<'static> {
        let mut response = rendezvous_capnp::RegisterPeerResponse::new_default();
        response.set_request_pk(false);
        response.set_result(error);
        response
    }

    // 创建RegisterPeerResponse成功响应
    fn create_register_peer_response_success(&self) -> rendezvous_capnp::RegisterPeerResponse::Builder<'static> {
        let mut response = rendezvous_capnp::RegisterPeerResponse::new_default();
        response.set_request_pk(true);
        response.set_result(rendezvous_capnp::RegisterResult::Ok);
        response
    }

    // 处理PunchHoleRequest消息（Cap'n Proto版本）
    async fn handle_punch_hole_request_capnp(
        &mut self,
        punch_hole_request: rendezvous_capnp::PunchHoleRequest::Reader,
        addr: SocketAddr,
        socket: &mut FramedSocket,
        key: &str,
    ) -> ResultType<()> {
        // 读取字段
        let id = punch_hole_request.get_id()?;
        let nat_type = punch_hole_request.get_nat_type();
        let licence_key = punch_hole_request.get_licence_key()?;
        let conn_type = punch_hole_request.get_conn_type();
        let token = punch_hole_request.get_token()?;
        let version = punch_hole_request.get_version()?;
        
        log::debug!("Punch hole request from {}: id={}, nat_type={:?}", addr, id, nat_type);
        
        // 验证许可证密钥
        if !key.is_empty() && licence_key != key {
            log::warn!("Authentication failed from {} for peer {} - invalid key", addr, id);
            let response = self.create_punch_hole_response_license_mismatch();
            let serialized = CapnpSerializer::serialize_message(&response)?;
            socket.send_to(&serialized, &addr).await?;
            return Ok(());
        }
        
        // 获取peer信息
        let peer = match self.pm.get(&id).await {
            Some(peer) => peer,
            None => {
                let response = self.create_punch_hole_response_id_not_exist();
                let serialized = CapnpSerializer::serialize_message(&response)?;
                socket.send_to(&serialized, &addr).await?;
                return Ok(());
            }
        };
        
        // 检查peer是否在线
        let (elapsed, peer_addr) = {
            let r = peer.read().await;
            (r.last_reg_time.elapsed().as_millis() as i32, r.socket_addr)
        };
        
        if elapsed >= REG_TIMEOUT {
            let response = self.create_punch_hole_response_offline();
            let serialized = CapnpSerializer::serialize_message(&response)?;
            socket.send_to(&serialized, &addr).await?;
            return Ok(());
        }
        
        // 检查NAT类型和是否需要中继
        let peer_is_lan = self.is_lan(peer_addr);
        let is_lan = self.is_lan(addr);
        let mut relay_server = self.get_relay_server(addr.ip(), peer_addr.ip());
        
        if ALWAYS_USE_RELAY.load(Ordering::SeqCst) || (peer_is_lan ^ is_lan) {
            if peer_is_lan {
                relay_server = self.inner.local_ip.clone()
            }
            // 强制使用对称NAT类型
            nat_type = rendezvous_capnp::NatType::SymmetricNat;
        }
        
        // 检查是否在同一内网
        let same_intranet = (peer_is_lan && is_lan) || {
            match (peer_addr, addr) {
                (SocketAddr::V4(a), SocketAddr::V4(b)) => a.ip() == b.ip(),
                (SocketAddr::V6(a), SocketAddr::V6(b)) => a.ip() == b.ip(),
                _ => false,
            }
        };
        
        // 构建响应消息
        let mut response = rendezvous_capnp::PunchHoleResponse::new_default();
        
        if same_intranet {
            // 同一内网，获取本地地址
            let mut fetch_local_addr = rendezvous_capnp::FetchLocalAddr::new_default();
            fetch_local_addr.set_socket_addr(self.encode_socket_addr(&addr)?);
            fetch_local_addr.set_relay_server(&relay_server);
            
            let mut punch_hole = rendezvous_capnp::PunchHole::new_default();
            punch_hole.set_socket_addr(self.encode_socket_addr(&addr)?);
            punch_hole.set_nat_type(nat_type);
            punch_hole.set_relay_server(&relay_server);
            
            response.set_fetch_local_addr(fetch_local_addr);
        } else {
            // 不同内网，执行打洞
            let mut punch_hole = rendezvous_capnp::PunchHole::new_default();
            punch_hole.set_socket_addr(self.encode_socket_addr(&peer_addr)?);
            punch_hole.set_nat_type(nat_type);
            punch_hole.set_relay_server(&relay_server);
            
            response.set_punch_hole(punch_hole);
        }
        
        // 序列化并发送响应
        let serialized = CapnpSerializer::serialize_message(&response)?;
        socket.send_to(&serialized, &addr).await?;
        
        Ok(())
    }

    // 编码socket地址为Cap'n Proto Data格式
    fn encode_socket_addr(&self, addr: &SocketAddr) -> Result<capnp::Data::Builder<'static>, CapnpError> {
        let mut data = capnp::Data::new_default();
        let addr_bytes = addr.to_string().into_bytes();
        data.init_data(addr_bytes.len() as u32);
        data.get_data().copy_from_slice(&addr_bytes);
        Ok(data)
    }

    // 创建PunchHoleResponse错误响应
    fn create_punch_hole_response_license_mismatch(&self) -> rendezvous_capnp::PunchHoleResponse::Builder<'static> {
        let mut response = rendezvous_capnp::PunchHoleResponse::new_default();
        response.set_failure(rendezvous_capnp::Failure::LicenseMismatch);
        response
    }

    // 创建PunchHoleResponse ID不存在响应
    fn create_punch_hole_response_id_not_exist(&self) -> rendezvous_capnp::PunchHoleResponse::Builder<'static> {
        let mut response = rendezvous_capnp::PunchHoleResponse::new_default();
        response.set_failure(rendezvous_capnp::Failure::IdNotExist);
        response
    }

    // 创建PunchHoleResponse离线响应
    fn create_punch_hole_response_offline(&self) -> rendezvous_capnp::PunchHoleResponse::Builder<'static> {
        let mut response = rendezvous_capnp::PunchHoleResponse::new_default();
        response.set_failure(rendezvous_capnp::Failure::Offline);
        response
    }

    // 其他消息处理方法类似...
    async fn handle_punch_hole_capnp(&mut self, punch_hole: rendezvous_capnp::PunchHole::Reader, addr: SocketAddr, socket: &mut FramedSocket) -> ResultType<()> {
        // 实现PunchHole消息处理
        log::debug!("Punch hole message from {:?}", addr);
        Ok(())
    }

    async fn handle_punch_hole_sent_capnp(&mut self, punch_hole_sent: rendezvous_capnp::PunchHoleSent::Reader, addr: SocketAddr, socket: &mut FramedSocket) -> ResultType<()> {
        // 实现PunchHoleSent消息处理
        log::debug!("Punch hole sent message from {:?}", addr);
        Ok(())
    }

    async fn handle_punch_hole_response_capnp(&mut self, punch_hole_response: rendezvous_capnp::PunchHoleResponse::Reader, addr: SocketAddr, socket: &mut FramedSocket) -> ResultType<()> {
        // 实现PunchHoleResponse消息处理
        log::debug!("Punch hole response message from {:?}", addr);
        Ok(())
    }

    async fn handle_fetch_local_addr_capnp(&mut self, fetch_local_addr: rendezvous_capnp::FetchLocalAddr::Reader, addr: SocketAddr, socket: &mut FramedSocket) -> ResultType<()> {
        // 实现FetchLocalAddr消息处理
        log::debug!("Fetch local addr message from {:?}", addr);
        Ok(())
    }

    async fn handle_local_addr_capnp(&mut self, local_addr: rendezvous_capnp::LocalAddr::Reader, addr: SocketAddr, socket: &mut FramedSocket) -> ResultType<()> {
        // 实现LocalAddr消息处理
        log::debug!("Local addr message from {:?}", addr);
        Ok(())
    }

    async fn handle_configure_update_capnp(&mut self, configure_update: rendezvous_capnp::ConfigureUpdate::Reader, addr: SocketAddr, socket: &mut FramedSocket) -> ResultType<()> {
        // 实现ConfigureUpdate消息处理
        log::debug!("Configure update message from {:?}", addr);
        Ok(())
    }

    async fn handle_software_update_capnp(&mut self, software_update: rendezvous_capnp::SoftwareUpdate::Reader, addr: SocketAddr, socket: &mut FramedSocket) -> ResultType<()> {
        // 实现SoftwareUpdate消息处理
        log::debug!("Software update message from {:?}", addr);
        Ok(())
    }

    async fn handle_register_pk_capnp(&mut self, register_pk: rendezvous_capnp::RegisterPk::Reader, addr: SocketAddr, socket: &mut FramedSocket) -> ResultType<()> {
        // 实现RegisterPk消息处理
        log::debug!("Register PK message from {:?}", addr);
        Ok(())
    }

    async fn handle_register_pk_response_capnp(&mut self, register_pk_response: rendezvous_capnp::RegisterPkResponse::Reader, addr: SocketAddr, socket: &mut FramedSocket) -> ResultType<()> {
        // 实现RegisterPkResponse消息处理
        log::debug!("Register PK response message from {:?}", addr);
        Ok(())
    }

    async fn handle_request_relay_capnp(&mut self, request_relay: rendezvous_capnp::RequestRelay::Reader, addr: SocketAddr, socket: &mut FramedSocket, key: &str) -> ResultType<()> {
        // 实现RequestRelay消息处理
        log::debug!("Request relay message from {:?}", addr);
        Ok(())
    }

    async fn handle_relay_response_capnp(&mut self, relay_response: rendezvous_capnp::RelayResponse::Reader, addr: SocketAddr, socket: &mut FramedSocket) -> ResultType<()> {
        // 实现RelayResponse消息处理
        log::debug!("Relay response message from {:?}", addr);
        Ok(())
    }

    async fn handle_test_nat_request_capnp(&mut self, test_nat_request: rendezvous_capnp::TestNatRequest::Reader, addr: SocketAddr, socket: &mut FramedSocket) -> ResultType<()> {
        // 实现TestNatRequest消息处理
        log::debug!("Test NAT request message from {:?}", addr);
        Ok(())
    }

    async fn handle_test_nat_response_capnp(&mut self, test_nat_response: rendezvous_capnp::TestNatResponse::Reader, addr: SocketAddr, socket: &mut FramedSocket) -> ResultType<()> {
        // 实现TestNatResponse消息处理
        log::debug!("Test NAT response message from {:?}", addr);
        Ok(())
    }

    async fn handle_peer_discovery_capnp(&mut self, peer_discovery: rendezvous_capnp::PeerDiscovery::Reader, addr: SocketAddr, socket: &mut FramedSocket) -> ResultType<()> {
        // 实现PeerDiscovery消息处理
        log::debug!("Peer discovery message from {:?}", addr);
        Ok(())
    }

    async fn handle_online_request_capnp(&mut self, online_request: rendezvous_capnp::OnlineRequest::Reader, addr: SocketAddr, socket: &mut FramedSocket) -> ResultType<()> {
        // 实现OnlineRequest消息处理
        log::debug!("Online request message from {:?}", addr);
        Ok(())
    }

    async fn handle_online_response_capnp(&mut self, online_response: rendezvous_capnp::OnlineResponse::Reader, addr: SocketAddr, socket: &mut FramedSocket) -> ResultType<()> {
        // 实现OnlineResponse消息处理
        log::debug!("Online response message from {:?}", addr);
        Ok(())
    }

    async fn handle_key_exchange_capnp(&mut self, key_exchange: rendezvous_capnp::KeyExchange::Reader, addr: SocketAddr, socket: &mut FramedSocket) -> ResultType<()> {
        // 实现KeyExchange消息处理
        log::debug!("Key exchange message from {:?}", addr);
        Ok(())
    }

    async fn handle_health_check_capnp(&mut self, health_check: rendezvous_capnp::HealthCheck::Reader, addr: SocketAddr, socket: &mut FramedSocket) -> ResultType<()> {
        // 实现HealthCheck消息处理
        log::debug!("Health check message from {:?}", addr);
        Ok(())
    }

    // 处理TCP连接
    async fn handle_listener(
        &mut self,
        stream: FramedStream,
        addr: SocketAddr,
        key: &str,
        ws: bool,
    ) -> ResultType<()> {
        // 使用Cap'n Proto传输器处理TCP连接
        let transport = CapnpFramedTransport::new(stream);
        
        // 处理所有消息
        transport.next_message(|message| {
            self.handle_rendezvous_message(message, addr, &mut socket, key)
        }).await
    }

    // 检查地址是否在LAN内
    fn is_lan(&self, addr: SocketAddr) -> bool {
        if let Some(mask) = self.inner.mask {
            mask.contains(try_into_v4(addr).ip())
        } else {
            try_into_v4(addr).ip().is_private()
        }
    }

    // 获取中继服务器
    fn get_relay_server(&self, ip1: IpAddr, ip2: IpAddr) -> String {
        if self.relay_servers.is_empty() {
            if !self.inner.local_ip.is_empty() {
                return self.inner.local_ip.clone();
            }
        }
        
        for rs in &*self.relay_servers {
            for r in rs.split(',') {
                let r = r.trim();
                if r.contains(&ip1.to_string()) || r.contains(&ip2.to_string()) {
                    return r.to_string();
                }
            }
        }
        
        if let Some(first) = self.relay_servers.first() {
            first.clone()
        } else {
            self.inner.local_ip.clone()
        }
    }

    // 解析中继服务器配置
    fn parse_relay_servers(&mut self, data: &str) {
        self.relay_servers = Arc::new(
            data.split(',')
                .map(|x| x.trim().to_string())
                .filter(|x| !x.is_empty())
                .collect(),
        );
        log::info!("Relay servers: {:?}", self.relay_servers);
    }

    // 获取服务器密钥对
    fn get_server_sk(key: &str) -> (String, Vec<u8>) {
        if let Ok(data) = std::fs::read_to_string("id_ed25519") {
            let data = data.trim();
            if let Some(sk) = data.split(' ').nth(1) {
                return (data.to_owned(), sk.as_bytes().to_vec());
            }
        }
        
        let sk = sign::generate_keypair();
        let pk = sk.public_key();
        let pk = base64::encode(pk.as_bytes());
        let sk_bytes = sk.as_bytes().to_vec();
        log::info!("Key: {}=", pk);
        (pk, sk_bytes)
    }
}
