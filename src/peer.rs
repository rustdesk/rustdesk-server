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
    pub(crate) static ref MAX_PEER_CACHE_ENTRIES: usize = cache_limit("MAX_PEER_CACHE_ENTRIES", 100_000);
    pub(crate) static ref MAX_IP_CACHE_ENTRIES: usize = cache_limit("MAX_IP_CACHE_ENTRIES", 10_000);
}

/// Reads a valid positive cache limit or returns the configured default.
fn cache_limit(name: &str, default: usize) -> usize {
    get_arg(name).parse::<usize>().ok().filter(|n| *n > 0 && *n <= 1_000_000).unwrap_or(default)
}

/// Returns the existing peer or inserts a new one only while capacity remains.
fn cache_peer(map: &mut HashMap<String, LockPeer>, id: &str, peer: LockPeer, limit: usize) -> Option<LockPeer> {
    if let Some(existing) = map.get(id) { return Some(existing.clone()); }
    if map.len() >= limit { return None; }
    map.insert(id.to_owned(), peer.clone());
    Some(peer)
}

/// Removes expired peers while retaining active or externally held entries.
fn prune_peers(map: &mut HashMap<String, LockPeer>) {
    map.retain(|_, peer| Arc::strong_count(peer) > 1 || peer.try_read().map(|peer| {
        peer.last_reg_time.elapsed().as_secs() < 600 || peer.reg_pk.1.elapsed().as_secs() < 600
    }).unwrap_or(true));
}
pub const IP_CHANGE_DUR: u64 = 180;
pub const IP_CHANGE_DUR_X2: u64 = IP_CHANGE_DUR * 2;
pub const DAY_SECONDS: u64 = 3600 * 24;
pub const IP_BLOCK_DUR: u64 = 60;
// Same bound as the Pro server: a longer id is not a normal RustDesk id or hostname,
// and every peer entry and punch-request record keeps its own copy.
pub const MAX_PEER_ID_LEN: usize = 253;

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
}

impl PeerMap {
    pub(crate) async fn new() -> ResultType<Self> {
        let db = get_arg_opt("DB_URL").unwrap_or_else(|| {
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
        let pm = Self {
            map: Default::default(),
            db: database::Database::new(&db).await?,
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
                Ok(guid) => {
                    peer.write().await.guid = guid;
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

    /// Returns a cached peer or reads its durable record without replacing an existing identity.
    #[inline]
    pub(crate) async fn get(&self, id: &str) -> Option<LockPeer> {
        self.get_with_limit(id, *MAX_PEER_CACHE_ENTRIES).await
    }

    /// Loads a durable peer even when the selected cache limit prevents insertion.
    async fn get_with_limit(&self, id: &str, limit: usize) -> Option<LockPeer> {
        let p = self.map.read().await.get(id).cloned();
        if p.is_some() {
            return p;
        } else if let Ok(Some(v)) = self.db.get_peer(id).await {
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
            let cached = cache_peer(&mut *self.map.write().await, id, peer.clone(), limit);
            // Durable lookups remain available even when no new cache slot fits.
            return Some(cached.unwrap_or(peer));
        }
        None
    }

    /// Returns or admits a peer only when it can be kept in the bounded cache.
    #[inline]
    pub(crate) async fn get_or(&self, id: &str) -> Option<LockPeer> {
        if let Some(p) = self.get(id).await {
            return cache_peer(&mut *self.map.write().await, id, p, *MAX_PEER_CACHE_ENTRIES);
        }
        let mut w = self.map.write().await;
        cache_peer(&mut w, id, LockPeer::default(), *MAX_PEER_CACHE_ENTRIES)
    }

    /// Expires inactive peers and outdated IP bookkeeping without deleting durable records.
    pub(crate) async fn prune(&self) {
        prune_peers(&mut *self.map.write().await);
        IP_BLOCKER.lock().await.retain(|_, (a, b)| {
            a.1.elapsed().as_secs() <= IP_BLOCK_DUR || b.1.elapsed().as_secs() <= DAY_SECONDS
        });
        IP_CHANGES.lock().await.retain(|_, (time, _)| time.elapsed().as_secs() < IP_CHANGE_DUR_X2);
    }

    /// Returns a peer only when it is present in the in-memory cache.
    #[inline]
    pub(crate) async fn get_in_memory(&self, id: &str) -> Option<LockPeer> {
        self.map.read().await.get(id).cloned()
    }

    #[inline]
    pub(crate) async fn is_in_memory(&self, id: &str) -> bool {
        self.map.read().await.contains_key(id)
    }
}

#[cfg(test)]
mod bookkeeping_tests {
    use super::*;
    use hbb_common::tokio;

    /// Checks that a full cache does not hide a durable peer record.
    #[tokio::test]
    async fn durable_lookup_remains_readable_when_the_cache_is_full() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("peers.sqlite3").to_string_lossy().replace('\\', "/");
        let db = database::Database::new(&format!("sqlite://{path}?mode=rwc")).await.unwrap();
        db.insert_peer("durable", &[1], &[2], "{}").await.unwrap();
        let peers = PeerMap { map: Default::default(), db };
        let online = LockPeer::default();
        online.write().await.last_reg_time = Instant::now();
        peers.map.write().await.insert("online".to_owned(), online.clone());
        let loaded = peers.get_with_limit("durable", 1).await.unwrap();
        assert_eq!(&loaded.read().await.pk[..], &[2]);
        assert!(!peers.is_in_memory("durable").await);
        let mut map = peers.map.write().await;
        assert!(Arc::ptr_eq(map.get("online").unwrap(), &online));
        assert!(cache_peer(&mut map, "durable", loaded, 1).is_none());
        assert_eq!(map.len(), 1);
    }

    /// Checks the capacity limit without replacing an existing cached identity.
    #[test]
    fn capacity_rejects_new_peers_but_preserves_existing_identity() {
        let mut map = HashMap::new();
        let peer = LockPeer::default();
        cache_peer(&mut map, "peer1", peer.clone(), 1).unwrap();
        assert!(cache_peer(&mut map, "peer2", LockPeer::default(), 1).is_none());
        assert!(Arc::ptr_eq(&peer, &cache_peer(&mut map, "peer1", LockPeer::default(), 1).unwrap()));
        assert_eq!(map.len(), 1);
    }

    /// Checks that cleanup preserves active peers, in-use peers, and database records.
    #[tokio::test]
    async fn cleanup_preserves_online_and_in_use_peers_and_durable_records() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("peers.sqlite3").to_string_lossy().replace('\\', "/");
        let db = database::Database::new(&format!("sqlite://{path}?mode=rwc")).await.unwrap();
        db.insert_peer("expired", &[1], &[2], "{}").await.unwrap();
        let peers = PeerMap { map: Default::default(), db };
        let expired = LockPeer::default();
        let online = LockPeer::default();
        online.write().await.last_reg_time = Instant::now();
        let in_use = LockPeer::default();
        {
            let mut map = peers.map.write().await;
            map.insert("expired".to_owned(), expired);
            map.insert("online".to_owned(), online);
            map.insert("in_use".to_owned(), in_use.clone());
        }
        peers.prune().await;
        assert!(!peers.is_in_memory("expired").await);
        assert!(peers.is_in_memory("online").await);
        assert!(peers.is_in_memory("in_use").await);
        assert!(peers.db.get_peer("expired").await.unwrap().is_some());
        drop(in_use);
        peers.prune().await;
        assert!(!peers.is_in_memory("in_use").await);
    }
}
