use crate::database::{
    CreateUserRequest, Database, User, UserDevice, USER_ROLE_ADMIN, USER_ROLE_USER,
};
use crate::device_api;
use crate::device_pages;
use crate::password_reset::change_password;
use crate::web::{forgot_password, forgot_password_page, reset_password, reset_password_page};
use axum::{
    extract::{Extension, Path, Query},
    http::{header, HeaderMap, StatusCode},
    response::{IntoResponse, Json},
    routing::{delete, get, post, put},
    Router,
};
use chrono::{Duration, Utc};
use jsonwebtoken::{encode, EncodingKey, Header};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use tower_http::cors::CorsLayer;

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

#[derive(Debug, Serialize, Deserialize)]
pub struct LoginResponse {
    pub token: String,
    pub user: UserInfo,
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

pub fn create_api_router(db: Database, jwt_secret: String) -> Router {
    let state = ApiState { db, jwt_secret };

    Router::new()
        .route("/", get(root_handler))
        .route("/login", get(login_page))
        .route("/register", get(register_page))
        .route("/forgot-password", get(forgot_password_page))
        .route("/reset-password", get(reset_password_page))
        .route("/dashboard", get(device_pages::dashboard_page))
        .route("/devices", get(device_pages::devices_page))
        .route("/api/login", post(login))
        .route("/api/register", post(register))
        .route("/api/forgot-password", post(forgot_password))
        .route("/api/reset-password", post(reset_password))
        .route("/api/change-password", post(change_password))
        .route("/api/users", get(list_users).post(admin_create_user))
        .route(
            "/api/users/:id",
            get(get_user).put(update_user).delete(delete_user),
        )
        .route("/api/users/:id/role", put(update_user_role))
        .route("/api/users/:id/devices", get(get_user_devices)) // .post(add_device)
        .route("/api/users/:id/devices/:device_id", delete(remove_device))
        .route("/api/devices/:device_id/owner", get(get_device_owner))
        .route("/api/devices", post(device_api::add_device))
        .route(
            "/api/devices/:device_id",
            delete(device_api::remove_device_by_id),
        )
        .layer(CorsLayer::permissive())
        .layer(axum::Extension(state))
}

async fn root_handler() -> impl IntoResponse {
    let html = r#"<!DOCTYPE html>
<html>
<head>
    <title>NAT Server API</title>
    <meta charset="utf-8">
    <style>
        body { font-family: Arial, sans-serif; margin: 40px; }
        .container { max-width: 800px; margin: 0 auto; }
        .endpoint { background: #f5f5f5; padding: 10px; margin: 5px 0; border-radius: 5px; }
        .method { display: inline-block; padding: 2px 8px; border-radius: 3px; color: white; font-weight: bold; }
        .get { background: #61affe; }
        .post { background: #49cc90; }
        .put { background: #fca130; }
        .delete { background: #f93e3e; }
    </style>
</head>
<body>
    <div class="container">
        <h1>🚀 NAT Server API</h1>
        <p>RustDesk Server Fork - REST API Interface</p>

        <h2>📋 Available Endpoints</h2>

        <div class="endpoint">
            <span class="method post">POST</span> <code>/api/login</code> - User authentication
        </div>

        <div class="endpoint">
            <span class="method post">POST</span> <code>/api/register</code> - User registration
        </div>

        <div class="endpoint">
            <span class="method post">POST</span> <code>/api/forgot-password</code> - Password reset request
        </div>

        <div class="endpoint">
            <span class="method post">POST</span> <code>/api/reset-password</code> - Password reset confirmation
        </div>

        <div class="endpoint">
            <span class="method get">GET</span> <code>/api/users</code> - List all users
        </div>

        <div class="endpoint">
            <span class="method get">GET</span> <code>/api/users/:id</code> - Get user by ID
        </div>

        <div class="endpoint">
            <span class="method put">PUT</span> <code>/api/users/:id</code> - Update user
        </div>

        <div class="endpoint">
            <span class="method delete">DELETE</span> <code>/api/users/:id</code> - Delete user
        </div>

        <h2>🔧 Usage Example</h2>
        <pre><code># Login
curl -X POST http://localhost:8080/api/login \
  -H "Content-Type: application/json" \
  -d '{"username": "admin", "password": "password"}'

# Login with device binding (optional device_id = RustDesk ID registered in user_devices)
curl -X POST http://localhost:8080/api/login \
  -H "Content-Type: application/json" \
  -d '{"username": "admin", "password": "password", "device_id": "123456789"}'

# Register
curl -X POST http://localhost:8080/api/register \
  -H "Content-Type: application/json" \
  -d '{"username": "testuser", "email": "test@example.com", "password": "password123", "confirm_password": "password123"}'</code></pre>

        <h2>📝 Notes</h2>
        <ul>
            <li>API uses JSON for request/response format</li>
            <li>JWT authentication required for protected endpoints</li>
            <li>CORS enabled for web interface access</li>
            <li>Database: SQLite (./db_v2.sqlite3)</li>
        </ul>
    </div>
</body>
</html>"#;

    axum::response::Html(html).into_response()
}

async fn login(
    Extension(state): Extension<ApiState>,
    Json(request): Json<LoginRequest>,
) -> Result<Json<ApiResponse<LoginResponse>>, StatusCode> {
    match state.db.get_user_by_username(&request.username).await {
        Ok(Some(user)) => {
            if !user.is_active {
                return Ok(Json(ApiResponse::error("用户账户已被禁用".to_string())));
            }

            match state
                .db
                .verify_password(&request.password, &user.password_hash)
                .await
            {
                Ok(true) => {
                    let udid = match request
                        .device_id
                        .as_ref()
                        .map(|s| s.trim())
                        .filter(|s| !s.is_empty())
                    {
                        None => None,
                        Some(did) => match state.db.get_user_device_row_id(user.id, did).await {
                            Ok(Some(id)) => Some(id),
                            Ok(None) => {
                                return Ok(Json(ApiResponse::error(
                                    "指定的设备不属于当前用户或未激活".to_string(),
                                )));
                            }
                            Err(_) => {
                                return Ok(Json(ApiResponse::error("查询设备失败".to_string())));
                            }
                        },
                    };
                    let expiration = Utc::now() + Duration::hours(24);
                    let claims = Claims {
                        sub: user.id.to_string(),
                        username: user.username.clone(),
                        exp: expiration.timestamp(),
                        iat: Utc::now().timestamp(),
                        udid,
                        role: if user.role == USER_ROLE_ADMIN {
                            USER_ROLE_ADMIN.to_string()
                        } else {
                            USER_ROLE_USER.to_string()
                        },
                    };

                    let token = encode(
                        &Header::default(),
                        &claims,
                        &EncodingKey::from_secret(state.jwt_secret.as_ref()),
                    )
                    .unwrap_or_default();

                    let response = LoginResponse {
                        token,
                        user: user.into(),
                    };

                    Ok(Json(ApiResponse::success(response)))
                }
                Ok(false) => Ok(Json(ApiResponse::error("密码错误".to_string()))),
                Err(_) => Ok(Json(ApiResponse::error("认证失败".to_string()))),
            }
        }
        Ok(None) => Ok(Json(ApiResponse::error("用户不存在".to_string()))),
        Err(_) => Ok(Json(ApiResponse::error("数据库错误".to_string()))),
    }
}

async fn register(
    Extension(state): Extension<ApiState>,
    Json(request): Json<RegisterRequest>,
) -> Result<Json<ApiResponse<UserInfo>>, StatusCode> {
    // 验证输入
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

    // 检查用户名是否已存在
    match state.db.get_user_by_username(&request.username).await {
        Ok(Some(_)) => {
            return Ok(Json(ApiResponse::error("用户名已存在".to_string())));
        }
        Ok(None) => {}
        Err(_) => {
            return Ok(Json(ApiResponse::error("数据库错误".to_string())));
        }
    }

    // 检查邮箱是否已存在
    match state.db.get_user_by_email(&request.email).await {
        Ok(Some(_)) => {
            return Ok(Json(ApiResponse::error("邮箱已被注册".to_string())));
        }
        Ok(None) => {}
        Err(_) => {
            return Ok(Json(ApiResponse::error("数据库错误".to_string())));
        }
    }

    // 创建用户
    let create_request = CreateUserRequest {
        username: request.username.clone(),
        email: request.email.clone(),
        password: request.password.clone(),
    };

    match state.db.create_user(&create_request).await {
        Ok(user_id) => {
            // 获取刚创建的用户信息
            match state.db.get_user_by_id(user_id).await {
                Ok(Some(user)) => Ok(Json(ApiResponse::success(user.into()))),
                Ok(None) => Ok(Json(ApiResponse::error("创建用户后查询失败".to_string()))),
                Err(_) => Ok(Json(ApiResponse::error("查询用户信息失败".to_string()))),
            }
        }
        Err(_) => Ok(Json(ApiResponse::error("注册失败".to_string()))),
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

async fn login_page() -> impl IntoResponse {
    let html = r#"<!DOCTYPE html>
<html lang="zh-CN">
<head>
    <meta charset="UTF-8">
    <meta name="viewport" content="width=device-width, initial-scale=1.0">
    <title>用户登录 - NAT Server</title>
    <style>
        * {
            margin: 0;
            padding: 0;
            box-sizing: border-box;
        }

        body {
            font-family: 'Segoe UI', Tahoma, Geneva, Verdana, sans-serif;
            background: linear-gradient(135deg, #667eea 0%, #764ba2 100%);
            min-height: 100vh;
            display: flex;
            align-items: center;
            justify-content: center;
        }

        .login-container {
            background: white;
            padding: 2rem;
            border-radius: 10px;
            box-shadow: 0 15px 35px rgba(0, 0, 0, 0.1);
            width: 100%;
            max-width: 400px;
        }

        .login-header {
            text-align: center;
            margin-bottom: 2rem;
        }

        .login-header h1 {
            color: #333;
            font-size: 2rem;
            margin-bottom: 0.5rem;
        }

        .login-header p {
            color: #666;
            font-size: 0.9rem;
        }

        .form-group {
            margin-bottom: 1.5rem;
        }

        .form-group label {
            display: block;
            margin-bottom: 0.5rem;
            color: #333;
            font-weight: 500;
        }

        .form-group input {
            width: 100%;
            padding: 0.75rem;
            border: 2px solid #e1e5e9;
            border-radius: 5px;
            font-size: 1rem;
            transition: border-color 0.3s;
        }

        .form-group input:focus {
            outline: none;
            border-color: #667eea;
        }

        .login-btn {
            width: 100%;
            padding: 0.75rem;
            background: linear-gradient(135deg, #667eea 0%, #764ba2 100%);
            color: white;
            border: none;
            border-radius: 5px;
            font-size: 1rem;
            font-weight: 500;
            cursor: pointer;
            transition: transform 0.2s;
        }

        .login-btn:hover {
            transform: translateY(-2px);
        }

        .login-btn:active {
            transform: translateY(0);
        }

        .register-link {
            text-align: center;
            margin-top: 1.5rem;
            color: #666;
        }

        .register-link a {
            color: #667eea;
            text-decoration: none;
            font-weight: 500;
        }

        .register-link a:hover {
            text-decoration: underline;
        }

        .error-message {
            background: #fee;
            color: #c33;
            padding: 0.75rem;
            border-radius: 5px;
            margin-bottom: 1rem;
            display: none;
        }

        .success-message {
            background: #efe;
            color: #3c3;
            padding: 0.75rem;
            border-radius: 5px;
            margin-bottom: 1rem;
            display: none;
        }
    </style>
</head>
<body>
    <div class="login-container">
        <div class="login-header">
            <h1>用户登录</h1>
            <p>欢迎使用 NAT Server</p>
        </div>

        <div class="error-message" id="error-message"></div>
        <div class="success-message" id="success-message"></div>

        <form id="login-form">
            <div class="form-group">
                <label for="username">用户名</label>
                <input type="text" id="username" name="username" required>
            </div>

            <div class="form-group">
                <label for="password">密码</label>
                <input type="password" id="password" name="password" required>
            </div>

            <button type="submit" class="login-btn">登录</button>
        </form>

        <div class="register-link">
            还没有账户？ <a href="/register">立即注册</a>
        </div>
        <div class="register-link">
            忘记密码？ <a href="/forgot-password">重置密码</a>
        </div>
    </div>

    <script>
        document.getElementById('login-form').addEventListener('submit', async function(e) {
            e.preventDefault();

            const username = document.getElementById('username').value;
            const password = document.getElementById('password').value;
            const errorDiv = document.getElementById('error-message');
            const successDiv = document.getElementById('success-message');

            errorDiv.style.display = 'none';
            successDiv.style.display = 'none';

            try {
                const response = await fetch('/api/login', {
                    method: 'POST',
                    headers: {
                        'Content-Type': 'application/json',
                    },
                    body: JSON.stringify({ username, password }),
                });

                const result = await response.json();

                if (result.success) {
                    successDiv.textContent = '登录成功！正在跳转...';
                    successDiv.style.display = 'block';

                    // 保存token到localStorage
                    localStorage.setItem('jwt_token', result.data.token);
                    localStorage.setItem('user_info', JSON.stringify(result.data.user));

                    // 跳转到主页或仪表板
                    setTimeout(() => {
                        window.location.href = '/';
                    }, 1500);
                } else {
                    errorDiv.textContent = result.message;
                    errorDiv.style.display = 'block';
                }
            } catch (error) {
                errorDiv.textContent = '网络错误，请重试';
                errorDiv.style.display = 'block';
            }
        });
    </script>
</body>
</html>"#;

    axum::response::Html(html).into_response()
}

async fn register_page() -> impl IntoResponse {
    let html = r#"<!DOCTYPE html>
<html lang="zh-CN">
<head>
    <meta charset="UTF-8">
    <meta name="viewport" content="width=device-width, initial-scale=1.0">
    <title>用户注册 - NAT Server</title>
    <style>
        * {
            margin: 0;
            padding: 0;
            box-sizing: border-box;
        }

        body {
            font-family: 'Segoe UI', Tahoma, Geneva, Verdana, sans-serif;
            background: linear-gradient(135deg, #667eea 0%, #764ba2 100%);
            min-height: 100vh;
            display: flex;
            align-items: center;
            justify-content: center;
        }

        .register-container {
            background: white;
            padding: 2rem;
            border-radius: 10px;
            box-shadow: 0 15px 35px rgba(0, 0, 0, 0.1);
            width: 100%;
            max-width: 400px;
        }

        .register-header {
            text-align: center;
            margin-bottom: 2rem;
        }

        .register-header h1 {
            color: #333;
            font-size: 2rem;
            margin-bottom: 0.5rem;
        }

        .register-header p {
            color: #666;
            font-size: 0.9rem;
        }

        .form-group {
            margin-bottom: 1.5rem;
        }

        .form-group label {
            display: block;
            margin-bottom: 0.5rem;
            color: #333;
            font-weight: 500;
        }

        .form-group input {
            width: 100%;
            padding: 0.75rem;
            border: 2px solid #e1e5e9;
            border-radius: 5px;
            font-size: 1rem;
            transition: border-color 0.3s;
        }

        .form-group input:focus {
            outline: none;
            border-color: #667eea;
        }

        .register-btn {
            width: 100%;
            padding: 0.75rem;
            background: linear-gradient(135deg, #667eea 0%, #764ba2 100%);
            color: white;
            border: none;
            border-radius: 5px;
            font-size: 1rem;
            font-weight: 500;
            cursor: pointer;
            transition: transform 0.2s;
        }

        .register-btn:hover {
            transform: translateY(-2px);
        }

        .register-btn:active {
            transform: translateY(0);
        }

        .login-link {
            text-align: center;
            margin-top: 1.5rem;
            color: #666;
        }

        .login-link a {
            color: #667eea;
            text-decoration: none;
            font-weight: 500;
        }

        .login-link a:hover {
            text-decoration: underline;
        }

        .error-message {
            background: #fee;
            color: #c33;
            padding: 0.75rem;
            border-radius: 5px;
            margin-bottom: 1rem;
            display: none;
        }

        .success-message {
            background: #efe;
            color: #3c3;
            padding: 0.75rem;
            border-radius: 5px;
            margin-bottom: 1rem;
            display: none;
        }

        .password-requirements {
            font-size: 0.8rem;
            color: #666;
            margin-top: 0.25rem;
        }
    </style>
</head>
<body>
    <div class="register-container">
        <div class="register-header">
            <h1>用户注册</h1>
            <p>创建您的 NAT Server 账户</p>
        </div>

        <div class="error-message" id="error-message"></div>
        <div class="success-message" id="success-message"></div>

        <form id="register-form">
            <div class="form-group">
                <label for="username">用户名</label>
                <input type="text" id="username" name="username" required>
            </div>

            <div class="form-group">
                <label for="email">邮箱</label>
                <input type="email" id="email" name="email" required>
            </div>

            <div class="form-group">
                <label for="password">密码</label>
                <input type="password" id="password" name="password" required>
                <div class="password-requirements">密码至少6位字符</div>
            </div>

            <div class="form-group">
                <label for="confirm_password">确认密码</label>
                <input type="password" id="confirm_password" name="confirm_password" required>
            </div>

            <button type="submit" class="register-btn">注册</button>
        </form>

        <div class="login-link">
            已有账户？ <a href="/login">立即登录</a>
        </div>
    </div>

    <script>
        document.getElementById('register-form').addEventListener('submit', async function(e) {
            e.preventDefault();

            const username = document.getElementById('username').value;
            const email = document.getElementById('email').value;
            const password = document.getElementById('password').value;
            const confirmPassword = document.getElementById('confirm_password').value;
            const errorDiv = document.getElementById('error-message');
            const successDiv = document.getElementById('success-message');

            errorDiv.style.display = 'none';
            successDiv.style.display = 'none';

            // 客户端验证
            if (password !== confirmPassword) {
                errorDiv.textContent = '两次输入的密码不一致';
                errorDiv.style.display = 'block';
                return;
            }

            if (password.length < 6) {
                errorDiv.textContent = '密码长度至少6位';
                errorDiv.style.display = 'block';
                return;
            }

            try {
                const response = await fetch('/api/register', {
                    method: 'POST',
                    headers: {
                        'Content-Type': 'application/json',
                    },
                    body: JSON.stringify({
                        username,
                        email,
                        password,
                        confirm_password: confirmPassword
                    }),
                });

                const result = await response.json();

                if (result.success) {
                    successDiv.textContent = '注册成功！正在跳转到登录页面...';
                    successDiv.style.display = 'block';

                    setTimeout(() => {
                        window.location.href = '/login';
                    }, 2000);
                } else {
                    errorDiv.textContent = result.message;
                    errorDiv.style.display = 'block';
                }
            } catch (error) {
                errorDiv.textContent = '网络错误，请重试';
                errorDiv.style.display = 'block';
            }
        });
    </script>
</body>
</html>"#;

    axum::response::Html(html).into_response()
}

// ─── 连接监控 API ────────────────────────────────────────────────────────────────────

/// GET /api/monitor/connections — 返回当前在线连接列表（需登录）
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

/// GET /api/monitor/stats — 返回统计摘要（需登录）
pub async fn monitor_stats(
    Extension(state): Extension<ApiState>,
    headers: axum::http::HeaderMap,
) -> impl IntoResponse {
    if jwt_user_id_from_headers(&state.jwt_secret, &headers).is_err() {
        return Json(ApiResponse::<serde_json::Value> {
            success: false,
            data: None,
            message: "Unauthorized".to_string(),
        });
    }

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

    let total_users = state
        .db
        .list_users(Some(10000), Some(0))
        .await
        .map(|v| v.len())
        .unwrap_or(0);

    Json(ApiResponse {
        success: true,
        data: Some(serde_json::json!({
            "online_count": online_count,
            "total_peers": total_peers,
            "total_users": total_users,
        })),
        message: String::new(),
    })
}
