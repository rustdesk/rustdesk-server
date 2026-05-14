//! GUI 应用桥接层
//!
//! 负责：
//! 1. 将 Slint 窗口的所有 `callback` 连接到 IPC 命令
//! 2. 启动定时器，轮询 IPC 获取最新状态并推送到 UI
//! 3. 驱动托盘事件循环
//!
//! 架构：
//! ```text
//!   Slint 主线程
//!   ├─ UI 渲染（Slint event loop）
//!   ├─ Slint Timer (200ms) ──► poll_status() ──► IPC ──► set_xxx()
//!   ├─ Slint Timer (50ms)  ──► TrayManager::poll() ──► 窗口 show/hide
//!   └─ 所有 callback         ──► IPC JSON 命令
//!
//!   tokio Runtime（后台线程池）
//!   ├─ RendezvousMediator::start_all()
//!   ├─ lan::start_listening()
//!   ├─ ipc::start_ipc_server()
//!   └─ auth::start_token_refresh_watcher()
//! ```

use crate::config::ClientConfig;
use crate::ui::tray::{TrayAction, TrayManager};
use core_common::log;
use slint::{ComponentHandle, ModelRc, VecModel};
use std::{
    sync::{Arc, Mutex},
    time::Duration,
};

slint::include_modules!();

// ──────────────────────────────────────────────────────────────────────────────
// 入口函数
// ──────────────────────────────────────────────────────────────────────────────

/// 运行 GUI（在主线程阻塞，直到窗口关闭）
pub fn run_gui(ipc_port: u16) -> Result<(), slint::PlatformError> {
    // 创建 Slint 主窗口
    let window = AppWindow::new()?;

    // 初始化配置显示值
    init_config_fields(&window);

    // ── 系统托盘 ─────────────────────────────────────────────────────────────
    let tray = match TrayManager::new() {
        Ok(t) => {
            log::info!("[gui] 系统托盘已创建");
            Some(Arc::new(Mutex::new(t)))
        }
        Err(e) => {
            log::warn!("[gui] 系统托盘创建失败（将以无托盘模式运行）: {}", e);
            None
        }
    };

    // ── 绑定所有 Slint 回调 ──────────────────────────────────────────────────
    bind_callbacks(&window, ipc_port);

    // ── 状态轮询 Timer（每 500ms）────────────────────────────────────────────
    {
        let win = window.as_weak();
        let tray_ref = tray.clone();
        let timer = slint::Timer::default();
        timer.start(
            slint::TimerMode::Repeated,
            Duration::from_millis(500),
            move || {
                let Some(w) = win.upgrade() else { return };
                poll_and_update(&w, ipc_port, tray_ref.clone());
            },
        );
        // 让 timer 随窗口生命周期存在（泄漏给全局，简单有效）
        std::mem::forget(timer);
    }

    // ── 托盘事件 Timer（每 50ms）────────────────────────────────────────────
    if let Some(tray_ref) = tray {
        let win = window.as_weak();
        let timer = slint::Timer::default();
        timer.start(
            slint::TimerMode::Repeated,
            Duration::from_millis(50),
            move || {
                let guard = match tray_ref.lock() {
                    Ok(g) => g,
                    Err(_) => return,
                };
                if let Some(action) = guard.poll() {
                    drop(guard); // 释放锁再操作窗口
                    let Some(w) = win.upgrade() else { return };
                    handle_tray_action(&w, action);
                }
            },
        );
        std::mem::forget(timer);
    }

    // ── 立即拉取一次状态（延迟 300ms，等待守护进程 IPC 就绪）────────────────
    {
        let win = window.as_weak(); // Weak<T> 实现了 Send
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(400));
            let _ = slint::invoke_from_event_loop(move || {
                if let Some(w) = win.upgrade() {
                    poll_and_update(&w, ipc_port, None);
                }
            });
        });
    }

    // 启动 Slint 事件循环（阻塞直到窗口关闭）
    window.run()
}

// ──────────────────────────────────────────────────────────────────────────────
// 初始化：从配置文件读取默认值填入设置页
// ──────────────────────────────────────────────────────────────────────────────

fn init_config_fields(w: &AppWindow) {
    let cfg = ClientConfig::get();
    w.set_cfg_server(cfg.rendezvous_servers.as_str().into());
    w.set_cfg_relay(cfg.relay_server.as_str().into());
    w.set_cfg_api_url(cfg.api_url.as_str().into());
    w.set_cfg_ipc_port(cfg.ipc_port.to_string().as_str().into());
    w.set_peer_id(ClientConfig::get_id().as_str().into());
}

// ──────────────────────────────────────────────────────────────────────────────
// 状态轮询：通过 IPC 获取最新数据并更新 Slint 属性
// ──────────────────────────────────────────────────────────────────────────────

fn poll_and_update(w: &AppWindow, ipc_port: u16, tray: Option<Arc<Mutex<TrayManager>>>) {
    // 通过阻塞方式调用 IPC（IPC 连接非常快，< 5ms）
    let status = blocking_ipc(ipc_port, r#"{"cmd":"get_status"}"#);
    let config = blocking_ipc(ipc_port, r#"{"cmd":"get_config"}"#);
    let conns = blocking_ipc(ipc_port, r#"{"cmd":"get_connections"}"#);
    let peers = blocking_ipc(ipc_port, r#"{"cmd":"get_peers"}"#);
    let auth = blocking_ipc(ipc_port, r#"{"cmd":"auth_status"}"#);

    // ── 在线状态 ─────────────────────────────────────────────────────────────
    if let Ok(v) = serde_json::from_str::<serde_json::Value>(&status) {
        let online = v["online"].as_bool().unwrap_or(false);
        w.set_online(online);
        let nat_raw = v["nat_type"].as_i64().unwrap_or(0);
        w.set_nat_type(nat_type_name(nat_raw).into());

        // 更新托盘图标
        if let Some(t) = &tray {
            if let Ok(mut g) = t.lock() {
                g.set_online(online);
            }
        }
    }

    // ── 服务器地址 ────────────────────────────────────────────────────────────
    if let Ok(v) = serde_json::from_str::<serde_json::Value>(&config) {
        if let Some(id) = v["id"].as_str() {
            w.set_peer_id(id.into());
        }
        // 从配置文件拿服务器地址
        let cfg = ClientConfig::get();
        w.set_server_addr(cfg.rendezvous_servers.as_str().into());
    }

    // ── 活跃连接 ──────────────────────────────────────────────────────────────
    if let Ok(v) = serde_json::from_str::<serde_json::Value>(&conns) {
        if let Some(arr) = v["connections"].as_array() {
            let items: Vec<ActiveConn> = arr
                .iter()
                .map(|c| ActiveConn {
                    uuid: c["uuid"].as_str().unwrap_or("").into(),
                    conn_type: c["conn_type"].as_str().unwrap_or("").into(),
                    peer_addr: c["peer_addr"].as_str().unwrap_or("").into(),
                    local_port: c["local_port"].as_i64().unwrap_or(0) as i32,
                    bytes_sent: format_bytes(c["bytes_sent"].as_u64().unwrap_or(0)).into(),
                    bytes_recv: format_bytes(c["bytes_recv"].as_u64().unwrap_or(0)).into(),
                    created_at: format_ts(c["created_at"].as_u64().unwrap_or(0)).into(),
                })
                .collect();
            w.set_connections(ModelRc::new(VecModel::from(items)));
        }
    }

    // ── LAN 节点 ──────────────────────────────────────────────────────────────
    if let Ok(v) = serde_json::from_str::<serde_json::Value>(&peers) {
        if let Some(arr) = v["peers"].as_array() {
            let items: Vec<LanPeer> = arr
                .iter()
                .map(|p| LanPeer {
                    id: p["id"].as_str().unwrap_or("").into(),
                    ip: p["ip"].as_str().unwrap_or("").into(),
                    hostname: p["hostname"].as_str().unwrap_or("").into(),
                    username: p["username"].as_str().unwrap_or("").into(),
                    platform: p["platform"].as_str().unwrap_or("").into(),
                    online: p["online"].as_bool().unwrap_or(false),
                })
                .collect();
            w.set_lan_peers(ModelRc::new(VecModel::from(items)));
        }
    }

    // ── 认证状态 ──────────────────────────────────────────────────────────────
    if let Ok(v) = serde_json::from_str::<serde_json::Value>(&auth) {
        if let Some(a) = v.get("auth") {
            let logged_in = a["logged_in"].as_bool().unwrap_or(false);
            w.set_logged_in(logged_in);
            w.set_username(a["username"].as_str().unwrap_or("").into());
            w.set_role(a["role"].as_str().unwrap_or("").into());

            let remaining = a["token_remaining_secs"].as_i64().unwrap_or(0);
            w.set_token_remaining(format_duration(remaining).into());
        }
    }
}

// ──────────────────────────────────────────────────────────────────────────────
// 回调绑定：将 Slint callbacks 连接到 IPC 命令
// ──────────────────────────────────────────────────────────────────────────────

fn bind_callbacks(w: &AppWindow, ipc_port: u16) {
    // ── 刷新 ─────────────────────────────────────────────────────────────────
    {
        let win = w.as_weak();
        w.on_do_refresh(move || {
            let Some(w) = win.upgrade() else { return };
            poll_and_update(&w, ipc_port, None);
        });
    }

    // ── 连接对端 ─────────────────────────────────────────────────────────────
    {
        let win = w.as_weak();
        w.on_do_connect(move |peer_id, local_port| {
            let Some(w) = win.upgrade() else { return };
            w.set_connecting(true);
            w.set_connect_result("".into());

            let peer_id = peer_id.to_string();
            let port_str = local_port.to_string();
            let cmd = format!(
                r#"{{"cmd":"connect","peer_id":"{}","local_port":{}}}"#,
                peer_id,
                port_str.parse::<u16>().unwrap_or(0)
            );

            let win2 = win.upgrade().unwrap();
            let resp = blocking_ipc(ipc_port, &cmd);
            win2.set_connecting(false);

            if let Ok(v) = serde_json::from_str::<serde_json::Value>(&resp) {
                if let Some(port) = v["local_port"].as_u64() {
                    win2.set_connect_result(
                        format!("✅ 隧道已建立！连接 127.0.0.1:{} 访问对端", port).into(),
                    );
                } else {
                    let err = v["error"].as_str().unwrap_or("未知错误");
                    win2.set_connect_result(format!("❌ {}", err).into());
                }
            }
        });
    }

    // ── LAN 发现 ─────────────────────────────────────────────────────────────
    {
        let win = w.as_weak();
        w.on_do_discover(move || {
            let Some(w) = win.upgrade() else { return };
            w.set_discovering(true);

            // 发现命令耗时 ~3s，放后台线程
            let win2 = w.as_weak();
            std::thread::spawn(move || {
                let resp = blocking_ipc(ipc_port, r#"{"cmd":"discover"}"#);
                let _ = slint::invoke_from_event_loop(move || {
                    let Some(w) = win2.upgrade() else { return };
                    w.set_discovering(false);

                    if let Ok(v) = serde_json::from_str::<serde_json::Value>(&resp) {
                        if let Some(arr) = v["peers"].as_array() {
                            let items: Vec<LanPeer> = arr
                                .iter()
                                .map(|p| LanPeer {
                                    id: p["id"].as_str().unwrap_or("").into(),
                                    ip: p["ip"].as_str().unwrap_or("").into(),
                                    hostname: p["hostname"].as_str().unwrap_or("").into(),
                                    username: p["username"].as_str().unwrap_or("").into(),
                                    platform: p["platform"].as_str().unwrap_or("").into(),
                                    online: p["online"].as_bool().unwrap_or(false),
                                })
                                .collect();
                            w.set_lan_peers(ModelRc::new(VecModel::from(items)));
                        }
                    }
                });
            });
        });
    }

    // ── 断开连接 ─────────────────────────────────────────────────────────────
    {
        let win = w.as_weak();
        w.on_close_conn(move |uuid| {
            let cmd = format!(r#"{{"cmd":"close_conn","uuid":"{}"}}"#, uuid);
            blocking_ipc(ipc_port, &cmd);
            // 刷新列表
            if let Some(w) = win.upgrade() {
                poll_and_update(&w, ipc_port, None);
            }
        });
    }

    // ── 连接 LAN 节点 ────────────────────────────────────────────────────────
    {
        let win = w.as_weak();
        w.on_connect_peer(move |peer_id| {
            let Some(w) = win.upgrade() else { return };
            let cmd = format!(
                r#"{{"cmd":"connect","peer_id":"{}","local_port":0}}"#,
                peer_id
            );
            w.set_connecting(true);
            w.set_page(0); // 跳到首页显示结果

            let resp = blocking_ipc(ipc_port, &cmd);
            let win2 = win.upgrade().unwrap();
            win2.set_connecting(false);

            if let Ok(v) = serde_json::from_str::<serde_json::Value>(&resp) {
                if let Some(port) = v["local_port"].as_u64() {
                    win2.set_connect_result(format!("✅ 已连接！本地端口 {}", port).into());
                } else {
                    let err = v["error"].as_str().unwrap_or("连接失败");
                    win2.set_connect_result(format!("❌ {}", err).into());
                }
            }
        });
    }

    // ── 登录 ─────────────────────────────────────────────────────────────────
    {
        let win = w.as_weak();
        w.on_do_login(move || {
            let Some(w) = win.upgrade() else { return };
            let username = w.get_login_user().to_string();
            let password = w.get_login_pass().to_string();
            if username.is_empty() || password.is_empty() {
                return;
            }

            w.set_account_busy(true);
            w.set_account_status("".into());

            let cmd = format!(
                r#"{{"cmd":"auth_login","username":"{}","password":"{}"}}"#,
                escape_json(&username),
                escape_json(&password)
            );
            let win2 = win.upgrade().unwrap();
            let resp = blocking_ipc(ipc_port, &cmd);
            win2.set_account_busy(false);

            if let Ok(v) = serde_json::from_str::<serde_json::Value>(&resp) {
                if let Some(a) = v.get("auth") {
                    win2.set_logged_in(a["logged_in"].as_bool().unwrap_or(false));
                    win2.set_username(a["username"].as_str().unwrap_or("").into());
                    win2.set_role(a["role"].as_str().unwrap_or("").into());
                    win2.set_account_status("✅ 登录成功".into());
                    win2.set_login_pass("".into());
                } else if let Some(e) = v["error"].as_str() {
                    win2.set_account_status(format!("❌ {}", e).into());
                }
            }
        });
    }

    // ── 注销 ─────────────────────────────────────────────────────────────────
    {
        let win = w.as_weak();
        w.on_do_logout(move || {
            blocking_ipc(ipc_port, r#"{"cmd":"auth_logout"}"#);
            if let Some(w) = win.upgrade() {
                w.set_logged_in(false);
                w.set_username("".into());
                w.set_role("".into());
                w.set_account_status("✅ 已退出登录".into());
            }
        });
    }

    // ── 注册 ─────────────────────────────────────────────────────────────────
    {
        let win = w.as_weak();
        w.on_do_register(move || {
            let Some(w) = win.upgrade() else { return };
            let username = w.get_login_user().to_string();
            let password = w.get_login_pass().to_string();
            let email    = w.get_reg_email().to_string();
            let dname    = w.get_reg_device_name().to_string();

            if username.is_empty() || password.is_empty() || email.is_empty() { return; }
            w.set_account_busy(true);

            let cmd = format!(
                r#"{{"cmd":"auth_register","username":"{}","email":"{}","password":"{}","device_name":"{}"}}"#,
                escape_json(&username), escape_json(&email),
                escape_json(&password), escape_json(&dname)
            );
            let win2 = win.upgrade().unwrap();
            let resp  = blocking_ipc(ipc_port, &cmd);
            win2.set_account_busy(false);

            if let Ok(v) = serde_json::from_str::<serde_json::Value>(&resp) {
                if let Some(a) = v.get("auth") {
                    win2.set_logged_in(a["logged_in"].as_bool().unwrap_or(false));
                    win2.set_username(a["username"].as_str().unwrap_or("").into());
                    win2.set_role(a["role"].as_str().unwrap_or("").into());
                    win2.set_account_status("✅ 注册并登录成功".into());
                    win2.set_show_register(false);
                } else if let Some(e) = v["error"].as_str() {
                    win2.set_account_status(format!("❌ {}", e).into());
                }
            }
        });
    }

    // ── 修改密码 ─────────────────────────────────────────────────────────────
    {
        let win = w.as_weak();
        w.on_do_change_password(move || {
            let Some(w) = win.upgrade() else { return };
            let old = w.get_old_pass().to_string();
            let new = w.get_new_pass().to_string();
            if old.is_empty() || new.is_empty() {
                return;
            }

            w.set_account_busy(true);
            let cmd = format!(
                r#"{{"cmd":"auth_change_password","old_password":"{}","new_password":"{}"}}"#,
                escape_json(&old),
                escape_json(&new)
            );
            let win2 = win.upgrade().unwrap();
            let resp = blocking_ipc(ipc_port, &cmd);
            win2.set_account_busy(false);

            if let Ok(v) = serde_json::from_str::<serde_json::Value>(&resp) {
                if v["ok"].as_bool().unwrap_or(false) {
                    win2.set_account_status("✅ 密码已修改，请重新登录".into());
                    win2.set_logged_in(false);
                    win2.set_old_pass("".into());
                    win2.set_new_pass("".into());
                } else {
                    let e = v["error"].as_str().unwrap_or("修改失败");
                    win2.set_account_status(format!("❌ {}", e).into());
                }
            }
        });
    }

    // ── 移除设备 ─────────────────────────────────────────────────────────────
    {
        let win = w.as_weak();
        w.on_do_remove_device(move |device_id| {
            let cmd = format!(
                r#"{{"cmd":"auth_remove_device","device_id":"{}"}}"#,
                device_id
            );
            blocking_ipc(ipc_port, &cmd);
            // 刷新设备列表
            if let Some(w) = win.upgrade() {
                refresh_devices(&w, ipc_port);
            }
        });
    }

    // ── 保存设置 ─────────────────────────────────────────────────────────────
    {
        let win = w.as_weak();
        w.on_do_save_settings(move || {
            let Some(w) = win.upgrade() else { return };
            let server = w.get_cfg_server().to_string();
            let relay = w.get_cfg_relay().to_string();
            let api_url = w.get_cfg_api_url().to_string();
            let ipc_port_str = w.get_cfg_ipc_port().to_string();

            ClientConfig::update(|c| {
                c.rendezvous_servers = server.clone();
                c.relay_server = relay.clone();
                c.api_url = api_url.clone();
                if let Ok(p) = ipc_port_str.parse::<u16>() {
                    c.ipc_port = p;
                }
            });

            w.set_settings_status("✅ 配置已保存".into());
            log::info!("[gui] 设置已保存，触发中介重启");

            // 通知中介重连
            blocking_ipc(ipc_port, r#"{"cmd":"restart_mediator"}"#);
        });
    }

    // ── 重启中介 ─────────────────────────────────────────────────────────────
    {
        let win = w.as_weak();
        w.on_do_restart_mediator(move || {
            blocking_ipc(ipc_port, r#"{"cmd":"restart_mediator"}"#);
            if let Some(w) = win.upgrade() {
                w.set_settings_status("✅ 中介已重启".into());
            }
        });
    }

    // ── 打开配置文件 ─────────────────────────────────────────────────────────
    w.on_do_open_config(|| {
        let path = ClientConfig::config_path();
        log::info!("[gui] 打开配置文件: {}", path.display());
        #[cfg(target_os = "windows")]
        let _ = std::process::Command::new("explorer").arg(&path).spawn();
        #[cfg(target_os = "macos")]
        let _ = std::process::Command::new("open").arg(&path).spawn();
        #[cfg(target_os = "linux")]
        let _ = std::process::Command::new("xdg-open").arg(&path).spawn();
    });

    // ── 切换注册/登录表单 ────────────────────────────────────────────────────
    {
        let win = w.as_weak();
        w.on_toggle_register(move || {
            if let Some(w) = win.upgrade() {
                let cur = w.get_show_register();
                w.set_show_register(!cur);
                w.set_account_status("".into());
            }
        });
    }
}

// ──────────────────────────────────────────────────────────────────────────────
// 托盘事件处理
// ──────────────────────────────────────────────────────────────────────────────

fn handle_tray_action(w: &AppWindow, action: TrayAction) {
    match action {
        TrayAction::ToggleWindow => {
            let visible = w.window().is_visible();
            if visible {
                w.window().hide().ok();
            } else {
                w.window().show().ok();
                w.window().request_redraw();
            }
        }
        TrayAction::GoHome => {
            w.window().show().ok();
            w.set_page(0);
        }
        TrayAction::GoConnect => {
            w.window().show().ok();
            w.set_page(0);
        }
        TrayAction::GoAccount => {
            w.window().show().ok();
            w.set_page(3);
        }
        TrayAction::Quit => slint::quit_event_loop().ok().unwrap_or(()),
    }
}

// ──────────────────────────────────────────────────────────────────────────────
// 设备列表刷新
// ──────────────────────────────────────────────────────────────────────────────

fn refresh_devices(w: &AppWindow, ipc_port: u16) {
    w.set_devices_loading(true);
    let resp = blocking_ipc(ipc_port, r#"{"cmd":"auth_list_devices"}"#);
    w.set_devices_loading(false);

    if let Ok(v) = serde_json::from_str::<serde_json::Value>(&resp) {
        if let Some(arr) = v["devices"].as_array() {
            let items: Vec<BoundDevice> = arr
                .iter()
                .map(|d| BoundDevice {
                    id: d["id"].as_i64().unwrap_or(0) as i32,
                    device_id: d["device_id"].as_str().unwrap_or("").into(),
                    device_name: d["device_name"].as_str().unwrap_or("").into(),
                    is_active: d["is_active"].as_bool().unwrap_or(false),
                    created_at: d["created_at"].as_str().unwrap_or("").into(),
                })
                .collect();
            w.set_devices(ModelRc::new(VecModel::from(items)));
        }
    }
}

// ──────────────────────────────────────────────────────────────────────────────
// IPC 同步调用（在 Slint 主线程安全使用，因为 IPC 极快）
// ──────────────────────────────────────────────────────────────────────────────

fn blocking_ipc(ipc_port: u16, cmd: &str) -> String {
    use std::io::{BufRead, BufReader, Write};
    use std::net::TcpStream;

    let addr = format!("127.0.0.1:{}", ipc_port);
    let Ok(mut stream) = TcpStream::connect_timeout(
        &addr.parse().unwrap_or("127.0.0.1:21114".parse().unwrap()),
        Duration::from_millis(800),
    ) else {
        return r#"{"error":"IPC 连接失败（守护进程未运行？）"}"#.to_owned();
    };

    let mut msg = cmd.to_owned();
    msg.push('\n');
    if stream.write_all(msg.as_bytes()).is_err() {
        return r#"{"error":"IPC 写入失败"}"#.to_owned();
    }

    let mut reader = BufReader::new(stream);
    let mut resp = String::new();
    if reader.read_line(&mut resp).is_err() {
        return r#"{"error":"IPC 读取失败"}"#.to_owned();
    }
    resp.trim().to_owned()
}

// ──────────────────────────────────────────────────────────────────────────────
// 格式化工具函数
// ──────────────────────────────────────────────────────────────────────────────

fn nat_type_name(t: i64) -> &'static str {
    match t {
        1 => "对称 NAT（仅中继）",
        2 => "对称 UDP 防火墙",
        3 => "完全锥形（最优）",
        4 => "受限锥形",
        5 => "端口受限锥形",
        _ => "未知",
    }
}

fn format_bytes(bytes: u64) -> String {
    if bytes < 1024 {
        format!("{} B", bytes)
    } else if bytes < 1024 * 1024 {
        format!("{:.1} KB", bytes as f64 / 1024.0)
    } else {
        format!("{:.2} MB", bytes as f64 / 1024.0 / 1024.0)
    }
}

fn format_ts(ts: u64) -> String {
    if ts == 0 {
        return "—".to_owned();
    }
    // 简单格式：当前时间减去 ts 的差值
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let diff = now.saturating_sub(ts);
    if diff < 60 {
        format!("{}秒前", diff)
    } else if diff < 3600 {
        format!("{}分钟前", diff / 60)
    } else {
        format!("{}小时前", diff / 3600)
    }
}

fn format_duration(secs: i64) -> String {
    if secs <= 0 {
        return "已过期".to_owned();
    }
    if secs < 60 {
        format!("{}秒", secs)
    } else if secs < 3600 {
        format!("{}分钟", secs / 60)
    } else {
        format!("{:.1}小时", secs as f64 / 3600.0)
    }
}

/// 转义 JSON 字符串中的特殊字符
fn escape_json(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
        .replace('\r', "\\r")
}
