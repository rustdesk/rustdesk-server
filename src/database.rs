use async_trait::async_trait;
use hbb_common::{log, ResultType};
use sqlx::{
    sqlite::SqliteConnectOptions, ConnectOptions, Connection, Error as SqlxError, SqliteConnection,
    Row,
};
use std::{ops::DerefMut, str::FromStr};

type Pool = deadpool::managed::Pool<DbPool>;

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

#[derive(Clone, Debug, Default)]
pub struct AdminUser {
    pub id: i64,
    pub username: String,
    pub password_hash: String,
    pub role: String,
    pub status: i64,
    pub created_at: String,
}

#[derive(Clone, Debug, Default)]
pub struct PeerRecord {
    pub id: String,
    pub created_at: String,
    pub status: Option<i64>,
    pub note: Option<String>,
    pub info: String,
}

#[derive(Clone, Debug, Default)]
pub struct DeviceGroup {
    pub id: i64,
    pub name: String,
    pub created_at: String,
}

#[derive(Clone, Debug, Default)]
pub struct AbProfileRecord {
    pub guid: String,
    pub owner_user_id: i64,
    pub name: String,
    pub note: Option<String>,
    pub rule: i64,
}

#[derive(Clone, Debug, Default)]
pub struct AbPeerRecord {
    pub id: String,
    pub hash: String,
    pub password: String,
    pub username: String,
    pub hostname: String,
    pub platform: String,
    pub alias: String,
    pub tags: serde_json::Value,
    pub note: String,
    pub same_server: Option<i64>,
}

#[derive(Clone, Debug, Default)]
pub struct AbTagRecord {
    pub name: String,
    pub color: i64,
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
        let pool = Pool::new(
            DbPool {
                url: url.to_owned(),
            },
            n,
        );
        let _ = pool.get().await?;
        let db = Database { pool };
        db.create_tables().await?;
        db.ensure_default_admin().await?;
        Ok(db)
    }

    async fn create_tables(&self) -> ResultType<()> {
        sqlx::query(
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

            create table if not exists admin_user (
                id integer primary key autoincrement,
                username varchar(100) not null unique,
                password_hash text not null,
                role varchar(20) not null default 'user',
                status tinyint not null default 1,
                created_at datetime not null default(current_timestamp)
            );

            create table if not exists user_client_acl (
                id integer primary key autoincrement,
                user_id integer not null,
                client_id varchar(100) not null,
                created_at datetime not null default(current_timestamp),
                unique(user_id, client_id),
                foreign key(user_id) references admin_user(id) on delete cascade
            );
            create index if not exists index_user_client_acl_user on user_client_acl (user_id);
            create index if not exists index_user_client_acl_client on user_client_acl (client_id);

            create table if not exists device_group (
                id integer primary key autoincrement,
                name varchar(120) not null unique,
                created_at datetime not null default(current_timestamp)
            );

            create table if not exists device_group_member (
                id integer primary key autoincrement,
                group_id integer not null,
                peer_id varchar(100) not null,
                created_at datetime not null default(current_timestamp),
                unique(group_id, peer_id),
                foreign key(group_id) references device_group(id) on delete cascade
            );
            create index if not exists index_group_member_group on device_group_member (group_id);
            create index if not exists index_group_member_peer on device_group_member (peer_id);

            create table if not exists user_group_acl (
                id integer primary key autoincrement,
                user_id integer not null,
                group_id integer not null,
                created_at datetime not null default(current_timestamp),
                unique(user_id, group_id),
                foreign key(user_id) references admin_user(id) on delete cascade,
                foreign key(group_id) references device_group(id) on delete cascade
            );
            create index if not exists index_user_group_acl_user on user_group_acl (user_id);
            create index if not exists index_user_group_acl_group on user_group_acl (group_id);

            create table if not exists ab_profile (
                guid varchar(64) primary key not null,
                owner_user_id integer not null,
                name varchar(120) not null,
                note text,
                rule integer not null default 3,
                created_at datetime not null default(current_timestamp)
            );
            create unique index if not exists index_ab_profile_owner_name on ab_profile (owner_user_id, name);

            create table if not exists ab_peer (
                id integer primary key autoincrement,
                ab_guid varchar(64) not null,
                peer_id varchar(100) not null,
                alias varchar(255),
                tags text,
                hash text,
                password text,
                username varchar(255),
                hostname varchar(255),
                platform varchar(255),
                note text,
                same_server tinyint,
                created_at datetime not null default(current_timestamp),
                unique(ab_guid, peer_id),
                foreign key(ab_guid) references ab_profile(guid) on delete cascade
            );
            create index if not exists index_ab_peer_ab on ab_peer (ab_guid);
            create index if not exists index_ab_peer_peer on ab_peer (peer_id);

            create table if not exists ab_tag (
                id integer primary key autoincrement,
                ab_guid varchar(64) not null,
                name varchar(120) not null,
                color integer not null default 0,
                created_at datetime not null default(current_timestamp),
                unique(ab_guid, name),
                foreign key(ab_guid) references ab_profile(guid) on delete cascade
            );
            create index if not exists index_ab_tag_ab on ab_tag (ab_guid);
        "
        )
        .execute(self.pool.get().await?.deref_mut())
        .await?;
        self.ensure_schema_columns().await?;
        Ok(())
    }

    async fn ensure_schema_columns(&self) -> ResultType<()> {
        if !self.table_has_column("admin_user", "status").await? {
            sqlx::query("alter table admin_user add column status tinyint not null default 1")
                .execute(self.pool.get().await?.deref_mut())
                .await?;
        }
        Ok(())
    }

    async fn table_has_column(&self, table: &str, col: &str) -> ResultType<bool> {
        let sql = format!("pragma table_info({table})");
        let rows = sqlx::query(&sql)
            .fetch_all(self.pool.get().await?.deref_mut())
            .await?;
        for row in rows {
            let name: String = row.try_get("name")?;
            if name == col {
                return Ok(true);
            }
        }
        Ok(false)
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
    ) -> ResultType<Vec<u8>> {
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
        Ok(guid)
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

    pub async fn list_peers(&self) -> ResultType<Vec<PeerRecord>> {
        let rows = sqlx::query(
            "select id, created_at, status, note, info from peer order by datetime(created_at) desc",
        )
        .fetch_all(self.pool.get().await?.deref_mut())
        .await?;
        let mut out = Vec::with_capacity(rows.len());
        for row in rows {
            out.push(PeerRecord {
                id: row.try_get("id")?,
                created_at: row.try_get("created_at")?,
                status: row.try_get("status").ok(),
                note: row.try_get("note").ok(),
                info: row.try_get("info").unwrap_or_default(),
            });
        }
        Ok(out)
    }

    pub async fn list_users(&self) -> ResultType<Vec<AdminUser>> {
        let rows = sqlx::query(
            "select id, username, password_hash, role, status, created_at from admin_user order by id asc",
        )
        .fetch_all(self.pool.get().await?.deref_mut())
        .await?;
        let mut out = Vec::with_capacity(rows.len());
        for row in rows {
            out.push(AdminUser {
                id: row.try_get("id")?,
                username: row.try_get("username")?,
                password_hash: row.try_get("password_hash")?,
                role: row.try_get("role")?,
                status: row.try_get("status").unwrap_or(1),
                created_at: row.try_get("created_at")?,
            });
        }
        Ok(out)
    }

    pub async fn get_user_by_name(&self, username: &str) -> ResultType<Option<AdminUser>> {
        let row = sqlx::query(
            "select id, username, password_hash, role, status, created_at from admin_user where username = ?",
        )
        .bind(username)
        .fetch_optional(self.pool.get().await?.deref_mut())
        .await?;
        if let Some(row) = row {
            Ok(Some(AdminUser {
                id: row.try_get("id")?,
                username: row.try_get("username")?,
                password_hash: row.try_get("password_hash")?,
                role: row.try_get("role")?,
                status: row.try_get("status").unwrap_or(1),
                created_at: row.try_get("created_at")?,
            }))
        } else {
            Ok(None)
        }
    }

    pub async fn get_user_by_id(&self, user_id: i64) -> ResultType<Option<AdminUser>> {
        let row = sqlx::query(
            "select id, username, password_hash, role, status, created_at from admin_user where id = ?",
        )
        .bind(user_id)
        .fetch_optional(self.pool.get().await?.deref_mut())
        .await?;
        if let Some(row) = row {
            Ok(Some(AdminUser {
                id: row.try_get("id")?,
                username: row.try_get("username")?,
                password_hash: row.try_get("password_hash")?,
                role: row.try_get("role")?,
                status: row.try_get("status").unwrap_or(1),
                created_at: row.try_get("created_at")?,
            }))
        } else {
            Ok(None)
        }
    }

    pub async fn create_user(&self, username: &str, password_hash: &str, role: &str) -> ResultType<()> {
        sqlx::query("insert into admin_user(username, password_hash, role, status) values(?, ?, ?, 1)")
            .bind(username)
            .bind(password_hash)
            .bind(role)
            .execute(self.pool.get().await?.deref_mut())
            .await?;
        Ok(())
    }

    pub async fn set_user_status(&self, user_id: i64, status: i64) -> ResultType<()> {
        sqlx::query("update admin_user set status = ? where id = ?")
            .bind(status)
            .bind(user_id)
            .execute(self.pool.get().await?.deref_mut())
            .await?;
        Ok(())
    }

    pub async fn delete_user(&self, user_id: i64) -> ResultType<()> {
        sqlx::query("delete from admin_user where id = ?")
            .bind(user_id)
            .execute(self.pool.get().await?.deref_mut())
            .await?;
        Ok(())
    }

    pub async fn list_user_client_acl(&self, user_id: i64) -> ResultType<Vec<String>> {
        let rows = sqlx::query("select client_id from user_client_acl where user_id = ?")
            .bind(user_id)
            .fetch_all(self.pool.get().await?.deref_mut())
            .await?;
        let mut out = Vec::with_capacity(rows.len());
        for row in rows {
            out.push(row.try_get("client_id")?);
        }
        Ok(out)
    }

    pub async fn grant_user_client_acl(&self, user_id: i64, client_id: &str) -> ResultType<()> {
        sqlx::query("insert or ignore into user_client_acl(user_id, client_id) values(?, ?)")
            .bind(user_id)
            .bind(client_id)
            .execute(self.pool.get().await?.deref_mut())
            .await?;
        Ok(())
    }

    pub async fn revoke_user_client_acl(&self, user_id: i64, client_id: &str) -> ResultType<()> {
        sqlx::query("delete from user_client_acl where user_id = ? and client_id = ?")
            .bind(user_id)
            .bind(client_id)
            .execute(self.pool.get().await?.deref_mut())
            .await?;
        Ok(())
    }

    pub async fn set_peer_status(&self, client_id: &str, status: i64) -> ResultType<()> {
        sqlx::query("update peer set status = ? where id = ?")
            .bind(status)
            .bind(client_id)
            .execute(self.pool.get().await?.deref_mut())
            .await?;
        Ok(())
    }

    pub async fn delete_peer(&self, client_id: &str) -> ResultType<()> {
        sqlx::query("delete from user_client_acl where client_id = ?")
            .bind(client_id)
            .execute(self.pool.get().await?.deref_mut())
            .await?;
        sqlx::query("delete from device_group_member where peer_id = ?")
            .bind(client_id)
            .execute(self.pool.get().await?.deref_mut())
            .await?;
        sqlx::query("delete from peer where id = ?")
            .bind(client_id)
            .execute(self.pool.get().await?.deref_mut())
            .await?;
        Ok(())
    }

    pub async fn list_groups(&self) -> ResultType<Vec<DeviceGroup>> {
        let rows = sqlx::query("select id, name, created_at from device_group order by id asc")
            .fetch_all(self.pool.get().await?.deref_mut())
            .await?;
        let mut out = Vec::with_capacity(rows.len());
        for row in rows {
            out.push(DeviceGroup {
                id: row.try_get("id")?,
                name: row.try_get("name")?,
                created_at: row.try_get("created_at")?,
            });
        }
        Ok(out)
    }

    pub async fn create_group(&self, name: &str) -> ResultType<()> {
        sqlx::query("insert into device_group(name) values(?)")
            .bind(name)
            .execute(self.pool.get().await?.deref_mut())
            .await?;
        Ok(())
    }

    pub async fn delete_group(&self, group_id: i64) -> ResultType<()> {
        sqlx::query("delete from device_group where id = ?")
            .bind(group_id)
            .execute(self.pool.get().await?.deref_mut())
            .await?;
        Ok(())
    }

    pub async fn list_group_peers(&self, group_id: i64) -> ResultType<Vec<String>> {
        let rows = sqlx::query("select peer_id from device_group_member where group_id = ?")
            .bind(group_id)
            .fetch_all(self.pool.get().await?.deref_mut())
            .await?;
        let mut out = Vec::with_capacity(rows.len());
        for row in rows {
            out.push(row.try_get("peer_id")?);
        }
        Ok(out)
    }

    pub async fn add_group_peer(&self, group_id: i64, peer_id: &str) -> ResultType<()> {
        sqlx::query("insert or ignore into device_group_member(group_id, peer_id) values(?, ?)")
            .bind(group_id)
            .bind(peer_id)
            .execute(self.pool.get().await?.deref_mut())
            .await?;
        Ok(())
    }

    pub async fn remove_group_peer(&self, group_id: i64, peer_id: &str) -> ResultType<()> {
        sqlx::query("delete from device_group_member where group_id = ? and peer_id = ?")
            .bind(group_id)
            .bind(peer_id)
            .execute(self.pool.get().await?.deref_mut())
            .await?;
        Ok(())
    }

    pub async fn list_user_group_acl(&self, user_id: i64) -> ResultType<Vec<i64>> {
        let rows = sqlx::query("select group_id from user_group_acl where user_id = ?")
            .bind(user_id)
            .fetch_all(self.pool.get().await?.deref_mut())
            .await?;
        let mut out = Vec::with_capacity(rows.len());
        for row in rows {
            out.push(row.try_get("group_id")?);
        }
        Ok(out)
    }

    pub async fn grant_user_group_acl(&self, user_id: i64, group_id: i64) -> ResultType<()> {
        sqlx::query("insert or ignore into user_group_acl(user_id, group_id) values(?, ?)")
            .bind(user_id)
            .bind(group_id)
            .execute(self.pool.get().await?.deref_mut())
            .await?;
        Ok(())
    }

    pub async fn revoke_user_group_acl(&self, user_id: i64, group_id: i64) -> ResultType<()> {
        sqlx::query("delete from user_group_acl where user_id = ? and group_id = ?")
            .bind(user_id)
            .bind(group_id)
            .execute(self.pool.get().await?.deref_mut())
            .await?;
        Ok(())
    }

    pub async fn list_group_peers_for_user(&self, user_id: i64) -> ResultType<Vec<String>> {
        let rows = sqlx::query(
            "select distinct gm.peer_id
             from user_group_acl uga
             inner join device_group_member gm on gm.group_id = uga.group_id
             where uga.user_id = ?",
        )
        .bind(user_id)
        .fetch_all(self.pool.get().await?.deref_mut())
        .await?;
        let mut out = Vec::with_capacity(rows.len());
        for row in rows {
            out.push(row.try_get("peer_id")?);
        }
        Ok(out)
    }

    pub async fn ensure_personal_ab(&self, user_id: i64, username: &str) -> ResultType<AbProfileRecord> {
        let row = sqlx::query(
            "select guid, owner_user_id, name, note, rule from ab_profile where owner_user_id = ? and name = ?",
        )
        .bind(user_id)
        .bind("My address book")
        .fetch_optional(self.pool.get().await?.deref_mut())
        .await?;
        if let Some(row) = row {
            return Ok(AbProfileRecord {
                guid: row.try_get("guid")?,
                owner_user_id: row.try_get("owner_user_id")?,
                name: row.try_get("name")?,
                note: row.try_get("note").ok(),
                rule: row.try_get("rule").unwrap_or(3),
            });
        }
        let guid = uuid::Uuid::new_v4().to_string();
        sqlx::query("insert into ab_profile(guid, owner_user_id, name, note, rule) values(?, ?, ?, ?, 3)")
            .bind(&guid)
            .bind(user_id)
            .bind("My address book")
            .bind(format!("personal address book of {username}"))
            .execute(self.pool.get().await?.deref_mut())
            .await?;
        Ok(AbProfileRecord {
            guid,
            owner_user_id: user_id,
            name: "My address book".to_owned(),
            note: None,
            rule: 3,
        })
    }

    pub async fn get_ab_profile(&self, guid: &str) -> ResultType<Option<AbProfileRecord>> {
        let row = sqlx::query(
            "select guid, owner_user_id, name, note, rule from ab_profile where guid = ?",
        )
        .bind(guid)
        .fetch_optional(self.pool.get().await?.deref_mut())
        .await?;
        if let Some(row) = row {
            Ok(Some(AbProfileRecord {
                guid: row.try_get("guid")?,
                owner_user_id: row.try_get("owner_user_id")?,
                name: row.try_get("name")?,
                note: row.try_get("note").ok(),
                rule: row.try_get("rule").unwrap_or(3),
            }))
        } else {
            Ok(None)
        }
    }

    pub async fn list_shared_ab_profiles(&self, user_id: i64) -> ResultType<Vec<AbProfileRecord>> {
        let rows = sqlx::query(
            "select guid, owner_user_id, name, note, rule from ab_profile where owner_user_id = ? and name <> 'My address book' order by name asc",
        )
        .bind(user_id)
        .fetch_all(self.pool.get().await?.deref_mut())
        .await?;
        let mut out = Vec::with_capacity(rows.len());
        for row in rows {
            out.push(AbProfileRecord {
                guid: row.try_get("guid")?,
                owner_user_id: row.try_get("owner_user_id")?,
                name: row.try_get("name")?,
                note: row.try_get("note").ok(),
                rule: row.try_get("rule").unwrap_or(3),
            });
        }
        Ok(out)
    }

    pub async fn list_ab_peers(&self, ab_guid: &str) -> ResultType<Vec<AbPeerRecord>> {
        let rows = sqlx::query(
            "select peer_id, hash, password, username, hostname, platform, alias, tags, note, same_server
             from ab_peer where ab_guid = ? order by id desc",
        )
        .bind(ab_guid)
        .fetch_all(self.pool.get().await?.deref_mut())
        .await?;
        let mut out = Vec::with_capacity(rows.len());
        for row in rows {
            let tags_raw: String = row.try_get("tags").unwrap_or_else(|_| "[]".to_owned());
            let tags = serde_json::from_str::<serde_json::Value>(&tags_raw)
                .unwrap_or_else(|_| serde_json::json!([]));
            out.push(AbPeerRecord {
                id: row.try_get("peer_id")?,
                hash: row.try_get("hash").unwrap_or_default(),
                password: row.try_get("password").unwrap_or_default(),
                username: row.try_get("username").unwrap_or_default(),
                hostname: row.try_get("hostname").unwrap_or_default(),
                platform: row.try_get("platform").unwrap_or_default(),
                alias: row.try_get("alias").unwrap_or_default(),
                tags,
                note: row.try_get("note").unwrap_or_default(),
                same_server: row.try_get("same_server").ok(),
            });
        }
        Ok(out)
    }

    pub async fn add_ab_peer(&self, ab_guid: &str, peer: &serde_json::Value) -> ResultType<()> {
        let id = peer.get("id").and_then(|v| v.as_str()).unwrap_or("").trim();
        if id.is_empty() {
            return Ok(());
        }
        let alias = peer.get("alias").and_then(|v| v.as_str()).unwrap_or("");
        let hash = peer.get("hash").and_then(|v| v.as_str()).unwrap_or("");
        let password = peer.get("password").and_then(|v| v.as_str()).unwrap_or("");
        let username = peer.get("username").and_then(|v| v.as_str()).unwrap_or("");
        let hostname = peer.get("hostname").and_then(|v| v.as_str()).unwrap_or("");
        let platform = peer.get("platform").and_then(|v| v.as_str()).unwrap_or("");
        let note = peer.get("note").and_then(|v| v.as_str()).unwrap_or("");
        let same_server = peer.get("same_server").and_then(|v| v.as_i64());
        let tags = peer.get("tags").cloned().unwrap_or_else(|| serde_json::json!([]));
        sqlx::query(
            "insert or replace into ab_peer(ab_guid, peer_id, alias, tags, hash, password, username, hostname, platform, note, same_server)
             values(?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(ab_guid)
        .bind(id)
        .bind(alias)
        .bind(tags.to_string())
        .bind(hash)
        .bind(password)
        .bind(username)
        .bind(hostname)
        .bind(platform)
        .bind(note)
        .bind(same_server)
        .execute(self.pool.get().await?.deref_mut())
        .await?;
        Ok(())
    }

    pub async fn update_ab_peer_partial(&self, ab_guid: &str, patch: &serde_json::Value) -> ResultType<()> {
        let id = patch.get("id").and_then(|v| v.as_str()).unwrap_or("").trim();
        if id.is_empty() {
            return Ok(());
        }
        let mut current = sqlx::query(
            "select alias, tags, hash, password, username, hostname, platform, note, same_server
             from ab_peer where ab_guid = ? and peer_id = ?",
        )
        .bind(ab_guid)
        .bind(id)
        .fetch_optional(self.pool.get().await?.deref_mut())
        .await?;
        if current.is_none() {
            self.add_ab_peer(ab_guid, patch).await?;
            return Ok(());
        }
        let row = current.take().unwrap();
        let mut alias: String = row.try_get("alias").unwrap_or_default();
        let mut tags_raw: String = row.try_get("tags").unwrap_or_else(|_| "[]".to_owned());
        let mut hash: String = row.try_get("hash").unwrap_or_default();
        let mut password: String = row.try_get("password").unwrap_or_default();
        let mut username: String = row.try_get("username").unwrap_or_default();
        let mut hostname: String = row.try_get("hostname").unwrap_or_default();
        let mut platform: String = row.try_get("platform").unwrap_or_default();
        let mut note: String = row.try_get("note").unwrap_or_default();
        let mut same_server: Option<i64> = row.try_get("same_server").ok();

        if let Some(v) = patch.get("alias").and_then(|v| v.as_str()) { alias = v.to_owned(); }
        if patch.get("tags").is_some() { tags_raw = patch.get("tags").cloned().unwrap_or_else(|| serde_json::json!([])).to_string(); }
        if let Some(v) = patch.get("hash").and_then(|v| v.as_str()) { hash = v.to_owned(); }
        if let Some(v) = patch.get("password").and_then(|v| v.as_str()) { password = v.to_owned(); }
        if let Some(v) = patch.get("username").and_then(|v| v.as_str()) { username = v.to_owned(); }
        if let Some(v) = patch.get("hostname").and_then(|v| v.as_str()) { hostname = v.to_owned(); }
        if let Some(v) = patch.get("platform").and_then(|v| v.as_str()) { platform = v.to_owned(); }
        if let Some(v) = patch.get("note").and_then(|v| v.as_str()) { note = v.to_owned(); }
        if patch.get("same_server").is_some() { same_server = patch.get("same_server").and_then(|v| v.as_i64()); }

        sqlx::query(
            "update ab_peer set alias=?, tags=?, hash=?, password=?, username=?, hostname=?, platform=?, note=?, same_server=?
             where ab_guid=? and peer_id=?",
        )
        .bind(alias)
        .bind(tags_raw)
        .bind(hash)
        .bind(password)
        .bind(username)
        .bind(hostname)
        .bind(platform)
        .bind(note)
        .bind(same_server)
        .bind(ab_guid)
        .bind(id)
        .execute(self.pool.get().await?.deref_mut())
        .await?;
        Ok(())
    }

    pub async fn delete_ab_peers(&self, ab_guid: &str, ids: &[String]) -> ResultType<()> {
        for id in ids {
            sqlx::query("delete from ab_peer where ab_guid = ? and peer_id = ?")
                .bind(ab_guid)
                .bind(id)
                .execute(self.pool.get().await?.deref_mut())
                .await?;
        }
        Ok(())
    }

    pub async fn list_ab_tags(&self, ab_guid: &str) -> ResultType<Vec<AbTagRecord>> {
        let rows = sqlx::query("select name, color from ab_tag where ab_guid = ? order by name asc")
            .bind(ab_guid)
            .fetch_all(self.pool.get().await?.deref_mut())
            .await?;
        let mut out = Vec::with_capacity(rows.len());
        for row in rows {
            out.push(AbTagRecord {
                name: row.try_get("name")?,
                color: row.try_get("color").unwrap_or(0),
            });
        }
        Ok(out)
    }

    pub async fn add_ab_tag(&self, ab_guid: &str, name: &str, color: i64) -> ResultType<()> {
        sqlx::query("insert or replace into ab_tag(ab_guid, name, color) values(?, ?, ?)")
            .bind(ab_guid)
            .bind(name)
            .bind(color)
            .execute(self.pool.get().await?.deref_mut())
            .await?;
        Ok(())
    }

    pub async fn rename_ab_tag(&self, ab_guid: &str, old: &str, new_name: &str) -> ResultType<()> {
        sqlx::query("update ab_tag set name = ? where ab_guid = ? and name = ?")
            .bind(new_name)
            .bind(ab_guid)
            .bind(old)
            .execute(self.pool.get().await?.deref_mut())
            .await?;
        let peers = self.list_ab_peers(ab_guid).await?;
        for p in peers {
            let mut tags = match p.tags {
                serde_json::Value::Array(arr) => arr,
                _ => vec![],
            };
            let mut changed = false;
            for t in tags.iter_mut() {
                if t.as_str() == Some(old) {
                    *t = serde_json::Value::String(new_name.to_owned());
                    changed = true;
                }
            }
            if changed {
                sqlx::query("update ab_peer set tags = ? where ab_guid = ? and peer_id = ?")
                    .bind(serde_json::Value::Array(tags).to_string())
                    .bind(ab_guid)
                    .bind(&p.id)
                    .execute(self.pool.get().await?.deref_mut())
                    .await?;
            }
        }
        Ok(())
    }

    pub async fn update_ab_tag_color(&self, ab_guid: &str, name: &str, color: i64) -> ResultType<()> {
        sqlx::query("update ab_tag set color = ? where ab_guid = ? and name = ?")
            .bind(color)
            .bind(ab_guid)
            .bind(name)
            .execute(self.pool.get().await?.deref_mut())
            .await?;
        Ok(())
    }

    pub async fn delete_ab_tags(&self, ab_guid: &str, tags: &[String]) -> ResultType<()> {
        for t in tags {
            sqlx::query("delete from ab_tag where ab_guid = ? and name = ?")
                .bind(ab_guid)
                .bind(t)
                .execute(self.pool.get().await?.deref_mut())
                .await?;
        }
        Ok(())
    }
    pub async fn ensure_default_admin(&self) -> ResultType<()> {
        let username = std::env::var("ADMIN_USERNAME").unwrap_or_else(|_| "admin".to_owned());
        let password = std::env::var("ADMIN_PASSWORD").unwrap_or_else(|_| "admin123456".to_owned());
        let exists = sqlx::query("select id from admin_user where username = ?")
            .bind(&username)
            .fetch_optional(self.pool.get().await?.deref_mut())
            .await?;
        if exists.is_none() {
            let hashed = bcrypt::hash(password, bcrypt::DEFAULT_COST)?;
            sqlx::query("insert into admin_user(username, password_hash, role, status) values(?, ?, 'admin', 1)")
                .bind(username)
                .bind(hashed)
                .execute(self.pool.get().await?.deref_mut())
                .await?;
            log::info!("Created default admin user from ADMIN_USERNAME/ADMIN_PASSWORD env");
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use hbb_common::tokio;
    #[test]
    fn test_insert() {
        insert();
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



