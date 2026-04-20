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
const DEFAULT_MAX_IP_BLOCKER_ENTRIES: usize = 8_192;
const DEFAULT_MAX_IP_CHANGES_ENTRIES: usize = 8_192;
const DEFAULT_MAX_UNIQUE_IP_CHANGES_PER_ID: usize = 32;
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

fn max_ip_blocker_entries() -> usize {
    env_usize_or("MAX_IP_BLOCKER_ENTRIES", DEFAULT_MAX_IP_BLOCKER_ENTRIES)
}

fn max_ip_changes_entries() -> usize {
    env_usize_or("MAX_IP_CHANGES_ENTRIES", DEFAULT_MAX_IP_CHANGES_ENTRIES)
}

fn max_unique_ip_changes_per_id() -> usize {
    env_usize_or(
        "MAX_UNIQUE_IP_CHANGES_PER_ID",
        DEFAULT_MAX_UNIQUE_IP_CHANGES_PER_ID,
    )
}

pub(crate) fn prune_ip_blocker_entries(entries: &mut IpBlockMap) {
    entries.retain(|_, (short_window, daily_window)| {
        short_window.1.elapsed().as_secs() <= IP_BLOCK_DUR
            || daily_window.1.elapsed().as_secs() <= DAY_SECONDS
    });
}

fn evict_oldest_ip_blocker_entry(entries: &mut IpBlockMap) -> bool {
    let oldest_ip = entries
        .iter()
        .min_by_key(|(_, (short_window, daily_window))| short_window.1.max(daily_window.1))
        .map(|(ip, _)| ip.clone());
    if let Some(ip) = oldest_ip {
        entries.remove(&ip);
        return true;
    }
    false
}

pub(crate) fn allow_ip_registration_attempt(entries: &mut IpBlockMap, ip: &str, id: &str) -> bool {
    prune_ip_blocker_entries(entries);
    if !entries.contains_key(ip) {
        let max_entries = max_ip_blocker_entries();
        if max_entries > 0 && entries.len() >= max_entries && evict_oldest_ip_blocker_entry(entries)
        {
            record_protection_event("ip_blocker_entries_evicted");
        }
    }
    if let Some(old) = entries.get_mut(ip) {
        let now = Instant::now();
        let counter = &mut old.0;
        if counter.1.elapsed().as_secs() > IP_BLOCK_DUR {
            counter.0 = 0;
        } else if counter.0 > 30 {
            return false;
        }
        counter.0 += 1;
        counter.1 = now;

        let counter = &mut old.1;
        let is_new = counter.0.get(id).is_none();
        if counter.1.elapsed().as_secs() > DAY_SECONDS {
            counter.0.clear();
        } else if counter.0.len() > 300 {
            return !is_new;
        }
        if is_new {
            counter.0.insert(id.to_owned());
        }
        counter.1 = now;
    } else {
        entries.insert(
            ip.to_owned(),
            ((0, Instant::now()), (Default::default(), Instant::now())),
        );
    }
    true
}

pub(crate) fn prune_ip_change_entries(entries: &mut IpChangesMap) {
    entries.retain(|_, value| value.0.elapsed().as_secs() < IP_CHANGE_DUR_X2 && value.1.len() > 1);
}

fn evict_oldest_ip_change_entry(entries: &mut IpChangesMap) -> bool {
    let oldest_id = entries
        .iter()
        .min_by_key(|(_, (tm, _))| *tm)
        .map(|(id, _)| id.clone());
    if let Some(id) = oldest_id {
        entries.remove(&id);
        return true;
    }
    false
}

pub(crate) fn track_ip_change(entries: &mut IpChangesMap, id: &str, ip: &str) {
    prune_ip_change_entries(entries);
    if let Some((tm, ips)) = entries.get_mut(id) {
        if tm.elapsed().as_secs() > IP_CHANGE_DUR {
            *tm = Instant::now();
            ips.clear();
            ips.insert(ip.to_owned(), 1);
            return;
        }
        if let Some(value) = ips.get_mut(ip) {
            *value += 1;
            return;
        }
        if max_unique_ip_changes_per_id() > 0 && ips.len() >= max_unique_ip_changes_per_id() {
            record_protection_event("ip_change_ip_limit_hits");
            return;
        }
        ips.insert(ip.to_owned(), 1);
        return;
    }
    let max_entries = max_ip_changes_entries();
    if max_entries > 0 && entries.len() >= max_entries && evict_oldest_ip_change_entry(entries) {
        record_protection_event("ip_changes_entries_evicted");
    }
    entries.insert(
        id.to_owned(),
        (Instant::now(), HashMap::from([(ip.to_owned(), 1)])),
    );
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
        let max_cached_peers = env_usize_or("MAX_PEER_CACHE_SIZE", DEFAULT_MAX_PEER_CACHE_SIZE);
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
        let mut w = self.map.write().await;
        if let Some(p) = w.get(id) {
            return Some(p.clone());
        }
        let snapshot: Vec<(String, LockPeer)> = w
            .iter()
            .map(|(id, peer)| (id.clone(), peer.clone()))
            .collect();
        let mut entries = Vec::with_capacity(snapshot.len());
        let mut pending_ids_for_ip = HashSet::new();
        for (peer_id, peer) in snapshot {
            let peer = peer.read().await;
            if peer.info.ip == ip && peer.guid.is_empty() {
                pending_ids_for_ip.insert(peer_id.clone());
            }
            entries.push(PeerEvictionEntry {
                id: peer_id,
                inactive: peer.last_reg_time.elapsed().as_millis() as i32
                    >= PEER_CACHE_INACTIVE_TIMEOUT_MS,
                last_reg_time: peer.last_reg_time,
            });
        }
        let remove_ids = select_peer_ids_to_evict(entries, self.max_cached_peers);
        if !remove_ids.is_empty() {
            let remove_ids_set: HashSet<String> = remove_ids.iter().cloned().collect();
            for id in &remove_ids {
                w.remove(id);
            }
            pending_ids_for_ip.retain(|id| !remove_ids_set.contains(id));
        }
        if pending_registration_limit_exceeded(
            pending_ids_for_ip.len(),
            self.max_pending_registrations_per_ip,
        ) {
            return None;
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
        allow_ip_registration_attempt, pending_registration_limit_exceeded,
        prune_ip_change_entries, select_peer_ids_to_evict, track_ip_change, IpBlockMap,
        IpChangesMap, PeerEvictionEntry, DAY_SECONDS, IP_CHANGE_DUR_X2,
    };
    use std::{
        collections::{HashMap, HashSet},
        time::{Duration, Instant},
    };

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
        assert_eq!(
            remove,
            vec!["inactive-old".to_owned(), "inactive-new".to_owned()]
        );
    }

    #[test]
    fn allow_ip_registration_attempt_prunes_and_caps_entries() {
        std::env::set_var("MAX_IP_BLOCKER_ENTRIES", "2");
        let now = Instant::now();
        let stale = now
            .checked_sub(Duration::from_secs(DAY_SECONDS + 5))
            .unwrap_or(now);
        let older = now.checked_sub(Duration::from_secs(10)).unwrap_or(now);
        let newer = now.checked_sub(Duration::from_secs(1)).unwrap_or(now);
        let mut entries: IpBlockMap = HashMap::from([
            (
                "198.51.100.1".to_owned(),
                ((0, stale), (HashSet::new(), stale)),
            ),
            (
                "198.51.100.2".to_owned(),
                ((1, older), (HashSet::from(["peer-a".to_owned()]), older)),
            ),
            (
                "198.51.100.3".to_owned(),
                ((1, newer), (HashSet::from(["peer-b".to_owned()]), newer)),
            ),
        ]);
        assert!(allow_ip_registration_attempt(
            &mut entries,
            "198.51.100.4",
            "peer-c"
        ));
        assert_eq!(entries.len(), 2);
        assert!(!entries.contains_key("198.51.100.1"));
        assert!(!entries.contains_key("198.51.100.2"));
        assert!(entries.contains_key("198.51.100.3"));
        assert!(entries.contains_key("198.51.100.4"));
        std::env::remove_var("MAX_IP_BLOCKER_ENTRIES");
    }

    #[test]
    fn track_ip_change_prunes_and_limits_unique_ips() {
        std::env::set_var("MAX_IP_CHANGES_ENTRIES", "2");
        std::env::set_var("MAX_UNIQUE_IP_CHANGES_PER_ID", "2");
        let now = Instant::now();
        let stale = now
            .checked_sub(Duration::from_secs(IP_CHANGE_DUR_X2 + 5))
            .unwrap_or(now);
        let mut entries: IpChangesMap = HashMap::from([
            (
                "stale-id".to_owned(),
                (stale, HashMap::from([("198.51.100.1".to_owned(), 1)])),
            ),
            (
                "peer-a".to_owned(),
                (
                    now,
                    HashMap::from([
                        ("198.51.100.2".to_owned(), 1),
                        ("198.51.100.3".to_owned(), 1),
                    ]),
                ),
            ),
            (
                "peer-b".to_owned(),
                (
                    now,
                    HashMap::from([
                        ("198.51.100.4".to_owned(), 1),
                        ("198.51.100.5".to_owned(), 1),
                    ]),
                ),
            ),
        ]);
        prune_ip_change_entries(&mut entries);
        assert_eq!(entries.len(), 2);
        track_ip_change(&mut entries, "peer-a", "198.51.100.9");
        assert_eq!(entries["peer-a"].1.len(), 2);
        track_ip_change(&mut entries, "peer-c", "198.51.100.6");
        assert_eq!(entries.len(), 2);
        assert!(entries.contains_key("peer-a"));
        assert!(entries.contains_key("peer-c"));
        std::env::remove_var("MAX_IP_CHANGES_ENTRIES");
        std::env::remove_var("MAX_UNIQUE_IP_CHANGES_PER_ID");
    }
}
