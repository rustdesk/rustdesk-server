//! 客户端配置模块
//!
//! 封装 core_common::config::Config，并提供客户端专属配置读写。
//! 配置文件路径：`~/.nat-client/config.toml`（Unix）或 `%APPDATA%\nat-client\config.toml`（Windows）

use core_common::{log, ResultType};
use once_cell::sync::Lazy;
use serde_derive::{Deserialize, Serialize};
use std::{
    io::{Read, Write},
    path::PathBuf,
    sync::RwLock,
};

// ──────────────────────────────────────────────────────────────────────────────
// 配置数据结构
// ──────────────────────────────────────────────────────────────────────────────

/// 客户端专属配置（持久化到 TOML 文件）
#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct ClientConfig {
    /// 对端 ID（9位数字字符串），留空时自动生成
    #[serde(default)]
    pub id: String,

    /// 设备 UUID（用于 RegisterPk），留空时自动生成
    #[serde(default)]
    pub uuid: String,

    /// Ed25519 私钥（base64），留空时自动生成
    #[serde(default)]
    pub sk: String,

    /// Ed25519 公钥（base64），与 sk 配对
    #[serde(default)]
    pub pk: String,

    /// 是否已由服务器确认公钥
    #[serde(default)]
    pub key_confirmed: bool,

    /// 各主机的公钥确认状态：host_prefix -> bool
    #[serde(default)]
    pub host_keys_confirmed: std::collections::HashMap<String, bool>,

    /// 目标 rendezvous 服务器地址（host:port 或 host），逗号分隔
    #[serde(default)]
    pub rendezvous_servers: String,

    /// 中继服务器地址（可为空，由服务器提供）
    #[serde(default)]
    pub relay_server: String,

    /// IPC 监听端口（默认 21114）
    #[serde(default = "default_ipc_port")]
    pub ipc_port: u16,

    /// 直接访问监听端口（默认 0，0 = 禁用）
    #[serde(default)]
    pub direct_listen_port: u16,

    /// NAT 类型缓存（0=未知，1=对称，2=对称UDP防火墙，3=完全锥形，4=受限锥形，5=端口受限锥形）
    #[serde(default)]
    pub nat_type: i32,

    // ── 用户认证字段 ────────────────────────────────────────────────────────
    /// nat-server 的 HTTP API 地址（如 http://1.2.3.4:8080），留空则从 rendezvous_servers 推导
    #[serde(default)]
    pub api_url: String,

    /// 当前登录用户的 JWT token（由 /api/login 返回）
    #[serde(default)]
    pub auth_token: String,

    /// JWT 过期时间（Unix 秒级时间戳，0 = 未登录）
    #[serde(default)]
    pub auth_token_expires: i64,

    /// 当前登录的用户 ID
    #[serde(default)]
    pub auth_user_id: i64,

    /// 当前登录的用户名
    #[serde(default)]
    pub auth_username: String,

    /// 当前登录的角色（"user" 或 "admin"）
    #[serde(default)]
    pub auth_role: String,

    /// 本设备在服务器 user_devices 表中的行 ID（udid）；0 = 未绑定
    #[serde(default)]
    pub auth_device_row_id: i64,
}

fn default_ipc_port() -> u16 {
    21114
}

// ──────────────────────────────────────────────────────────────────────────────
// 全局配置单例
// ──────────────────────────────────────────────────────────────────────────────

static CONFIG: Lazy<RwLock<ClientConfig>> = Lazy::new(|| RwLock::new(ClientConfig::load()));

impl ClientConfig {
    // ── 路径 ─────────────────────────────────────────────────────────────────

    /// 返回配置文件路径
    pub fn config_path() -> PathBuf {
        let dir = if cfg!(windows) {
            std::env::var("APPDATA")
                .map(PathBuf::from)
                .unwrap_or_else(|_| PathBuf::from("."))
        } else {
            std::env::var("HOME")
                .map(|h| PathBuf::from(h).join(".config"))
                .unwrap_or_else(|_| PathBuf::from("."))
        };
        dir.join("nat-client").join("config.toml")
    }

    // ── 加载 / 保存 ──────────────────────────────────────────────────────────

    /// 从文件加载配置；文件不存在时返回默认值
    pub fn load() -> Self {
        let path = Self::config_path();
        if let Ok(mut f) = std::fs::File::open(&path) {
            let mut s = String::new();
            if f.read_to_string(&mut s).is_ok() {
                if let Ok(cfg) = toml::from_str::<Self>(&s) {
                    return cfg;
                }
            }
        }
        Self::default()
    }

    /// 将当前配置写入文件
    pub fn save(&self) {
        let path = Self::config_path();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).ok();
        }
        match toml::to_string_pretty(self) {
            Ok(s) => {
                if let Ok(mut f) = std::fs::File::create(&path) {
                    f.write_all(s.as_bytes()).ok();
                }
            }
            Err(e) => log::error!("序列化配置失败: {}", e),
        }
    }

    // ── 全局访问 ─────────────────────────────────────────────────────────────

    /// 读取全局配置
    pub fn get() -> ClientConfig {
        CONFIG.read().unwrap().clone()
    }

    /// 修改全局配置并保存
    pub fn update(f: impl FnOnce(&mut ClientConfig)) {
        let mut cfg = CONFIG.write().unwrap();
        f(&mut cfg);
        cfg.save();
    }

    // ── 快捷函数 ─────────────────────────────────────────────────────────────

    pub fn get_id() -> String {
        CONFIG.read().unwrap().id.clone()
    }

    pub fn set_id(id: &str) {
        Self::update(|c| c.id = id.to_owned());
    }

    pub fn get_uuid_bytes() -> Vec<u8> {
        let s = CONFIG.read().unwrap().uuid.clone();
        base64_decode(&s).unwrap_or_default()
    }

    pub fn get_key_pair() -> (Vec<u8>, Vec<u8>) {
        let cfg = CONFIG.read().unwrap();
        let sk = base64_decode(&cfg.sk).unwrap_or_default();
        let pk = base64_decode(&cfg.pk).unwrap_or_default();
        (sk, pk)
    }

    pub fn get_key_confirmed() -> bool {
        CONFIG.read().unwrap().key_confirmed
    }

    pub fn set_key_confirmed(v: bool) {
        Self::update(|c| c.key_confirmed = v);
    }

    pub fn get_host_key_confirmed(prefix: &str) -> bool {
        CONFIG
            .read()
            .unwrap()
            .host_keys_confirmed
            .get(prefix)
            .copied()
            .unwrap_or(false)
    }

    pub fn set_host_key_confirmed(prefix: &str, v: bool) {
        Self::update(|c| {
            c.host_keys_confirmed.insert(prefix.to_owned(), v);
        });
    }

    pub fn get_rendezvous_servers() -> Vec<String> {
        CONFIG
            .read()
            .unwrap()
            .rendezvous_servers
            .split(',')
            .filter(|s| !s.trim().is_empty())
            .map(|s| s.trim().to_owned())
            .collect()
    }

    pub fn get_relay_server() -> String {
        CONFIG.read().unwrap().relay_server.clone()
    }

    pub fn get_ipc_port() -> u16 {
        CONFIG.read().unwrap().ipc_port
    }

    pub fn get_nat_type() -> i32 {
        CONFIG.read().unwrap().nat_type
    }

    pub fn set_nat_type(t: i32) {
        Self::update(|c| c.nat_type = t);
    }

    // ── 认证快捷方法 ────────────────────────────────────────────────────────────────

    /// 返回当前 JWT token（若已过期则返回空字符串）
    pub fn get_auth_token() -> Option<String> {
        let cfg = CONFIG.read().unwrap();
        if cfg.auth_token.is_empty() {
            return None;
        }
        // 检查是否过期（留 30 秒容错量）
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64;
        if cfg.auth_token_expires > 0 && now >= cfg.auth_token_expires - 30 {
            return None; // 已过期
        }
        Some(cfg.auth_token.clone())
    }

    /// 返回是否已登录（token 存在且未过期）
    pub fn is_logged_in() -> bool {
        Self::get_auth_token().is_some()
    }

    /// 保存登录结果
    pub fn save_login(
        token: &str,
        expires: i64,
        user_id: i64,
        username: &str,
        role: &str,
        device_row_id: i64,
    ) {
        Self::update(|c| {
            c.auth_token = token.to_owned();
            c.auth_token_expires = expires;
            c.auth_user_id = user_id;
            c.auth_username = username.to_owned();
            c.auth_role = role.to_owned();
            c.auth_device_row_id = device_row_id;
        });
    }

    /// 清除登录状态
    pub fn clear_login() {
        Self::update(|c| {
            c.auth_token = String::new();
            c.auth_token_expires = 0;
            c.auth_user_id = 0;
            c.auth_username = String::new();
            c.auth_role = String::new();
            c.auth_device_row_id = 0;
        });
    }

    /// 返回服务器 HTTP API 地址（自动从 rendezvous_servers 推导）
    pub fn get_api_url() -> String {
        let url = CONFIG.read().unwrap().api_url.clone();
        if !url.is_empty() {
            return url;
        }
        // 尝试从 rendezvous_servers 推导：提取 host，加上 默认 API 端口 8080
        let servers = Self::get_rendezvous_servers();
        if let Some(first) = servers.first() {
            let host = if first.contains(':') {
                first.split(':').next().unwrap_or(first).to_owned()
            } else {
                first.clone()
            };
            return format!("http://{}:8080", host);
        }
        String::new()
    }

    pub fn set_api_url(url: &str) {
        Self::update(|c| c.api_url = url.to_owned());
    }
}

// ──────────────────────────────────────────────────────────────────────────────
// 初始化：确保 ID / UUID / 密钥对已生成
// ──────────────────────────────────────────────────────────────────────────────

/// 初始化配置：若 ID / UUID / 密钥缺失则自动生成，支持命令行覆盖。
pub fn init(
    rendezvous_servers: Option<String>,
    relay_server: Option<String>,
    id_override: Option<String>,
    ipc_port: Option<u16>,
) -> ResultType<()> {
    use sodiumoxide::crypto::sign;

    // 确保 sodiumoxide 初始化
    sodiumoxide::init().ok();

    ClientConfig::update(|cfg| {
        // 覆盖 rendezvous 服务器
        if let Some(s) = rendezvous_servers {
            cfg.rendezvous_servers = s;
        }
        if let Some(s) = relay_server {
            cfg.relay_server = s;
        }
        if let Some(p) = ipc_port {
            cfg.ipc_port = p;
        }

        // ID
        if let Some(id) = id_override {
            cfg.id = id;
        }
        if cfg.id.is_empty() {
            cfg.id = generate_id();
            log::info!("生成新 Peer ID: {}", cfg.id);
        }

        // UUID
        if cfg.uuid.is_empty() {
            cfg.uuid = base64_encode(uuid::Uuid::new_v4().as_bytes().to_vec());
            log::info!("生成新设备 UUID");
        }

        // 密钥对
        if cfg.sk.is_empty() || cfg.pk.is_empty() {
            let (pk_bytes, sk) = sign::gen_keypair();
            cfg.sk = base64_encode(sk.0.to_vec());
            cfg.pk = base64_encode(pk_bytes.0.to_vec());
            cfg.key_confirmed = false;
            log::info!("生成新 Ed25519 密钥对，公钥: {}", cfg.pk);
        }
    });

    log::info!("客户端配置初始化完成，ID={}", ClientConfig::get_id());
    Ok(())
}

// ──────────────────────────────────────────────────────────────────────────────
// 工具函数
// ──────────────────────────────────────────────────────────────────────────────

/// 生成 9 位随机数字 ID（仿 RustDesk 风格）
fn generate_id() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let t = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .subsec_nanos();
    // 用时间戳 + 随机数组合生成 9 位数字
    let r: u32 = rand::random::<u32>() ^ t;
    format!("{:09}", r % 1_000_000_000)
}

pub fn base64_encode(data: Vec<u8>) -> String {
    use base64::Engine as _;
    base64::engine::general_purpose::STANDARD.encode(data)
}

pub fn base64_decode(s: &str) -> Option<Vec<u8>> {
    use base64::Engine as _;
    base64::engine::general_purpose::STANDARD.decode(s).ok()
}
