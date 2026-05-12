use crate::database::{Database, CreateUserRequest, CreateDeviceRequest, User, UserDevice};
use core_common::{log, ResultType};
use sqlx::{Row, SqlitePool};

impl Database {
    // 使用动态查询避免编译时检查问题
    pub async fn create_user_v2(&self, request: &CreateUserRequest) -> ResultType<i64> {
        let password_hash = bcrypt::hash(&request.password, bcrypt::DEFAULT_COST)
            .map_err(|e| core_common::anyhow::anyhow!("Failed to hash password: {}", e))?;
        
        let pool = self.pool.get().await?;
        let result = sqlx::query("insert into users (username, email, password_hash) values (?, ?, ?)")
            .bind(&request.username)
            .bind(&request.email)
            .bind(&password_hash)
            .execute(&*pool)
            .await?;
        
        Ok(result.last_insert_rowid())
    }

    pub async fn get_user_by_id_v2(&self, user_id: i64) -> ResultType<Option<User>> {
        let pool = self.pool.get().await?;
        let row = sqlx::query("select id, username, email, password_hash, created_at, updated_at, is_active from users where id = ?")
            .bind(user_id)
            .fetch_optional(&*pool)
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

    pub async fn get_user_by_username_v2(&self, username: &str) -> ResultType<Option<User>> {
        let pool = self.pool.get().await?;
        let row = sqlx::query("select id, username, email, password_hash, created_at, updated_at, is_active from users where username = ?")
            .bind(username)
            .fetch_optional(&*pool)
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

    pub async fn get_user_by_email_v2(&self, email: &str) -> ResultType<Option<User>> {
        let pool = self.pool.get().await?;
        let row = sqlx::query("select id, username, email, password_hash, created_at, updated_at, is_active from users where email = ?")
            .bind(email)
            .fetch_optional(&*pool)
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

    pub async fn list_users_v2(&self, limit: Option<i64>, offset: Option<i64>) -> ResultType<Vec<User>> {
        let limit = limit.unwrap_or(50);
        let offset = offset.unwrap_or(0);
        
        let pool = self.pool.get().await?;
        let rows = sqlx::query("select id, username, email, password_hash, created_at, updated_at, is_active from users order by created_at desc limit ? offset ?")
            .bind(limit)
            .bind(offset)
            .fetch_all(&*pool)
            .await?;
        
        let mut users = Vec::new();
        for row in rows {
            users.push(User {
                id: row.get("id"),
                username: row.get("username"),
                email: row.get("email"),
                password_hash: row.get("password_hash"),
                created_at: row.get("created_at"),
                updated_at: row.get("updated_at"),
                is_active: row.get("is_active"),
            });
        }
        
        Ok(users)
    }

    pub async fn delete_user_v2(&self, user_id: i64) -> ResultType<()> {
        let pool = self.pool.get().await?;
        sqlx::query("delete from users where id = ?")
            .bind(user_id)
            .execute(&*pool)
            .await?;
        Ok(())
    }

    pub async fn add_device_to_user_v2(&self, request: &CreateDeviceRequest) -> ResultType<i64> {
        // 检查用户设备数量限制
        let pool = self.pool.get().await?;
        let device_count: i64 = sqlx::query_scalar("select count(*) from user_devices where user_id = ? and is_active = 1")
            .bind(request.user_id)
            .fetch_one(&*pool)
            .await
            .unwrap_or(0);
        
        if device_count >= 10 {
            return Err(core_common::anyhow::anyhow!("用户设备数量已达到上限（10个）"));
        }
        
        let result = sqlx::query("insert or replace into user_devices (user_id, device_id, device_name) values (?, ?, ?)")
            .bind(request.user_id)
            .bind(&request.device_id)
            .bind(&request.device_name)
            .execute(&*pool)
            .await?;
        
        Ok(result.last_insert_rowid())
    }

    pub async fn get_user_devices_v2(&self, user_id: i64) -> ResultType<Vec<UserDevice>> {
        let pool = self.pool.get().await?;
        let rows = sqlx::query("select id, user_id, device_id, device_name, created_at, is_active from user_devices where user_id = ? and is_active = 1 order by created_at desc")
            .bind(user_id)
            .fetch_all(&*pool)
            .await?;
        
        let mut devices = Vec::new();
        for row in rows {
            devices.push(UserDevice {
                id: row.get("id"),
                user_id: row.get("user_id"),
                device_id: row.get("device_id"),
                device_name: row.get("device_name"),
                created_at: row.get("created_at"),
                is_active: row.get("is_active"),
            });
        }
        
        Ok(devices)
    }

    pub async fn remove_device_from_user_v2(&self, user_id: i64, device_id: &str) -> ResultType<()> {
        let pool = self.pool.get().await?;
        sqlx::query("update user_devices set is_active = 0 where user_id = ? and device_id = ?")
            .bind(user_id)
            .bind(device_id)
            .execute(&*pool)
            .await?;
        Ok(())
    }

    pub async fn get_device_owner_v2(&self, device_id: &str) -> ResultType<Option<User>> {
        let pool = self.pool.get().await?;
        let row = sqlx::query("select u.id, u.username, u.email, u.password_hash, u.created_at, u.updated_at, u.is_active from users u join user_devices ud on u.id = ud.user_id where ud.device_id = ? and ud.is_active = 1 and u.is_active = 1")
            .bind(device_id)
            .fetch_optional(&*pool)
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
}
