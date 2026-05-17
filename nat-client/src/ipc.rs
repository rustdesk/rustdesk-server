//! IPC 服务器模块（参照 RustDesk ipc.rs）
//!
//! 提供本地 TCP 控制接口（127.0.0.1:21114），使用换行符分隔的 JSON 协议。
//!
//! ## 支持的命令
//!
//! | 命令 JSON | 响应 JSON | 说明 |
//! |-----------|-----------|------|
//! | `{"cmd":"get_id"}` | `{"id":"..."}` | 获取本机 Peer ID |
//! | `{"cmd":"get_status"}` | `{"online":true,"nat_type":3}` | 获取在线状态 |
//! | `{"cmd":"get_peers"}` | `{"peers":[...]}` | 获取局域网发现的节点 |
//! | `{"cmd":"discover"}` | `{"peers":[...]}` | 立即触发局域网扫描 |
//! | `{"cmd":"get_connections"}` | `{"connections":[...]}` | 获取活跃连接 |
//! | `{"cmd":"restart_mediator"}` | `{"ok":true}` | 重启渲染同端中介 |
//! | `{"cmd":"connect","peer_id":"...","local_port":0}` | `{"local_port":NNNN}` | 发起连接 |
//! | `{"cmd":"close_conn","uuid":"..."}` | `{"ok":true}` | 关闭指定连接 |
//! | `{"cmd":"ping"}` | `{"pong":true}` | 健康检查 |

use crate::auth::{self, AuthStatus};
use crate::config::{ClientConfig, ForwardRule};
use crate::lan;
use crate::port_forward::{get_active_connections, scan_local_services};
use crate::rendezvous_mediator::{self, ONLINE};
use core_common::{log, ResultType};
use serde_derive::{Deserialize, Serialize};
use std::{net::SocketAddr, sync::atomic::Ordering};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    net::{TcpListener, TcpStream},
};

// ──────────────────────────────────────────────────────────────────────────────
// 协议数据类型
// ──────────────────────────────────────────────────────────────────────────────

/// IPC 请求
#[derive(Debug, Deserialize)]
pub struct IpcRequest {
    /// 命令名称
    pub cmd: String,
    /// 目标 peer_id（用于 connect 命令）
    #[serde(default)]
    pub peer_id: String,
    /// 本地监听端口（用于 connect 命令，0 = 自动分配）
    #[serde(default)]
    pub local_port: u16,
    /// 连接 UUID（用于 close_conn 命令）
    #[serde(default)]
    pub uuid: String,
    // ── 端口转发规则参数 ────────────────────────────────────────────────
    /// 规则名称（用于 add_rule 命令）
    #[serde(default)]
    pub rule_name: String,
    /// 规则 ID（用于 remove_rule 命令）
    #[serde(default)]
    pub rule_id: String,
    /// 转发目标主机（默认 127.0.0.1）
    #[serde(default)]
    pub target_host: String,
    /// 转发目标端口
    #[serde(default)]
    pub target_port: u16,
    /// 仅允许该 peer_id 触发规则（空 = 任意对端）
    #[serde(default)]
    pub peer_id_filter: String,
    // ── 认证相关参数 ───────────────────────────────────────────────────
    #[serde(default)]
    pub username: String,
    #[serde(default)]
    pub password: String,
    #[serde(default)]
    pub email: String,
    #[serde(default)]
    pub device_name: String,
    #[serde(default)]
    pub old_password: String,
    #[serde(default)]
    pub new_password: String,
    #[serde(default)]
    pub device_id: String,
}

/// IPC 响应（通用）
#[derive(Debug, Serialize)]
struct IpcResponse {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ok: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub online: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub nat_type: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub peers: Option<Vec<serde_json::Value>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub connections: Option<Vec<serde_json::Value>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub local_port: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pong: Option<bool>,
    // ── 认证相关字段 ───────────────────────────────────────────────────
    #[serde(skip_serializing_if = "Option::is_none")]
    pub auth: Option<AuthStatus>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub devices: Option<Vec<serde_json::Value>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user: Option<serde_json::Value>,
    // ── 转发规则 / 服务扫描 ────────────────────────────────────────────
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rules: Option<Vec<serde_json::Value>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub services: Option<Vec<serde_json::Value>>,
}

impl Default for IpcResponse {
    fn default() -> Self {
        Self {
            ok: None,
            error: None,
            id: None,
            online: None,
            nat_type: None,
            peers: None,
            connections: None,
            local_port: None,
            pong: None,
            auth: None,
            devices: None,
            user: None,
            rules: None,
            services: None,
        }
    }
}

// ──────────────────────────────────────────────────────────────────────────────
// IPC 服务器
// ──────────────────────────────────────────────────────────────────────────────

/// 启动 IPC 服务器（永不返回）
pub async fn start_ipc_server(port: u16) -> ResultType<()> {
    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    let listener = TcpListener::bind(addr).await?;
    log::info!("[ipc] 控制接口监听在 127.0.0.1:{}", port);

    loop {
        match listener.accept().await {
            Ok((stream, from)) => {
                log::debug!("[ipc] 新连接来自 {}", from);
                tokio::spawn(async move {
                    if let Err(e) = handle_client(stream).await {
                        log::debug!("[ipc] 客户端断开: {}", e);
                    }
                });
            }
            Err(e) => {
                log::error!("[ipc] accept 错误: {}", e);
            }
        }
    }
}

/// 处理单个 IPC 客户端连接（逐行读取 JSON 请求，逐行写回 JSON 响应）
async fn handle_client(stream: TcpStream) -> ResultType<()> {
    let (rx, mut tx) = stream.into_split();
    let mut reader = BufReader::new(rx);

    loop {
        let mut line = String::new();
        let n = reader.read_line(&mut line).await?;
        if n == 0 {
            break; // 连接关闭
        }

        let line = line.trim();
        if line.is_empty() {
            continue;
        }

        let resp = match serde_json::from_str::<IpcRequest>(line) {
            Ok(req) => dispatch(req).await,
            Err(e) => {
                log::warn!("[ipc] 解析请求失败: {} (raw={})", e, line);
                IpcResponse {
                    error: Some(format!("invalid json: {}", e)),
                    ..Default::default()
                }
            }
        };

        let mut resp_str =
            serde_json::to_string(&resp).unwrap_or_else(|_| r#"{"error":"序列化失败"}"#.to_owned());
        resp_str.push('\n');
        tx.write_all(resp_str.as_bytes()).await?;
    }

    Ok(())
}

// ──────────────────────────────────────────────────────────────────────────────
// 命令分发
// ──────────────────────────────────────────────────────────────────────────────

async fn dispatch(req: IpcRequest) -> IpcResponse {
    match req.cmd.as_str() {
        // ── 健康检查 ─────────────────────────────────────────────────────────
        "ping" => IpcResponse {
            pong: Some(true),
            ..Default::default()
        },

        // ── 获取本机 ID ──────────────────────────────────────────────────────
        "get_id" => IpcResponse {
            id: Some(ClientConfig::get_id()),
            ..Default::default()
        },

        // ── 获取在线状态和 NAT 类型 ───────────────────────────────────────────
        "get_status" => IpcResponse {
            online: Some(ONLINE.load(Ordering::SeqCst)),
            nat_type: Some(ClientConfig::get_nat_type()),
            ..Default::default()
        },

        // ── 获取已发现的局域网节点 ────────────────────────────────────────────
        "get_peers" => {
            let peers = lan::get_peers();
            let json_peers = peers
                .iter()
                .map(|p| serde_json::to_value(p).unwrap_or_default())
                .collect();
            IpcResponse {
                peers: Some(json_peers),
                ..Default::default()
            }
        }

        // ── 立即触发局域网扫描 ────────────────────────────────────────────────
        "discover" => {
            // 在后台线程执行（阻塞 3 秒）
            let result = tokio::task::spawn_blocking(|| lan::discover())
                .await
                .unwrap_or_else(|e| Err(core_common::anyhow::anyhow!("{}", e)));

            match result {
                Ok(peers) => {
                    let json_peers = peers
                        .iter()
                        .map(|p| serde_json::to_value(p).unwrap_or_default())
                        .collect();
                    IpcResponse {
                        peers: Some(json_peers),
                        ..Default::default()
                    }
                }
                Err(e) => IpcResponse {
                    error: Some(format!("发现失败: {}", e)),
                    ..Default::default()
                },
            }
        }

        // ── 获取活跃连接列表 ─────────────────────────────────────────────────
        "get_connections" => {
            let conns = get_active_connections();
            let json_conns = conns
                .iter()
                .map(|c| {
                    // 排除不可序列化的 close_tx 字段（已标 #[serde(skip)]）
                    serde_json::to_value(c).unwrap_or_default()
                })
                .collect();
            IpcResponse {
                connections: Some(json_conns),
                ..Default::default()
            }
        }

        // ── 重启渲染同端中介 ─────────────────────────────────────────────────
        "restart_mediator" => {
            rendezvous_mediator::RendezvousMediator::restart();
            IpcResponse {
                ok: Some(true),
                ..Default::default()
            }
        }

        // ── 发起连接到对端 ────────────────────────────────────────────────────
        // 请求：{"cmd":"connect","peer_id":"123456789","local_port":0}
        // 响应：{"local_port":54321}  ← 连接 127.0.0.1:local_port 即可访问对端
        "connect" => {
            if req.peer_id.is_empty() {
                return IpcResponse {
                    error: Some("peer_id 不能为空".to_owned()),
                    ..Default::default()
                };
            }
            match rendezvous_mediator::connect_to_peer(req.peer_id, req.local_port).await {
                Ok(port) => IpcResponse {
                    local_port: Some(port),
                    ..Default::default()
                },
                Err(e) => IpcResponse {
                    error: Some(format!("连接失败: {}", e)),
                    ..Default::default()
                },
            }
        }

        // ── 关闭指定连接 ─────────────────────────────────────────────────────
        "close_conn" => {
            if req.uuid.is_empty() {
                return IpcResponse {
                    error: Some("uuid 不能为空".to_owned()),
                    ..Default::default()
                };
            }
            let conns = get_active_connections();
            if let Some(conn) = conns.iter().find(|c| c.uuid == req.uuid) {
                conn.close();
                IpcResponse {
                    ok: Some(true),
                    ..Default::default()
                }
            } else {
                IpcResponse {
                    error: Some(format!("未找到连接 uuid={}", req.uuid)),
                    ..Default::default()
                }
            }
        }

        // ── 获取配置 ─────────────────────────────────────────────────────────
        "get_config" => {
            let cfg = ClientConfig::get();
            IpcResponse {
                ok: Some(true),
                id: Some(cfg.id),
                online: Some(ONLINE.load(Ordering::SeqCst)),
                nat_type: Some(cfg.nat_type),
                ..Default::default()
            }
        }

        // ── 认证状态查询 ────────────────────────────────────────────────────────────
        // 请求: {"cmd":"auth_status"}
        // 响应: {"auth":{"logged_in":true,"username":"alice",...}}
        "auth_status" => IpcResponse {
            auth: Some(AuthStatus::from_config()),
            ..Default::default()
        },

        // ── 登录 ──────────────────────────────────────────────────────────────────
        // 请求: {"cmd":"auth_login","username":"alice","password":"pass","device_name":"My PC"}
        // 响应: {"auth":{"logged_in":true,...}} | {"error":"..."}
        "auth_login" => {
            if req.username.is_empty() || req.password.is_empty() {
                return IpcResponse {
                    error: Some("用户名和密码不能为空".to_owned()),
                    ..Default::default()
                };
            }
            let dname: Option<String> = if req.device_name.is_empty() {
                None
            } else {
                Some(req.device_name.clone())
            };
            // 登录是阀塞操作，在 spawn_blocking 中执行
            let username = req.username.clone();
            let password = req.password.clone();
            let result = tokio::task::spawn_blocking(move || {
                auth::login(&username, &password, dname.as_deref())
            })
            .await
            .unwrap_or_else(|e| Err(core_common::anyhow::anyhow!("{}", e)));

            match result {
                Ok(status) => {
                    // 登录成功后触发中介重连，让新 token 生效
                    rendezvous_mediator::RendezvousMediator::restart();
                    IpcResponse {
                        auth: Some(status),
                        ..Default::default()
                    }
                }
                Err(e) => IpcResponse {
                    error: Some(format!("登录失败: {}", e)),
                    ..Default::default()
                },
            }
        }

        // ── 注却 ──────────────────────────────────────────────────────────────────
        "auth_logout" => {
            auth::logout();
            // 注销后重连，让 RegisterPk 不再携带 token
            rendezvous_mediator::RendezvousMediator::restart();
            IpcResponse {
                ok: Some(true),
                ..Default::default()
            }
        }

        // ── 注册新用户 ────────────────────────────────────────────────────────────
        // 请求: {"cmd":"auth_register","username":"bob","email":"b@x.com","password":"pass"}
        "auth_register" => {
            if req.username.is_empty() || req.email.is_empty() || req.password.is_empty() {
                return IpcResponse {
                    error: Some("用户名、邮箱、密码不能为空".to_owned()),
                    ..Default::default()
                };
            }
            let username = req.username.clone();
            let email = req.email.clone();
            let password = req.password.clone();
            let dname: Option<String> = if req.device_name.is_empty() {
                None
            } else {
                Some(req.device_name.clone())
            };
            let result = tokio::task::spawn_blocking(move || {
                auth::register(&username, &email, &password, dname.as_deref())
            })
            .await
            .unwrap_or_else(|e| Err(core_common::anyhow::anyhow!("{}", e)));

            match result {
                Ok(status) => {
                    rendezvous_mediator::RendezvousMediator::restart();
                    IpcResponse {
                        auth: Some(status),
                        ..Default::default()
                    }
                }
                Err(e) => IpcResponse {
                    error: Some(format!("注册失败: {}", e)),
                    ..Default::default()
                },
            }
        }

        // ── 修改密码 ────────────────────────────────────────────────────────────
        // 请求: {"cmd":"auth_change_password","old_password":"old","new_password":"new"}
        "auth_change_password" => {
            if req.old_password.is_empty() || req.new_password.is_empty() {
                return IpcResponse {
                    error: Some("旧密码和新密码不能为空".to_owned()),
                    ..Default::default()
                };
            }
            let op = req.old_password.clone();
            let np = req.new_password.clone();
            let result = tokio::task::spawn_blocking(move || auth::change_password(&op, &np))
                .await
                .unwrap_or_else(|e| Err(core_common::anyhow::anyhow!("{}", e)));

            match result {
                Ok(_) => IpcResponse {
                    ok: Some(true),
                    ..Default::default()
                },
                Err(e) => IpcResponse {
                    error: Some(format!("{}", e)),
                    ..Default::default()
                },
            }
        }

        // ── 查看当前用户的设备列表 ─────────────────────────────────────────
        // 请求: {"cmd":"auth_list_devices"}
        "auth_list_devices" => {
            let result = tokio::task::spawn_blocking(auth::list_devices)
                .await
                .unwrap_or_else(|e| Err(core_common::anyhow::anyhow!("{}", e)));

            match result {
                Ok(devs) => {
                    let json_devs: Vec<serde_json::Value> = devs
                        .iter()
                        .map(|d| serde_json::to_value(d).unwrap_or_default())
                        .collect();
                    IpcResponse {
                        devices: Some(json_devs),
                        ..Default::default()
                    }
                }
                Err(e) => IpcResponse {
                    error: Some(format!("{}", e)),
                    ..Default::default()
                },
            }
        }

        // ── 移除设备 ────────────────────────────────────────────────────────────
        // 请求: {"cmd":"auth_remove_device","device_id":"386742019"}
        "auth_remove_device" => {
            if req.device_id.is_empty() {
                return IpcResponse {
                    error: Some("device_id 不能为空".to_owned()),
                    ..Default::default()
                };
            }
            let did = req.device_id.clone();
            let result = tokio::task::spawn_blocking(move || auth::remove_device(&did))
                .await
                .unwrap_or_else(|e| Err(core_common::anyhow::anyhow!("{}", e)));

            match result {
                Ok(_) => IpcResponse {
                    ok: Some(true),
                    ..Default::default()
                },
                Err(e) => IpcResponse {
                    error: Some(format!("{}", e)),
                    ..Default::default()
                },
            }
        }

        // ── 获取用户信息 ────────────────────────────────────────────────────────
        // 请求: {"cmd":"auth_profile"}
        "auth_profile" => {
            let result = tokio::task::spawn_blocking(auth::get_user_info)
                .await
                .unwrap_or_else(|e| Err(core_common::anyhow::anyhow!("{}", e)));

            match result {
                Ok(info) => IpcResponse {
                    user: Some(serde_json::to_value(info).unwrap_or_default()),
                    ..Default::default()
                },
                Err(e) => IpcResponse {
                    error: Some(format!("{}", e)),
                    ..Default::default()
                },
            }
        }

        // ── 列出所有转发规则 ──────────────────────────────────────────────────
        // 请求: {"cmd":"list_rules"}
        // 响应: {"rules":[{"id":"...","name":"SSH","target_host":"127.0.0.1","target_port":22,...}]}
        "list_rules" => {
            let rules = ClientConfig::get_rules();
            let json_rules = rules
                .iter()
                .map(|r| serde_json::to_value(r).unwrap_or_default())
                .collect();
            IpcResponse {
                rules: Some(json_rules),
                ..Default::default()
            }
        }

        // ── 添加转发规则 ──────────────────────────────────────────────────────
        // 请求: {"cmd":"add_rule","rule_name":"SSH","target_port":22,"target_host":"127.0.0.1","peer_id_filter":""}
        // 响应: {"ok":true,"rules":[...]}
        "add_rule" => {
            if req.rule_name.is_empty() || req.target_port == 0 {
                return IpcResponse {
                    error: Some("rule_name 和 target_port 不能为空".to_owned()),
                    ..Default::default()
                };
            }
            let mut rule = ForwardRule::new(&req.rule_name, req.target_port);
            if !req.target_host.is_empty() {
                rule.target_host = req.target_host.clone();
            }
            if !req.peer_id_filter.is_empty() {
                rule.peer_id = req.peer_id_filter.clone();
            }
            ClientConfig::add_rule(rule);
            let rules = ClientConfig::get_rules()
                .iter()
                .map(|r| serde_json::to_value(r).unwrap_or_default())
                .collect();
            IpcResponse {
                ok: Some(true),
                rules: Some(rules),
                ..Default::default()
            }
        }

        // ── 删除转发规则 ──────────────────────────────────────────────────────
        // 请求: {"cmd":"remove_rule","rule_id":"uuid-..."}
        // 响应: {"ok":true}
        "remove_rule" => {
            if req.rule_id.is_empty() {
                return IpcResponse {
                    error: Some("rule_id 不能为空".to_owned()),
                    ..Default::default()
                };
            }
            ClientConfig::remove_rule(&req.rule_id);
            IpcResponse {
                ok: Some(true),
                ..Default::default()
            }
        }

        // ── 扫描本机正在监听的服务 ────────────────────────────────────────────
        // 请求: {"cmd":"scan_services"}
        // 响应: {"services":[{"port":22,"name":"SSH","target":"127.0.0.1:22"},...]}
        "scan_services" => {
            let svcs = scan_local_services().await;
            let json_svcs = svcs
                .iter()
                .map(|s| serde_json::to_value(s).unwrap_or_default())
                .collect();
            IpcResponse {
                services: Some(json_svcs),
                ..Default::default()
            }
        }

        // ── 未知命令 ────────────────────────────────────────────────────────────────
        other => IpcResponse {
            error: Some(format!("未知命令: {}", other)),
            ..Default::default()
        },
    }
}

// ──────────────────────────────────────────────────────────────────────────────
// IPC 客户端工具函数（供命令行子命令使用）
// ──────────────────────────────────────────────────────────────────────────────

/// 向 IPC 服务器发送单条命令并返回响应字符串
pub async fn send_command(port: u16, cmd_json: &str) -> ResultType<String> {
    let addr = format!("127.0.0.1:{}", port);
    let mut stream = TcpStream::connect(&addr).await?;

    let mut msg = cmd_json.to_owned();
    msg.push('\n');
    stream.write_all(msg.as_bytes()).await?;

    // 读取一行响应
    let mut reader = BufReader::new(stream);
    let mut resp = String::new();
    reader.read_line(&mut resp).await?;
    Ok(resp.trim().to_owned())
}

/// 格式化打印 IPC 响应（调试用）
pub fn pretty_print(json_str: &str) {
    match serde_json::from_str::<serde_json::Value>(json_str) {
        Ok(v) => {
            println!(
                "{}",
                serde_json::to_string_pretty(&v).unwrap_or_else(|_| json_str.to_owned())
            );
        }
        Err(_) => {
            println!("{}", json_str);
        }
    }
}
