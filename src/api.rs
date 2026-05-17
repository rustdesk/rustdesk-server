use crate::database::{
    CreateUserRequest, Database, User, UserDevice, USER_ROLE_ADMIN, USER_ROLE_USER,
};
use crate::subscription::{SubscriptionPlan, UserSubscription, FREE_DEVICE_LIMIT, FREE_SPEED_KBPS};
use axum::{
    extract::{Extension, Path, Query},
    http::{header, HeaderMap, StatusCode},
    response::{IntoResponse, Json},
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Clone)]
pub struct ApiState {
    pub db: Database,
    pub jwt_secret: String,
}

/// Resolve `users.id` from `Authorization: Bearer <jwt>` (same secret as login).
pub(crate) fn jwt_user_id_from_headers(
    jwt_secret: &str,
    headers: &HeaderMap,
) -> Result<i64, StatusCode> {
    let token = extract_bearer_token(headers)?;
    jwt_sub_user_id(jwt_secret, token).map_err(|_| StatusCode::UNAUTHORIZED)
}

fn extract_bearer_token(headers: &HeaderMap) -> Result<&str, StatusCode> {
    let raw = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .ok_or(StatusCode::UNAUTHORIZED)?;
    let token = if let Some(rest) = raw.strip_prefix("Bearer ") {
        rest.trim()
    } else {
        raw.trim()
    };
    if token.is_empty() {
        return Err(StatusCode::UNAUTHORIZED);
    }
    Ok(token)
}

fn decode_claims(secret: &str, token: &str) -> Result<Claims, ()> {
    use jsonwebtoken::{decode, DecodingKey, Validation};
    let td = decode::<Claims>(
        token,
        &DecodingKey::from_secret(secret.as_ref()),
        &Validation::default(),
    )
    .map_err(|_| ())?;
    Ok(td.claims)
}

fn jwt_sub_user_id(secret: &str, token: &str) -> Result<i64, ()> {
    let claims = decode_claims(secret, token)?;
    claims.sub.trim().parse::<i64>().map_err(|_| ())
}

pub(crate) async fn db_user_is_admin(db: &Database, user_id: i64) -> bool {
    match db.get_user_by_id(user_id).await {
        Ok(Some(u)) => u.role == USER_ROLE_ADMIN,
        _ => false,
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ApiResponse<T> {
    pub success: bool,
    pub data: Option<T>,
    pub message: String,
}

impl<T> ApiResponse<T> {
    pub fn success(data: T) -> Self {
        Self {
            success: true,
            data: Some(data),
            message: "操作成功".to_string(),
        }
    }

    pub fn error(message: String) -> Self {
        Self {
            success: false,
            data: None,
            message,
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct LoginRequest {
    pub username: String,
    pub password: String,
    /// Optional RustDesk peer id / `user_devices.device_id` to embed `udid` (row id) in JWT for rendezvous binding.
    #[serde(default)]
    pub device_id: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct RegisterRequest {
    pub username: String,
    pub email: String,
    pub password: String,
    pub confirm_password: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct PasswordResetRequest {
    pub email: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct PasswordResetConfirmRequest {
    pub token: String,
    pub new_password: String,
    pub confirm_password: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ChangePasswordRequest {
    pub old_password: String,
    pub new_password: String,
    pub confirm_password: String,
}

#[derive(Debug, Serialize)]
pub struct SubscriptionPlanInfo {
    pub id: i64,
    pub name: String,
    pub display_name: String,
    pub device_limit: i64,
    pub speed_limit_kbps: i64,
    pub price_monthly: f64,
    pub description: String,
}

impl From<SubscriptionPlan> for SubscriptionPlanInfo {
    fn from(p: SubscriptionPlan) -> Self {
        Self {
            id: p.id,
            name: p.name,
            display_name: p.display_name,
            device_limit: p.device_limit,
            speed_limit_kbps: p.speed_limit_kbps,
            price_monthly: p.price_monthly,
            description: p.description,
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct UserSubscriptionInfo {
    pub id: Option<i64>,
    pub plan_name: String,
    pub plan_display_name: String,
    pub device_limit: i64,
    pub speed_limit_kbps: i64,
    pub started_at: Option<String>,
    pub expires_at: Option<String>,
    pub is_active: bool,
    pub current_device_count: i64,
    pub notes: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct LoginResponse {
    pub token: String,
    pub user: UserInfo,
    pub subscription: UserSubscriptionInfo,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct UserInfo {
    pub id: i64,
    pub username: String,
    pub email: String,
    pub created_at: String,
    pub is_active: bool,
    /// `admin` or `user`
    pub role: String,
}

impl From<User> for UserInfo {
    fn from(user: User) -> Self {
        Self {
            id: user.id,
            username: user.username,
            email: user.email,
            created_at: user.created_at.to_rfc3339(),
            is_active: user.is_active,
            role: user.role,
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Claims {
    pub sub: String, // user id
    pub username: String,
    pub exp: i64,
    pub iat: i64,
    /// `user_devices.id` when client selected a device at login (optional).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub udid: Option<i64>,
    #[serde(default = "default_claim_role")]
    pub role: String,
}

fn default_claim_role() -> String {
    USER_ROLE_USER.to_string()
}

#[derive(Debug, Deserialize)]
pub struct ListQuery {
    pub limit: Option<i64>,
    pub offset: Option<i64>,
}

#[derive(Debug, Serialize)]
pub struct DeviceInfo {
    pub id: i64,
    pub device_id: String,
    pub device_name: Option<String>,
    pub created_at: String,
    pub is_active: bool,
}

impl From<UserDevice> for DeviceInfo {
    fn from(device: UserDevice) -> Self {
        Self {
            id: device.id,
            device_id: device.device_id,
            device_name: device.device_name,
            created_at: device.created_at.to_rfc3339(),
            is_active: device.is_active,
        }
    }
}


pub async fn list_users(
    Extension(state): Extension<ApiState>,
    headers: HeaderMap,
    Query(query): Query<ListQuery>,
) -> Result<Json<ApiResponse<Vec<UserInfo>>>, StatusCode> {
    let caller = jwt_user_id_from_headers(&state.jwt_secret, &headers)?;
    if !db_user_is_admin(&state.db, caller).await {
        return Err(StatusCode::FORBIDDEN);
    }
    match state.db.list_users_simple(query.limit, query.offset).await {
        Ok(users) => {
            let user_infos: Vec<UserInfo> = users.into_iter().map(|u| u.into()).collect();
            Ok(Json(ApiResponse::success(user_infos)))
        }
        Err(_) => Ok(Json(ApiResponse::error("获取用户列表失败".to_string()))),
    }
}

pub async fn get_user(
    Extension(state): Extension<ApiState>,
    headers: HeaderMap,
    Path(user_id): Path<i64>,
) -> Result<Json<ApiResponse<UserInfo>>, StatusCode> {
    let caller = jwt_user_id_from_headers(&state.jwt_secret, &headers)?;
    if caller != user_id && !db_user_is_admin(&state.db, caller).await {
        return Err(StatusCode::FORBIDDEN);
    }
    match state.db.get_user_by_id(user_id).await {
        Ok(Some(user)) => Ok(Json(ApiResponse::success(user.into()))),
        Ok(None) => Ok(Json(ApiResponse::error("用户不存在".to_string()))),
        Err(_) => Ok(Json(ApiResponse::error("数据库错误".to_string()))),
    }
}

pub async fn update_user(
    Extension(state): Extension<ApiState>,
    headers: HeaderMap,
    Path(user_id): Path<i64>,
    Json(mut request): Json<HashMap<String, String>>,
) -> Result<Json<ApiResponse<UserInfo>>, StatusCode> {
    let caller = jwt_user_id_from_headers(&state.jwt_secret, &headers)?;
    if caller != user_id && !db_user_is_admin(&state.db, caller).await {
        return Err(StatusCode::FORBIDDEN);
    }
    let username = request.remove("username");
    let email = request.remove("email");

    // 验证邮箱格式
    if let Some(ref email) = email {
        if !email.contains('@') {
            return Ok(Json(ApiResponse::error("邮箱格式不正确".to_string())));
        }
    }

    match state
        .db
        .update_user(user_id, username.as_deref(), email.as_deref())
        .await
    {
        Ok(_) => match state.db.get_user_by_id(user_id).await {
            Ok(Some(user)) => Ok(Json(ApiResponse::success(user.into()))),
            Ok(None) => Ok(Json(ApiResponse::error("用户不存在".to_string()))),
            Err(_) => Ok(Json(ApiResponse::error("数据库错误".to_string()))),
        },
        Err(e) => Ok(Json(ApiResponse::error(format!("更新用户失败: {}", e)))),
    }
}

pub async fn delete_user(
    Extension(state): Extension<ApiState>,
    headers: HeaderMap,
    Path(user_id): Path<i64>,
) -> Result<Json<ApiResponse<()>>, StatusCode> {
    let caller = jwt_user_id_from_headers(&state.jwt_secret, &headers)?;
    if caller != user_id && !db_user_is_admin(&state.db, caller).await {
        return Err(StatusCode::FORBIDDEN);
    }
    if let Ok(Some(target)) = state.db.get_user_by_id(user_id).await {
        if target.role == USER_ROLE_ADMIN {
            let n = state
                .db
                .count_admins()
                .await
                .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
            if n <= 1 {
                return Ok(Json(ApiResponse::error(
                    "不可删除最后一个管理员".to_string(),
                )));
            }
        }
    }
    match state.db.delete_user_simple(user_id).await {
        Ok(_) => Ok(Json(ApiResponse::success(()))),
        Err(e) => Ok(Json(ApiResponse::error(format!("删除用户失败: {}", e)))),
    }
}

#[derive(Debug, Deserialize)]
pub struct AdminCreateUserRequest {
    pub username: String,
    pub email: String,
    pub password: String,
    pub confirm_password: String,
}

/// Create a user account (admin only). Same validation as public registration.
pub async fn admin_create_user(
    Extension(state): Extension<ApiState>,
    headers: HeaderMap,
    Json(request): Json<AdminCreateUserRequest>,
) -> Result<Json<ApiResponse<UserInfo>>, StatusCode> {
    let caller = jwt_user_id_from_headers(&state.jwt_secret, &headers)?;
    if !db_user_is_admin(&state.db, caller).await {
        return Err(StatusCode::FORBIDDEN);
    }
    if request.username.trim().is_empty() {
        return Ok(Json(ApiResponse::error("用户名不能为空".to_string())));
    }
    if request.email.trim().is_empty() {
        return Ok(Json(ApiResponse::error("邮箱不能为空".to_string())));
    }
    if request.password.len() < 6 {
        return Ok(Json(ApiResponse::error("密码长度至少6位".to_string())));
    }
    if request.password != request.confirm_password {
        return Ok(Json(ApiResponse::error("两次输入的密码不一致".to_string())));
    }
    match state.db.get_user_by_username(&request.username).await {
        Ok(Some(_)) => return Ok(Json(ApiResponse::error("用户名已存在".to_string()))),
        Ok(None) => {}
        Err(_) => return Ok(Json(ApiResponse::error("数据库错误".to_string()))),
    }
    match state.db.get_user_by_email(&request.email).await {
        Ok(Some(_)) => return Ok(Json(ApiResponse::error("邮箱已被注册".to_string()))),
        Ok(None) => {}
        Err(_) => return Ok(Json(ApiResponse::error("数据库错误".to_string()))),
    }
    let create_request = CreateUserRequest {
        username: request.username.clone(),
        email: request.email.clone(),
        password: request.password.clone(),
    };
    match state.db.create_user(&create_request).await {
        Ok(user_id) => match state.db.get_user_by_id(user_id).await {
            Ok(Some(user)) => Ok(Json(ApiResponse::success(user.into()))),
            Ok(None) => Ok(Json(ApiResponse::error("创建用户后查询失败".to_string()))),
            Err(_) => Ok(Json(ApiResponse::error("查询用户信息失败".to_string()))),
        },
        Err(_) => Ok(Json(ApiResponse::error("创建用户失败".to_string()))),
    }
}

#[derive(Debug, Deserialize)]
pub struct UpdateUserRoleBody {
    pub role: String,
}

pub async fn update_user_role(
    Extension(state): Extension<ApiState>,
    headers: HeaderMap,
    Path(user_id): Path<i64>,
    Json(body): Json<UpdateUserRoleBody>,
) -> Result<Json<ApiResponse<UserInfo>>, StatusCode> {
    let caller = jwt_user_id_from_headers(&state.jwt_secret, &headers)?;
    if !db_user_is_admin(&state.db, caller).await {
        return Err(StatusCode::FORBIDDEN);
    }
    let new_role = if body.role.trim() == USER_ROLE_ADMIN {
        USER_ROLE_ADMIN
    } else {
        USER_ROLE_USER
    };
    if new_role == USER_ROLE_USER {
        if let Ok(Some(t)) = state.db.get_user_by_id(user_id).await {
            if t.role == USER_ROLE_ADMIN {
                let n = state
                    .db
                    .count_admins()
                    .await
                    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
                if n <= 1 {
                    return Ok(Json(ApiResponse::error(
                        "不可撤销最后一个管理员".to_string(),
                    )));
                }
            }
        }
    }
    state
        .db
        .set_user_role(user_id, new_role)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    match state.db.get_user_by_id(user_id).await {
        Ok(Some(user)) => Ok(Json(ApiResponse::success(user.into()))),
        Ok(None) => Ok(Json(ApiResponse::error("用户不存在".to_string()))),
        Err(_) => Ok(Json(ApiResponse::error("数据库错误".to_string()))),
    }
}

pub async fn get_user_devices(
    Extension(state): Extension<ApiState>,
    headers: HeaderMap,
    Path(user_id): Path<i64>,
) -> Result<Json<ApiResponse<Vec<DeviceInfo>>>, StatusCode> {
    let caller = jwt_user_id_from_headers(&state.jwt_secret, &headers)?;
    if caller != user_id && !db_user_is_admin(&state.db, caller).await {
        return Err(StatusCode::FORBIDDEN);
    }
    match state.db.get_user_devices_simple(user_id).await {
        Ok(devices) => {
            let device_infos: Vec<DeviceInfo> = devices.into_iter().map(|d| d.into()).collect();
            Ok(Json(ApiResponse::success(device_infos)))
        }
        Err(e) => Ok(Json(ApiResponse::error(format!("获取设备列表失败: {}", e)))),
    }
}

pub async fn remove_device(
    Extension(state): Extension<ApiState>,
    headers: HeaderMap,
    Path((user_id, device_id)): Path<(i64, String)>,
) -> Result<Json<ApiResponse<()>>, StatusCode> {
    let caller = jwt_user_id_from_headers(&state.jwt_secret, &headers)?;
    if caller != user_id {
        return Err(StatusCode::FORBIDDEN);
    }
    match state
        .db
        .remove_device_from_user_simple(user_id, &device_id)
        .await
    {
        Ok(_) => Ok(Json(ApiResponse::success(()))),
        Err(e) => Ok(Json(ApiResponse::<()>::error(format!(
            "移除设备失败: {}",
            e
        )))),
    }
}

pub async fn get_device_owner(
    Extension(state): Extension<ApiState>,
    Path(device_id): Path<String>,
) -> Result<Json<ApiResponse<UserInfo>>, StatusCode> {
    match state.db.get_device_owner_simple(&device_id).await {
        Ok(Some(user)) => Ok(Json(ApiResponse::success(user.into()))),
        Ok(None) => Ok(Json(ApiResponse::<UserInfo>::error(
            "设备未被分配给任何用户".to_string(),
        ))),
        Err(e) => Ok(Json(ApiResponse::<UserInfo>::error(format!(
            "查询设备所有者失败: {}",
            e
        )))),
    }
}

pub async fn monitor_connections(
    Extension(state): Extension<ApiState>,
    headers: axum::http::HeaderMap,
) -> impl IntoResponse {
    if jwt_user_id_from_headers(&state.jwt_secret, &headers).is_err() {
        return Json(ApiResponse::<Vec<serde_json::Value>> {
            success: false,
            data: None,
            message: "Unauthorized".to_string(),
        });
    }

    let timeout_ms: u128 = 30_000;
    let now = std::time::Instant::now();

    let connections: Vec<serde_json::Value> = {
        let map = crate::peer::ONLINE_PEERS
            .read()
            .unwrap_or_else(|e| e.into_inner());
        map.values()
            .filter(|c| now.duration_since(c.last_seen).as_millis() < timeout_ms)
            .map(|c| {
                serde_json::json!({
                    "peer_id": c.peer_id,
                    "ip": c.ip,
                    "user_id": c.user_id,
                    "connected_at": c.connected_at.to_rfc3339(),
                    "last_seen_secs": now.duration_since(c.last_seen).as_secs(),
                })
            })
            .collect()
    };

    Json(ApiResponse {
        success: true,
        data: Some(connections),
        message: String::new(),
    })
}

// ─── 订阅辅助函数 ────────────────────────────────────────────────────────────────

pub async fn build_subscription_info(db: &Database, user_id: i64) -> UserSubscriptionInfo {
    let sub = db.get_user_active_subscription(user_id).await.ok().flatten();
    let current_device_count = db.count_user_active_devices(user_id).await.unwrap_or(0);
    match sub {
        Some(s) => UserSubscriptionInfo {
            id: Some(s.id),
            plan_name: s.plan_name.clone(),
            plan_display_name: s.plan_display_name.clone(),
            device_limit: s.device_limit,
            speed_limit_kbps: s.speed_limit_kbps,
            started_at: Some(s.started_at.to_rfc3339()),
            expires_at: s.expires_at.map(|t| t.to_rfc3339()),
            is_active: s.is_active,
            current_device_count,
            notes: s.notes,
        },
        None => UserSubscriptionInfo {
            id: None,
            plan_name: "free".to_string(),
            plan_display_name: "免费版".to_string(),
            device_limit: FREE_DEVICE_LIMIT,
            speed_limit_kbps: FREE_SPEED_KBPS,
            started_at: None,
            expires_at: None,
            is_active: true,
            current_device_count,
            notes: None,
        },
    }
}

// ─── 订阅 API ────────────────────────────────────────────────────────────────────

/// GET /api/subscription/plans — 公开，无需登录
pub async fn list_subscription_plans(
    Extension(state): Extension<ApiState>,
) -> Result<Json<ApiResponse<Vec<SubscriptionPlanInfo>>>, StatusCode> {
    match state.db.get_subscription_plans().await {
        Ok(plans) => {
            let infos: Vec<SubscriptionPlanInfo> = plans.into_iter().map(|p| p.into()).collect();
            Ok(Json(ApiResponse::success(infos)))
        }
        Err(e) => Ok(Json(ApiResponse::error(format!("获取套餐列表失败: {}", e)))),
    }
}

/// GET /api/subscription/my — 需登录，返回自己的订阅 + current_device_count
pub async fn get_my_subscription(
    Extension(state): Extension<ApiState>,
    headers: HeaderMap,
) -> Result<Json<ApiResponse<UserSubscriptionInfo>>, StatusCode> {
    let user_id = jwt_user_id_from_headers(&state.jwt_secret, &headers)?;
    let info = build_subscription_info(&state.db, user_id).await;
    Ok(Json(ApiResponse::success(info)))
}

#[derive(Debug, Deserialize)]
pub struct AdminCreateSubscriptionRequest {
    pub user_id: i64,
    /// plan_id 优先，若为 None 则用 plan_name 查
    pub plan_id: Option<i64>,
    pub plan_name: Option<String>,
    /// ISO 8601 格式，如 "2026-12-31T00:00:00Z"，None 表示永久
    pub expires_at: Option<String>,
    pub notes: Option<String>,
}

/// POST /api/admin/subscriptions — 管理员，创建/升级用户订阅
pub async fn admin_create_subscription(
    Extension(state): Extension<ApiState>,
    headers: HeaderMap,
    Json(body): Json<AdminCreateSubscriptionRequest>,
) -> Result<Json<ApiResponse<i64>>, StatusCode> {
    let caller = jwt_user_id_from_headers(&state.jwt_secret, &headers)?;
    if !db_user_is_admin(&state.db, caller).await {
        return Err(StatusCode::FORBIDDEN);
    }

    // 解析 plan_id
    let plan_id = if let Some(pid) = body.plan_id {
        pid
    } else if let Some(ref pname) = body.plan_name {
        match state.db.get_plan_id_by_name(pname).await {
            Ok(Some(id)) => id,
            Ok(None) => {
                return Ok(Json(ApiResponse::error(format!(
                    "套餐 '{}' 不存在",
                    pname
                ))));
            }
            Err(e) => return Ok(Json(ApiResponse::error(format!("查询套餐失败: {}", e)))),
        }
    } else {
        return Ok(Json(ApiResponse::error(
            "必须提供 plan_id 或 plan_name".to_string(),
        )));
    };

    let expires_at: Option<DateTime<Utc>> = match body.expires_at {
        Some(ref s) => match s.parse::<DateTime<Utc>>() {
            Ok(dt) => Some(dt),
            Err(_) => {
                return Ok(Json(ApiResponse::error(
                    "expires_at 格式不正确，需要 ISO 8601".to_string(),
                )));
            }
        },
        None => None,
    };

    match state
        .db
        .create_user_subscription(body.user_id, plan_id, expires_at, body.notes)
        .await
    {
        Ok(sub_id) => Ok(Json(ApiResponse::success(sub_id))),
        Err(e) => Ok(Json(ApiResponse::error(format!("创建订阅失败: {}", e)))),
    }
}

/// GET /api/admin/subscriptions — 管理员，分页列表
pub async fn admin_list_subscriptions(
    Extension(state): Extension<ApiState>,
    headers: HeaderMap,
    Query(query): Query<ListQuery>,
) -> Result<Json<ApiResponse<Vec<UserSubscription>>>, StatusCode> {
    let caller = jwt_user_id_from_headers(&state.jwt_secret, &headers)?;
    if !db_user_is_admin(&state.db, caller).await {
        return Err(StatusCode::FORBIDDEN);
    }
    match state
        .db
        .list_subscriptions(query.limit, query.offset)
        .await
    {
        Ok(subs) => Ok(Json(ApiResponse::success(subs))),
        Err(e) => Ok(Json(ApiResponse::error(format!("获取订阅列表失败: {}", e)))),
    }
}

#[derive(Debug, Deserialize)]
pub struct AdminUpdateSubscriptionRequest {
    pub plan_id: Option<i64>,
    /// Some(Some("...")) 更新到某时间; Some(None) 清除到期时间; None 不改
    pub expires_at: Option<Option<String>>,
    pub notes: Option<String>,
}

/// PUT /api/admin/subscriptions/:id — 管理员，修改订阅
pub async fn admin_update_subscription(
    Extension(state): Extension<ApiState>,
    headers: HeaderMap,
    Path(sub_id): Path<i64>,
    Json(body): Json<AdminUpdateSubscriptionRequest>,
) -> Result<Json<ApiResponse<()>>, StatusCode> {
    let caller = jwt_user_id_from_headers(&state.jwt_secret, &headers)?;
    if !db_user_is_admin(&state.db, caller).await {
        return Err(StatusCode::FORBIDDEN);
    }

    // 解析 expires_at
    let expires_at: Option<Option<DateTime<Utc>>> = match body.expires_at {
        None => None,
        Some(None) => Some(None),
        Some(Some(ref s)) => match s.parse::<DateTime<Utc>>() {
            Ok(dt) => Some(Some(dt)),
            Err(_) => {
                return Ok(Json(ApiResponse::error(
                    "expires_at 格式不正确，需要 ISO 8601".to_string(),
                )));
            }
        },
    };

    match state
        .db
        .update_user_subscription(sub_id, body.plan_id, expires_at, body.notes)
        .await
    {
        Ok(_) => Ok(Json(ApiResponse::success(()))),
        Err(e) => Ok(Json(ApiResponse::error(format!("更新订阅失败: {}", e)))),
    }
}

/// DELETE /api/admin/subscriptions/:id — 管理员，撤销订阅（置 is_active=0）
pub async fn admin_deactivate_subscription(
    Extension(state): Extension<ApiState>,
    headers: HeaderMap,
    Path(sub_id): Path<i64>,
) -> Result<Json<ApiResponse<()>>, StatusCode> {
    let caller = jwt_user_id_from_headers(&state.jwt_secret, &headers)?;
    if !db_user_is_admin(&state.db, caller).await {
        return Err(StatusCode::FORBIDDEN);
    }
    match state.db.deactivate_user_subscription(sub_id).await {
        Ok(_) => Ok(Json(ApiResponse::success(()))),
        Err(e) => Ok(Json(ApiResponse::error(format!("撤销订阅失败: {}", e)))),
    }
}

/// GET /api/monitor/stats — 返回统计摘要（需登录；total_users 仅管理员可见）
pub async fn monitor_stats(
    Extension(state): Extension<ApiState>,
    headers: axum::http::HeaderMap,
) -> impl IntoResponse {
    let caller = match jwt_user_id_from_headers(&state.jwt_secret, &headers) {
        Ok(id) => id,
        Err(_) => {
            return Json(ApiResponse::<serde_json::Value> {
                success: false,
                data: None,
                message: "Unauthorized".to_string(),
            });
        }
    };

    let timeout_ms: u128 = 30_000;
    let now = std::time::Instant::now();

    let online_count = {
        let map = crate::peer::ONLINE_PEERS
            .read()
            .unwrap_or_else(|e| e.into_inner());
        map.values()
            .filter(|c| now.duration_since(c.last_seen).as_millis() < timeout_ms)
            .count()
    };

    let total_peers = {
        let map = crate::peer::ONLINE_PEERS
            .read()
            .unwrap_or_else(|e| e.into_inner());
        map.len()
    };

    let is_admin = db_user_is_admin(&state.db, caller).await;

    let mut stats = serde_json::json!({
        "online_count": online_count,
        "total_peers": total_peers,
    });

    if is_admin {
        let total_users = state
            .db
            .list_users(Some(10000), Some(0))
            .await
            .map(|v| v.len())
            .unwrap_or(0);
        stats["total_users"] = serde_json::json!(total_users);
    }

    Json(ApiResponse {
        success: true,
        data: Some(stats),
        message: String::new(),
    })
}
