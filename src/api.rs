use crate::database::{Database, CreateUserRequest, CreateDeviceRequest, User, UserDevice};
use crate::database_simple::*;
use axum::{
    extract::{Path, Query, Extension},
    http::StatusCode,
    response::Json,
    routing::{get, post, put, delete},
    Router,
    Server,
};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tower_http::cors::CorsLayer;
use jsonwebtoken::{encode, EncodingKey, Header};
use chrono::{Duration, Utc};

#[derive(Clone)]
pub struct ApiState {
    pub db: Database,
    pub jwt_secret: String,
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
}

impl From<User> for UserInfo {
    fn from(user: User) -> Self {
        Self {
            id: user.id,
            username: user.username,
            email: user.email,
            created_at: user.created_at.to_rfc3339(),
            is_active: user.is_active,
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Claims {
    pub sub: String, // user id
    pub username: String,
    pub exp: i64,
    pub iat: i64,
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
        .route("/api/login", post(login))
        .route("/api/users", get(list_users)) // .post(create_user)
        .route("/api/users/:id", get(get_user).put(update_user).delete(delete_user))
        .route("/api/users/:id/devices", get(get_user_devices)) // .post(add_device)
        .route("/api/users/:id/devices/:device_id", delete(remove_device))
        .route("/api/devices/:device_id/owner", get(get_device_owner))
                .layer(CorsLayer::permissive())
        .layer(axum::Extension(state))
}

async fn root_handler() -> &'static str {
    r#"<!DOCTYPE html>
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
        
        <div class="endpoint">
            <span class="method get">GET</span> <code>/api/users/:id/devices</code> - Get user devices
        </div>
        
        <div class="endpoint">
            <span class="method delete">DELETE</span> <code>/api/users/:id/devices/:device_id</code> - Remove device
        </div>
        
        <div class="endpoint">
            <span class="method get">GET</span> <code>/api/devices/:device_id/owner</code> - Get device owner
        </div>
        
        <h2>🔧 Usage Example</h2>
        <pre><code># Login
curl -X POST http://localhost:8080/api/login \
  -H "Content-Type: application/json" \
  -d '{"username": "admin", "password": "password"}'

# Get users
curl -X GET http://localhost:8080/api/users</code></pre>
        
        <h2>📝 Notes</h2>
        <ul>
            <li>API uses JSON for request/response format</li>
            <li>JWT authentication required for protected endpoints</li>
            <li>CORS enabled for web interface access</li>
            <li>Database: SQLite (./db_v2.sqlite3)</li>
        </ul>
    </div>
</body>
</html>"#
}

async fn login(
    Extension(state): Extension<ApiState>,
    Json(request): Json<LoginRequest>,
) -> Result<Json<ApiResponse<LoginResponse>>, StatusCode> {
    match state.db.get_user_by_username_simple(&request.username).await {
        Ok(Some(user)) => {
            if !user.is_active {
                return Ok(Json(ApiResponse::error("用户账户已被禁用".to_string())));
            }
            
            match state.db.verify_password(&request.password, &user.password_hash).await {
                Ok(true) => {
                    let expiration = Utc::now() + Duration::hours(24);
                    let claims = Claims {
                        sub: user.id.to_string(),
                        username: user.username.clone(),
                        exp: expiration.timestamp(),
                        iat: Utc::now().timestamp(),
                    };
                    
                    let token = encode(
                        &Header::default(),
                        &claims,
                        &EncodingKey::from_secret(state.jwt_secret.as_ref()),
                    ).unwrap_or_default();
                    
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

async fn create_user(
    Extension(state): Extension<ApiState>,
    Json(request): Json<CreateUserRequest>,
) -> Result<Json<ApiResponse<UserInfo>>, StatusCode> {
    // 验证输入
    if request.username.len() < 3 {
        return Ok(Json(ApiResponse::error("用户名至少需要3个字符".to_string())));
    }
    
    if request.password.len() < 6 {
        return Ok(Json(ApiResponse::error("密码至少需要6个字符".to_string())));
    }
    
    if !request.email.contains('@') {
        return Ok(Json(ApiResponse::error("邮箱格式不正确".to_string())));
    }
    
    // 检查用户名和邮箱是否已存在
    if let Ok(Some(_)) = state.db.get_user_by_username_simple(&request.username).await {
        return Ok(Json(ApiResponse::error("用户名已存在".to_string())));
    }
    
    if let Ok(Some(_)) = state.db.get_user_by_email_simple(&request.email).await {
        return Ok(Json(ApiResponse::error("邮箱已存在".to_string())));
    }
    
    match state.db.create_user_simple(&request).await {
        Ok(user_id) => {
            match state.db.get_user_by_id(user_id).await {
                Ok(Some(user)) => Ok(Json(ApiResponse::success(user.into()))),
                Ok(None) => Ok(Json(ApiResponse::error("创建用户后查询失败".to_string()))),
                Err(_) => Ok(Json(ApiResponse::error("数据库错误".to_string()))),
            }
        }
        Err(e) => Ok(Json(ApiResponse::error(format!("创建用户失败: {}", e)))),
    }
}

async fn list_users(
    Extension(state): Extension<ApiState>,
    Query(query): Query<ListQuery>,
) -> Result<Json<ApiResponse<Vec<UserInfo>>>, StatusCode> {
    match state.db.list_users_simple(query.limit, query.offset).await {
        Ok(users) => {
            let user_infos: Vec<UserInfo> = users.into_iter().map(|u| u.into()).collect();
            Ok(Json(ApiResponse::success(user_infos)))
        }
        Err(_) => Ok(Json(ApiResponse::error("获取用户列表失败".to_string()))),
    }
}

async fn get_user(
    Extension(state): Extension<ApiState>,
    Path(user_id): Path<i64>,
) -> Result<Json<ApiResponse<UserInfo>>, StatusCode> {
    match state.db.get_user_by_id(user_id).await {
        Ok(Some(user)) => Ok(Json(ApiResponse::success(user.into()))),
        Ok(None) => Ok(Json(ApiResponse::error("用户不存在".to_string()))),
        Err(_) => Ok(Json(ApiResponse::error("数据库错误".to_string()))),
    }
}

async fn update_user(
    Extension(state): Extension<ApiState>,
    Path(user_id): Path<i64>,
    Json(mut request): Json<HashMap<String, String>>,
) -> Result<Json<ApiResponse<UserInfo>>, StatusCode> {
    let username = request.remove("username");
    let email = request.remove("email");
    
    // 验证邮箱格式
    if let Some(ref email) = email {
        if !email.contains('@') {
            return Ok(Json(ApiResponse::error("邮箱格式不正确".to_string())));
        }
    }
    
    match state.db.update_user(user_id, username.as_deref(), email.as_deref()).await {
        Ok(_) => {
            match state.db.get_user_by_id(user_id).await {
                Ok(Some(user)) => Ok(Json(ApiResponse::success(user.into()))),
                Ok(None) => Ok(Json(ApiResponse::error("用户不存在".to_string()))),
                Err(_) => Ok(Json(ApiResponse::error("数据库错误".to_string()))),
            }
        }
        Err(e) => Ok(Json(ApiResponse::error(format!("更新用户失败: {}", e)))),
    }
}

async fn delete_user(
    Extension(state): Extension<ApiState>,
    Path(user_id): Path<i64>,
) -> Result<Json<ApiResponse<()>>, StatusCode> {
    match state.db.delete_user_simple(user_id).await {
        Ok(_) => Ok(Json(ApiResponse::success(()))),
        Err(e) => Ok(Json(ApiResponse::error(format!("删除用户失败: {}", e)))),
    }
}

async fn get_user_devices(
    Extension(state): Extension<ApiState>,
    Path(user_id): Path<i64>,
) -> Result<Json<ApiResponse<Vec<DeviceInfo>>>, StatusCode> {
    match state.db.get_user_devices_simple(user_id).await {
        Ok(devices) => {
            let device_infos: Vec<DeviceInfo> = devices.into_iter().map(|d| d.into()).collect();
            Ok(Json(ApiResponse::success(device_infos)))
        }
        Err(e) => Ok(Json(ApiResponse::error(format!("获取设备列表失败: {}", e)))),
    }
}

async fn add_device(
    Extension(state): Extension<ApiState>,
    Path(user_id): Path<i64>,
    Json(request): Json<CreateDeviceRequest>,
) -> Result<Json<ApiResponse<DeviceInfo>>, StatusCode> {
    // 验证用户存在
    match state.db.get_user_by_id(user_id).await {
        Ok(Some(_)) => {
            // 创建设备关联请求
            let device_request = CreateDeviceRequest {
                user_id,
                device_id: request.device_id,
                device_name: request.device_name,
            };
            
            match state.db.add_device_to_user_simple(&device_request).await {
                Ok(device_relation_id) => {
                    // 获取设备信息
                    match state.db.get_user_devices_simple(user_id).await {
                        Ok(devices) => {
                            if let Some(device) = devices.into_iter().find(|d| d.id == device_relation_id) {
                                Ok(Json(ApiResponse::success(device.into())))
                            } else {
                                Ok(Json(ApiResponse::error("添加设备后查询失败".to_string())))
                            }
                        }
                        Err(_) => Ok(Json(ApiResponse::error("查询设备信息失败".to_string()))),
                    }
                }
                Err(e) => Ok(Json(ApiResponse::error(format!("添加设备失败: {}", e)))),
            }
        }
        Ok(None) => Ok(Json(ApiResponse::error("用户不存在".to_string()))),
        Err(_) => Ok(Json(ApiResponse::error("数据库错误".to_string()))),
    }
}

async fn remove_device(
    Extension(state): Extension<ApiState>,
    Path((user_id, device_id)): Path<(i64, String)>,
) -> Result<Json<ApiResponse<()>>, StatusCode> {
    match state.db.remove_device_from_user_simple(user_id, &device_id).await {
        Ok(_) => Ok(Json(ApiResponse::success(()))),
        Err(e) => Ok(Json(ApiResponse::<()>::error(format!("移除设备失败: {}", e)))),
    }
}

async fn get_device_owner(
    Extension(state): Extension<ApiState>,
    Path(device_id): Path<String>,
) -> Result<Json<ApiResponse<UserInfo>>, StatusCode> {
    match state.db.get_device_owner_simple(&device_id).await {
        Ok(Some(user)) => Ok(Json(ApiResponse::success(user.into()))),
        Ok(None) => Ok(Json(ApiResponse::<UserInfo>::error("设备未被分配给任何用户".to_string()))),
        Err(e) => Ok(Json(ApiResponse::<UserInfo>::error(format!("查询设备所有者失败: {}", e)))),
    }
}
