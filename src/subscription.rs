use crate::database::Database;
use chrono::{DateTime, Utc};
use core_common::ResultType;
use serde::{Deserialize, Serialize};
use sqlx::Row;

pub const FREE_DEVICE_LIMIT: i64 = 2;
pub const FREE_SPEED_KBPS: i64 = 1024;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubscriptionPlan {
    pub id: i64,
    pub name: String,
    pub display_name: String,
    pub device_limit: i64,     // 0 = 不限
    pub speed_limit_kbps: i64, // 0 = 不限
    pub price_monthly: f64,
    pub description: String,
    pub is_active: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserSubscription {
    pub id: i64,
    pub user_id: i64,
    pub plan_id: i64,
    pub plan_name: String,
    pub plan_display_name: String,
    pub device_limit: i64,
    pub speed_limit_kbps: i64,
    pub started_at: DateTime<Utc>,
    pub expires_at: Option<DateTime<Utc>>,
    pub is_active: bool,
    pub notes: Option<String>,
    pub created_at: DateTime<Utc>,
}

impl Database {
    pub async fn migrate_subscriptions(&self) -> ResultType<()> {
        let mut conn = self.pool.get().await?;

        sqlx::query(
            "CREATE TABLE IF NOT EXISTS subscription_plans (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                name TEXT UNIQUE NOT NULL,
                display_name TEXT NOT NULL,
                device_limit INTEGER NOT NULL DEFAULT 2,
                speed_limit_kbps INTEGER NOT NULL DEFAULT 1024,
                price_monthly REAL NOT NULL DEFAULT 0,
                description TEXT NOT NULL DEFAULT '',
                is_active BOOLEAN NOT NULL DEFAULT 1
            );",
        )
        .execute(&mut *conn)
        .await?;

        sqlx::query(
            "CREATE TABLE IF NOT EXISTS user_subscriptions (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                user_id INTEGER NOT NULL,
                plan_id INTEGER NOT NULL,
                started_at DATETIME NOT NULL DEFAULT (current_timestamp),
                expires_at DATETIME,
                is_active BOOLEAN NOT NULL DEFAULT 1,
                notes TEXT,
                created_at DATETIME NOT NULL DEFAULT (current_timestamp),
                updated_at DATETIME NOT NULL DEFAULT (current_timestamp),
                FOREIGN KEY (user_id) REFERENCES users(id) ON DELETE CASCADE,
                FOREIGN KEY (plan_id) REFERENCES subscription_plans(id)
            );",
        )
        .execute(&mut *conn)
        .await?;

        for sql in [
            "CREATE INDEX IF NOT EXISTS idx_user_subscriptions_user_id ON user_subscriptions(user_id)",
            "CREATE INDEX IF NOT EXISTS idx_user_subscriptions_active ON user_subscriptions(is_active)",
        ] {
            if let Err(e) = sqlx::query(sql).execute(&mut *conn).await {
                core_common::log::warn!("subscription index migration `{}`: {}", sql, e);
            }
        }

        // 种子数据
        sqlx::query(
            "INSERT OR IGNORE INTO subscription_plans(name,display_name,device_limit,speed_limit_kbps,price_monthly,description) VALUES
            ('free','免费版',2,1024,0,'最多2台设备，限速1Mbps'),
            ('basic','基础版',5,5120,9.9,'最多5台设备，限速5Mbps'),
            ('pro','专业版',20,0,29.9,'最多20台设备，不限速'),
            ('enterprise','企业版',0,0,99.9,'无限设备，不限速，优先支持')",
        )
        .execute(&mut *conn)
        .await?;

        Ok(())
    }

    pub async fn get_subscription_plans(&self) -> ResultType<Vec<SubscriptionPlan>> {
        let mut conn = self.pool.get().await?;
        let rows = sqlx::query(
            "SELECT id, name, display_name, device_limit, speed_limit_kbps, price_monthly, description, is_active
             FROM subscription_plans ORDER BY price_monthly ASC",
        )
        .fetch_all(&mut *conn)
        .await?;

        let mut plans = Vec::new();
        for row in rows {
            plans.push(SubscriptionPlan {
                id: row.get("id"),
                name: row.get("name"),
                display_name: row.get("display_name"),
                device_limit: row.get("device_limit"),
                speed_limit_kbps: row.get("speed_limit_kbps"),
                price_monthly: row.get("price_monthly"),
                description: row.get("description"),
                is_active: row.get("is_active"),
            });
        }
        Ok(plans)
    }

    pub async fn get_user_active_subscription(
        &self,
        user_id: i64,
    ) -> ResultType<Option<UserSubscription>> {
        let mut conn = self.pool.get().await?;
        let row = sqlx::query(
            "SELECT us.id, us.user_id, us.plan_id, sp.name as plan_name, sp.display_name as plan_display_name,
                    sp.device_limit, sp.speed_limit_kbps, us.started_at, us.expires_at,
                    us.is_active, us.notes, us.created_at
             FROM user_subscriptions us
             JOIN subscription_plans sp ON us.plan_id = sp.id
             WHERE us.user_id = ? AND us.is_active = 1
               AND (us.expires_at IS NULL OR us.expires_at > datetime('now'))
             ORDER BY us.created_at DESC
             LIMIT 1",
        )
        .bind(user_id)
        .fetch_optional(&mut *conn)
        .await?;

        if let Some(row) = row {
            Ok(Some(UserSubscription {
                id: row.get("id"),
                user_id: row.get("user_id"),
                plan_id: row.get("plan_id"),
                plan_name: row.get("plan_name"),
                plan_display_name: row.get("plan_display_name"),
                device_limit: row.get("device_limit"),
                speed_limit_kbps: row.get("speed_limit_kbps"),
                started_at: row.get("started_at"),
                expires_at: row.get("expires_at"),
                is_active: row.get("is_active"),
                notes: row.get("notes"),
                created_at: row.get("created_at"),
            }))
        } else {
            Ok(None)
        }
    }

    pub async fn get_user_device_limit(&self, user_id: i64) -> ResultType<i64> {
        match self.get_user_active_subscription(user_id).await? {
            Some(sub) => Ok(sub.device_limit),
            None => Ok(FREE_DEVICE_LIMIT),
        }
    }

    pub async fn get_user_speed_limit(&self, user_id: i64) -> ResultType<i64> {
        match self.get_user_active_subscription(user_id).await? {
            Some(sub) => Ok(sub.speed_limit_kbps),
            None => Ok(FREE_SPEED_KBPS),
        }
    }

    pub async fn create_user_subscription(
        &self,
        user_id: i64,
        plan_id: i64,
        expires_at: Option<DateTime<Utc>>,
        notes: Option<String>,
    ) -> ResultType<i64> {
        let mut conn = self.pool.get().await?;

        // 先把该用户旧订阅 is_active 设为 0
        sqlx::query(
            "UPDATE user_subscriptions SET is_active = 0, updated_at = current_timestamp WHERE user_id = ? AND is_active = 1",
        )
        .bind(user_id)
        .execute(&mut *conn)
        .await?;

        // 插入新订阅
        let result = sqlx::query(
            "INSERT INTO user_subscriptions (user_id, plan_id, expires_at, notes) VALUES (?, ?, ?, ?)",
        )
        .bind(user_id)
        .bind(plan_id)
        .bind(expires_at)
        .bind(notes)
        .execute(&mut *conn)
        .await?;

        Ok(result.last_insert_rowid())
    }

    pub async fn update_user_subscription(
        &self,
        sub_id: i64,
        plan_id: Option<i64>,
        expires_at: Option<Option<DateTime<Utc>>>,
        notes: Option<String>,
    ) -> ResultType<()> {
        let mut conn = self.pool.get().await?;

        let has_plan = plan_id.is_some();
        let has_expires = expires_at.is_some();
        let has_notes = notes.is_some();

        if !has_plan && !has_expires && !has_notes {
            return Ok(());
        }

        let mut sql = String::from("UPDATE user_subscriptions SET updated_at = current_timestamp");
        if has_plan {
            sql.push_str(", plan_id = ?");
        }
        if has_expires {
            sql.push_str(", expires_at = ?");
        }
        if has_notes {
            sql.push_str(", notes = ?");
        }
        sql.push_str(" WHERE id = ?");

        let mut q = sqlx::query(&sql);
        if let Some(pid) = plan_id {
            q = q.bind(pid);
        }
        if let Some(exp) = expires_at {
            q = q.bind(exp);
        }
        if let Some(n) = notes {
            q = q.bind(n);
        }
        q = q.bind(sub_id);

        q.execute(&mut *conn).await?;
        Ok(())
    }

    pub async fn deactivate_user_subscription(&self, sub_id: i64) -> ResultType<()> {
        let mut conn = self.pool.get().await?;
        sqlx::query(
            "UPDATE user_subscriptions SET is_active = 0, updated_at = current_timestamp WHERE id = ?",
        )
        .bind(sub_id)
        .execute(&mut *conn)
        .await?;
        Ok(())
    }

    pub async fn list_subscriptions(
        &self,
        limit: Option<i64>,
        offset: Option<i64>,
    ) -> ResultType<Vec<UserSubscription>> {
        let limit = limit.unwrap_or(50);
        let offset = offset.unwrap_or(0);

        let mut conn = self.pool.get().await?;
        let rows = sqlx::query(
            "SELECT us.id, us.user_id, us.plan_id, sp.name as plan_name, sp.display_name as plan_display_name,
                    sp.device_limit, sp.speed_limit_kbps, us.started_at, us.expires_at,
                    us.is_active, us.notes, us.created_at
             FROM user_subscriptions us
             JOIN subscription_plans sp ON us.plan_id = sp.id
             ORDER BY us.created_at DESC
             LIMIT ? OFFSET ?",
        )
        .bind(limit)
        .bind(offset)
        .fetch_all(&mut *conn)
        .await?;

        let mut subs = Vec::new();
        for row in rows {
            subs.push(UserSubscription {
                id: row.get("id"),
                user_id: row.get("user_id"),
                plan_id: row.get("plan_id"),
                plan_name: row.get("plan_name"),
                plan_display_name: row.get("plan_display_name"),
                device_limit: row.get("device_limit"),
                speed_limit_kbps: row.get("speed_limit_kbps"),
                started_at: row.get("started_at"),
                expires_at: row.get("expires_at"),
                is_active: row.get("is_active"),
                notes: row.get("notes"),
                created_at: row.get("created_at"),
            });
        }
        Ok(subs)
    }

    pub async fn get_plan_id_by_name(&self, plan_name: &str) -> ResultType<Option<i64>> {
        let mut conn = self.pool.get().await?;
        let id: Option<i64> =
            sqlx::query_scalar("SELECT id FROM subscription_plans WHERE name = ?")
                .bind(plan_name)
                .fetch_optional(&mut *conn)
                .await?;
        Ok(id)
    }

    pub async fn count_user_active_devices(&self, user_id: i64) -> ResultType<i64> {
        let mut conn = self.pool.get().await?;
        let count: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM user_devices WHERE user_id = ? AND is_active = 1",
        )
        .bind(user_id)
        .fetch_one(&mut *conn)
        .await
        .unwrap_or(0);
        Ok(count)
    }
}
