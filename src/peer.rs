// 导入公共模块
use crate::common::*;
// 导入数据库模块
use crate::database;
// 导入核心公共模块
use core_common::{
    bytes::Bytes,
    log,
    rendezvous_proto::*,
    tokio::sync::{Mutex, RwLock},
    ResultType,
};
// 导入序列化/反序列化支持
use serde_derive::{Deserialize, Serialize};
// 导入标准库模块
use std::{collections::HashMap, collections::HashSet, net::SocketAddr, sync::Arc, time::Instant};

// IP阻塞映射类型：存储IP地址到阻塞信息和变化记录的映射
type IpBlockMap = HashMap<String, ((u32, Instant), (HashSet<String>, Instant))>;
// 用户状态映射类型：存储用户ID到状态信息的映射
type UserStatusMap = HashMap<Vec<u8>, Arc<(Option<Vec<u8>>, bool)>>;
// IP变化映射类型：存储IP地址到变化记录的映射
type IpChangesMap = HashMap<String, (Instant, HashMap<String, i32>)>;
// 全局静态变量定义
lazy_static::lazy_static! {
    /// IP阻塞器：存储IP地址的阻塞信息
    pub(crate) static ref IP_BLOCKER: Mutex<IpBlockMap> = Default::default();
    /// 用户状态：存储用户的状态信息
    pub(crate) static ref USER_STATUS: RwLock<UserStatusMap> = Default::default();
    /// IP变化记录：存储IP地址的变化历史
    pub(crate) static ref IP_CHANGES: Mutex<IpChangesMap> = Default::default();
}
// IP变化检测间隔（秒）
pub const IP_CHANGE_DUR: u64 = 180;
// IP变化检测间隔的2倍（秒）
pub const IP_CHANGE_DUR_X2: u64 = IP_CHANGE_DUR * 2;
// 一天的秒数
pub const DAY_SECONDS: u64 = 3600 * 24;
// IP阻塞持续时间（秒）
pub const IP_BLOCK_DUR: u64 = 60;

/// Peer信息结构体
/// 用于存储Peer的基本信息
#[derive(Debug, Default, Serialize, Deserialize, Clone)]
pub(crate) struct PeerInfo {
    /// IP地址
    #[serde(default)]
    pub(crate) ip: String,
}

/// Peer结构体
/// 存储连接的对等端信息
pub(crate) struct Peer {
    /// 套接字地址
    pub(crate) socket_addr: SocketAddr,
    /// 最后注册时间
    pub(crate) last_reg_time: Instant,
    /// 全局唯一标识符
    pub(crate) guid: Vec<u8>,
    /// UUID字节数据
    pub(crate) uuid: Bytes,
    /// 公钥字节数据
    pub(crate) pk: Bytes,
    // 用户信息（已注释）
    // pub(crate) user: Option<Vec<u8>>,
    /// Peer基本信息
    pub(crate) info: PeerInfo,
    // 是否禁用（已注释）
    // pub(crate) disabled: bool,
    /// 公钥注册频率和最后注册时间
    pub(crate) reg_pk: (u32, Instant), // how often register_pk
}

/// Peer的默认实现
impl Default for Peer {
    fn default() -> Self {
        Self {
            // 默认套接字地址
            socket_addr: "0.0.0.0:0".parse().unwrap(),
            // 默认最后注册时间为过期时间
            last_reg_time: get_expired_time(),
            // 默认空GUID
            guid: Vec::new(),
            // 默认空UUID
            uuid: Bytes::new(),
            // 默认空公钥
            pk: Bytes::new(),
            // 默认Peer信息
            info: Default::default(),
            // 默认用户信息（已注释）
            // user: None,
            // 默认不禁用（已注释）
            // disabled: false,
            // 默认公钥注册频率为0，最后注册时间为过期时间
            reg_pk: (0, get_expired_time()),
        }
    }
}

// Peer锁类型：提供线程安全的Peer访问
pub(crate) type LockPeer = Arc<RwLock<Peer>>;

/// Peer映射结构体
/// 管理所有Peer的映射和数据库访问
#[derive(Clone)]
pub(crate) struct PeerMap {
    /// 内存中的Peer映射
    map: Arc<RwLock<HashMap<String, LockPeer>>>,
    /// 数据库实例
    pub(crate) db: database::Database,
}

impl PeerMap {
    /// 创建新的Peer映射实例
    /// # 返回值
    /// 返回PeerMap实例或错误
    pub(crate) async fn new() -> ResultType<Self> {
        // 从环境变量获取数据库URL，或使用默认值
        let db = std::env::var("DB_URL").unwrap_or({
            let mut db = "db_v2.sqlite3".to_owned();
            // Windows平台配置
            #[cfg(all(windows, not(debug_assertions)))]
            {
                if let Some(path) = core_common::config::Config::icon_path().parent() {
                    db = format!("{}\\{}", path.to_str().unwrap_or("."), db);
                }
            }

            // 非Windows平台配置
            #[cfg(not(windows))]
            {
                db = format!("./{db}");
            }

            db
        });
        // 记录数据库URL
        log::info!("DB_URL={}", db);
        // 创建Peer映射实例
        let pm = Self {
            // 初始化空的Peer映射
            map: Default::default(),
            // 初始化数据库连接
            db: database::Database::new(&db).await?,
        };
        Ok(pm)
    }

    /// 更新Peer的公钥信息
    /// # 参数
    /// * `id` - Peer ID
    /// * `peer` - Peer锁实例
    /// * `addr` - 套接字地址
    /// * `uuid` - UUID字节数据
    /// * `pk` - 公钥字节数据
    /// * `ip` - IP地址字符串
    /// # 返回值
    /// 返回注册结果
    #[inline]
    pub(crate) async fn update_pk(
        &mut self,
        id: String,
        peer: LockPeer,
        addr: SocketAddr,
        uuid: Bytes,
        pk: Bytes,
        ip: String,
    ) -> register_pk_response::Result {
        // 记录更新操作
        log::info!("update_pk {} {:?} {:?} {:?}", id, addr, uuid, pk);
        // 更新Peer信息并获取序列化字符串和GUID
        let (info_str, guid) = {
            let mut w = peer.write().await;
            // 更新套接字地址
            w.socket_addr = addr;
            // 更新UUID
            w.uuid = uuid.clone();
            // 更新公钥
            w.pk = pk.clone();
            // 更新最后注册时间
            w.last_reg_time = Instant::now();
            // 更新IP地址
            w.info.ip = ip;
            // 序化Peer信息为JSON字符串
            (
                serde_json::to_string(&w.info).unwrap_or_default(),
                w.guid.clone(),
            )
        };
        // 如果GUID为空，说明是新Peer，需要插入数据库
        if guid.is_empty() {
            match self.db.insert_peer(&id, &uuid, &pk, &info_str).await {
                Err(err) => {
                    log::error!("db.insert_peer failed: {}", err);
                    return register_pk_response::Result::SERVER_ERROR;
                }
                Ok(guid) => {
                    // 更新Peer的GUID
                    peer.write().await.guid = guid;
                }
            }
        } else {
            // 如果GUID不为空，更新现有Peer的公钥
            if let Err(err) = self.db.update_pk(&guid, &id, &pk, &info_str).await {
                log::error!("db.update_pk failed: {}", err);
                return register_pk_response::Result::SERVER_ERROR;
            }
            log::info!("pk updated instead of insert");
        }
        register_pk_response::Result::OK
    }

    /// 获取Peer信息
    /// 先从内存中查找，如果没有则从数据库加载
    /// # 参数
    /// * `id` - Peer ID
    /// # 返回值
    /// 返回Peer锁实例或None
    #[inline]
    pub(crate) async fn get(&self, id: &str) -> Option<LockPeer> {
        // 先从内存映射中查找
        let p = self.map.read().await.get(id).cloned();
        if p.is_some() {
            return p;
        } else if let Ok(Some(v)) = self.db.get_peer(id).await {
            // 从数据库中加载Peer信息
            let peer = Peer {
                guid: v.guid,
                uuid: v.uuid.into(),
                pk: v.pk.into(),
                // user: v.user, // 用户信息（已注释）
                // 反序列化Peer信息
                info: serde_json::from_str::<PeerInfo>(&v.info).unwrap_or_default(),
                // disabled: v.status == Some(0), // 禁用状态（已注释）
                ..Default::default()
            };
            // 创建Peer锁实例
            let peer = Arc::new(RwLock::new(peer));
            // 将Peer添加到内存映射中
            self.map.write().await.insert(id.to_owned(), peer.clone());
            return Some(peer);
        }
        None
    }

    /// 获取Peer信息，如果不存在则创建默认Peer
    /// # 参数
    /// * `id` - Peer ID
    /// # 返回值
    /// 返回Peer锁实例
    #[inline]
    pub(crate) async fn get_or(&self, id: &str) -> LockPeer {
        // 先尝试获取现有Peer
        if let Some(p) = self.get(id).await {
            return p;
        }
        // 获取映射的写锁
        let mut w = self.map.write().await;
        // 再次检查（防止竞态条件）
        if let Some(p) = w.get(id) {
            return p.clone();
        }
        // 创建默认Peer
        let tmp = LockPeer::default();
        // 将默认Peer插入映射
        w.insert(id.to_owned(), tmp.clone());
        tmp
    }

    /// 仅从内存中获取Peer信息
    /// 不访问数据库，只检查内存映射
    /// # 参数
    /// * `id` - Peer ID
    /// # 返回值
    /// 返回内存中的Peer锁实例或None
    #[inline]
    pub(crate) async fn get_in_memory(&self, id: &str) -> Option<LockPeer> {
        self.map.read().await.get(id).cloned()
    }

    /// 检查Peer是否在内存中
    /// # 参数
    /// * `id` - Peer ID
    /// # 返回值
    /// 返回是否在内存中
    #[inline]
    pub(crate) async fn is_in_memory(&self, id: &str) -> bool {
        self.map.read().await.contains_key(id)
    }
}
