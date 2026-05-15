//! 渲染同端中介模块（参照 RustDesk rendezvous_mediator.rs）
//!
//! 功能：
//! 1. 以 TCP 或 UDP 方式连接到 hbbs（rendezvous server）
//! 2. 周期性注册自身 ID / 公钥
//! 3. 处理服务器推送的打洞请求、中继请求、本地地址查询
//! 4. 支持 punch hole（TCP & UDP 打洞）和中继连接

use crate::config::ClientConfig;
use crate::port_forward::PortForwardManager;
use core_common::{
    allow_err,
    anyhow::{self, bail},
    bytes,
    config::{CONNECT_TIMEOUT, REG_INTERVAL, RENDEZVOUS_PORT},
    futures::future::join_all,
    log,
    protobuf::Message as _,
    rendezvous_codec::{self, Protocol},
    rendezvous_proto::{
        register_pk_response, rendezvous_message, FetchLocalAddr, LocalAddr, PunchHole,
        PunchHoleSent, RegisterPk, RelayResponse, RendezvousMessage, RequestRelay,
    },
    sleep,
    socket_client::{self, connect_tcp},
    AddrMangle, ResultType,
};
use std::{
    net::SocketAddr,
    sync::atomic::{AtomicBool, Ordering},
    time::Instant,
};

/// 按配置的线路协议发送 `RendezvousMessage`（proto3 或 capnp）
async fn send_rendezvous(
    conn: &mut core_common::Stream,
    msg: &RendezvousMessage,
    wire: Protocol,
) -> ResultType<()> {
    if let Some(b) = rendezvous_codec::serialize(msg, wire) {
        conn.send_bytes(b).await
    } else {
        conn.send(msg).await
    }
}

// ──────────────────────────────────────────────────────────────────────────────
// 全局状态
// ──────────────────────────────────────────────────────────────────────────────

/// 当前是否在线（已成功向服务器注册）
pub static ONLINE: AtomicBool = AtomicBool::new(false);
/// 信号中介重启
static SHOULD_EXIT: AtomicBool = AtomicBool::new(false);
static MANUAL_RESTARTED: AtomicBool = AtomicBool::new(false);

// ──────────────────────────────────────────────────────────────────────────────
// RendezvousMediator 结构体
// ──────────────────────────────────────────────────────────────────────────────

#[derive(Clone)]
pub struct RendezvousMediator {
    /// 服务器地址（含端口）
    host: String,
    /// host 前缀（用于 key 确认状态存储）
    host_prefix: String,
    /// 与 hbbs 之间的帧编码（proto3 / capnp）
    wire: Protocol,
}

impl RendezvousMediator {
    // ── 控制接口 ─────────────────────────────────────────────────────────────

    /// 触发中介重启（在配置变更时调用）
    pub fn restart() {
        SHOULD_EXIT.store(true, Ordering::SeqCst);
        MANUAL_RESTARTED.store(true, Ordering::SeqCst);
        log::info!("[mediator] 收到重启信号");
    }

    // ── 主循环 ───────────────────────────────────────────────────────────────

    /// 启动所有已配置的 rendezvous 服务器连接（永不返回）
    pub async fn start_all() {
        loop {
            let servers = ClientConfig::get_rendezvous_servers();
            if servers.is_empty() {
                log::warn!("[mediator] 未配置 rendezvous 服务器，30 秒后重试");
                sleep(30.0).await;
                continue;
            }

            SHOULD_EXIT.store(false, Ordering::SeqCst);
            MANUAL_RESTARTED.store(false, Ordering::SeqCst);

            let mut futs = Vec::new();
            for host in servers {
                futs.push(tokio::spawn(async move {
                    if let Err(e) = RendezvousMediator::start(host.clone()).await {
                        log::error!("[mediator] {} 连接错误: {}", host, e);
                    }
                    SHOULD_EXIT.store(true, Ordering::SeqCst);
                }));
            }

            join_all(futs).await;
            ONLINE.store(false, Ordering::SeqCst);

            if !MANUAL_RESTARTED.load(Ordering::SeqCst) {
                log::info!("[mediator] 5 秒后重连...");
                sleep(5.0).await;
            } else {
                sleep(0.1).await;
            }
        }
    }

    /// 对单个服务器启动连接
    pub async fn start(host: String) -> ResultType<()> {
        let host_with_port = socket_client::check_port(&host, RENDEZVOUS_PORT);
        Self::start_tcp(host_with_port).await
    }

    // ── TCP 模式（主要模式）─────────────────────────────────────────────────

    /// TCP 方式连接到 hbbs 并保持注册
    async fn start_tcp(host: String) -> ResultType<()> {
        let mut conn = connect_tcp(host.clone(), CONNECT_TIMEOUT).await?;

        let host_prefix = Self::get_host_prefix(&host);
        let wire = ClientConfig::get_rendezvous_wire_protocol();
        let rz = RendezvousMediator {
            host: host.clone(),
            host_prefix: host_prefix.clone(),
            wire,
        };
        log::info!("[mediator] TCP 连接成功: {}（线路协议: {:?}）", host, wire);

        // 重置本 host 的 key 确认状态，确保本轮重新验证
        ClientConfig::set_host_key_confirmed(&host_prefix, false);

        let mut timer = tokio::time::interval(std::time::Duration::from_millis(500));
        let mut last_register_sent: Option<Instant> = None;
        let mut last_recv_msg = Instant::now();

        loop {
            tokio::select! {
                // ── 接收服务器消息 ──────────────────────────────────────────
                res = conn.next() => {
                    last_recv_msg = Instant::now();
                    let bytes = match res {
                        Some(Ok(b)) => b,
                        Some(Err(e)) => bail!("读取错误: {}", e),
                        None => bail!("连接被服务器重置"),
                    };
                    if bytes.is_empty() {
                        // 心跳：原样回应
                        conn.send_bytes(bytes::Bytes::new()).await?;
                        #[allow(clippy::needless_continue)]
                        continue;
                    }
                    let msg = rendezvous_codec::parse(&bytes)
                        .ok_or_else(|| anyhow::anyhow!("无法解析 rendezvous 消息（proto3/capnp）"))?;
                    rz.handle_msg(msg.union, &mut conn, &mut last_register_sent)
                        .await?;
                }

                // ── 定时器：保活 & 注册 ────────────────────────────────────
                _ = timer.tick() => {
                    if SHOULD_EXIT.load(Ordering::SeqCst) {
                        log::info!("[mediator] {} 收到退出信号", host);
                        break;
                    }

                    // keep-alive 超时检测（1.5 × 15s = 22.5s）
                    if last_recv_msg.elapsed().as_millis() > 22_500 {
                        bail!("[mediator] {} 心跳超时，重新连接", host);
                    }

                    // 需要重新注册时发送 RegisterPk / RegisterPeer
                    let need_reg = !ClientConfig::get_key_confirmed()
                        || !ClientConfig::get_host_key_confirmed(&host_prefix);
                    let elapsed = last_register_sent
                        .map(|t| t.elapsed().as_millis() as i64)
                        .unwrap_or(REG_INTERVAL);
                    if need_reg && elapsed >= REG_INTERVAL {
                        log::info!("=============elapsed:{:?}  REG_INTERVAL: {:?} need_reg:{:?}====", elapsed,REG_INTERVAL,need_reg);
                        rz.register_pk(&mut conn).await?;
                        last_register_sent = Some(Instant::now());
                    }
                }
            }
        }

        Ok(())
    }

    // ── 消息处理 ─────────────────────────────────────────────────────────────

    async fn handle_msg(
        &self,
        msg: Option<rendezvous_message::Union>,
        conn: &mut core_common::Stream,
        last_register_sent: &mut Option<Instant>,
    ) -> ResultType<()> {
        match msg {
            // ── RegisterPeerResponse：服务器要求提交公钥 ──────────────────
            Some(rendezvous_message::Union::RegisterPeerResponse(rpr)) => {
                // 计算延迟
                let latency = last_register_sent
                    .map(|t| t.elapsed().as_micros() as i64)
                    .unwrap_or(0);
                *last_register_sent = None;
                log::debug!("[mediator] {} 延迟 {}ms", self.host, latency / 1000);

                if rpr.request_pk {
                    log::info!("[mediator] {} 要求提交公钥", self.host);
                    self.register_pk(conn).await?;
                } else {
                    // 已注册成功
                    ONLINE.store(true, Ordering::SeqCst);
                }
            }

            // ── RegisterPkResponse：公钥注册结果 ─────────────────────────
            Some(rendezvous_message::Union::RegisterPkResponse(rpr)) => {
                match rpr.result.enum_value() {
                    Ok(register_pk_response::Result::OK) => {
                        ClientConfig::set_key_confirmed(true);
                        ClientConfig::set_host_key_confirmed(&self.host_prefix, true);
                        ONLINE.store(true, Ordering::SeqCst);
                        log::info!("[mediator] {} 公钥确认成功，已上线", self.host);
                    }
                    Ok(register_pk_response::Result::UUID_MISMATCH) => {
                        log::warn!("[mediator] {} UUID 不匹配，重新生成 ID", self.host);
                        ClientConfig::set_key_confirmed(false);
                        // 重新注册
                        self.register_pk(conn).await?;
                    }
                    _ => {
                        log::error!("[mediator] {} 公钥注册失败", self.host);
                    }
                }
                // 更新 keep-alive 间隔（服务器可能指定）
                if rpr.keep_alive > 0 {
                    log::info!("[mediator] keep-alive = {}s", rpr.keep_alive);
                }
            }

            // ── PunchHole：有对端要连接本机 ──────────────────────────────
            Some(rendezvous_message::Union::PunchHole(ph)) => {
                let rz = self.clone();
                tokio::spawn(async move {
                    allow_err!(rz.handle_punch_hole(ph).await);
                });
            }

            // ── RequestRelay：服务器要求通过中继建连 ─────────────────────
            Some(rendezvous_message::Union::RequestRelay(rr)) => {
                let rz = self.clone();
                tokio::spawn(async move {
                    allow_err!(rz.handle_request_relay(rr).await);
                });
            }

            // ── FetchLocalAddr：对端在同一 NAT 后，服务器来取本机地址 ────
            Some(rendezvous_message::Union::FetchLocalAddr(fla)) => {
                let rz = self.clone();
                tokio::spawn(async move {
                    allow_err!(rz.handle_fetch_local_addr(fla).await);
                });
            }

            // ── ConfigureUpdate：服务器推送新配置 ────────────────────────
            Some(rendezvous_message::Union::ConfigureUpdate(cu)) => {
                log::info!("[mediator] 收到服务器配置更新，serial={}", cu.serial);
                let servers = cu.rendezvous_servers.join(",");
                if !servers.is_empty()
                    && servers != ClientConfig::get_rendezvous_servers().join(",")
                {
                    log::info!("[mediator] rendezvous 服务器列表更新: {}", servers);
                    ClientConfig::update(|c| c.rendezvous_servers = servers);
                    Self::restart();
                }
            }

            _ => {}
        }
        Ok(())
    }

    // ── 注册 ─────────────────────────────────────────────────────────────────

    /// 发送 RegisterPk（首次注册或 UUID 不匹配后调用）
    ///
    /// 若用户已登录，自动在 `user_token` 字段携带 JWT，
    /// 服务端（nat-server peer.rs）收到后完成用户-设备绑定。
    async fn register_pk(&self, conn: &mut core_common::Stream) -> ResultType<()> {
        let id = ClientConfig::get_id();
        let pk = ClientConfig::get_key_pair().1;
        let uuid = ClientConfig::get_uuid_bytes();

        // 读取当前有效 JWT（未登录或已过期时为空字符串）
        let user_token = ClientConfig::get_auth_token().unwrap_or_default();
        if !user_token.is_empty() {
            log::debug!("[mediator] RegisterPk 携带 user_token（用户已登录）");
        }

        let mut msg = RendezvousMessage::new();
        msg.set_register_pk(RegisterPk {
            id,
            uuid: uuid.into(),
            pk: pk.into(),
            user_token,
            ..Default::default()
        });
        send_rendezvous(conn, &msg, self.wire).await?;
        log::debug!("[mediator] RegisterPk 已发送至 {}", self.host);
        Ok(())
    }

    // ── 打洞处理 ─────────────────────────────────────────────────────────────

    /// 处理 PunchHole：有对端要连接本机
    ///
    /// 策略：
    /// 1. 若对端是对称 NAT 或强制中继 → 走中继
    /// 2. 否则尝试 TCP 直连打洞；失败则回落到中继
    async fn handle_punch_hole(&self, ph: PunchHole) -> ResultType<()> {
        let peer_addr = AddrMangle::decode(&ph.socket_addr);
        log::info!(
            "[mediator] PunchHole 请求来自 {:?}, nat_type={:?}",
            peer_addr,
            ph.nat_type
        );

        let relay_server = self.get_relay_server(ph.relay_server.clone());
        let uuid = uuid::Uuid::new_v4().to_string();

        // 强制中继，或对称 NAT
        let force_relay = ph.force_relay
            || ph.nat_type.enum_value_or_default()
                == core_common::rendezvous_proto::NatType::SYMMETRIC;

        if force_relay {
            log::info!("[mediator] 对端强制中继，uuid={}", uuid);
            return self
                .create_relay_connection(ph.socket_addr.to_vec(), relay_server, uuid, true, true)
                .await;
        }

        // 尝试 TCP 打洞
        match self
            .punch_tcp_hole(
                peer_addr,
                ph.socket_addr.to_vec(),
                relay_server.clone(),
                uuid.clone(),
            )
            .await
        {
            Ok(_) => {
                log::info!("[mediator] TCP 打洞成功");
            }
            Err(e) => {
                log::warn!("[mediator] TCP 打洞失败: {}，回落到中继", e);
                self.create_relay_connection(
                    ph.socket_addr.to_vec(),
                    relay_server,
                    uuid,
                    true,
                    true,
                )
                .await?;
            }
        }
        Ok(())
    }

    /// 发起 TCP 打洞
    ///
    /// 原理：
    /// 1. 先连接 rendezvous server（复用本地端口）
    /// 2. 向本地端口发一个 SYN（打开 NAT 映射）
    /// 3. 向 hbbs 发送 PunchHoleSent，告知对端我的地址
    /// 4. 对端 SYN 到达后建立真正的 TCP 连接
    async fn punch_tcp_hole(
        &self,
        peer_addr: SocketAddr,
        socket_addr_bytes: Vec<u8>,
        relay_server: String,
        uuid: String,
    ) -> ResultType<()> {
        // 通过服务器建一条新 TCP 连接，获取本机出口地址
        let mut socket = connect_tcp(self.host.clone(), CONNECT_TIMEOUT).await?;
        let local_addr = socket.local_addr();
        log::debug!("[mediator] TCP 打洞本机地址: {}", local_addr);

        // 尝试向对端发 SYN（忽略失败，目的是打开 NAT 映射）
        let la = local_addr;
        allow_err!(socket_client::connect_tcp_local(peer_addr, Some(la), 30).await);

        // 通过服务器连接通知对端
        let mut msg_out = RendezvousMessage::new();
        msg_out.set_punch_hole_sent(PunchHoleSent {
            socket_addr: socket_addr_bytes.into(),
            id: ClientConfig::get_id(),
            relay_server,
            version: env!("CARGO_PKG_VERSION").to_owned(),
            ..Default::default()
        });
        let out_bytes = if let Some(b) = rendezvous_codec::serialize(&msg_out, self.wire) {
            b.to_vec()
        } else {
            msg_out.write_to_bytes()?
        };
        socket.send_raw(out_bytes).await?;

        // 注册到 PortForwardManager，等待对端连入
        PortForwardManager::register_inbound(local_addr, peer_addr, uuid).await;
        Ok(())
    }

    // ── 中继处理 ─────────────────────────────────────────────────────────────

    /// 处理 RequestRelay：服务器要求通过中继建连
    async fn handle_request_relay(&self, rr: RequestRelay) -> ResultType<()> {
        let peer_addr = AddrMangle::decode(&rr.socket_addr);
        log::info!(
            "[mediator] RequestRelay peer={:?} uuid={} relay={}",
            peer_addr,
            rr.uuid,
            rr.relay_server
        );
        let relay_server = self.get_relay_server(rr.relay_server.clone());
        self.create_relay_connection(
            rr.socket_addr.to_vec(),
            relay_server,
            rr.uuid,
            rr.secure,
            false,
        )
        .await
    }

    /// 连接中继服务器（hbbr）并建立双向数据隧道
    ///
    /// 流程：
    /// 1. 连接 hbbr
    /// 2. 发送 RelayResponse（携带 uuid 和本机 ID）
    /// 3. 将中继连接注册到 PortForwardManager
    async fn create_relay_connection(
        &self,
        socket_addr: Vec<u8>,
        relay_server: String,
        uuid: String,
        secure: bool,
        initiate: bool,
    ) -> ResultType<()> {
        let peer_addr = AddrMangle::decode(&socket_addr);
        log::info!(
            "[mediator] 连接中继服务器 {} uuid={} peer={:?}",
            relay_server,
            uuid,
            peer_addr
        );

        // 连接到 hbbs，通知其我们准备好了（同 RustDesk 协议）
        let mut hbbs_conn = connect_tcp(self.host.clone(), CONNECT_TIMEOUT).await?;
        let mut rr = RelayResponse {
            socket_addr: socket_addr.into(),
            version: env!("CARGO_PKG_VERSION").to_owned(),
            ..Default::default()
        };
        if initiate {
            rr.uuid = uuid.clone();
            rr.relay_server = relay_server.clone();
            rr.set_id(ClientConfig::get_id());
        }
        let mut msg_out = RendezvousMessage::new();
        msg_out.set_relay_response(rr);
        send_rendezvous(&mut hbbs_conn, &msg_out, self.wire).await?;

        // 连接到 hbbr
        let relay_addr = socket_client::check_port(&relay_server, RENDEZVOUS_PORT + 1);
        log::info!("[mediator] 连接 hbbr: {}", relay_addr);
        let relay_conn = connect_tcp(relay_addr, CONNECT_TIMEOUT).await?;

        // 注册到 PortForwardManager
        PortForwardManager::register_relay(uuid, peer_addr, relay_conn, secure).await;
        Ok(())
    }

    // ── 局域网直连处理 ────────────────────────────────────────────────────────

    /// 处理 FetchLocalAddr：对端与本机在同一 NAT 后，服务器来取本机局域网地址
    async fn handle_fetch_local_addr(&self, fla: FetchLocalAddr) -> ResultType<()> {
        let peer_addr = AddrMangle::decode(&fla.socket_addr);
        log::info!("[mediator] FetchLocalAddr peer={:?}", peer_addr);

        // 新建一条 TCP 连接以获取正确的本机地址
        let mut socket = connect_tcp(self.host.clone(), CONNECT_TIMEOUT).await?;
        let local_addr = socket.local_addr();

        let relay_server = self.get_relay_server(fla.relay_server.clone());

        let mut msg_out = RendezvousMessage::new();
        msg_out.set_local_addr(LocalAddr {
            id: ClientConfig::get_id(),
            socket_addr: AddrMangle::encode(peer_addr).into(),
            local_addr: AddrMangle::encode(local_addr).into(),
            relay_server,
            version: env!("CARGO_PKG_VERSION").to_owned(),
            ..Default::default()
        });
        let out_bytes = if let Some(b) = rendezvous_codec::serialize(&msg_out, self.wire) {
            b.to_vec()
        } else {
            msg_out.write_to_bytes()?
        };
        socket.send_raw(out_bytes).await?;

        log::info!("[mediator] LocalAddr 已发送: local={}", local_addr);
        Ok(())
    }

    // ── 工具函数 ─────────────────────────────────────────────────────────────

    fn get_host_prefix(host: &str) -> String {
        // 去掉端口
        let host_only = if let Some(idx) = host.rfind(':') {
            &host[..idx]
        } else {
            host
        };
        host_only
            .split('.')
            .next()
            .map(|x| {
                if x.parse::<i32>().is_ok() {
                    host_only.to_owned()
                } else {
                    x.to_owned()
                }
            })
            .unwrap_or_else(|| host_only.to_owned())
    }

    /// 获取中继服务器地址：优先本地配置 > 服务器提供 > host+1 端口
    fn get_relay_server(&self, provided: String) -> String {
        let local = ClientConfig::get_relay_server();
        if !local.is_empty() {
            return local;
        }
        if !provided.is_empty() {
            return provided;
        }
        socket_client::increase_port(&self.host, 1)
    }
}

// ──────────────────────────────────────────────────────────────────────────────
// 对外发起连接（本机想连对端）
// ──────────────────────────────────────────────────────────────────────────────

/// 向指定 peer_id 发起连接请求
///
/// 返回本地监听端口，调用方可向该端口建立 TCP 连接，数据将被透明转发到对端。
pub async fn connect_to_peer(peer_id: String, local_port: u16) -> ResultType<u16> {
    use core_common::rendezvous_proto::PunchHoleRequest;

    let servers = ClientConfig::get_rendezvous_servers();
    if servers.is_empty() {
        bail!("未配置 rendezvous 服务器");
    }
    let host = socket_client::check_port(&servers[0], RENDEZVOUS_PORT);

    let wire = ClientConfig::get_rendezvous_wire_protocol();

    log::info!("[mediator] 发起连接到 peer={}", peer_id);

    let mut conn = connect_tcp(host.clone(), CONNECT_TIMEOUT).await?;

    // 发送打洞请求
    let mut msg = RendezvousMessage::new();
    msg.set_punch_hole_request(PunchHoleRequest {
        id: peer_id.clone(),
        token: ClientConfig::get_id(), // 用自己的 ID 作为临时令牌
        nat_type: core_common::rendezvous_proto::NatType::UNKNOWN_NAT.into(),
        licence_key: String::new(),
        ..Default::default()
    });
    send_rendezvous(&mut conn, &msg, wire).await?;

    // 等待响应（PunchHole 或 RelayResponse）
    let bytes = tokio::time::timeout(
        std::time::Duration::from_millis(CONNECT_TIMEOUT),
        conn.next(),
    )
    .await
    .map_err(|_| anyhow::anyhow!("等待服务器响应超时"))?
    .ok_or_else(|| anyhow::anyhow!("连接被重置"))??;

    let msg = rendezvous_codec::parse(&bytes)
        .ok_or_else(|| anyhow::anyhow!("无法解析 rendezvous 响应（proto3/capnp）"))?;

    // 根据服务器响应决定连接方式
    let actual_port = match msg.union {
        Some(rendezvous_message::Union::PunchHoleResponse(phr)) => {
            // 直连
            let peer_addr = AddrMangle::decode(&phr.socket_addr);
            log::info!("[mediator] 直连模式: {:?}", peer_addr);
            PortForwardManager::create_outbound_direct(local_port, peer_addr).await?
        }
        Some(rendezvous_message::Union::RelayResponse(rr)) => {
            // 中继模式
            log::info!("[mediator] 中继模式: relay={}", rr.relay_server);
            let relay_addr = socket_client::check_port(&rr.relay_server, RENDEZVOUS_PORT + 1);
            let relay_conn = connect_tcp(relay_addr, CONNECT_TIMEOUT).await?;
            PortForwardManager::create_outbound_relay(local_port, relay_conn, rr.uuid, false)
                .await?
        }
        other => {
            bail!("收到意外的响应: {:?}", other.map(|u| format!("{:?}", u)));
        }
    };

    log::info!("[mediator] 本地监听端口: {}", actual_port);
    Ok(actual_port)
}
