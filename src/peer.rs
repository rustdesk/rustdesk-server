use crate::common::*;
use crate::database;
use hbb_common::{
    bytes::Bytes,
    log,
    rendezvous_proto::*,
    tokio::sync::{Mutex, RwLock},
    ResultType,
};
use serde_derive::{Deserialize, Serialize};
use std::{collections::HashMap, collections::HashSet, net::SocketAddr, sync::Arc, time::Instant};

type IpBlockMap = HashMap<String, ((u32, Instant), (HashSet<String>, Instant))>;
type UserStatusMap = HashMap<Vec<u8>, Arc<(Option<Vec<u8>>, bool)>>;
type IpChangesMap = HashMap<String, (Instant, HashMap<String, i32>)>;
lazy_static::lazy_static! {
    pub(crate) static ref IP_BLOCKER: Mutex<IpBlockMap> = Default::default();
    pub(crate) static ref USER_STATUS: RwLock<UserStatusMap> = Default::default();
    pub(crate) static ref IP_CHANGES: Mutex<IpChangesMap> = Default::default();
}
pub const IP_CHANGE_DUR: u64 = 180;
pub const IP_CHANGE_DUR_X2: u64 = IP_CHANGE_DUR * 2;
pub const DAY_SECONDS: u64 = 3600 * 24;
pub const IP_BLOCK_DUR: u64 = 60;
const DEFAULT_MAX_PEER_CACHE_SIZE: usize = 16_384;
const DEFAULT_MAX_PENDING_REGISTRATIONS_PER_IP: usize = 64;
const PEER_CACHE_INACTIVE_TIMEOUT_MS: i32 = 30_000;

#[derive(Debug, Default, Serialize, Deserialize, Clone)]
pub(crate) struct PeerInfo {
    #[serde(default)]
    pub(crate) ip: String,
}

pub(crate) struct Peer {
    pub(crate) socket_addr: SocketAddr,
    pub(crate) last_reg_time: Instant,
    pub(crate) guid: Vec<u8>,
    pub(crate) uuid: Bytes,
    pub(crate) pk: Bytes,
    // pub(crate) user: Option<Vec<u8>>,
    pub(crate) info: PeerInfo,
    // pub(crate) disabled: bool,
    pub(crate) reg_pk: (u32, Instant), // how often register_pk
}

impl Default for Peer {
    fn default() -> Self {
        Self {
            socket_addr: "0.0.0.0:0".parse().unwrap(),
            last_reg_time: get_expired_time(),
            guid: Vec::new(),
            uuid: Bytes::new(),
            pk: Bytes::new(),
            info: Default::default(),
            // user: None,
            // disabled: false,
            reg_pk: (0, get_expired_time()),
        }
    }
}

pub(crate) type LockPeer = Arc<RwLock<Peer>>;

#[derive(Clone)]
pub(crate) struct PeerMap {
    map: Arc<RwLock<HashMap<String, LockPeer>>>,
    pub(crate) db: database::Database,
    max_cached_peers: usize,
    max_pending_registrations_per_ip: usize,
}

struct PeerEvictionEntry {
    id: String,
    inactive: bool,
    last_reg_time: Instant,
}

fn env_usize_or(name: &str, default: usize) -> usize {
    std::env::var(name)
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(default)
}

fn pending_registration_limit_exceeded(pending_for_ip: usize, max_pending: usize) -> bool {
    max_pending > 0 && pending_for_ip >= max_pending
}

fn select_peer_ids_to_evict(
    entries: Vec<PeerEvictionEntry>,
    max_cached_peers: usize,
) -> Vec<String> {
    if max_cached_peers == 0 || entries.len() < max_cached_peers {
        return vec![];
    }
    let to_remove = entries.len() + 1 - max_cached_peers;
    let mut entries = entries;
    entries.sort_by_key(|entry| (!entry.inactive, entry.last_reg_time));
    entries
        .into_iter()
        .take(to_remove)
        .map(|entry| entry.id)
        .collect()
}

impl PeerMap {
    pub(crate) async fn new() -> ResultType<Self> {
        let db = std::env::var("DB_URL").unwrap_or({
            let mut db = "db_v2.sqlite3".to_owned();
            #[cfg(all(windows, not(debug_assertions)))]
            {
                if let Some(path) = hbb_common::config::Config::icon_path().parent() {
                    db = format!("{}\\{}", path.to_str().unwrap_or("."), db);
                }
            }
            #[cfg(not(windows))]
            {
                db = format!("./{db}");
            }
            db
        });
        log::info!("DB_URL={}", db);
        let max_cached_peers =
            env_usize_or("MAX_PEER_CACHE_SIZE", DEFAULT_MAX_PEER_CACHE_SIZE);
        let max_pending_registrations_per_ip = env_usize_or(
            "MAX_PENDING_REGISTRATIONS_PER_IP",
            DEFAULT_MAX_PENDING_REGISTRATIONS_PER_IP,
        );
        log::info!("MAX_PEER_CACHE_SIZE={}", max_cached_peers);
        log::info!(
            "MAX_PENDING_REGISTRATIONS_PER_IP={}",
            max_pending_registrations_per_ip
        );
        let pm = Self {
            map: Default::default(),
            db: database::Database::new(&db).await?,
            max_cached_peers,
            max_pending_registrations_per_ip,
        };
        Ok(pm)
    }

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
        log::info!("update_pk {} {:?} {:?} {:?}", id, addr, uuid, pk);
        let (info_str, guid) = {
            let mut w = peer.write().await;
            w.socket_addr = addr;
            w.uuid = uuid.clone();
            w.pk = pk.clone();
            w.last_reg_time = Instant::now();
            w.info.ip = ip;
            (
                serde_json::to_string(&w.info).unwrap_or_default(),
                w.guid.clone(),
            )
        };
        if guid.is_empty() {
            match self.db.insert_peer(&id, &uuid, &pk, &info_str).await {
                Err(err) => {
                    log::error!("db.insert_peer failed: {}", err);
                    return register_pk_response::Result::SERVER_ERROR;
                }
                Ok(database::InsertPeerResult::Inserted(guid)) => {
                    peer.write().await.guid = guid;
                }
                Ok(database::InsertPeerResult::PeerLimitReached) => {
                    return register_pk_response::Result::PEER_LIMIT_REACHED;
                }
            }
        } else {
            if let Err(err) = self.db.update_pk(&guid, &id, &pk, &info_str).await {
                log::error!("db.update_pk failed: {}", err);
                return register_pk_response::Result::SERVER_ERROR;
            }
            log::info!("pk updated instead of insert");
        }
        register_pk_response::Result::OK
    }

    #[inline]
    pub(crate) async fn get(&self, id: &str) -> Option<LockPeer> {
        let p = self.map.read().await.get(id).cloned();
        if p.is_some() {
            return p;
        } else if let Ok(Some(v)) = self.db.get_peer(id).await {
            self.prune_cache_for_insert().await;
            let peer = Peer {
                guid: v.guid,
                uuid: v.uuid.into(),
                pk: v.pk.into(),
                // user: v.user,
                info: serde_json::from_str::<PeerInfo>(&v.info).unwrap_or_default(),
                // disabled: v.status == Some(0),
                ..Default::default()
            };
            let peer = Arc::new(RwLock::new(peer));
            self.map.write().await.insert(id.to_owned(), peer.clone());
            return Some(peer);
        }
        None
    }

    pub(crate) async fn get_or_for_registration(&self, id: &str, ip: &str) -> Option<LockPeer> {
        if let Some(p) = self.get(id).await {
            return Some(p);
        }
        if !self.can_cache_pending_registration(ip).await {
            return None;
        }
        self.prune_cache_for_insert().await;
        let mut w = self.map.write().await;
        if let Some(p) = w.get(id) {
            return Some(p.clone());
        }
        let tmp = Arc::new(RwLock::new(Peer {
            info: PeerInfo { ip: ip.to_owned() },
            ..Default::default()
        }));
        w.insert(id.to_owned(), tmp.clone());
        Some(tmp)
    }

    #[inline]
    pub(crate) async fn get_in_memory(&self, id: &str) -> Option<LockPeer> {
        self.map.read().await.get(id).cloned()
    }

    #[inline]
    pub(crate) async fn is_in_memory(&self, id: &str) -> bool {
        self.map.read().await.contains_key(id)
    }

    async fn can_cache_pending_registration(&self, ip: &str) -> bool {
        let snapshot: Vec<LockPeer> = self.map.read().await.values().cloned().collect();
        let mut pending_for_ip = 0usize;
        for peer in snapshot {
            let peer = peer.read().await;
            if peer.info.ip == ip && peer.guid.is_empty() {
                pending_for_ip += 1;
            }
        }
        !pending_registration_limit_exceeded(
            pending_for_ip,
            self.max_pending_registrations_per_ip,
        )
    }

    async fn prune_cache_for_insert(&self) {
        let snapshot: Vec<(String, LockPeer)> = self
            .map
            .read()
            .await
            .iter()
            .map(|(id, peer)| (id.clone(), peer.clone()))
            .collect();
        let mut entries = Vec::with_capacity(snapshot.len());
        for (id, peer) in snapshot {
            let peer = peer.read().await;
            entries.push(PeerEvictionEntry {
                id,
                inactive: peer.last_reg_time.elapsed().as_millis() as i32
                    >= PEER_CACHE_INACTIVE_TIMEOUT_MS,
                last_reg_time: peer.last_reg_time,
            });
        }
        let remove_ids = select_peer_ids_to_evict(entries, self.max_cached_peers);
        if remove_ids.is_empty() {
            return;
        }
        let mut map = self.map.write().await;
        for id in remove_ids {
            map.remove(&id);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        pending_registration_limit_exceeded, select_peer_ids_to_evict, PeerEvictionEntry,
    };
    use std::time::{Duration, Instant};

    #[test]
    fn pending_registration_limit_flags_excessive_per_ip_growth() {
        assert!(!pending_registration_limit_exceeded(0, 64));
        assert!(!pending_registration_limit_exceeded(63, 64));
        assert!(pending_registration_limit_exceeded(64, 64));
    }

    #[test]
    fn peer_cache_eviction_prefers_inactive_then_oldest_entries() {
        let now = Instant::now();
        let old = now.checked_sub(Duration::from_secs(120)).unwrap_or(now);
        let newer = now.checked_sub(Duration::from_secs(30)).unwrap_or(now);
        let remove = select_peer_ids_to_evict(
            vec![
                PeerEvictionEntry {
                    id: "inactive-old".to_owned(),
                    inactive: true,
                    last_reg_time: old,
                },
                PeerEvictionEntry {
                    id: "active-old".to_owned(),
                    inactive: false,
                    last_reg_time: old,
                },
                PeerEvictionEntry {
                    id: "inactive-new".to_owned(),
                    inactive: true,
                    last_reg_time: newer,
                },
            ],
            2,
        );
        assert_eq!(remove, vec!["inactive-old".to_owned(), "inactive-new".to_owned()]);
    }
}
