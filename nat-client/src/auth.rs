//! 用户认证与设备管理模块
//!
//! 封装对 nat-server HTTP REST API 的调用：
//! - 注册 / 登录 / 注销
//! - 设备绑定（将本机 Peer ID 注册到当前账户）
//! - Token 自动续期（登录时检测，过期前主动刷新）
//!
//! ## 集成要点
//!
//! 登录成功后，JWT 存入 `ClientConfig::auth_token`。
//! `RendezvousMediator` 注册时会读取该 token 并写入 `RegisterPk.user_token`，
//! 服务端（nat-server peer.rs）收到后完成用户-设备绑定。

use crate::config::ClientConfig;
use core_common::{log, ResultType};
use serde::{Deserialize, Serialize};
use std::time::Duration;

// ──────────────────────────────────────────────────────────────────────────────
// REST API 数据结构（与 nat-server/src/api.rs 保持一致）
// ──────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Serialize)]
struct LoginRequest<'a> {
    username: &'a str,
    password: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    device_id: Option<&'a str>,
}

#[derive(Debug, Serialize)]
struct RegisterRequest<'a> {
    username: &'a str,
    email: &'a str,
    password: &'a str,
    confirm_password: &'a str,
}

#[derive(Debug, Serialize)]
struct AddDeviceRequest<'a> {
    device_id: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    device_name: Option<&'a str>,
}

#[derive(Debug, Serialize)]
struct ChangePasswordRequest<'a> {
    old_password: &'a str,
    new_password: &'a str,
    confirm_password: &'a str,
}

// ── 服务端响应 ──────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct ApiResponse<T> {
    success: bool,
    data: Option<T>,
    message: String,
}

#[derive(Debug, Deserialize)]
struct LoginData {
    token: String,
    user: UserInfo,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct UserInfo {
    pub id: i64,
    pub username: String,
    pub email: String,
    pub is_active: bool,
    pub role: String,
    pub created_at: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct DeviceInfo {
    pub id: i64,
    pub device_id: String,
    pub device_name: Option<String>,
    pub is_active: bool,
    pub created_at: String,
}

/// JWT Claims（用于解析过期时间，不做验证）
#[derive(Debug, Deserialize)]
struct JwtClaims {
    pub sub: String,
    pub username: String,
    pub exp: i64,
    #[serde(default)]
    pub udid: Option<i64>,
    #[serde(default)]
    pub role: String,
}

// ──────────────────────────────────────────────────────────────────────────────
// HTTP 客户端工厂
// ──────────────────────────────────────────────────────────────────────────────

fn build_client() -> ResultType<reqwest::blocking::Client> {
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(15))
        .build()?;
    Ok(client)
}

/// 解析 JWT payload（不验签，仅读取 claims）
fn parse_jwt_claims(token: &str) -> Option<JwtClaims> {
    let parts: Vec<&str> = token.split('.').collect();
    if parts.len() < 2 {
        return None;
    }
    // Base64 URL decode（padding 可能缺失）
    let payload = parts[1];
    let padded = match payload.len() % 4 {
        2 => format!("{}==", payload),
        3 => format!("{}=", payload),
        _ => payload.to_owned(),
    };
    use base64::Engine as _;
    let decoded = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(payload)
        .or_else(|_| base64::engine::general_purpose::URL_SAFE.decode(&padded))
        .ok()?;
    serde_json::from_slice::<JwtClaims>(&decoded).ok()
}

// ──────────────────────────────────────────────────────────────────────────────
// 公开 API
// ──────────────────────────────────────────────────────────────────────────────

/// 注册新用户
///
/// 注册成功后自动调用 `login` 完成登录并绑定设备。
pub fn register(
    username: &str,
    email: &str,
    password: &str,
    device_name: Option<&str>,
) -> ResultType<AuthStatus> {
    let api_url = ClientConfig::get_api_url();
    if api_url.is_empty() {
        return Err(core_common::anyhow::anyhow!(
            "未配置服务器地址，请先指定 --server"
        ));
    }

    let client = build_client()?;
    let url = format!("{}/api/register", api_url);

    log::info!("[auth] 注册用户 {} → {}", username, url);

    let resp: ApiResponse<UserInfo> = client
        .post(&url)
        .json(&RegisterRequest {
            username,
            email,
            password,
            confirm_password: password,
        })
        .send()?
        .json()?;

    if !resp.success {
        return Err(core_common::anyhow::anyhow!("注册失败: {}", resp.message));
    }

    log::info!("[auth] 注册成功，自动登录...");

    // 注册成功后立即登录
    login(username, password, device_name)
}

/// 登录并完成设备绑定
///
/// 流程：
/// 1. POST /api/login（携带 device_id 以获取含 udid 的 JWT）
/// 2. 若设备不存在，先 POST /api/devices 绑定，再重新登录
/// 3. 保存 JWT 到本地配置
pub fn login(username: &str, password: &str, device_name: Option<&str>) -> ResultType<AuthStatus> {
    let api_url = ClientConfig::get_api_url();
    if api_url.is_empty() {
        return Err(core_common::anyhow::anyhow!("未配置服务器地址"));
    }

    let my_peer_id = ClientConfig::get_id();
    let client = build_client()?;

    // ── 第 1 步：先尝试携带 device_id 登录 ──────────────────────────────
    let login_url = format!("{}/api/login", api_url);
    log::info!("[auth] 登录 {} → {}", username, login_url);

    let resp1: ApiResponse<serde_json::Value> = client
        .post(&login_url)
        .json(&LoginRequest {
            username,
            password,
            device_id: Some(&my_peer_id),
        })
        .send()?
        .json()?;

    // 若登录成功（device_id 已绑定），直接解析并保存
    if resp1.success {
        if let Some(data) = &resp1.data {
            if let (Some(token), Some(user_val)) = (data["token"].as_str(), data.get("user")) {
                let user: UserInfo = serde_json::from_value(user_val.clone())?;
                let claims = parse_jwt_claims(token);
                let expires = claims.as_ref().map(|c| c.exp).unwrap_or(0);
                let udid = claims.as_ref().and_then(|c| c.udid).unwrap_or(0);

                ClientConfig::save_login(token, expires, user.id, &user.username, &user.role, udid);
                log::info!(
                    "[auth] 登录成功 user_id={} role={} udid={}",
                    user.id,
                    user.role,
                    udid
                );
                return Ok(AuthStatus::from_config());
            }
        }
    }

    // ── 第 2 步：device_id 未绑定 → 先用无 device_id 登录，取得 JWT ──────
    log::info!("[auth] 设备未绑定，先匿名登录再注册设备");
    let resp2: ApiResponse<serde_json::Value> = client
        .post(&login_url)
        .json(&LoginRequest {
            username,
            password,
            device_id: None,
        })
        .send()?
        .json()?;

    if !resp2.success {
        return Err(core_common::anyhow::anyhow!("登录失败: {}", resp2.message));
    }

    let data2 = resp2
        .data
        .ok_or_else(|| core_common::anyhow::anyhow!("登录响应缺少 data 字段"))?;
    let tmp_token = data2["token"]
        .as_str()
        .ok_or_else(|| core_common::anyhow::anyhow!("响应缺少 token"))?
        .to_owned();

    // ── 第 3 步：注册设备 ───────────────────────────────────────────────
    let device_url = format!("{}/api/devices", api_url);
    let dname = device_name.map(|s| s.to_owned()).unwrap_or_else(|| {
        format!(
            "nat-client@{}",
            whoami::fallible::hostname().unwrap_or_else(|_| "unknown".to_owned())
        )
    });

    log::info!("[auth] 绑定设备 {} ({})", my_peer_id, dname);

    let add_resp: ApiResponse<DeviceInfo> = client
        .post(&device_url)
        .bearer_auth(&tmp_token)
        .json(&AddDeviceRequest {
            device_id: &my_peer_id,
            device_name: Some(&dname),
        })
        .send()?
        .json()?;

    if !add_resp.success {
        // 设备可能已绑定（409 场景），不视为致命错误
        log::warn!("[auth] 设备注册响应: {}", add_resp.message);
    } else {
        log::info!("[auth] 设备注册成功");
    }

    // ── 第 4 步：再次携带 device_id 登录，获取含 udid 的 JWT ────────────
    let resp3: ApiResponse<serde_json::Value> = client
        .post(&login_url)
        .json(&LoginRequest {
            username,
            password,
            device_id: Some(&my_peer_id),
        })
        .send()?
        .json()?;

    if !resp3.success {
        // 退回到匿名 token（不影响连通性，只是无 udid）
        log::warn!("[auth] 携带 device_id 再次登录失败，使用匿名 token");
        let user_val = data2
            .get("user")
            .cloned()
            .ok_or_else(|| core_common::anyhow::anyhow!("响应缺少 user"))?;
        let user: UserInfo = serde_json::from_value(user_val)?;
        let claims = parse_jwt_claims(&tmp_token);
        let expires = claims.as_ref().map(|c| c.exp).unwrap_or(0);
        ClientConfig::save_login(&tmp_token, expires, user.id, &user.username, &user.role, 0);
        return Ok(AuthStatus::from_config());
    }

    let data3 = resp3.data.unwrap_or_default();
    let final_token = data3["token"].as_str().unwrap_or(&tmp_token).to_owned();
    let user: UserInfo =
        serde_json::from_value(data3.get("user").cloned().unwrap_or(data2["user"].clone()))?;
    let claims = parse_jwt_claims(&final_token);
    let expires = claims.as_ref().map(|c| c.exp).unwrap_or(0);
    let udid = claims.as_ref().and_then(|c| c.udid).unwrap_or(0);

    ClientConfig::save_login(
        &final_token,
        expires,
        user.id,
        &user.username,
        &user.role,
        udid,
    );
    log::info!("[auth] 完整登录成功 user_id={} udid={}", user.id, udid);
    Ok(AuthStatus::from_config())
}

/// 注销当前登录（仅清除本地 token，服务端 JWT 无状态不需服务端操作）
pub fn logout() {
    ClientConfig::clear_login();
    log::info!("[auth] 已注销");
}

/// 修改密码
pub fn change_password(old_password: &str, new_password: &str) -> ResultType<()> {
    let token = ClientConfig::get_auth_token()
        .ok_or_else(|| core_common::anyhow::anyhow!("未登录，请先执行 nat-client login"))?;
    let api_url = ClientConfig::get_api_url();

    let client = build_client()?;
    let url = format!("{}/api/change-password", api_url);

    let resp: ApiResponse<serde_json::Value> = client
        .post(&url)
        .bearer_auth(&token)
        .json(&ChangePasswordRequest {
            old_password,
            new_password,
            confirm_password: new_password,
        })
        .send()?
        .json()?;

    if resp.success {
        log::info!("[auth] 密码修改成功，请重新登录");
        ClientConfig::clear_login();
        Ok(())
    } else {
        Err(core_common::anyhow::anyhow!(
            "修改密码失败: {}",
            resp.message
        ))
    }
}

/// 获取当前用户的设备列表
pub fn list_devices() -> ResultType<Vec<DeviceInfo>> {
    let token =
        ClientConfig::get_auth_token().ok_or_else(|| core_common::anyhow::anyhow!("未登录"))?;
    let api_url = ClientConfig::get_api_url();
    let user_id = ClientConfig::get().auth_user_id;

    let client = build_client()?;
    let url = format!("{}/api/users/{}/devices", api_url, user_id);

    let resp: ApiResponse<Vec<DeviceInfo>> = client.get(&url).bearer_auth(&token).send()?.json()?;

    if resp.success {
        Ok(resp.data.unwrap_or_default())
    } else {
        Err(core_common::anyhow::anyhow!(
            "获取设备列表失败: {}",
            resp.message
        ))
    }
}

/// 移除当前用户的某台设备
pub fn remove_device(device_id: &str) -> ResultType<()> {
    let token =
        ClientConfig::get_auth_token().ok_or_else(|| core_common::anyhow::anyhow!("未登录"))?;
    let api_url = ClientConfig::get_api_url();

    let client = build_client()?;
    let url = format!("{}/api/devices/{}", api_url, device_id);

    let resp: ApiResponse<serde_json::Value> =
        client.delete(&url).bearer_auth(&token).send()?.json()?;

    if resp.success {
        log::info!("[auth] 设备 {} 已移除", device_id);
        Ok(())
    } else {
        Err(core_common::anyhow::anyhow!(
            "移除设备失败: {}",
            resp.message
        ))
    }
}

/// 获取当前用户的信息（用于 profile 展示）
pub fn get_user_info() -> ResultType<UserInfo> {
    let token =
        ClientConfig::get_auth_token().ok_or_else(|| core_common::anyhow::anyhow!("未登录"))?;
    let api_url = ClientConfig::get_api_url();
    let user_id = ClientConfig::get().auth_user_id;

    let client = build_client()?;
    let url = format!("{}/api/users/{}", api_url, user_id);

    let resp: ApiResponse<UserInfo> = client.get(&url).bearer_auth(&token).send()?.json()?;

    resp.data
        .ok_or_else(|| core_common::anyhow::anyhow!("获取用户信息失败: {}", resp.message))
}

// ──────────────────────────────────────────────────────────────────────────────
// 认证状态（IPC 与 CLI 响应用）
// ──────────────────────────────────────────────────────────────────────────────

/// 当前认证状态快照
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthStatus {
    /// 是否已登录（token 有效）
    pub logged_in: bool,
    /// 用户 ID（0 = 未登录）
    pub user_id: i64,
    /// 用户名
    pub username: String,
    /// 角色（"user" / "admin" / ""）
    pub role: String,
    /// 设备行 ID（0 = 未绑定）
    pub device_row_id: i64,
    /// token 过期时间（Unix 秒，0 = 未登录）
    pub token_expires: i64,
    /// token 剩余有效秒数（负数 = 已过期）
    pub token_remaining_secs: i64,
}

impl AuthStatus {
    pub fn from_config() -> Self {
        let cfg = ClientConfig::get();
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64;
        let logged_in = ClientConfig::is_logged_in();
        let remaining = if cfg.auth_token_expires > 0 {
            cfg.auth_token_expires - now
        } else {
            0
        };
        AuthStatus {
            logged_in,
            user_id: cfg.auth_user_id,
            username: cfg.auth_username,
            role: cfg.auth_role,
            device_row_id: cfg.auth_device_row_id,
            token_expires: cfg.auth_token_expires,
            token_remaining_secs: remaining,
        }
    }

    pub fn not_logged_in() -> Self {
        AuthStatus {
            logged_in: false,
            user_id: 0,
            username: String::new(),
            role: String::new(),
            device_row_id: 0,
            token_expires: 0,
            token_remaining_secs: 0,
        }
    }
}

// ──────────────────────────────────────────────────────────────────────────────
// Token 自动续期任务（后台）
// ──────────────────────────────────────────────────────────────────────────────

/// 启动 token 续期检查（每 5 分钟检查一次；若剩余时效 < 1 小时则尝试刷新）
///
/// 目前服务端无 refresh_token 接口，所以仅打印警告提示用户重新登录。
pub async fn start_token_refresh_watcher() {
    loop {
        tokio::time::sleep(std::time::Duration::from_secs(300)).await;

        let cfg = ClientConfig::get();
        if cfg.auth_token.is_empty() {
            continue;
        }

        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64;

        let remaining = cfg.auth_token_expires - now;
        if remaining <= 0 {
            log::warn!(
                "[auth] JWT 已过期（用户 {}），rendezvous 注册将切换为匿名模式。\
                请执行 `nat-client login` 重新登录。",
                cfg.auth_username
            );
            // 清除过期 token，避免继续携带无效 token 注册
            ClientConfig::clear_login();
        } else if remaining < 3600 {
            log::info!(
                "[auth] JWT 将在 {} 秒后过期，请及时执行 `nat-client login` 重新登录",
                remaining
            );
        }
    }
}
