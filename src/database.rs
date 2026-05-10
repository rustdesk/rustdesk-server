use async_trait::async_trait;
use hbb_common::{log, ResultType};
use sqlx::{
    sqlite::SqliteConnectOptions, ConnectOptions, Connection, Error as SqlxError, SqliteConnection, Row,
};
use std::{ops::DerefMut, str::FromStr};
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
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct User {
    pub id: i64,
    pub username: String,
    pub email: String,
    pub password_hash: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
    pub is_active: bool,
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
        
        // 创建 peer 表（原有功能）
        sqlx::query("
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
        ")
        .execute(&mut *conn)
        .await?;

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

        Ok(())
    }

    pub async fn get_peer(&self, id: &str) -> ResultType<Option<Peer>> {
        let mut conn = self.pool.get().await?;
        let row = sqlx::query("select guid, id, uuid, pk, user, status, info from peer where id = ?")
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
            }))
        } else {
            Ok(None)
        }
    }

    pub async fn insert_peer(
        &self,
        id: &str,
        uuid: &[u8],
        pk: &[u8],
        info: &str,
    ) -> ResultType<Vec<u8>> {
        let guid = uuid::Uuid::new_v4().as_bytes().to_vec();
        let mut conn = self.pool.get().await?;
        sqlx::query("insert into peer(guid, id, uuid, pk, info) values(?, ?, ?, ?, ?)")
            .bind(&guid)
            .bind(id)
            .bind(uuid)
            .bind(pk)
            .bind(info)
            .execute(&mut *conn)
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
        let mut conn = self.pool.get().await?;
        sqlx::query("update peer set id=?, pk=?, info=? where guid=?")
            .bind(id)
            .bind(pk)
            .bind(info)
            .bind(guid)
            .execute(&mut *conn)
            .await?;
        
        log::info!("pk updated instead of insert");
        Ok(())
    }

    // 用户管理方法
    pub async fn create_user(&self, request: &CreateUserRequest) -> ResultType<i64> {
        let password_hash = bcrypt::hash(&request.password, bcrypt::DEFAULT_COST)
            .map_err(|e| hbb_common::anyhow::anyhow!("Failed to hash password: {}", e))?;
        
        let mut conn = self.pool.get().await?;
        let result = sqlx::query("insert into users (username, email, password_hash) values (?, ?, ?)")
            .bind(&request.username)
            .bind(&request.email)
            .bind(&password_hash)
            .execute(&mut *conn)
            .await?;
        
        Ok(result.last_insert_rowid())
    }

    pub async fn get_user_by_id(&self, user_id: i64) -> ResultType<Option<User>> {
        let mut conn = self.pool.get().await?;
        let row = sqlx::query("select id, username, email, password_hash, created_at, updated_at, is_active from users where id = ?")
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
            }))
        } else {
            Ok(None)
        }
    }

    pub async fn get_user_by_username(&self, username: &str) -> ResultType<Option<User>> {
        let mut conn = self.pool.get().await?;
        let row = sqlx::query("select id, username, email, password_hash, created_at, updated_at, is_active from users where username = ?")
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
            }))
        } else {
            Ok(None)
        }
    }

    pub async fn get_user_by_email(&self, email: &str) -> ResultType<Option<User>> {
        let mut conn = self.pool.get().await?;
        let row = sqlx::query("select id, username, email, password_hash, created_at, updated_at, is_active from users where email = ?")
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
        let users = sqlx::query("select id, username, email, password_hash, created_at, updated_at, is_active from users order by created_at desc limit ? offset ?")
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
            });
        }
        
        Ok(user_list)
    }

    // 设备管理方法
    pub async fn add_device_to_user(&self, request: &CreateDeviceRequest) -> ResultType<i64> {
        // 检查用户设备数量限制
        let device_count: i64 = sqlx::query_scalar("select count(*) from user_devices where user_id = ? and is_active = 1")
            .bind(request.user_id)
            .fetch_one(self.pool.get().await?.deref_mut())
            .await
            .unwrap_or(0);
        
        if device_count >= 10 {
            return Err(hbb_common::anyhow::anyhow!("用户设备数量已达到上限（10个）"));
        }
        
        let result = sqlx::query("insert or replace into user_devices (user_id, device_id, device_name) values (?, ?, ?)")
            .bind(request.user_id)
            .bind(&request.device_id)
            .bind(&request.device_name)
            .execute(self.pool.get().await?.deref_mut())
            .await?;
        
        Ok(result.last_insert_rowid())
    }

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

    pub async fn remove_device_from_user(&self, user_id: i64, device_id: &str) -> ResultType<()> {
        sqlx::query("update user_devices set is_active = 0 where user_id = ? and device_id = ?")
            .bind(user_id)
            .bind(device_id)
            .execute(self.pool.get().await?.deref_mut())
            .await?;
        Ok(())
    }

    pub async fn get_device_owner(&self, device_id: &str) -> ResultType<Option<User>> {
        let mut conn = self.pool.get().await?;
        let user = sqlx::query("select u.id, u.username, u.email, u.password_hash, u.created_at, u.updated_at, u.is_active from users u join user_devices ud on u.id = ud.user_id where ud.device_id = ? and ud.is_active = 1 and u.is_active = 1")
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
            }))
        } else {
            Ok(None)
        }
    }

    pub async fn verify_password(&self, password: &str, hash: &str) -> ResultType<bool> {
        Ok(bcrypt::verify(password, hash).unwrap_or(false))
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
