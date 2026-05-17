//! nat-client — 完整内网穿透客户端（含 Slint 桌面 GUI + 系统托盘）
//!
//! ## 启动模式
//!
//! | 模式 | 命令 | 说明 |
//! |---|---|---|
//! | GUI 模式 | `nat-client gui --server 1.2.3.4` | 启动守护进程 + 显示桌面窗口 + 系统托盘 |
//! | 守护进程 | `nat-client daemon --server 1.2.3.4` | 纯后台模式，无 GUI |
//! | CLI 命令 | `nat-client status` | 向已运行的守护进程发送 IPC 命令 |
//!
//! ## 架构
//!
//! ```text
//! GUI 模式：
//!   主线程 → Slint 事件循环（窗口 + 托盘）
//!   tokio runtime（4线程）→ 中介 / LAN / IPC 服务器
//!
//! Daemon 模式：
//!   tokio::main → 中介 / LAN / IPC 服务器（无 GUI）
//! ```

mod auth;
mod config;
mod ipc;
mod lan;
mod port_forward;
mod rendezvous_mediator;
mod ui;

use clap::{Parser, Subcommand};
use config::{ClientConfig, RendezvousWireProtocol};
use core_common::{log, ResultType};

// ──────────────────────────────────────────────────────────────────────────────
// CLI 定义
// ──────────────────────────────────────────────────────────────────────────────

#[derive(Parser)]
#[command(
    name = "nat-client",
    version = env!("CARGO_PKG_VERSION"),
    about = "完整的内网穿透客户端，与 nat-server (hbbs/hbbr) 配合使用"
)]
struct Cli {
    #[command(subcommand)]
    command: Commands,

    /// IPC 控制端口（默认 21114）
    #[arg(long, global = true, default_value = "21114")]
    ipc_port: u16,
}

#[derive(Subcommand)]
enum Commands {
    // ── GUI 模式 ─────────────────────────────────────────────────────────────
    /// 启动桌面 GUI（含系统托盘），同时在后台运行守护进程
    Gui {
        /// rendezvous 服务器地址
        #[arg(short, long)]
        server: String,
        #[arg(short, long)]
        relay: Option<String>,
        #[arg(long)]
        id: Option<String>,
        #[arg(long, default_value = "21114")]
        ipc_port: u16,
        #[arg(long, default_value = "info")]
        log_level: String,
        /// 与 hbbs 的线路协议（默认 proto3；capnp 须与服务器/对端一致）
        #[arg(long = "rendezvous-protocol")]
        rendezvous_protocol: Option<RendezvousWireProtocol>,
    },

    // ── 守护进程模式 ──────────────────────────────────────────────────────────
    /// 启动后台守护进程（无 GUI）
    Daemon {
        #[arg(short, long)]
        server: String,
        #[arg(short, long)]
        relay: Option<String>,
        #[arg(long)]
        id: Option<String>,
        #[arg(long, default_value = "21114")]
        ipc_port: u16,
        #[arg(long, default_value = "info")]
        log_level: String,
        #[arg(long = "rendezvous-protocol")]
        rendezvous_protocol: Option<RendezvousWireProtocol>,
    },

    // ── 基础查询 ──────────────────────────────────────────────────────────────
    /// 查看本机 Peer ID
    Id,
    /// 查看在线状态
    Status,
    /// 扫描局域网节点（约 3 秒）
    Discover,
    /// 查看缓存的局域网节点
    Peers,
    /// 查看活跃连接
    Connections,

    // ── 连接 ──────────────────────────────────────────────────────────────────
    /// 连接到指定对端
    Connect {
        #[arg(short, long)]
        peer_id: String,
        /// 本地监听端口（0 = 自动分配）
        #[arg(short, long, default_value = "0")]
        local_port: u16,
    },
    /// 关闭指定连接
    Close {
        #[arg(short, long)]
        uuid: String,
    },
    /// 重启 rendezvous 中介
    Restart,

    // ── 用户管理 ──────────────────────────────────────────────────────────────
    /// 注册新账户
    Register {
        #[arg(short, long)]
        username: String,
        #[arg(short, long)]
        email: String,
        #[arg(short, long)]
        password: String,
        #[arg(long)]
        device_name: Option<String>,
    },
    /// 登录
    Login {
        #[arg(short, long)]
        username: String,
        #[arg(short, long)]
        password: String,
        #[arg(long)]
        device_name: Option<String>,
    },
    /// 注销
    Logout,
    /// 查看认证状态
    AuthStatus,
    /// 修改密码
    ChangePassword {
        #[arg(long)]
        old_password: String,
        #[arg(long)]
        new_password: String,
    },
    /// 查看绑定设备
    Devices,
    /// 移除绑定设备
    RemoveDevice {
        #[arg(short, long)]
        device_id: String,
    },
    /// 查看用户资料
    Profile,

    // ── 端口转发规则管理 ──────────────────────────────────────────────────────
    /// 列出所有已保存的转发规则
    ListRules,
    /// 添加一条端口转发规则（对端连入时将流量转发到指定本地服务）
    AddRule {
        /// 规则名称（如 SSH、HTTP、MySQL）
        #[arg(short, long)]
        name: String,
        /// 转发目标端口（本机服务端口，如 22、80、3306）
        #[arg(short, long)]
        target_port: u16,
        /// 转发目标主机（默认 127.0.0.1）
        #[arg(long, default_value = "127.0.0.1")]
        target_host: String,
        /// 仅允许该 peer_id 的对端触发此规则（留空表示任意对端）
        #[arg(long, default_value = "")]
        peer_id_filter: String,
    },
    /// 删除一条转发规则（通过 list-rules 查看规则 ID）
    RemoveRule {
        #[arg(short, long)]
        rule_id: String,
    },
    /// 扫描本机当前正在监听的常见服务（SSH、HTTP、MySQL 等）
    ScanServices,

    // ── 调试 ──────────────────────────────────────────────────────────────────
    /// 发送原始 JSON 命令（调试用）
    Send { json: String },
}

// ──────────────────────────────────────────────────────────────────────────────
// 主函数：不使用 #[tokio::main]，手动管理运行时
// ──────────────────────────────────────────────────────────────────────────────

fn main() {
    let cli = Cli::parse();
    let ipc_port = cli.ipc_port;

    match cli.command {
        // ── GUI 模式：Slint 跑主线程，tokio 跑后台 ───────────────────────────
        Commands::Gui {
            server,
            relay,
            id,
            ipc_port,
            log_level,
            rendezvous_protocol,
        } => {
            init_logger(&log_level);
            log::info!(
                "=== nat-client v{} GUI 模式启动 ===",
                env!("CARGO_PKG_VERSION")
            );

            // 初始化配置
            config::init(
                Some(server.clone()),
                relay.clone(),
                id,
                Some(ipc_port),
                rendezvous_protocol,
            )
            .expect("配置初始化失败");

            // 启动 tokio 后台运行时（多线程，专用于异步服务）
            let rt = tokio::runtime::Builder::new_multi_thread()
                .worker_threads(4)
                .enable_all()
                .build()
                .expect("tokio runtime 创建失败");

            start_background_services(&rt, ipc_port);
            log::info!("[main] 后台服务已启动，IPC 端口 {}", ipc_port);

            // 在主线程运行 Slint GUI + 系统托盘（阻塞直到窗口关闭）
            match ui::app::run_gui(ipc_port) {
                Ok(_) => log::info!("[gui] 窗口已关闭"),
                Err(e) => log::error!("[gui] 窗口运行失败: {}", e),
            }

            // GUI 退出后关闭 tokio
            rt.shutdown_timeout(std::time::Duration::from_secs(2));
            log::info!("[main] 程序已退出");
        }

        // ── 守护进程模式：纯 tokio，无 GUI ───────────────────────────────────
        Commands::Daemon {
            server,
            relay,
            id,
            ipc_port,
            log_level,
            rendezvous_protocol,
        } => {
            init_logger(&log_level);
            log::info!(
                "=== nat-client v{} 守护进程模式 ===",
                env!("CARGO_PKG_VERSION")
            );

            let rt = tokio::runtime::Builder::new_multi_thread()
                .worker_threads(4)
                .enable_all()
                .build()
                .expect("tokio runtime 创建失败");

            rt.block_on(async move {
                config::init(
                    Some(server),
                    relay,
                    id,
                    Some(ipc_port),
                    rendezvous_protocol,
                )
                .expect("配置初始化失败");

                log::info!("本机 Peer ID: {}", ClientConfig::get_id());

                // 启动所有后台服务
                start_all_async(ipc_port).await;

                // 等待信号
                wait_for_signal().await;
                log::info!("收到终止信号，退出");
            });
        }

        // ── 以下均为 CLI 命令（通过 IPC 与守护进程通信）─────────────────────
        cmd => {
            // CLI 命令只需极轻量的 tokio
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("tokio runtime 创建失败");
            rt.block_on(async move {
                run_cli_command(cmd, ipc_port).await;
            });
        }
    }
}

// ──────────────────────────────────────────────────────────────────────────────
// 后台服务启动（供 GUI 和 Daemon 模式共用）
// ──────────────────────────────────────────────────────────────────────────────

/// 在 tokio runtime 中启动所有后台异步服务
fn start_background_services(rt: &tokio::runtime::Runtime, ipc_port: u16) {
    // LAN 发现监听（阻塞 → 独立 OS 线程）
    std::thread::spawn(|| loop {
        if let Err(e) = lan::start_listening() {
            log::error!("[lan] 监听错误: {}，3秒后重启", e);
            std::thread::sleep(std::time::Duration::from_secs(3));
        }
    });

    // Rendezvous 中介
    rt.spawn(async { rendezvous_mediator::RendezvousMediator::start_all().await });

    // IPC 服务器
    rt.spawn(async move {
        if let Err(e) = ipc::start_ipc_server(ipc_port).await {
            log::error!("[ipc] 服务器错误: {}", e);
        }
    });

    // Token 过期监控
    rt.spawn(async { auth::start_token_refresh_watcher().await });

    // 当前认证状态提示
    if ClientConfig::is_logged_in() {
        let cfg = ClientConfig::get();
        log::info!(
            "已登录用户: {} ({})，device_row_id={}",
            cfg.auth_username,
            cfg.auth_role,
            cfg.auth_device_row_id
        );
    } else {
        log::info!("当前未登录，以匿名模式运行");
    }
}

/// 在当前 tokio 上下文中启动所有服务（Daemon 模式）
async fn start_all_async(ipc_port: u16) {
    // LAN 发现监听（阻塞 → 独立 OS 线程）
    std::thread::spawn(|| loop {
        if let Err(e) = lan::start_listening() {
            log::error!("[lan] 监听错误: {}，3秒后重启", e);
            std::thread::sleep(std::time::Duration::from_secs(3));
        }
    });

    tokio::spawn(async { rendezvous_mediator::RendezvousMediator::start_all().await });
    tokio::spawn(async move {
        if let Err(e) = ipc::start_ipc_server(ipc_port).await {
            log::error!("[ipc] 服务器错误: {}", e);
        }
    });
    tokio::spawn(async { auth::start_token_refresh_watcher().await });

    log::info!("守护进程运行中，按 Ctrl+C 退出...");
}

// ──────────────────────────────────────────────────────────────────────────────
// CLI 命令执行（通过 IPC）
// ──────────────────────────────────────────────────────────────────────────────

async fn run_cli_command(cmd: Commands, ipc_port: u16) {
    match cmd {
        Commands::Id => {
            let r = ipc::send_command(ipc_port, r#"{"cmd":"get_id"}"#)
                .await
                .unwrap_or_default();
            ipc::pretty_print(&r);
        }
        Commands::Status => {
            let r = ipc::send_command(ipc_port, r#"{"cmd":"get_status"}"#)
                .await
                .unwrap_or_default();
            ipc::pretty_print(&r);
        }
        Commands::Peers => {
            let r = ipc::send_command(ipc_port, r#"{"cmd":"get_peers"}"#)
                .await
                .unwrap_or_default();
            ipc::pretty_print(&r);
        }
        Commands::Discover => {
            println!("正在扫描局域网（约 3 秒）...");
            let r = ipc::send_command(ipc_port, r#"{"cmd":"discover"}"#)
                .await
                .unwrap_or_default();
            ipc::pretty_print(&r);
        }
        Commands::Connections => {
            let r = ipc::send_command(ipc_port, r#"{"cmd":"get_connections"}"#)
                .await
                .unwrap_or_default();
            ipc::pretty_print(&r);
        }
        Commands::Connect {
            peer_id,
            local_port,
        } => {
            let cmd =
                serde_json::json!({ "cmd":"connect","peer_id":peer_id,"local_port":local_port })
                    .to_string();
            println!("正在连接对端 {}...", peer_id);
            let r = ipc::send_command(ipc_port, &cmd).await.unwrap_or_default();
            ipc::pretty_print(&r);
            if let Ok(v) = serde_json::from_str::<serde_json::Value>(&r) {
                if let Some(p) = v["local_port"].as_u64() {
                    println!("\n✅ 隧道已建立！连接 127.0.0.1:{} 即可访问对端", p);
                }
            }
        }
        Commands::Close { uuid } => {
            let cmd = serde_json::json!({ "cmd":"close_conn","uuid":uuid }).to_string();
            let r = ipc::send_command(ipc_port, &cmd).await.unwrap_or_default();
            ipc::pretty_print(&r);
        }
        Commands::Restart => {
            let r = ipc::send_command(ipc_port, r#"{"cmd":"restart_mediator"}"#)
                .await
                .unwrap_or_default();
            ipc::pretty_print(&r);
        }
        Commands::Register {
            username,
            email,
            password,
            device_name,
        } => {
            let cmd = serde_json::json!({
                "cmd":"auth_register","username":username,"email":email,
                "password":password,"device_name":device_name.unwrap_or_default()
            })
            .to_string();
            println!("注册用户 {}...", username);
            let r = ipc::send_command(ipc_port, &cmd).await.unwrap_or_default();
            ipc::pretty_print(&r);
        }
        Commands::Login {
            username,
            password,
            device_name,
        } => {
            let cmd = serde_json::json!({
                "cmd":"auth_login","username":username,
                "password":password,"device_name":device_name.unwrap_or_default()
            })
            .to_string();
            println!("登录中...");
            let r = ipc::send_command(ipc_port, &cmd).await.unwrap_or_default();
            ipc::pretty_print(&r);
        }
        Commands::Logout => {
            let r = ipc::send_command(ipc_port, r#"{"cmd":"auth_logout"}"#)
                .await
                .unwrap_or_default();
            ipc::pretty_print(&r);
        }
        Commands::AuthStatus => {
            let r = ipc::send_command(ipc_port, r#"{"cmd":"auth_status"}"#)
                .await
                .unwrap_or_default();
            ipc::pretty_print(&r);
        }
        Commands::ChangePassword {
            old_password,
            new_password,
        } => {
            let cmd = serde_json::json!({
                "cmd":"auth_change_password","old_password":old_password,"new_password":new_password
            })
            .to_string();
            let r = ipc::send_command(ipc_port, &cmd).await.unwrap_or_default();
            ipc::pretty_print(&r);
        }
        Commands::Devices => {
            let r = ipc::send_command(ipc_port, r#"{"cmd":"auth_list_devices"}"#)
                .await
                .unwrap_or_default();
            ipc::pretty_print(&r);
        }
        Commands::RemoveDevice { device_id } => {
            let cmd =
                serde_json::json!({ "cmd":"auth_remove_device","device_id":device_id }).to_string();
            let r = ipc::send_command(ipc_port, &cmd).await.unwrap_or_default();
            ipc::pretty_print(&r);
        }
        Commands::Profile => {
            let r = ipc::send_command(ipc_port, r#"{"cmd":"auth_profile"}"#)
                .await
                .unwrap_or_default();
            ipc::pretty_print(&r);
        }
        Commands::ListRules => {
            let r = ipc::send_command(ipc_port, r#"{"cmd":"list_rules"}"#)
                .await
                .unwrap_or_default();
            ipc::pretty_print(&r);
        }
        Commands::AddRule {
            name,
            target_port,
            target_host,
            peer_id_filter,
        } => {
            let cmd = serde_json::json!({
                "cmd": "add_rule",
                "rule_name": name,
                "target_port": target_port,
                "target_host": target_host,
                "peer_id_filter": peer_id_filter,
            })
            .to_string();
            let r = ipc::send_command(ipc_port, &cmd).await.unwrap_or_default();
            if let Ok(v) = serde_json::from_str::<serde_json::Value>(&r) {
                if v["ok"].as_bool().unwrap_or(false) {
                    println!("✅ 规则已添加：{} → {}:{}", name, target_host, target_port);
                }
            }
            ipc::pretty_print(&r);
        }
        Commands::RemoveRule { rule_id } => {
            let cmd = serde_json::json!({ "cmd": "remove_rule", "rule_id": rule_id }).to_string();
            let r = ipc::send_command(ipc_port, &cmd).await.unwrap_or_default();
            ipc::pretty_print(&r);
        }
        Commands::ScanServices => {
            println!("正在扫描本机服务（约 200ms）...");
            let r = ipc::send_command(ipc_port, r#"{"cmd":"scan_services"}"#)
                .await
                .unwrap_or_default();
            if let Ok(v) = serde_json::from_str::<serde_json::Value>(&r) {
                if let Some(svcs) = v["services"].as_array() {
                    if svcs.is_empty() {
                        println!("未检测到常用服务正在监听");
                    } else {
                        println!("\n检测到以下服务：");
                        println!("{:<8} {:<16} {}", "端口", "服务名", "可添加规则命令");
                        println!("{}", "-".repeat(60));
                        for s in svcs {
                            let port = s["port"].as_u64().unwrap_or(0);
                            let name = s["name"].as_str().unwrap_or("?");
                            println!(
                                "{:<8} {:<16} nat-client add-rule -n {} -t {}",
                                port, name, name, port
                            );
                        }
                    }
                    return;
                }
            }
            ipc::pretty_print(&r);
        }
        Commands::Send { json } => {
            let r = ipc::send_command(ipc_port, &json).await.unwrap_or_default();
            ipc::pretty_print(&r);
        }
        // GUI / Daemon 已在 main() 中处理
        _ => {}
    }
}

// ──────────────────────────────────────────────────────────────────────────────
// 工具函数
// ──────────────────────────────────────────────────────────────────────────────

fn init_logger(level: &str) {
    use env_logger::Env;
    env_logger::Builder::from_env(Env::default().default_filter_or(level))
        .format_timestamp_millis()
        .filter_module("icu_provider", log::LevelFilter::Off)
        .init();
}

#[cfg(unix)]
async fn wait_for_signal() {
    use tokio::signal::unix::{signal, SignalKind};
    let mut term = signal(SignalKind::terminate()).expect("SIGTERM 注册失败");
    let mut intr = signal(SignalKind::interrupt()).expect("SIGINT 注册失败");
    tokio::select! {
        _ = term.recv() => log::info!("收到 SIGTERM"),
        _ = intr.recv() => log::info!("收到 SIGINT"),
    }
}

#[cfg(not(unix))]
async fn wait_for_signal() {
    tokio::signal::ctrl_c().await.expect("Ctrl+C 注册失败");
    log::info!("收到 Ctrl+C");
}
