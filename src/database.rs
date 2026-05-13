use async_trait::async_trait;
use core_common::{log, ResultType};
use sqlx::{
    sqlite::SqliteConnectOptions, ConnectOptions, Connection, Error as SqlxError, SqliteConnection, Row,
};
use std::{ops::DerefMut, str::FromStr};
use uuid::Uuid;
//use sqlx::postgres::PgPoolOptions;
//use sqlx::mysql::MySqlPoolOptions;

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
    pub pool: Pool,
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
    pub created_at: Option<chrono::DateTime<chrono::Utc>>,
    /// `users.id` when bound via RegisterPk + JWT
    pub bound_user_id: Option<i64>,
    /// `user_devices.id` (PK) when bound
    pub bound_device_row_id: Option<i64>,
}

/// `users.role`: `admin` or `user`.
pub const USER_ROLE_USER: &str = "user";
pub const USER_ROLE_ADMIN: &str = "admin";

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct User {
    pub id: i64,
    pub username: String,
    pub email: String,
    pub password_hash: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
    pub is_active: bool,
    pub role: String,
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct UserDevice {
    pub id: i64,
    pub user_id: i64,
    pub device_id: String,
    pub device_name: Option<String>,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub is_active: bool,
}

#[derive(Debug, Clone)]
pub struct CreateUserRequest {
    pub username: String,
    pub email: String,
    pub password: String,
}

#[derive(Debug, Clone)]
pub struct CreateDeviceRequest {
    pub user_id: i64,
    pub device_id: String,
    pub device_name: Option<String>,
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
        let _ = pool.get().await?; // test
        let db = Database { pool };
        db.create_tables().await?;
        Ok(db)
    }

    async fn create_tables(&self) -> ResultType<()> {
        let mut conn = self.pool.get().await?;
        
        // peer 表：先创建与上游兼容的基础结构（便于旧库升级），再追加列与索引
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
        ",
        )
        .execute(&mut *conn)
        .await?;

        for sql in [
            "ALTER TABLE peer ADD COLUMN user_id INTEGER",
            "ALTER TABLE peer ADD COLUMN device_id INTEGER",
            "ALTER TABLE peer ADD COLUMN disabled boolean NOT NULL DEFAULT 0",
        ] {
            if let Err(e) = sqlx::query(sql).execute(&mut *conn).await {
                let msg = e.to_string();
                if !msg.contains("duplicate column name") {
                    log::warn!("peer column migration `{}`: {}", sql, e);
                }
            }
        }

        for sql in [
            "create index if not exists index_peer_user_id on peer (user_id)",
            "create index if not exists index_peer_device_id on peer (device_id)",
            "create index if not exists index_peer_disabled on peer (disabled)",
        ] {
            if let Err(e) = sqlx::query(sql).execute(&mut *conn).await {
                log::warn!("peer index migration `{}`: {}", sql, e);
            }
        }

        // 创建用户表
        sqlx::query("
            create table if not exists users (
                id integer primary key autoincrement,
                username varchar(50) unique not null,
                email varchar(100) unique not null,
                password_hash varchar(255) not null,
                created_at datetime not null default(current_timestamp),
                updated_at datetime not null default(current_timestamp),
                is_active boolean not null default 1
            );
            create index if not exists index_users_username on users (username);
            create index if not exists index_users_email on users (email);
            create index if not exists index_users_active on users (is_active);
        ")
        .execute(&mut *conn)
        .await?;

        // 创建用户设备关系表
        sqlx::query("
            create table if not exists user_devices (
                id integer primary key autoincrement,
                user_id integer not null,
                device_id varchar(100) not null,
                device_name varchar(100),
                created_at datetime not null default(current_timestamp),
                is_active boolean not null default 1,
                foreign key (user_id) references users (id) on delete cascade,
                unique(user_id, device_id)
            );
            create index if not exists index_user_devices_user_id on user_devices (user_id);
            create index if not exists index_user_devices_device_id on user_devices (device_id);
            create index if not exists index_user_devices_active on user_devices (is_active);
        ")
        .execute(&mut *conn)
        .await?;

        // 创建密码重置令牌表
        sqlx::query("
            create table if not exists password_reset_tokens (
                id integer primary key autoincrement,
                user_id integer not null,
                token varchar(255) unique not null,
                expires_at datetime not null,
                created_at datetime not null default(current_timestamp),
                is_used boolean not null default 0,
                foreign key (user_id) references users (id) on delete cascade
            );
            create index if not exists index_password_reset_tokens_user_id on password_reset_tokens (user_id);
            create index if not exists index_password_reset_tokens_token on password_reset_tokens (token);
            create index if not exists index_password_reset_tokens_expires_at on password_reset_tokens (expires_at);
        ")
        .execute(&mut *conn)
        .await?;

        self.migrate_users_add_role_column().await?;
        self.bootstrap_admin_if_none().await?;

        Ok(())
    }

    async fn migrate_users_add_role_column(&self) -> ResultType<()> {
        let mut conn = self.pool.get().await?;
        let sql = "ALTER TABLE users ADD COLUMN role TEXT NOT NULL DEFAULT 'user'";
        if let Err(e) = sqlx::query(sql).execute(&mut *conn).await {
            let msg = e.to_string();
            if !msg.contains("duplicate column name") {
                log::warn!("users.role migration `{}`: {}", sql, e);
            }
        }
        Ok(())
    }

    /// When no row has `role = admin`, promote one account so the server remains manageable.
    async fn bootstrap_admin_if_none(&self) -> ResultType<()> {
        let mut conn = self.pool.get().await?;
        let admin_n: i64 = sqlx::query_scalar("select count(*) from users where role = 'admin'")
            .fetch_one(&mut *conn)
            .await
            .unwrap_or(0);
        if admin_n > 0 {
            return Ok(());
        }
        let total: i64 = sqlx::query_scalar("select count(*) from users")
            .fetch_one(&mut *conn)
            .await
            .unwrap_or(0);
        if total == 0 {
            return Ok(());
        }
        let mut promoted: u64 = 0;
        if let Ok(name) = std::env::var("BOOTSTRAP_ADMIN_USERNAME") {
            let name = name.trim();
            if !name.is_empty() {
                let r = sqlx::query("update users set role = 'admin' where username = ?")
                    .bind(name)
                    .execute(&mut *conn)
                    .await?;
                promoted = r.rows_affected();
            }
        }
        if promoted == 0 {
            sqlx::query("update users set role = 'admin' where id = (select min(id) from users)")
                .execute(&mut *conn)
                .await?;
            log::warn!(
                "No administrator found; promoted the earliest user (minimum id) to admin. \
                 Set BOOTSTRAP_ADMIN_USERNAME to pick a specific username on first bootstrap."
            );
        } else {
            log::info!("Bootstrap: assigned admin role to BOOTSTRAP_ADMIN_USERNAME");
        }
        Ok(())
    }

    pub async fn count_admins(&self) -> ResultType<i64> {
        let mut conn = self.pool.get().await?;
        let n: i64 = sqlx::query_scalar("select count(*) from users where role = 'admin'")
            .fetch_one(&mut *conn)
            .await?;
        Ok(n)
    }

    pub async fn set_user_role(&self, user_id: i64, role: &str) -> ResultType<()> {
        let role = if role == USER_ROLE_ADMIN {
            USER_ROLE_ADMIN
        } else {
            USER_ROLE_USER
        };
        let mut conn = self.pool.get().await?;
        sqlx::query("update users set role = ?, updated_at = current_timestamp where id = ?")
            .bind(role)
            .bind(user_id)
            .execute(&mut *conn)
            .await?;
        Ok(())
    }

    pub async fn get_peer(&self, id: &str) -> ResultType<Option<Peer>> {
        let mut conn = self.pool.get().await?;
        let row = sqlx::query(
            "select guid, id, uuid, pk, user, status, info, created_at, user_id, device_id from peer where id = ?",
        )
            .bind(id)
            .fetch_optional(&mut *conn)
            .await?;

        if let Some(row) = row {
            Ok(Some(Peer {
                guid: row.get("guid"),
                id: row.get("id"),
                uuid: row.get("uuid"),
                pk: row.get("pk"),
                user: row.get("user"),
                info: serde_json::from_str(row.get::<&str, _>("info")).unwrap_or_default(),
                status: row.get("status"),
                created_at: row.get("created_at"),
                bound_user_id: row.get::<Option<i64>, _>("user_id"),
                bound_device_row_id: row.get::<Option<i64>, _>("device_id"),
            }))
        } else {
            Ok(None)
        }
    }

    /// Resolve `user_devices.id` for rendezvous binding: JWT may carry explicit `udid`, else match RustDesk peer id string.
    pub async fn resolve_peer_device_binding(
        &self,
        user_id: i64,
        udid_from_claim: Option<i64>,
        rustdesk_peer_id: &str,
    ) -> ResultType<i64> {
        let mut conn = self.pool.get().await?;
        if let Some(udid) = udid_from_claim {
            let dev: Option<String> = sqlx::query_scalar(
                "select device_id from user_devices where id = ? and user_id = ? and is_active = 1",
            )
            .bind(udid)
            .bind(user_id)
            .fetch_optional(&mut *conn)
            .await?;
            let dev = dev.ok_or_else(|| {
                core_common::anyhow::anyhow!("JWT udid is not a valid device for this user")
            })?;
            if dev != rustdesk_peer_id {
                return Err(core_common::anyhow::anyhow!(
                    "JWT device does not match registering peer id"
                )
                .into());
            }
            return Ok(udid);
        }
        let row: Option<i64> = sqlx::query_scalar(
            "select id from user_devices where user_id = ? and device_id = ? and is_active = 1 limit 1",
        )
        .bind(user_id)
        .bind(rustdesk_peer_id)
        .fetch_optional(&mut *conn)
        .await?;
        row.ok_or_else(|| {
            core_common::anyhow::anyhow!(
                "Peer id is not registered as a device for this user (login with device_id or add device first)"
            )
            .into()
        })
    }

    /// For login: optional device row id embedded in JWT when client passes RustDesk `device_id` string.
    pub async fn get_user_device_row_id(
        &self,
        user_id: i64,
        device_id: &str,
    ) -> ResultType<Option<i64>> {
        let mut conn = self.pool.get().await?;
        let row: Option<i64> = sqlx::query_scalar(
            "select id from user_devices where user_id = ? and device_id = ? and is_active = 1 limit 1",
        )
        .bind(user_id)
        .bind(device_id)
        .fetch_optional(&mut *conn)
        .await?;
        Ok(row)
    }

    pub async fn insert_peer(
        &self,
        id: &str,
        uuid: &[u8],
        pk: &[u8],
        info: &str,
    ) -> ResultType<Vec<u8>> {
        let mut conn = self.pool.get().await?;
        let guid = Uuid::new_v4().as_bytes().to_vec();
        sqlx::query(
            "insert or replace into peer(guid, id, uuid, pk, info) values (?, ?, ?, ?, ?)",
        )
        .bind(&guid)
        .bind(id)
        .bind(uuid)
        .bind(pk)
        .bind(info)
        .execute(conn.deref_mut())
        .await?;
        Ok(guid)
    }

    pub async fn insert_peer_with_user(&self, id: &str, uuid: &[u8], pk: &[u8], info: &str, 
        user_id: Option<i64>, device_id: Option<i64>) -> ResultType<Vec<u8>> {
        let mut conn = self.pool.get().await?;
        let guid = Uuid::new_v4().as_bytes().to_vec();
        sqlx::query(
            "insert or replace into peer(guid, id, uuid, pk, info, user_id, device_id) values (?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&guid)
        .bind(id)
        .bind(uuid)
        .bind(pk)
        .bind(info)
        .bind(user_id)
        .bind(device_id)
        .execute(conn.deref_mut())
        .await?;
        Ok(guid)
    }

    pub async fn update_pk(&self, guid: &[u8], id: &str, pk: &[u8], info: &str) -> ResultType<()> {
        let mut conn = self.pool.get().await?;
        sqlx::query("update peer set pk=?, info=? where guid=? and id=?")
            .bind(pk)
            .bind(info)
            .bind(guid)
            .bind(id)
            .execute(conn.deref_mut())
            .await?;
        Ok(())
    }

    pub async fn update_pk_with_user(&self, guid: &[u8], id: &str, pk: &[u8], info: &str,
        user_id: Option<i64>, device_id: Option<i64>) -> ResultType<()> {
        let mut conn = self.pool.get().await?;
        sqlx::query(
            "update peer set pk=?, info=?, user_id=?, device_id=? where guid=? and id=?",
        )
        .bind(pk)
        .bind(info)
        .bind(user_id)
        .bind(device_id)
        .bind(guid)
        .bind(id)
        .execute(conn.deref_mut())
        .await?;
        Ok(())
    }

    // 用户管理方法
    pub async fn create_user(&self, request: &CreateUserRequest) -> ResultType<i64> {
        let password_hash = bcrypt::hash(&request.password, bcrypt::DEFAULT_COST)
            .map_err(|e| core_common::anyhow::anyhow!("Failed to hash password: {}", e))?;
        
        let mut conn = self.pool.get().await?;
        let result = sqlx::query(
            "insert into users (username, email, password_hash, role) values (?, ?, ?, ?)",
        )
            .bind(&request.username)
            .bind(&request.email)
            .bind(&password_hash)
            .bind(USER_ROLE_USER)
            .execute(&mut *conn)
            .await?;
        
        Ok(result.last_insert_rowid())
    }

    pub async fn get_user_by_id(&self, user_id: i64) -> ResultType<Option<User>> {
        let mut conn = self.pool.get().await?;
        let row = sqlx::query("select id, username, email, password_hash, created_at, updated_at, is_active, role from users where id = ?")
            .bind(user_id)
            .fetch_optional(&mut *conn)
            .await?;
        
        if let Some(row) = row {
            Ok(Some(User {
                id: row.get("id"),
                username: row.get("username"),
                email: row.get("email"),
                password_hash: row.get("password_hash"),
                created_at: row.get("created_at"),
                updated_at: row.get("updated_at"),
                is_active: row.get("is_active"),
                role: row.get::<String, _>("role"),
            }))
        } else {
            Ok(None)
        }
    }

    pub async fn get_user_by_username(&self, username: &str) -> ResultType<Option<User>> {
        let mut conn = self.pool.get().await?;
        let row = sqlx::query("select id, username, email, password_hash, created_at, updated_at, is_active, role from users where username = ?")
            .bind(username)
            .fetch_optional(&mut *conn)
            .await?;
        
        if let Some(row) = row {
            Ok(Some(User {
                id: row.get("id"),
                username: row.get("username"),
                email: row.get("email"),
                password_hash: row.get("password_hash"),
                created_at: row.get("created_at"),
                updated_at: row.get("updated_at"),
                is_active: row.get("is_active"),
                role: row.get::<String, _>("role"),
            }))
        } else {
            Ok(None)
        }
    }

    pub async fn get_user_by_email(&self, email: &str) -> ResultType<Option<User>> {
        let mut conn = self.pool.get().await?;
        let row = sqlx::query("select id, username, email, password_hash, created_at, updated_at, is_active, role from users where email = ?")
            .bind(email)
            .fetch_optional(&mut *conn)
            .await?;
        
        if let Some(row) = row {
            Ok(Some(User {
                id: row.get("id"),
                username: row.get("username"),
                email: row.get("email"),
                password_hash: row.get("password_hash"),
                created_at: row.get("created_at"),
                updated_at: row.get("updated_at"),
                is_active: row.get("is_active"),
                role: row.get::<String, _>("role"),
            }))
        } else {
            Ok(None)
        }
    }

    pub async fn update_user(&self, user_id: i64, username: Option<&str>, email: Option<&str>) -> ResultType<()> {
        let mut query = "update users set updated_at = current_timestamp".to_string();
        let mut params = Vec::new();
        
        if let Some(username) = username {
            query.push_str(", username = ?");
            params.push(username);
        }
        
        if let Some(email) = email {
            query.push_str(", email = ?");
            params.push(email);
        }
        
        query.push_str(" where id = ?");
        
        let mut q = sqlx::query(&query);
        for param in params {
            q = q.bind(param);
        }
        q = q.bind(user_id);
        
        q.execute(self.pool.get().await?.deref_mut()).await?;
        Ok(())
    }

    pub async fn delete_user(&self, user_id: i64) -> ResultType<()> {
        sqlx::query("delete from users where id = ?")
            .bind(user_id)
            .execute(self.pool.get().await?.deref_mut())
            .await?;
        Ok(())
    }

    pub async fn list_users(&self, limit: Option<i64>, offset: Option<i64>) -> ResultType<Vec<User>> {
        let limit = limit.unwrap_or(50);
        let offset = offset.unwrap_or(0);
        
        let mut conn = self.pool.get().await?;
        let users = sqlx::query("select id, username, email, password_hash, created_at, updated_at, is_active, role from users order by created_at desc limit ? offset ?")
            .bind(limit)
            .bind(offset)
            .fetch_all(&mut *conn)
            .await?;
        
        let mut user_list = Vec::new();
        for row in users {
            user_list.push(User {
                id: row.get("id"),
                username: row.get("username"),
                email: row.get("email"),
                password_hash: row.get("password_hash"),
                created_at: row.get("created_at"),
                updated_at: row.get("updated_at"),
                is_active: row.get("is_active"),
                role: row.get::<String, _>("role"),
            });
        }
        
        Ok(user_list)
    }

    // 设备管理方法
    pub async fn add_device_to_user(&self, request: &CreateDeviceRequest) -> ResultType<i64> {
        // 验证用户存在且活跃
        let user_exists: bool = sqlx::query_scalar("select count(*) from users where id = ? and is_active = 1")
            .bind(request.user_id)
            .fetch_one(self.pool.get().await?.deref_mut())
            .await
            .unwrap_or(0) > 0;
        
        if !user_exists {
            return Err(core_common::anyhow::anyhow!("用户不存在或已被禁用"));
        }
        
        // 检查用户设备数量限制
        let device_count: i64 = sqlx::query_scalar("select count(*) from user_devices where user_id = ? and is_active = 1")
            .bind(request.user_id)
            .fetch_one(self.pool.get().await?.deref_mut())
            .await
            .unwrap_or(0);
        
        if device_count >= 10 {
            return Err(core_common::anyhow::anyhow!("用户设备数量已达到上限（10个）"));
        }
        
        // 检查设备ID是否已存在（同一用户下不能重复）
        let device_exists: bool = sqlx::query_scalar("select count(*) from user_devices where user_id = ? and device_id = ? and is_active = 1")
            .bind(request.user_id)
            .bind(&request.device_id)
            .fetch_one(self.pool.get().await?.deref_mut())
            .await
            .unwrap_or(0) > 0;
        
        if device_exists {
            return Err(core_common::anyhow::anyhow!("设备ID已存在"));
        }
        
        let result = sqlx::query("insert or replace into user_devices (user_id, device_id, device_name) values (?, ?, ?)")
            .bind(request.user_id)
            .bind(&request.device_id)
            .bind(&request.device_name)
            .execute(self.pool.get().await?.deref_mut())
            .await?;
        
        Ok(result.last_insert_rowid())
    }
    
    /// 获取用户的所有活跃设备
    pub async fn get_user_devices(&self, user_id: i64) -> ResultType<Vec<UserDevice>> {
        let mut conn = self.pool.get().await?;
        let devices = sqlx::query("select id, user_id, device_id, device_name, created_at, is_active from user_devices where user_id = ? and is_active = 1 order by created_at desc")
            .bind(user_id)
            .fetch_all(&mut *conn)
            .await?;
        
        let mut device_list = Vec::new();
        for row in devices {
            device_list.push(UserDevice {
                id: row.get("id"),
                user_id: row.get("user_id"),
                device_id: row.get("device_id"),
                device_name: row.get("device_name"),
                created_at: row.get("created_at"),
                is_active: row.get("is_active"),
            });
        }
        
        Ok(device_list)
    }
    
    /// 验证设备是否属于指定用户
    pub async fn validate_device_ownership(&self, user_id: i64, device_id: &str) -> ResultType<bool> {
        let count: i64 = sqlx::query_scalar("select count(*) from user_devices where user_id = ? and device_id = ? and is_active = 1")
            .bind(user_id)
            .bind(device_id)
            .fetch_one(self.pool.get().await?.deref_mut())
            .await
            .unwrap_or(0);
        
        Ok(count > 0)
    }

    pub async fn remove_device_from_user(&self, user_id: i64, device_id: &str) -> ResultType<()> {
        sqlx::query("update user_devices set is_active = 0, deleted_at = current_timestamp where user_id = ? and device_id = ?")
            .bind(user_id)
            .bind(device_id)
            .execute(self.pool.get().await?.deref_mut())
            .await?;
        Ok(())
    }

    pub async fn get_device_owner(&self, device_id: &str) -> ResultType<Option<User>> {
        let mut conn = self.pool.get().await?;
        let user = sqlx::query("select u.id, u.username, u.email, u.password_hash, u.created_at, u.updated_at, u.is_active, u.role from users u join user_devices ud on u.id = ud.user_id where ud.device_id = ? and ud.is_active = 1 and u.is_active = 1")
            .bind(device_id)
            .fetch_optional(&mut *conn)
            .await?;
        
        if let Some(row) = user {
            Ok(Some(User {
                id: row.get("id"),
                username: row.get("username"),
                email: row.get("email"),
                password_hash: row.get("password_hash"),
                created_at: row.get("created_at"),
                updated_at: row.get("updated_at"),
                is_active: row.get("is_active"),
                role: row.get::<String, _>("role"),
            }))
        } else {
            Ok(None)
        }
    }

    pub async fn verify_password(&self, password: &str, hash: &str) -> ResultType<bool> {
        Ok(bcrypt::verify(password, hash).unwrap_or(false))
    }

    // 密码重置相关方法
    pub async fn create_password_reset_token(&self, user_id: i64) -> ResultType<String> {
        use uuid::Uuid;
        
        // 生成唯一的重置令牌
        let token = Uuid::new_v4().to_string();
        let expires_at = chrono::Utc::now() + chrono::Duration::hours(1); // 1小时后过期
        
        let mut conn = self.pool.get().await?;
        sqlx::query("insert into password_reset_tokens (user_id, token, expires_at) values (?, ?, ?)")
            .bind(user_id)
            .bind(&token)
            .bind(expires_at)
            .execute(&mut *conn)
            .await?;
        
        Ok(token)
    }

    pub async fn validate_password_reset_token(&self, token: &str) -> ResultType<Option<i64>> {
        let mut conn = self.pool.get().await?;
        let row = sqlx::query("select user_id from password_reset_tokens where token = ? and expires_at > datetime('now') and is_used = 0")
            .bind(token)
            .fetch_optional(&mut *conn)
            .await?;
        
        if let Some(row) = row {
            Ok(Some(row.get("user_id")))
        } else {
            Ok(None)
        }
    }

    pub async fn reset_password(&self, user_id: i64, new_password: &str) -> ResultType<()> {
        let password_hash = bcrypt::hash(new_password, bcrypt::DEFAULT_COST)
            .map_err(|e| core_common::anyhow::anyhow!("Failed to hash password: {}", e))?;
        
        let mut conn = self.pool.get().await?;
        
        // 开始事务
        let mut tx = conn.begin().await?;
        
        // 更新密码
        sqlx::query("update users set password_hash = ?, updated_at = current_timestamp where id = ?")
            .bind(&password_hash)
            .bind(user_id)
            .execute(&mut *tx)
            .await?;
        
        // 标记该用户的所有重置令牌为已使用
        sqlx::query("update password_reset_tokens set is_used = 1 where user_id = ? and is_used = 0")
            .bind(user_id)
            .execute(&mut *tx)
            .await?;
        
        // 提交事务
        tx.commit().await?;
        
        Ok(())
    }

    pub async fn update_password(&self, user_id: i64, old_password: &str, new_password: &str) -> ResultType<()> {
        // 首先验证旧密码
        match self.get_user_by_id(user_id).await? {
            Some(user) => {
                if !bcrypt::verify(old_password, &user.password_hash).unwrap_or(false) {
                    return Err(core_common::anyhow::anyhow!("旧密码不正确"));
                }
            }
            None => {
                return Err(core_common::anyhow::anyhow!("用户不存在"));
            }
        }
        
        // 更新密码
        let password_hash = bcrypt::hash(new_password, bcrypt::DEFAULT_COST)
            .map_err(|e| core_common::anyhow::anyhow!("Failed to hash password: {}", e))?;
        
        let mut conn = self.pool.get().await?;
        sqlx::query("update users set password_hash = ?, updated_at = current_timestamp where id = ?")
            .bind(&password_hash)
            .bind(user_id)
            .execute(&mut *conn)
            .await?;
        
        Ok(())
    }

    pub async fn change_password(&self, user_id: i64, new_password: &str) -> ResultType<()> {
        let password_hash = bcrypt::hash(new_password, bcrypt::DEFAULT_COST)
            .map_err(|e| core_common::anyhow::anyhow!("Failed to hash password: {}", e))?;
        
        let mut conn = self.pool.get().await?;
        sqlx::query("update users set password_hash = ?, updated_at = current_timestamp where id = ?")
            .bind(&password_hash)
            .bind(user_id)
            .execute(&mut *conn)
            .await?;
        
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use core_common::tokio;
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
        core_common::futures::future::join_all(jobs).await;
    }
}
