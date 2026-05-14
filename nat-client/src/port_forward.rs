//! TCP 端口转发管理器
//!
//! 负责在 NAT 打洞或中继建连后，将本地 TCP 端口的流量透明转发到对端。
//!
//! 三种场景：
//! 1. **Inbound 直连**：对端打洞后直接连本机某端口，本模块接收并转发到本地服务
//! 2. **Outbound 直连**：本机发起连接，建立到对端的 TCP 隧道，监听本地端口
//! 3. **中继连接**：经由 hbbr 中继，双向代理 TCP 数据

use core_common::{log, ResultType};
use once_cell::sync::Lazy;
use serde_derive::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    net::SocketAddr,
    sync::{Arc, Mutex},
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    sync::oneshot,
};

// ──────────────────────────────────────────────────────────────────────────────
// 活跃连接信息
// ──────────────────────────────────────────────────────────────────────────────

/// 连接类型
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub enum ConnType {
    /// 直连（TCP 打洞）
    Direct,
    /// 中继（经由 hbbr）
    Relay,
}

/// 一条活跃的隧道连接记录
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ActiveConn {
    /// 连接 UUID
    pub uuid: String,
    /// 连接类型
    pub conn_type: ConnType,
    /// 对端地址
    pub peer_addr: String,
    /// 本地监听端口（出站连接时有值）
    pub local_port: Option<u16>,
    /// 已传输字节数（发送）
    pub bytes_sent: u64,
    /// 已传输字节数（接收）
    pub bytes_recv: u64,
    /// 建立时间（秒级 Unix 时间戳）
    pub created_at: u64,
    #[serde(skip)]
    /// 关闭信号发送端（调用 close() 时触发）
    pub close_tx: Option<Arc<Mutex<Option<oneshot::Sender<()>>>>>,
}

impl ActiveConn {
    pub fn close(&self) {
        if let Some(tx_arc) = &self.close_tx {
            if let Ok(mut guard) = tx_arc.lock() {
                if let Some(tx) = guard.take() {
                    let _ = tx.send(());
                }
            }
        }
    }
}

// ──────────────────────────────────────────────────────────────────────────────
// 全局连接表
// ──────────────────────────────────────────────────────────────────────────────

static ACTIVE_CONNS: Lazy<Arc<Mutex<HashMap<String, ActiveConn>>>> =
    Lazy::new(|| Arc::new(Mutex::new(HashMap::new())));

fn register_conn(conn: ActiveConn) {
    if let Ok(mut map) = ACTIVE_CONNS.lock() {
        map.insert(conn.uuid.clone(), conn);
    }
}

fn remove_conn(uuid: &str) {
    if let Ok(mut map) = ACTIVE_CONNS.lock() {
        map.remove(uuid);
    }
}

/// 返回所有活跃连接的快照
pub fn get_active_connections() -> Vec<ActiveConn> {
    ACTIVE_CONNS.lock().unwrap().values().cloned().collect()
}

fn unix_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

// ──────────────────────────────────────────────────────────────────────────────
// PortForwardManager
// ──────────────────────────────────────────────────────────────────────────────

pub struct PortForwardManager;

impl PortForwardManager {
    // ── Inbound 直连：等待对端主动连入 ───────────────────────────────────────

    /// 注册一个入站直连等待：在 `local_addr` 上监听，等待 `peer_addr` 连入后建立转发隧道
    ///
    /// 这是 PunchHole 流程中本机被动侧的处理：
    /// 服务器已通知对端连接本机的 local_addr，此处等待对端 TCP SYN 到达。
    pub async fn register_inbound(local_addr: SocketAddr, peer_addr: SocketAddr, uuid: String) {
        let uuid_clone = uuid.clone();
        tokio::spawn(async move {
            match Self::accept_inbound(local_addr, peer_addr, uuid_clone).await {
                Ok(_) => log::info!("[pf] 入站连接处理完毕"),
                Err(e) => log::error!("[pf] 入站连接错误: {}", e),
            }
        });
    }

    async fn accept_inbound(
        local_addr: SocketAddr,
        peer_addr: SocketAddr,
        uuid: String,
    ) -> ResultType<()> {
        // 监听对端打洞连入
        let listener = TcpListener::bind(local_addr).await?;
        log::info!(
            "[pf] 等待对端 {} 直连（本机 {}，uuid={}）",
            peer_addr,
            local_addr,
            uuid
        );

        // 等待 30 秒
        let accept =
            tokio::time::timeout(std::time::Duration::from_secs(30), listener.accept()).await;

        let (stream, from) = accept
            .map_err(|_| core_common::anyhow::anyhow!("等待对端连入超时"))?
            .map_err(|e| core_common::anyhow::anyhow!("accept 错误: {}", e))?;

        log::info!("[pf] 对端 {} 已连入（uuid={}）", from, uuid);

        let (close_tx, close_rx) = oneshot::channel::<()>();
        let close_arc = Arc::new(Mutex::new(Some(close_tx)));

        let conn = ActiveConn {
            uuid: uuid.clone(),
            conn_type: ConnType::Direct,
            peer_addr: peer_addr.to_string(),
            local_port: Some(local_addr.port()),
            bytes_sent: 0,
            bytes_recv: 0,
            created_at: unix_now(),
            close_tx: Some(Arc::clone(&close_arc)),
        };
        register_conn(conn);

        // 此处演示：直接将对端连接回环到本机同端口的服务（如本机 22/SSH）
        // 实际使用时应由调用方指定目标端口
        let target_port = local_addr.port();
        let target = format!("127.0.0.1:{}", target_port);

        match TcpStream::connect(&target).await {
            Ok(local_svc) => {
                log::info!("[pf] 连接本地服务 {}", target);
                Self::proxy_with_close(stream, local_svc, uuid.clone(), close_rx).await;
            }
            Err(e) => {
                log::warn!("[pf] 无法连接本地服务 {}: {}（连接仍保持）", target, e);
                // 保持对端连接，但不做转发
                tokio::time::sleep(std::time::Duration::from_secs(300)).await;
            }
        }

        remove_conn(&uuid);
        Ok(())
    }

    // ── Outbound 直连：本机发起连接到对端 ──────────────────────────────────

    /// 创建一个出站直连隧道
    ///
    /// 在 `local_port`（0 = 自动分配）上监听，本地应用连入后通过 TCP 直连到 `peer_addr`
    pub async fn create_outbound_direct(local_port: u16, peer_addr: SocketAddr) -> ResultType<u16> {
        let bind_addr = format!("127.0.0.1:{}", local_port);
        let listener = TcpListener::bind(&bind_addr).await?;
        let actual_port = listener.local_addr()?.port();
        log::info!(
            "[pf] 直连隧道监听 127.0.0.1:{} → {}",
            actual_port,
            peer_addr
        );

        tokio::spawn(async move {
            loop {
                match listener.accept().await {
                    Ok((local_stream, _from)) => {
                        let uuid = uuid::Uuid::new_v4().to_string();
                        log::info!("[pf] 本地应用连入，直连 {} uuid={}", peer_addr, uuid);
                        match TcpStream::connect(peer_addr).await {
                            Ok(remote_stream) => {
                                let (tx, rx) = oneshot::channel::<()>();
                                let close_arc = Arc::new(Mutex::new(Some(tx)));
                                let conn = ActiveConn {
                                    uuid: uuid.clone(),
                                    conn_type: ConnType::Direct,
                                    peer_addr: peer_addr.to_string(),
                                    local_port: Some(actual_port),
                                    bytes_sent: 0,
                                    bytes_recv: 0,
                                    created_at: unix_now(),
                                    close_tx: Some(Arc::clone(&close_arc)),
                                };
                                register_conn(conn);
                                let u2 = uuid.clone();
                                tokio::spawn(async move {
                                    Self::proxy_with_close(
                                        local_stream,
                                        remote_stream,
                                        u2.clone(),
                                        rx,
                                    )
                                    .await;
                                    remove_conn(&u2);
                                });
                            }
                            Err(e) => {
                                log::error!("[pf] 直连 {} 失败: {}", peer_addr, e);
                            }
                        }
                    }
                    Err(e) => {
                        log::error!("[pf] 直连隧道 accept 错误: {}", e);
                        break;
                    }
                }
            }
        });

        Ok(actual_port)
    }

    // ── 中继连接（入站被动侧）──────────────────────────────────────────────

    /// 注册一个中继连接（被动侧：服务器通知我们通过中继接受对端连接）
    pub async fn register_relay(
        uuid: String,
        peer_addr: SocketAddr,
        mut relay_conn: core_common::Stream,
        _secure: bool,
    ) {
        tokio::spawn(async move {
            log::info!("[pf] 中继连接已建立 uuid={} peer={}", uuid, peer_addr);

            // 向中继服务器发送握手（携带 uuid，让中继匹配双方）
            // 协议：先发 uuid 字符串长度 + uuid 字节
            let uuid_bytes = uuid.as_bytes();
            let mut handshake = Vec::new();
            handshake.push(uuid_bytes.len() as u8);
            handshake.extend_from_slice(uuid_bytes);
            if let Err(e) = relay_conn.send_raw(handshake).await {
                log::error!("[pf] 中继握手失败: {}", e);
                return;
            }

            // 转为原始 TCP 流进行双向代理
            // 由于 core_common::Stream 不直接暴露底层 TcpStream，
            // 此处通过 send_raw / next 模拟双向代理
            let (close_tx, close_rx) = oneshot::channel::<()>();
            let close_arc = Arc::new(Mutex::new(Some(close_tx)));

            let conn = ActiveConn {
                uuid: uuid.clone(),
                conn_type: ConnType::Relay,
                peer_addr: peer_addr.to_string(),
                local_port: None,
                bytes_sent: 0,
                bytes_recv: 0,
                created_at: unix_now(),
                close_tx: Some(Arc::clone(&close_arc)),
            };
            register_conn(conn);

            // 简化：保持连接存活直到对端关闭
            loop {
                match relay_conn.next().await {
                    Some(Ok(data)) => {
                        log::trace!("[pf] 中继收到 {} 字节", data.len());
                        // TODO: 将 data 转发到本地服务
                    }
                    _ => {
                        log::info!("[pf] 中继连接关闭 uuid={}", uuid);
                        break;
                    }
                }
            }
            remove_conn(&uuid);
        });
    }

    // ── Outbound 中继：本机发起中继连接 ────────────────────────────────────

    /// 创建一个出站中继隧道
    ///
    /// 在 `local_port` 上监听，本地应用连入后通过 hbbr 中继到对端
    pub async fn create_outbound_relay(
        local_port: u16,
        mut relay_conn: core_common::Stream,
        uuid: String,
        _secure: bool,
    ) -> ResultType<u16> {
        // 向中继发送握手
        let uuid_bytes = uuid.as_bytes();
        let mut handshake = Vec::new();
        handshake.push(uuid_bytes.len() as u8);
        handshake.extend_from_slice(uuid_bytes);
        relay_conn.send_raw(handshake).await?;
        log::info!("[pf] 中继握手完成 uuid={}", uuid);

        let bind_addr = format!("127.0.0.1:{}", local_port);
        let listener = TcpListener::bind(&bind_addr).await?;
        let actual_port = listener.local_addr()?.port();
        log::info!("[pf] 中继隧道监听 127.0.0.1:{}", actual_port);

        let relay_conn = Arc::new(tokio::sync::Mutex::new(relay_conn));

        tokio::spawn(async move {
            // 等待本地应用连入
            match listener.accept().await {
                Ok((local_stream, _from)) => {
                    let (close_tx, close_rx) = oneshot::channel::<()>();
                    let close_arc = Arc::new(Mutex::new(Some(close_tx)));
                    let conn = ActiveConn {
                        uuid: uuid.clone(),
                        conn_type: ConnType::Relay,
                        peer_addr: "relay".to_owned(),
                        local_port: Some(actual_port),
                        bytes_sent: 0,
                        bytes_recv: 0,
                        created_at: unix_now(),
                        close_tx: Some(Arc::clone(&close_arc)),
                    };
                    register_conn(conn);

                    // 拆分本地流
                    let (mut lr, mut lw) = local_stream.into_split();
                    let relay = Arc::clone(&relay_conn);

                    // 本地 → 中继
                    let u2 = uuid.clone();
                    let relay_send = Arc::clone(&relay);
                    let send_task = tokio::spawn(async move {
                        let mut buf = vec![0u8; 32 * 1024];
                        loop {
                            let n = match lr.read(&mut buf).await {
                                Ok(0) | Err(_) => break,
                                Ok(n) => n,
                            };
                            let mut r = relay_send.lock().await;
                            if r.send_raw(buf[..n].to_vec()).await.is_err() {
                                break;
                            }
                        }
                        log::debug!("[pf] 本地→中继通道关闭 uuid={}", u2);
                    });

                    // 中继 → 本地
                    let u3 = uuid.clone();
                    let recv_task = tokio::spawn(async move {
                        loop {
                            let data = {
                                let mut r = relay.lock().await;
                                r.next().await
                            };
                            match data {
                                Some(Ok(b)) => {
                                    if lw.write_all(&b).await.is_err() {
                                        break;
                                    }
                                }
                                _ => break,
                            }
                        }
                        log::debug!("[pf] 中继→本地通道关闭 uuid={}", u3);
                    });

                    // 等待任一方向关闭
                    tokio::select! {
                        _ = send_task => {}
                        _ = recv_task => {}
                        _ = close_rx => { log::info!("[pf] 中继连接被手动关闭 uuid={}", uuid); }
                    }
                    remove_conn(&uuid);
                }
                Err(e) => {
                    log::error!("[pf] 中继隧道 accept 错误: {}", e);
                }
            }
        });

        Ok(actual_port)
    }

    // ── 通用双向代理 ─────────────────────────────────────────────────────────

    /// 在两个 TCP 流之间做双向代理，直到任一端关闭或收到 close 信号
    async fn proxy_with_close(
        stream_a: TcpStream,
        stream_b: TcpStream,
        uuid: String,
        close_rx: oneshot::Receiver<()>,
    ) {
        let (mut a_rx, mut a_tx) = stream_a.into_split();
        let (mut b_rx, mut b_tx) = stream_b.into_split();

        let u1 = uuid.clone();
        let a_to_b = tokio::spawn(async move {
            let mut buf = vec![0u8; 32 * 1024];
            loop {
                let n = match a_rx.read(&mut buf).await {
                    Ok(0) | Err(_) => break,
                    Ok(n) => n,
                };
                if b_tx.write_all(&buf[..n]).await.is_err() {
                    break;
                }
            }
            log::debug!("[pf] A→B 关闭 uuid={}", u1);
        });

        let u2 = uuid.clone();
        let b_to_a = tokio::spawn(async move {
            let mut buf = vec![0u8; 32 * 1024];
            loop {
                let n = match b_rx.read(&mut buf).await {
                    Ok(0) | Err(_) => break,
                    Ok(n) => n,
                };
                if a_tx.write_all(&buf[..n]).await.is_err() {
                    break;
                }
            }
            log::debug!("[pf] B→A 关闭 uuid={}", u2);
        });

        tokio::select! {
            _ = a_to_b => {}
            _ = b_to_a => {}
            _ = close_rx => {
                log::info!("[pf] 双向代理被手动关闭 uuid={}", uuid);
            }
        }
    }
}
