use async_trait::async_trait;
use hbb_common::{log, ResultType};
use sqlx::{
    sqlite::SqliteConnectOptions, ConnectOptions, Connection, Error as SqlxError, SqliteConnection,
};
use std::{ops::DerefMut, str::FromStr};
//use sqlx::postgres::PgPoolOptions;
//use sqlx::mysql::MySqlPoolOptions;

type Pool = deadpool::managed::Pool<DbPool>;
const DEFAULT_MAX_TOTAL_PEER_RECORDS: usize = 100_000;
const DEFAULT_PEER_RECORD_RETENTION_DAYS: usize = 180;

pub struct DbPool {
    url: String,
}

#[async_trait]
impl deadpool::managed::Manager for DbPool {
    type Type = SqliteConnection;
    type Error = SqlxError;
    async fn create(&self) -> Result<SqliteConnection, SqlxError> {
        let mut opt = SqliteConnectOptions::from_str(&self.url).unwrap();
        opt.log_statements(log::LevelFilter::Debug);
        SqliteConnection::connect_with(&opt).await
    }
    async fn recycle(
        &self,
        obj: &mut SqliteConnection,
    ) -> deadpool::managed::RecycleResult<SqlxError> {
        Ok(obj.ping().await?)
    }
}

#[derive(Clone)]
pub struct Database {
    pool: Pool,
    max_total_peers: usize,
    peer_record_retention_days: usize,
}

pub enum InsertPeerResult {
    Inserted(Vec<u8>),
    PeerLimitReached,
}

fn env_usize_or(name: &str, default: usize) -> usize {
    std::env::var(name)
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(default)
}

fn peer_limit_reached(total_peers: i64, max_total_peers: usize) -> bool {
    max_total_peers > 0 && total_peers >= max_total_peers as i64
}

fn peer_retention_prune_enabled(days: usize) -> bool {
    days > 0
}

fn peer_retention_cutoff_arg(days: usize) -> String {
    format!("-{days} days")
}

#[derive(Default)]
pub struct Peer {
    pub guid: Vec<u8>,
    pub id: String,
    pub uuid: Vec<u8>,
    pub pk: Vec<u8>,
    pub user: Option<Vec<u8>>,
    pub info: String,
    pub status: Option<i64>,
}

impl Database {
    pub async fn new(url: &str) -> ResultType<Database> {
        if !std::path::Path::new(url).exists() {
            std::fs::File::create(url).ok();
        }
        let n: usize = std::env::var("MAX_DATABASE_CONNECTIONS")
            .unwrap_or_else(|_| "1".to_owned())
            .parse()
            .unwrap_or(1);
        log::debug!("MAX_DATABASE_CONNECTIONS={}", n);
        let max_total_peers = env_usize_or("MAX_TOTAL_PEER_RECORDS", DEFAULT_MAX_TOTAL_PEER_RECORDS);
        let peer_record_retention_days = env_usize_or(
            "PEER_RECORD_RETENTION_DAYS",
            DEFAULT_PEER_RECORD_RETENTION_DAYS,
        );
        log::info!("MAX_TOTAL_PEER_RECORDS={}", max_total_peers);
        log::info!("PEER_RECORD_RETENTION_DAYS={}", peer_record_retention_days);
        let pool = Pool::new(
            DbPool {
                url: url.to_owned(),
            },
            n,
        );
        let _ = pool.get().await?; // test
        let db = Database {
            pool,
            max_total_peers,
            peer_record_retention_days,
        };
        db.create_tables().await?;
        Ok(db)
    }

    async fn create_tables(&self) -> ResultType<()> {
        sqlx::query!(
            "
            create table if not exists peer (
                guid blob primary key not null,
                id varchar(100) not null,
                uuid blob not null,
                pk blob not null,
                created_at datetime not null default(current_timestamp),
                user blob,
                status tinyint,
                note varchar(300),
                info text not null
            ) without rowid;
            create unique index if not exists index_peer_id on peer (id);
            create index if not exists index_peer_user on peer (user);
            create index if not exists index_peer_created_at on peer (created_at);
            create index if not exists index_peer_status on peer (status);
        "
        )
        .execute(self.pool.get().await?.deref_mut())
        .await?;
        Ok(())
    }

    pub async fn get_peer(&self, id: &str) -> ResultType<Option<Peer>> {
        Ok(sqlx::query_as!(
            Peer,
            "select guid, id, uuid, pk, user, status, info from peer where id = ?",
            id
        )
        .fetch_optional(self.pool.get().await?.deref_mut())
        .await?)
    }

    pub async fn insert_peer(
        &self,
        id: &str,
        uuid: &[u8],
        pk: &[u8],
        info: &str,
    ) -> ResultType<InsertPeerResult> {
        if peer_limit_reached(self.peer_count().await?, self.max_total_peers) {
            if peer_retention_prune_enabled(self.peer_record_retention_days) {
                let deleted = self.prune_old_peer_records().await?;
                if deleted > 0 {
                    crate::common::record_protection_event("peer_records_pruned");
                    log::info!("pruned {} old peer records before inserting {}", deleted, id);
                }
            }
        }
        if peer_limit_reached(self.peer_count().await?, self.max_total_peers) {
            crate::common::record_protection_event("peer_limit_reached");
            log::warn!("peer record limit reached, rejecting new peer {}", id);
            return Ok(InsertPeerResult::PeerLimitReached);
        }
        let guid = uuid::Uuid::new_v4().as_bytes().to_vec();
        sqlx::query!(
            "insert into peer(guid, id, uuid, pk, info) values(?, ?, ?, ?, ?)",
            guid,
            id,
            uuid,
            pk,
            info
        )
        .execute(self.pool.get().await?.deref_mut())
        .await?;
        Ok(InsertPeerResult::Inserted(guid))
    }

    pub async fn update_pk(
        &self,
        guid: &Vec<u8>,
        id: &str,
        pk: &[u8],
        info: &str,
    ) -> ResultType<()> {
        sqlx::query!(
            "update peer set id=?, pk=?, info=? where guid=?",
            id,
            pk,
            info,
            guid
        )
        .execute(self.pool.get().await?.deref_mut())
        .await?;
        Ok(())
    }

    async fn peer_count(&self) -> ResultType<i64> {
        let row = sqlx::query!("select count(*) as count from peer")
            .fetch_one(self.pool.get().await?.deref_mut())
            .await?;
        Ok(row.count as i64)
    }

    async fn prune_old_peer_records(&self) -> ResultType<u64> {
        let cutoff = peer_retention_cutoff_arg(self.peer_record_retention_days);
        let result = sqlx::query("delete from peer where created_at < datetime('now', ?)")
            .bind(cutoff)
            .execute(self.pool.get().await?.deref_mut())
            .await?;
        Ok(result.rows_affected())
    }
}

#[cfg(test)]
mod tests {
    use super::{peer_limit_reached, peer_retention_cutoff_arg, peer_retention_prune_enabled};
    use hbb_common::tokio;
    use sqlx::Connection as _;
    use std::{path::PathBuf, time::{SystemTime, UNIX_EPOCH}};

    #[test]
    fn peer_limit_helper_rejects_when_total_reaches_cap() {
        assert!(!peer_limit_reached(99, 100));
        assert!(peer_limit_reached(100, 100));
        assert!(peer_limit_reached(101, 100));
    }

    #[test]
    fn peer_retention_helpers_use_configured_day_window() {
        assert!(peer_retention_prune_enabled(1));
        assert_eq!(peer_retention_cutoff_arg(180), "-180 days");
    }

    #[test]
    fn insert_peer_prunes_expired_records_before_enforcing_cap() {
        insert_peer_prunes_expired_records_before_enforcing_cap_();
    }

    #[test]
    fn test_insert() {
        insert();
    }

    fn temp_db_path(name: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        std::env::temp_dir().join(format!("rustdesk-server-{name}-{unique}.sqlite"))
    }

    #[tokio::main(flavor = "multi_thread")]
    async fn insert_peer_prunes_expired_records_before_enforcing_cap_() {
        let path = temp_db_path("peer-prune");
        let path_str = path.to_string_lossy().to_string();
        let mut db = super::Database::new(&path_str).await.unwrap();
        db.max_total_peers = 1;
        db.peer_record_retention_days = 1;

        let guid = uuid::Uuid::new_v4().as_bytes().to_vec();
        let empty_uuid = Vec::<u8>::new();
        let empty_pk = Vec::<u8>::new();
        let mut conn = sqlx::SqliteConnection::connect(&path_str).await.unwrap();
        sqlx::query!(
            "insert into peer(guid, id, uuid, pk, created_at, info) values(?, ?, ?, ?, datetime('now', '-2 days'), ?)",
            guid,
            "old-peer",
            empty_uuid,
            empty_pk,
            ""
        )
        .execute(&mut conn)
        .await
        .unwrap();

        let result = db
            .insert_peer("new-peer", &Vec::<u8>::new(), &Vec::<u8>::new(), "")
            .await
            .unwrap();
        assert!(matches!(result, super::InsertPeerResult::Inserted(_)));
        assert_eq!(db.peer_count().await.unwrap(), 1);
        assert!(db.get_peer("old-peer").await.unwrap().is_none());
        assert!(db.get_peer("new-peer").await.unwrap().is_some());

        std::fs::remove_file(path).ok();
    }

    #[tokio::main(flavor = "multi_thread")]
    async fn insert() {
        let db = super::Database::new("test.sqlite3").await.unwrap();
        let mut jobs = vec![];
        for i in 0..10000 {
            let cloned = db.clone();
            let id = i.to_string();
            let a = tokio::spawn(async move {
                let empty_vec = Vec::new();
                cloned
                    .insert_peer(&id, &empty_vec, &empty_vec, "")
                    .await
                    .unwrap();
            });
            jobs.push(a);
        }
        for i in 0..10000 {
            let cloned = db.clone();
            let id = i.to_string();
            let a = tokio::spawn(async move {
                cloned.get_peer(&id).await.unwrap();
            });
            jobs.push(a);
        }
        hbb_common::futures::future::join_all(jobs).await;
    }
}
