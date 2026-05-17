// Web handlers using loco-rs + Askama
use askama::Template;
use axum::{
    extract::{Extension, Query},
    http::StatusCode,
    response::{Html, IntoResponse, Json},
    routing::{delete, get, post, put},
    Router,
};
use serde::Deserialize;
use std::collections::HashMap;

use crate::{
    api::{ApiResponse, ApiState, LoginRequest, RegisterRequest, UserInfo},
    database::{CreateUserRequest, Database},
    views::{
        AdminSubscriptionsTemplate, DashboardTemplate, DevicesTemplate, ForgotPasswordTemplate,
        LoginTemplate, MonitorTemplate, RegisterTemplate, ResetPasswordTemplate,
        SubscriptionTemplate, UsersTemplate,
    },
};

#[derive(Deserialize)]
pub struct ForgotPasswordRequest {
    pub email: String,
}

#[derive(Deserialize)]
pub struct ResetPasswordRequest {
    pub token: String,
    pub password: String,
}

// Web handlers
pub async fn login_page() -> impl IntoResponse {
    let template = LoginTemplate {
        title: "用户登录".to_string(),
        current_user: None,
    };
    Html(
        template
            .render()
            .unwrap_or_else(|_| "Template error".to_string()),
    )
}

pub async fn register_page() -> impl IntoResponse {
    let template = RegisterTemplate {
        title: "用户注册".to_string(),
        current_user: None,
    };
    Html(
        template
            .render()
            .unwrap_or_else(|_| "Template error".to_string()),
    )
}

pub async fn forgot_password_page() -> impl IntoResponse {
    let template = ForgotPasswordTemplate {
        title: "忘记密码".to_string(),
        current_user: None,
    };
    Html(
        template
            .render()
            .unwrap_or_else(|_| "Template error".to_string()),
    )
}

pub async fn reset_password_page(
    Query(params): Query<HashMap<String, String>>,
) -> impl IntoResponse {
    let token = params.get("token").unwrap_or(&String::new()).clone();

    let template = ResetPasswordTemplate {
        title: "重置密码".to_string(),
        current_user: None,
        token,
    };
    Html(
        template
            .render()
            .unwrap_or_else(|_| "Template error".to_string()),
    )
}

pub async fn dashboard_page(
    Extension(_state): Extension<ApiState>,
) -> Result<impl IntoResponse, StatusCode> {
    let template = DashboardTemplate {
        title: "控制台".to_string(),
        current_user: None,
    };
    Ok(Html(
        template
            .render()
            .unwrap_or_else(|_| "Template error".to_string()),
    ))
}

pub async fn devices_page(
    Extension(_state): Extension<ApiState>,
) -> Result<impl IntoResponse, StatusCode> {
    let template = DevicesTemplate {
        title: "设备管理".to_string(),
        current_user: None,
    };
    Ok(Html(
        template
            .render()
            .unwrap_or_else(|_| "Template error".to_string()),
    ))
}

pub async fn users_page(
    Extension(_state): Extension<ApiState>,
) -> Result<impl IntoResponse, StatusCode> {
    let template = UsersTemplate {
        title: "用户管理".to_string(),
        current_user: None,
    };
    Ok(Html(
        template
            .render()
            .unwrap_or_else(|_| "Template error".to_string()),
    ))
}

pub async fn monitor_page() -> impl IntoResponse {
    let template = MonitorTemplate {
        title: "连接监控".to_string(),
        current_user: None,
    };
    Html(
        template
            .render()
            .unwrap_or_else(|_| "Template error".to_string()),
    )
}

pub async fn subscription_page() -> impl IntoResponse {
    let template = SubscriptionTemplate {
        title: "我的订阅".to_string(),
        current_user: None,
    };
    Html(
        template
            .render()
            .unwrap_or_else(|_| "Template error".to_string()),
    )
}

pub async fn admin_subscriptions_page() -> impl IntoResponse {
    let template = AdminSubscriptionsTemplate {
        title: "订阅管理".to_string(),
        current_user: None,
    };
    Html(
        template
            .render()
            .unwrap_or_else(|_| "Template error".to_string()),
    )
}

// API handlers for password reset
pub async fn forgot_password(
    Extension(_state): Extension<ApiState>,
    Json(request): Json<ForgotPasswordRequest>,
) -> Result<Json<ApiResponse<String>>, StatusCode> {
    // 检查用户是否存在
    match _state.db.get_user_by_email(&request.email).await {
        Ok(Some(_user)) => {
            // 在实际应用中，这里应该发送邮件
            // 为了演示，我们只是返回成功消息
            // TODO: 实现邮件发送功能
            Ok(Json(ApiResponse {
                success: true,
                message: "重置密码链接已发送到您的邮箱".to_string(),
                data: Some("success".to_string()),
            }))
        }
        Ok(None) => {
            // 为了安全，即使用户不存在也返回成功消息
            Ok(Json(ApiResponse {
                success: true,
                message: "如果邮箱存在，重置链接已发送".to_string(),
                data: Some("success".to_string()),
            }))
        }
        Err(_) => Err(StatusCode::INTERNAL_SERVER_ERROR),
    }
}

pub async fn reset_password(
    Extension(_state): Extension<ApiState>,
    Json(request): Json<ResetPasswordRequest>,
) -> Result<Json<ApiResponse<String>>, StatusCode> {
    // 在实际应用中，这里应该验证token的有效性
    // 为了演示，我们只是检查token是否为空
    if request.token.is_empty() {
        return Ok(Json(ApiResponse {
            success: false,
            message: "无效的重置令牌".to_string(),
            data: None,
        }));
    }

    // TODO: 实现真正的token验证和密码重置逻辑
    // 这里应该：
    // 1. 验证token是否有效且未过期
    // 2. 从token中获取用户ID
    // 3. 更新用户密码

    // 为了演示，我们假设token有效并返回成功
    Ok(Json(ApiResponse {
        success: true,
        message: "密码重置成功".to_string(),
        data: Some("success".to_string()),
    }))
}

// API handlers (保持原有逻辑)
pub async fn login(
    Extension(_state): Extension<ApiState>,
    Json(request): Json<LoginRequest>,
) -> Result<Json<ApiResponse<crate::api::LoginResponse>>, StatusCode> {
    match _state.db.get_user_by_username(&request.username).await {
        Ok(Some(user)) => {
            if !user.is_active {
                return Ok(Json(ApiResponse::<crate::api::LoginResponse>::error(
                    "用户账户已被禁用".to_string(),
                )));
            }
            match _state
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
                        Some(did) => match _state.db.get_user_device_row_id(user.id, did).await {
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
                    let now = chrono::Utc::now();
                    let claims = crate::api::Claims {
                        sub: user.id.to_string(),
                        username: user.username.clone(),
                        exp: (now + chrono::Duration::hours(24)).timestamp(),
                        iat: now.timestamp(),
                        udid,
                        role: user.role.clone(),
                    };

                    let token = match jsonwebtoken::encode(
                        &jsonwebtoken::Header::default(),
                        &claims,
                        &jsonwebtoken::EncodingKey::from_secret(_state.jwt_secret.as_ref()),
                    ) {
                        Ok(token) => token,
                        Err(_) => {
                            return Ok(Json(ApiResponse::<crate::api::LoginResponse>::error(
                                "生成令牌失败".to_string(),
                            )));
                        }
                    };

                    let user_id_for_sub = user.id;
                    let sub_info =
                        crate::api::build_subscription_info(&_state.db, user_id_for_sub).await;
                    let response = crate::api::LoginResponse {
                        token,
                        user: user.into(),
                        subscription: sub_info,
                    };

                    Ok(Json(ApiResponse::success(response)))
                }
                Ok(false) => Ok(Json(ApiResponse::<crate::api::LoginResponse>::error(
                    "用户名或密码错误".to_string(),
                ))),
                Err(_) => Ok(Json(ApiResponse::<crate::api::LoginResponse>::error(
                    "验证密码失败".to_string(),
                ))),
            }
        }
        Ok(None) => Ok(Json(ApiResponse::<crate::api::LoginResponse>::error(
            "用户不存在".to_string(),
        ))),
        Err(_) => Ok(Json(ApiResponse::<crate::api::LoginResponse>::error(
            "查询用户失败".to_string(),
        ))),
    }
}

pub async fn register(
    Extension(_state): Extension<ApiState>,
    Json(request): Json<RegisterRequest>,
) -> Result<Json<ApiResponse<UserInfo>>, StatusCode> {
    // 验证密码确认
    if request.password != request.confirm_password {
        return Ok(Json(ApiResponse::<UserInfo>::error(
            "两次输入的密码不一致".to_string(),
        )));
    }

    // 检查用户名是否已存在
    match _state.db.get_user_by_username(&request.username).await {
        Ok(Some(_)) => {
            return Ok(Json(ApiResponse::<UserInfo>::error(
                "用户名已存在".to_string(),
            )));
        }
        Ok(None) => {}
        Err(_) => {
            return Ok(Json(ApiResponse::<UserInfo>::error(
                "检查用户名失败".to_string(),
            )));
        }
    }

    // 检查邮箱是否已存在
    match _state.db.get_user_by_email(&request.email).await {
        Ok(Some(_)) => {
            return Ok(Json(ApiResponse::<UserInfo>::error(
                "邮箱已存在".to_string(),
            )));
        }
        Ok(None) => {}
        Err(_) => {
            return Ok(Json(ApiResponse::<UserInfo>::error(
                "检查邮箱失败".to_string(),
            )));
        }
    }

    let create_request = CreateUserRequest {
        username: request.username.clone(),
        email: request.email.clone(),
        password: request.password.clone(),
    };

    match _state.db.create_user(&create_request).await {
        Ok(user_id) => {
            // 获取刚创建的用户信息
            match _state.db.get_user_by_id(user_id).await {
                Ok(Some(user)) => Ok(Json(ApiResponse::success(user.into()))),
                Ok(None) => Ok(Json(ApiResponse::error("创建用户后查询失败".to_string()))),
                Err(_) => Ok(Json(ApiResponse::error("查询用户信息失败".to_string()))),
            }
        }
        Err(_) => Ok(Json(ApiResponse::error("注册失败".to_string()))),
    }
}

// 创建Web路由
pub fn create_web_router(db: Database, jwt_secret: String) -> Router {
    let state = ApiState { db, jwt_secret };

    Router::new()
        // 页面路由
        .route("/", get(login_page))
        .route("/login", get(login_page))
        .route("/register", get(register_page))
        .route("/forgot-password", get(forgot_password_page))
        .route("/reset-password", get(reset_password_page))
        .route("/dashboard", get(dashboard_page))
        .route("/devices", get(devices_page))
        .route("/users", get(users_page))
        .route("/monitor", get(monitor_page))
        .route("/subscription", get(subscription_page))
        .route("/admin/subscriptions", get(admin_subscriptions_page))
        // API路由
        .route("/api/login", post(login))
        .route("/api/register", post(register))
        .route("/api/forgot-password", post(forgot_password))
        .route("/api/reset-password", post(reset_password))
        .route(
            "/api/users",
            get(crate::api::list_users).post(crate::api::admin_create_user),
        )
        .route(
            "/api/users/:id",
            get(crate::api::get_user)
                .put(crate::api::update_user)
                .delete(crate::api::delete_user),
        )
        .route("/api/users/:id/role", put(crate::api::update_user_role))
        .route("/api/users/:id/devices", get(crate::api::get_user_devices))
        .route(
            "/api/users/:id/devices/:device_id",
            delete(crate::api::remove_device),
        )
        .route(
            "/api/devices/:device_id/owner",
            get(crate::api::get_device_owner),
        )
        .route("/api/devices", post(crate::device_api::add_device))
        .route(
            "/api/devices/:device_id",
            delete(crate::device_api::remove_device_by_id),
        )
        .route(
            "/api/monitor/connections",
            get(crate::api::monitor_connections),
        )
        .route("/api/monitor/stats", get(crate::api::monitor_stats))
        .route("/api/change-password", post(crate::password_reset::change_password))
        .route("/api/subscription/plans", get(crate::api::list_subscription_plans))
        .route("/api/subscription/my", get(crate::api::get_my_subscription))
        .route(
            "/api/admin/subscriptions",
            get(crate::api::admin_list_subscriptions).post(crate::api::admin_create_subscription),
        )
        .route(
            "/api/admin/subscriptions/:id",
            put(crate::api::admin_update_subscription)
                .delete(crate::api::admin_deactivate_subscription),
        )
        .layer(axum::middleware::from_fn(cors_middleware))
        .layer(axum::Extension(state))
}

// CORS中间件
async fn cors_middleware<B>(
    request: axum::http::Request<B>,
    next: axum::middleware::Next<B>,
) -> Result<axum::response::Response, StatusCode> {
    let mut response = next.run(request).await;

    let headers = response.headers_mut();
    headers.insert("Access-Control-Allow-Origin", "*".parse().unwrap());
    headers.insert(
        "Access-Control-Allow-Methods",
        "GET, POST, PUT, DELETE, OPTIONS".parse().unwrap(),
    );
    headers.insert(
        "Access-Control-Allow-Headers",
        "Content-Type, Authorization".parse().unwrap(),
    );

    Ok(response)
}
