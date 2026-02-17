use crate::{peer::PeerMap, rendezvous_server::list_punch_req_audits};
use axum::{
    extract::{Extension, Path, Query},
    http::{header, HeaderMap, StatusCode},
    response::{Html, IntoResponse, Redirect},
    routing::{delete, get, post, put},
    Json, Router,
};
use hbb_common::log;
use jsonwebtoken::{decode, encode, DecodingKey, EncodingKey, Header, Validation};
use serde_derive::{Deserialize, Serialize};
use std::{collections::HashSet, sync::Arc};
use tower_http::cors::CorsLayer;

#[derive(Clone)]
pub(crate) struct AppState {
    pm: PeerMap,
    jwt_secret: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Claims {
    sub: i64,
    username: String,
    role: String,
    exp: usize,
}

#[derive(Debug, Serialize)]
struct ApiError {
    message: String,
}

#[derive(Debug, Deserialize)]
struct LoginReq {
    username: String,
    password: String,
}

#[derive(Debug, Serialize)]
struct LoginResp {
    token: String,
    role: String,
    username: String,
    user_id: i64,
}

#[derive(Debug, Deserialize)]
struct CreateUserReq {
    username: String,
    password: String,
    role: Option<String>,
}

#[derive(Debug, Serialize)]
struct UserDto {
    id: i64,
    username: String,
    role: String,
    status: i64,
    created_at: String,
}

#[derive(Debug, Serialize)]
struct CurrentUserResp {
    id: i64,
    username: String,
    role: String,
    is_admin: bool,
}

#[derive(Debug, Serialize)]
struct ClientDto {
    id: String,
    created_at: String,
    status: Option<i64>,
    note: Option<String>,
    online: bool,
    last_seen_secs: Option<u64>,
    ip: String,
}

#[derive(Debug, Deserialize)]
struct AuditQuery {
    offset: Option<usize>,
    limit: Option<usize>,
}

#[derive(Debug, Serialize)]
struct AuditDto {
    timestamp: String,
    from_ip: String,
    to_ip: String,
    to_id: String,
}

#[derive(Debug, Serialize)]
struct GroupDto {
    id: i64,
    name: String,
    created_at: String,
}

#[derive(Debug, Deserialize)]
struct CreateGroupReq {
    name: String,
}

#[derive(Debug, Deserialize, Default)]
struct CompatLoginReq {
    username: Option<String>,
    password: Option<String>,
    id: Option<String>,
    uuid: Option<String>,
    #[serde(rename = "type")]
    req_type: Option<String>,
    #[serde(rename = "verificationCode")]
    verification_code: Option<String>,
    #[serde(rename = "tfaCode")]
    tfa_code: Option<String>,
    secret: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
struct PagingQuery {
    current: Option<usize>,
    #[serde(rename = "pageSize")]
    page_size: Option<usize>,
    status: Option<String>,
    accessible: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
struct AbPeersQuery {
    ab: Option<String>,
    current: Option<usize>,
    #[serde(rename = "pageSize")]
    page_size: Option<usize>,
}

#[derive(Debug, Deserialize)]
struct AbTagAddReq {
    name: String,
    color: Option<i64>,
}

#[derive(Debug, Deserialize)]
struct AbTagRenameReq {
    old: String,
    new: String,
}

#[derive(Debug, Deserialize)]
struct AbTagUpdateReq {
    name: String,
    color: i64,
}

#[derive(Debug, Deserialize)]
struct AuditUpdateReq {
    guid: Option<String>,
    note: Option<String>,
}

pub(crate) fn spawn_admin_http(pm: PeerMap, api_port: i32) {
    if api_port <= 0 || api_port > u16::MAX as i32 {
        log::error!("Invalid API port: {}", api_port);
        return;
    }
    let jwt_secret =
        std::env::var("ADMIN_JWT_SECRET").unwrap_or_else(|_| "change-this-secret".to_owned());
    let state = Arc::new(AppState { pm, jwt_secret });
    let app = Router::new()
        .route("/", get(index_redirect))
        .route("/admin", get(admin_redirect))
        .route("/admin/login", get(admin_login_page))
        .route("/admin/dashboard", get(admin_dashboard_page))
        .route("/admin/style.css", get(admin_style_css))
        .route("/admin/login.js", get(admin_login_js))
        .route("/admin/dashboard.js", get(admin_dashboard_js))
        .route("/api/health", get(health))
        .route("/api/login", post(compat_login))
        .route("/api/admin/login", post(login))
        .route("/api/currentUser", get(current_user).post(current_user_post))
        .route("/api/logout", post(logout))
        .route("/api/login-options", get(login_options))
        .route("/api/ab/settings", post(ab_settings))
        .route("/api/ab/personal", post(ab_personal))
        .route("/api/ab/shared/profiles", post(ab_shared_profiles))
        .route("/api/ab/peers", post(ab_peers))
        .route("/api/ab/tags/:guid", post(ab_tags))
        .route("/api/ab/peer/add/:guid", post(ab_peer_add))
        .route("/api/ab/peer/update/:guid", put(ab_peer_update))
        .route("/api/ab/peer/:guid", delete(ab_peer_delete))
        .route("/api/ab/tag/add/:guid", post(ab_tag_add))
        .route("/api/ab/tag/rename/:guid", put(ab_tag_rename))
        .route("/api/ab/tag/update/:guid", put(ab_tag_update))
        .route("/api/ab/tag/:guid", delete(ab_tag_delete))
        .route("/api/audit", put(update_audit_note))
        .route("/api/users", get(users_dispatch).post(create_user))
        .route("/api/users/:user_id", delete(delete_user))
        .route("/api/users/:user_id/enable", post(enable_user))
        .route("/api/users/:user_id/disable", post(disable_user))
        .route("/api/peers", get(peers_dispatch))
        .route("/api/peers/:peer_id", delete(delete_peer))
        .route("/api/peers/:peer_id/enable", post(enable_peer))
        .route("/api/peers/:peer_id/disable", post(disable_peer))
        .route("/api/users/:user_id/peers", get(list_user_peers))
        .route(
            "/api/users/:user_id/peers/:peer_id",
            post(grant_user_peer).delete(revoke_user_peer),
        )
        .route("/api/users/:user_id/groups", get(list_user_groups))
        .route(
            "/api/users/:user_id/groups/:group_id",
            post(grant_user_group).delete(revoke_user_group),
        )
        .route("/api/groups", get(list_groups).post(create_group))
        .route("/api/groups/:group_id", delete(delete_group))
        .route("/api/groups/:group_id/peers", get(list_group_peers))
        .route("/api/device-group/accessible", get(device_group_accessible))
        .route(
            "/api/groups/:group_id/peers/:peer_id",
            post(add_group_peer).delete(remove_group_peer),
        )
        .route("/api/audits/conn", get(list_conn_audits))
        // Compatibility routes for older API paths used in this repo
        .route("/api/admin/users", get(list_users).post(create_user))
        .route("/api/admin/clients", get(list_clients))
        .route(
            "/api/admin/users/:user_id/clients",
            get(list_user_peers),
        )
        .route(
            "/api/admin/users/:user_id/clients/:peer_id",
            post(grant_user_peer).delete(revoke_user_peer),
        )
        .layer(Extension(state))
        .layer(CorsLayer::permissive());
    hbb_common::tokio::spawn(async move {
        let addr = std::net::SocketAddr::from(([0, 0, 0, 0], api_port as u16));
        log::info!("Admin API listening on http://{}", addr);
        if let Err(err) = axum::Server::bind(&addr)
            .serve(app.into_make_service())
            .await
        {
            log::error!("Admin API stopped: {}", err);
        }
    });
}

async fn index_redirect() -> Redirect {
    Redirect::to("/admin/login")
}

async fn admin_redirect() -> Redirect {
    Redirect::to("/admin/login")
}

async fn admin_login_page() -> Html<&'static str> {
    Html(ADMIN_LOGIN_HTML)
}

async fn admin_dashboard_page() -> Html<&'static str> {
    Html(ADMIN_DASHBOARD_HTML)
}

async fn admin_style_css() -> impl IntoResponse {
    (
        [(header::CONTENT_TYPE, "text/css; charset=utf-8")],
        ADMIN_STYLE_CSS,
    )
}

async fn admin_login_js() -> impl IntoResponse {
    (
        [(header::CONTENT_TYPE, "application/javascript; charset=utf-8")],
        ADMIN_LOGIN_JS,
    )
}

async fn admin_dashboard_js() -> impl IntoResponse {
    (
        [(header::CONTENT_TYPE, "application/javascript; charset=utf-8")],
        ADMIN_DASHBOARD_JS,
    )
}

async fn health() -> Json<serde_json::Value> {
    Json(serde_json::json!({ "ok": true }))
}

async fn ensure_ab_profile_for_claims(
    state: &Arc<AppState>,
    claims: &Claims,
    guid: Option<&str>,
) -> Result<crate::database::AbProfileRecord, (StatusCode, Json<ApiError>)> {
    if let Some(guid) = guid {
        let profile = state
            .pm
            .db
            .get_ab_profile(guid)
            .await
            .map_err(internal_err)?;
        if let Some(p) = profile {
            if claims.role == "admin" || p.owner_user_id == claims.sub {
                return Ok(p);
            }
            return Err(auth_err("Permission denied"));
        }
    }
    let user = state
        .pm
        .db
        .get_user_by_id(claims.sub)
        .await
        .map_err(internal_err)?
        .ok_or_else(|| auth_err("Invalid user"))?;
    state
        .pm
        .db
        .ensure_personal_ab(user.id, &user.username)
        .await
        .map_err(internal_err)
}

async fn ab_settings(
    Extension(_state): Extension<Arc<AppState>>,
    _headers: HeaderMap,
) -> Json<serde_json::Value> {
    Json(serde_json::json!({ "max_peer_one_ab": 1000 }))
}

async fn ab_personal(
    Extension(state): Extension<Arc<AppState>>,
    headers: HeaderMap,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<ApiError>)> {
    let claims = auth_claims(&state, &headers)?;
    let p = ensure_ab_profile_for_claims(&state, &claims, None).await?;
    Ok(Json(serde_json::json!({ "guid": p.guid })))
}

async fn ab_shared_profiles(
    Extension(state): Extension<Arc<AppState>>,
    headers: HeaderMap,
    Query(q): Query<PagingQuery>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<ApiError>)> {
    let claims = auth_claims(&state, &headers)?;
    let rows = state
        .pm
        .db
        .list_shared_ab_profiles(claims.sub)
        .await
        .map_err(internal_err)?;
    let current = q.current.unwrap_or(1).max(1);
    let page_size = q.page_size.unwrap_or(100).clamp(1, 500);
    let start = (current - 1) * page_size;
    let data: Vec<serde_json::Value> = rows
        .iter()
        .skip(start)
        .take(page_size)
        .map(|r| {
            serde_json::json!({
                "guid": r.guid,
                "name": r.name,
                "owner": claims.username,
                "note": r.note.clone().unwrap_or_default(),
                "info": {},
                "rule": r.rule,
            })
        })
        .collect();
    Ok(Json(serde_json::json!({ "total": rows.len(), "data": data })))
}

async fn ab_peers(
    Extension(state): Extension<Arc<AppState>>,
    headers: HeaderMap,
    Query(q): Query<AbPeersQuery>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<ApiError>)> {
    let claims = auth_claims(&state, &headers)?;
    let profile = ensure_ab_profile_for_claims(&state, &claims, q.ab.as_deref()).await?;
    let rows = state
        .pm
        .db
        .list_ab_peers(&profile.guid)
        .await
        .map_err(internal_err)?;
    let current = q.current.unwrap_or(1).max(1);
    let page_size = q.page_size.unwrap_or(100).clamp(1, 500);
    let start = (current - 1) * page_size;
    let data: Vec<serde_json::Value> = rows
        .iter()
        .skip(start)
        .take(page_size)
        .map(|r| {
            serde_json::json!({
                "id": r.id,
                "hash": r.hash,
                "password": r.password,
                "username": r.username,
                "hostname": r.hostname,
                "platform": r.platform,
                "alias": r.alias,
                "tags": r.tags,
                "note": r.note,
                "same_server": r.same_server,
            })
        })
        .collect();
    Ok(Json(serde_json::json!({ "total": rows.len(), "data": data })))
}

async fn ab_tags(
    Extension(state): Extension<Arc<AppState>>,
    headers: HeaderMap,
    Path(guid): Path<String>,
) -> Result<Json<Vec<serde_json::Value>>, (StatusCode, Json<ApiError>)> {
    let claims = auth_claims(&state, &headers)?;
    let profile = ensure_ab_profile_for_claims(&state, &claims, Some(&guid)).await?;
    let rows = state
        .pm
        .db
        .list_ab_tags(&profile.guid)
        .await
        .map_err(internal_err)?;
    Ok(Json(
        rows.into_iter()
            .map(|t| serde_json::json!({ "name": t.name, "color": t.color }))
            .collect(),
    ))
}

async fn ab_peer_add(
    Extension(state): Extension<Arc<AppState>>,
    headers: HeaderMap,
    Path(guid): Path<String>,
    Json(body): Json<serde_json::Value>,
) -> Result<StatusCode, (StatusCode, Json<ApiError>)> {
    let claims = auth_claims(&state, &headers)?;
    let profile = ensure_ab_profile_for_claims(&state, &claims, Some(&guid)).await?;
    state
        .pm
        .db
        .add_ab_peer(&profile.guid, &body)
        .await
        .map_err(internal_err)?;
    Ok(StatusCode::OK)
}

async fn ab_peer_update(
    Extension(state): Extension<Arc<AppState>>,
    headers: HeaderMap,
    Path(guid): Path<String>,
    Json(body): Json<serde_json::Value>,
) -> Result<StatusCode, (StatusCode, Json<ApiError>)> {
    let claims = auth_claims(&state, &headers)?;
    let profile = ensure_ab_profile_for_claims(&state, &claims, Some(&guid)).await?;
    state
        .pm
        .db
        .update_ab_peer_partial(&profile.guid, &body)
        .await
        .map_err(internal_err)?;
    Ok(StatusCode::OK)
}

async fn ab_peer_delete(
    Extension(state): Extension<Arc<AppState>>,
    headers: HeaderMap,
    Path(guid): Path<String>,
    Json(ids): Json<Vec<String>>,
) -> Result<StatusCode, (StatusCode, Json<ApiError>)> {
    let claims = auth_claims(&state, &headers)?;
    let profile = ensure_ab_profile_for_claims(&state, &claims, Some(&guid)).await?;
    state
        .pm
        .db
        .delete_ab_peers(&profile.guid, &ids)
        .await
        .map_err(internal_err)?;
    Ok(StatusCode::OK)
}

async fn ab_tag_add(
    Extension(state): Extension<Arc<AppState>>,
    headers: HeaderMap,
    Path(guid): Path<String>,
    Json(req): Json<AbTagAddReq>,
) -> Result<StatusCode, (StatusCode, Json<ApiError>)> {
    let claims = auth_claims(&state, &headers)?;
    let profile = ensure_ab_profile_for_claims(&state, &claims, Some(&guid)).await?;
    state
        .pm
        .db
        .add_ab_tag(&profile.guid, req.name.trim(), req.color.unwrap_or(0))
        .await
        .map_err(internal_err)?;
    Ok(StatusCode::OK)
}

async fn ab_tag_rename(
    Extension(state): Extension<Arc<AppState>>,
    headers: HeaderMap,
    Path(guid): Path<String>,
    Json(req): Json<AbTagRenameReq>,
) -> Result<StatusCode, (StatusCode, Json<ApiError>)> {
    let claims = auth_claims(&state, &headers)?;
    let profile = ensure_ab_profile_for_claims(&state, &claims, Some(&guid)).await?;
    state
        .pm
        .db
        .rename_ab_tag(&profile.guid, req.old.trim(), req.new.trim())
        .await
        .map_err(internal_err)?;
    Ok(StatusCode::OK)
}

async fn ab_tag_update(
    Extension(state): Extension<Arc<AppState>>,
    headers: HeaderMap,
    Path(guid): Path<String>,
    Json(req): Json<AbTagUpdateReq>,
) -> Result<StatusCode, (StatusCode, Json<ApiError>)> {
    let claims = auth_claims(&state, &headers)?;
    let profile = ensure_ab_profile_for_claims(&state, &claims, Some(&guid)).await?;
    state
        .pm
        .db
        .update_ab_tag_color(&profile.guid, req.name.trim(), req.color)
        .await
        .map_err(internal_err)?;
    Ok(StatusCode::OK)
}

async fn ab_tag_delete(
    Extension(state): Extension<Arc<AppState>>,
    headers: HeaderMap,
    Path(guid): Path<String>,
    Json(tags): Json<Vec<String>>,
) -> Result<StatusCode, (StatusCode, Json<ApiError>)> {
    let claims = auth_claims(&state, &headers)?;
    let profile = ensure_ab_profile_for_claims(&state, &claims, Some(&guid)).await?;
    state
        .pm
        .db
        .delete_ab_tags(&profile.guid, &tags)
        .await
        .map_err(internal_err)?;
    Ok(StatusCode::OK)
}

async fn update_audit_note(
    Json(_req): Json<AuditUpdateReq>,
) -> StatusCode {
    StatusCode::OK
}
async fn login_options() -> Json<Vec<String>> {
    Json(vec![])
}

fn compat_user_payload(username: &str, role: &str, status: i64) -> serde_json::Value {
    serde_json::json!({
        "name": username,
        "display_name": username,
        "avatar": "",
        "email": "",
        "note": "",
        "status": status,
        "is_admin": role == "admin",
        "verifier": "",
        "info": {
            "email_verification": false,
            "email_alarm_notification": false,
            "login_device_whitelist": [],
            "other": {}
        }
    })
}

async fn compat_login(
    Extension(state): Extension<Arc<AppState>>,
    Json(req): Json<CompatLoginReq>,
) -> Json<serde_json::Value> {
    let req_type = req.req_type.unwrap_or_else(|| "account".to_owned());
    if req_type != "account" && req_type != "mobile" {
        return Json(serde_json::json!({ "error": "Unsupported login type" }));
    }
    let username = req.username.unwrap_or_default();
    let password = req.password.unwrap_or_default();
    if username.trim().is_empty() || password.is_empty() {
        return Json(serde_json::json!({ "error": "Invalid username or password" }));
    }
    let user = match state.pm.db.get_user_by_name(username.trim()).await {
        Ok(Some(v)) => v,
        _ => return Json(serde_json::json!({ "error": "Invalid username or password" })),
    };
    if user.status == 0 {
        return Json(serde_json::json!({ "error": "User is disabled" }));
    }
    if !bcrypt::verify(password, &user.password_hash).unwrap_or(false) {
        return Json(serde_json::json!({ "error": "Invalid username or password" }));
    }
    let claims = Claims {
        sub: user.id,
        username: user.username.clone(),
        role: user.role.clone(),
        exp: (chrono::Utc::now().timestamp() + 12 * 3600) as usize,
    };
    let token = match encode(
        &Header::default(),
        &claims,
        &EncodingKey::from_secret(state.jwt_secret.as_bytes()),
    ) {
        Ok(v) => v,
        Err(err) => return Json(serde_json::json!({ "error": err.to_string() })),
    };
    Json(serde_json::json!({
        "type": "access_token",
        "access_token": token,
        "user": compat_user_payload(&user.username, &user.role, user.status)
    }))
}

async fn current_user_post(
    Extension(state): Extension<Arc<AppState>>,
    headers: HeaderMap,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<ApiError>)> {
    let claims = auth_claims(&state, &headers)
        .map_err(|_| auth_err("Invalid token"))?;
    let user = state
        .pm
        .db
        .get_user_by_id(claims.sub)
        .await
        .map_err(internal_err)?;
    let user = match user {
        Some(v) => v,
        None => return Err(auth_err("Invalid token")),
    };
    Ok(Json(compat_user_payload(&user.username, &user.role, user.status)))
}

async fn users_dispatch(
    Extension(state): Extension<Arc<AppState>>,
    headers: HeaderMap,
    Query(q): Query<PagingQuery>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<ApiError>)> {
    let claims = auth_claims(&state, &headers)?;
    let users = if claims.role == "admin" {
        state.pm.db.list_users().await.map_err(internal_err)?
    } else {
        match state.pm.db.get_user_by_id(claims.sub).await.map_err(internal_err)? {
            Some(u) => vec![u],
            None => vec![],
        }
    };
    let users: Vec<_> = match q.status.as_deref() {
        Some("1") => users.into_iter().filter(|u| u.status == 1).collect(),
        Some("0") => users.into_iter().filter(|u| u.status == 0).collect(),
        _ => users,
    };

    if q.current.is_some() || q.page_size.is_some() {
        let current = q.current.unwrap_or(1).max(1);
        let page_size = q.page_size.unwrap_or(100).clamp(1, 500);
        let start = (current - 1) * page_size;
        let data: Vec<serde_json::Value> = users
            .iter()
            .skip(start)
            .take(page_size)
            .map(|u| compat_user_payload(&u.username, &u.role, u.status))
            .collect();
        return Ok(Json(serde_json::json!({ "total": users.len(), "data": data })));
    }
    let list: Vec<UserDto> = users
        .into_iter()
        .map(|u| UserDto {
            id: u.id,
            username: u.username,
            role: u.role,
            status: u.status,
            created_at: u.created_at,
        })
        .collect();
    Ok(Json(serde_json::to_value(list).unwrap_or_else(|_| serde_json::json!([]))))
}

async fn peers_dispatch(
    Extension(state): Extension<Arc<AppState>>,
    headers: HeaderMap,
    Query(q): Query<PagingQuery>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<ApiError>)> {
    let claims = auth_claims(&state, &headers)?;
    let rows = state.pm.db.list_peers().await.map_err(internal_err)?;
    let runtime = state.pm.get_runtime_status().await;
    let allow_set = if claims.role == "admin" {
        None
    } else {
        let direct = state
            .pm
            .db
            .list_user_client_acl(claims.sub)
            .await
            .map_err(internal_err)?;
        let grouped = state
            .pm
            .db
            .list_group_peers_for_user(claims.sub)
            .await
            .map_err(internal_err)?;
        let mut all = HashSet::new();
        all.extend(direct);
        all.extend(grouped);
        Some(all)
    };
    let filtered: Vec<_> = rows
        .into_iter()
        .filter(|r| match &allow_set {
            Some(ids) => ids.contains(&r.id),
            None => true,
        })
        .collect();

    let filtered: Vec<_> = match q.status.as_deref() {
        Some("1") => filtered.into_iter().filter(|r| r.status.unwrap_or(1) != 0).collect(),
        Some("0") => filtered.into_iter().filter(|r| r.status.unwrap_or(1) == 0).collect(),
        _ => filtered,
    };

    if q.current.is_some() || q.page_size.is_some() {
        let current = q.current.unwrap_or(1).max(1);
        let page_size = q.page_size.unwrap_or(100).clamp(1, 500);
        let start = (current - 1) * page_size;
        let data: Vec<serde_json::Value> = filtered
            .iter()
            .skip(start)
            .take(page_size)
            .map(|r| {
                let info = serde_json::from_str::<serde_json::Value>(&r.info)
                    .unwrap_or_else(|_| serde_json::json!({}));
                serde_json::json!({
                    "id": r.id,
                    "info": info,
                    "status": r.status.unwrap_or(1),
                    "user": "",
                    "user_name": "",
                    "device_group_name": "",
                    "note": r.note.clone().unwrap_or_default(),
                })
            })
            .collect();
        return Ok(Json(serde_json::json!({ "total": filtered.len(), "data": data })));
    }

    let out: Vec<ClientDto> = filtered
        .into_iter()
        .map(|r| {
            let rt = runtime.get(&r.id);
            ClientDto {
                id: r.id,
                created_at: r.created_at,
                status: r.status,
                note: r.note,
                online: rt.map(|x| x.online).unwrap_or(false),
                last_seen_secs: rt.map(|x| x.last_seen_secs),
                ip: rt.map(|x| x.ip.clone()).unwrap_or_default(),
            }
        })
        .collect();
    Ok(Json(serde_json::to_value(out).unwrap_or_else(|_| serde_json::json!([]))))
}

async fn device_group_accessible(
    Extension(state): Extension<Arc<AppState>>,
    headers: HeaderMap,
    Query(q): Query<PagingQuery>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<ApiError>)> {
    let claims = auth_claims(&state, &headers)?;
    let groups = state.pm.db.list_groups().await.map_err(internal_err)?;
    let visible: Vec<_> = if claims.role == "admin" {
        groups
    } else {
        let ids = state
            .pm
            .db
            .list_user_group_acl(claims.sub)
            .await
            .map_err(internal_err)?;
        let set: HashSet<i64> = ids.into_iter().collect();
        groups.into_iter().filter(|g| set.contains(&g.id)).collect()
    };
    let current = q.current.unwrap_or(1).max(1);
    let page_size = q.page_size.unwrap_or(100).clamp(1, 500);
    let start = (current - 1) * page_size;
    let data: Vec<serde_json::Value> = visible
        .iter()
        .skip(start)
        .take(page_size)
        .map(|g| serde_json::json!({"name": g.name}))
        .collect();
    Ok(Json(serde_json::json!({ "total": visible.len(), "data": data })))
}
async fn login(
    Extension(state): Extension<Arc<AppState>>,
    Json(req): Json<LoginReq>,
) -> Result<Json<LoginResp>, (StatusCode, Json<ApiError>)> {
    let user = state
        .pm
        .db
        .get_user_by_name(req.username.trim())
        .await
        .map_err(internal_err)?;
    let user = match user {
        Some(v) => v,
        None => return Err(auth_err("Invalid username or password")),
    };
    if user.status == 0 {
        return Err(auth_err("User is disabled"));
    }
    let ok = bcrypt::verify(req.password, &user.password_hash).unwrap_or(false);
    if !ok {
        return Err(auth_err("Invalid username or password"));
    }
    let claims = Claims {
        sub: user.id,
        username: user.username.clone(),
        role: user.role.clone(),
        exp: (chrono::Utc::now().timestamp() + 12 * 3600) as usize,
    };
    let token = encode(
        &Header::default(),
        &claims,
        &EncodingKey::from_secret(state.jwt_secret.as_bytes()),
    )
    .map_err(internal_err)?;
    Ok(Json(LoginResp {
        token,
        role: claims.role,
        username: claims.username,
        user_id: user.id,
    }))
}

async fn current_user(
    Extension(state): Extension<Arc<AppState>>,
    headers: HeaderMap,
) -> Result<Json<CurrentUserResp>, (StatusCode, Json<ApiError>)> {
    let claims = auth_claims(&state, &headers)?;
    Ok(Json(CurrentUserResp {
        id: claims.sub,
        username: claims.username,
        role: claims.role.clone(),
        is_admin: claims.role == "admin",
    }))
}

async fn logout() -> Json<serde_json::Value> {
    Json(serde_json::json!({ "ok": true }))
}

async fn list_users(
    Extension(state): Extension<Arc<AppState>>,
    headers: HeaderMap,
) -> Result<Json<Vec<UserDto>>, (StatusCode, Json<ApiError>)> {
    let claims = auth_claims(&state, &headers)?;
    require_admin(&claims)?;
    let users = state.pm.db.list_users().await.map_err(internal_err)?;
    Ok(Json(
        users
            .into_iter()
            .map(|u| UserDto {
                id: u.id,
                username: u.username,
                role: u.role,
                status: u.status,
                created_at: u.created_at,
            })
            .collect(),
    ))
}

async fn create_user(
    Extension(state): Extension<Arc<AppState>>,
    headers: HeaderMap,
    Json(req): Json<CreateUserReq>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<ApiError>)> {
    let claims = auth_claims(&state, &headers)?;
    require_admin(&claims)?;
    let username = req.username.trim();
    if username.len() < 3 || req.password.len() < 6 {
        return Err(bad_req("Username >=3 chars, password >=6 chars"));
    }
    let role = req.role.unwrap_or_else(|| "user".to_owned());
    if role != "admin" && role != "user" {
        return Err(bad_req("Role must be admin or user"));
    }
    let hash = bcrypt::hash(req.password, bcrypt::DEFAULT_COST).map_err(internal_err)?;
    state
        .pm
        .db
        .create_user(username, &hash, &role)
        .await
        .map_err(internal_err)?;
    Ok(Json(serde_json::json!({ "ok": true })))
}

async fn enable_user(
    Extension(state): Extension<Arc<AppState>>,
    headers: HeaderMap,
    Path(user_id): Path<i64>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<ApiError>)> {
    set_user_status(state, headers, user_id, 1).await
}

async fn disable_user(
    Extension(state): Extension<Arc<AppState>>,
    headers: HeaderMap,
    Path(user_id): Path<i64>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<ApiError>)> {
    set_user_status(state, headers, user_id, 0).await
}

async fn set_user_status(
    state: Arc<AppState>,
    headers: HeaderMap,
    user_id: i64,
    status: i64,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<ApiError>)> {
    let claims = auth_claims(&state, &headers)?;
    require_admin(&claims)?;
    if claims.sub == user_id && status == 0 {
        return Err(bad_req("Cannot disable current login user"));
    }
    let user = state.pm.db.get_user_by_id(user_id).await.map_err(internal_err)?;
    if user.is_none() {
        return Err(not_found("User not found"));
    }
    state
        .pm
        .db
        .set_user_status(user_id, status)
        .await
        .map_err(internal_err)?;
    Ok(Json(serde_json::json!({ "ok": true })))
}

async fn delete_user(
    Extension(state): Extension<Arc<AppState>>,
    headers: HeaderMap,
    Path(user_id): Path<i64>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<ApiError>)> {
    let claims = auth_claims(&state, &headers)?;
    require_admin(&claims)?;
    if claims.sub == user_id {
        return Err(bad_req("Cannot delete current login user"));
    }
    state
        .pm
        .db
        .delete_user(user_id)
        .await
        .map_err(internal_err)?;
    Ok(Json(serde_json::json!({ "ok": true })))
}

async fn list_clients(
    Extension(state): Extension<Arc<AppState>>,
    headers: HeaderMap,
) -> Result<Json<Vec<ClientDto>>, (StatusCode, Json<ApiError>)> {
    let claims = auth_claims(&state, &headers)?;
    let rows = state.pm.db.list_peers().await.map_err(internal_err)?;
    let runtime = state.pm.get_runtime_status().await;
    let allow_set = if claims.role == "admin" {
        None
    } else {
        let direct = state
            .pm
            .db
            .list_user_client_acl(claims.sub)
            .await
            .map_err(internal_err)?;
        let grouped = state
            .pm
            .db
            .list_group_peers_for_user(claims.sub)
            .await
            .map_err(internal_err)?;
        let mut all = HashSet::new();
        all.extend(direct);
        all.extend(grouped);
        Some(all)
    };
    let out = rows
        .into_iter()
        .filter(|r| match &allow_set {
            Some(ids) => ids.contains(&r.id),
            None => true,
        })
        .map(|r| {
            let rt = runtime.get(&r.id);
            ClientDto {
                id: r.id,
                created_at: r.created_at,
                status: r.status,
                note: r.note,
                online: rt.map(|x| x.online).unwrap_or(false),
                last_seen_secs: rt.map(|x| x.last_seen_secs),
                ip: rt.map(|x| x.ip.clone()).unwrap_or_default(),
            }
        })
        .collect();
    Ok(Json(out))
}

async fn enable_peer(
    Extension(state): Extension<Arc<AppState>>,
    headers: HeaderMap,
    Path(peer_id): Path<String>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<ApiError>)> {
    set_peer_status(state, headers, peer_id, 1).await
}

async fn disable_peer(
    Extension(state): Extension<Arc<AppState>>,
    headers: HeaderMap,
    Path(peer_id): Path<String>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<ApiError>)> {
    set_peer_status(state, headers, peer_id, 0).await
}

async fn set_peer_status(
    state: Arc<AppState>,
    headers: HeaderMap,
    peer_id: String,
    status: i64,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<ApiError>)> {
    let claims = auth_claims(&state, &headers)?;
    require_admin(&claims)?;
    state
        .pm
        .db
        .set_peer_status(peer_id.trim(), status)
        .await
        .map_err(internal_err)?;
    Ok(Json(serde_json::json!({ "ok": true })))
}

async fn delete_peer(
    Extension(state): Extension<Arc<AppState>>,
    headers: HeaderMap,
    Path(peer_id): Path<String>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<ApiError>)> {
    let claims = auth_claims(&state, &headers)?;
    require_admin(&claims)?;
    state
        .pm
        .db
        .delete_peer(peer_id.trim())
        .await
        .map_err(internal_err)?;
    Ok(Json(serde_json::json!({ "ok": true })))
}

async fn list_user_peers(
    Extension(state): Extension<Arc<AppState>>,
    headers: HeaderMap,
    Path(user_id): Path<i64>,
) -> Result<Json<Vec<String>>, (StatusCode, Json<ApiError>)> {
    let claims = auth_claims(&state, &headers)?;
    if claims.role != "admin" && claims.sub != user_id {
        return Err(auth_err("Permission denied"));
    }
    let ids = state
        .pm
        .db
        .list_user_client_acl(user_id)
        .await
        .map_err(internal_err)?;
    Ok(Json(ids))
}

async fn grant_user_peer(
    Extension(state): Extension<Arc<AppState>>,
    headers: HeaderMap,
    Path((user_id, peer_id)): Path<(i64, String)>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<ApiError>)> {
    let claims = auth_claims(&state, &headers)?;
    require_admin(&claims)?;
    state
        .pm
        .db
        .grant_user_client_acl(user_id, peer_id.trim())
        .await
        .map_err(internal_err)?;
    Ok(Json(serde_json::json!({ "ok": true })))
}

async fn revoke_user_peer(
    Extension(state): Extension<Arc<AppState>>,
    headers: HeaderMap,
    Path((user_id, peer_id)): Path<(i64, String)>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<ApiError>)> {
    let claims = auth_claims(&state, &headers)?;
    require_admin(&claims)?;
    state
        .pm
        .db
        .revoke_user_client_acl(user_id, peer_id.trim())
        .await
        .map_err(internal_err)?;
    Ok(Json(serde_json::json!({ "ok": true })))
}

async fn list_user_groups(
    Extension(state): Extension<Arc<AppState>>,
    headers: HeaderMap,
    Path(user_id): Path<i64>,
) -> Result<Json<Vec<i64>>, (StatusCode, Json<ApiError>)> {
    let claims = auth_claims(&state, &headers)?;
    if claims.role != "admin" && claims.sub != user_id {
        return Err(auth_err("Permission denied"));
    }
    let ids = state
        .pm
        .db
        .list_user_group_acl(user_id)
        .await
        .map_err(internal_err)?;
    Ok(Json(ids))
}

async fn grant_user_group(
    Extension(state): Extension<Arc<AppState>>,
    headers: HeaderMap,
    Path((user_id, group_id)): Path<(i64, i64)>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<ApiError>)> {
    let claims = auth_claims(&state, &headers)?;
    require_admin(&claims)?;
    state
        .pm
        .db
        .grant_user_group_acl(user_id, group_id)
        .await
        .map_err(internal_err)?;
    Ok(Json(serde_json::json!({ "ok": true })))
}

async fn revoke_user_group(
    Extension(state): Extension<Arc<AppState>>,
    headers: HeaderMap,
    Path((user_id, group_id)): Path<(i64, i64)>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<ApiError>)> {
    let claims = auth_claims(&state, &headers)?;
    require_admin(&claims)?;
    state
        .pm
        .db
        .revoke_user_group_acl(user_id, group_id)
        .await
        .map_err(internal_err)?;
    Ok(Json(serde_json::json!({ "ok": true })))
}

async fn list_groups(
    Extension(state): Extension<Arc<AppState>>,
    headers: HeaderMap,
) -> Result<Json<Vec<GroupDto>>, (StatusCode, Json<ApiError>)> {
    let claims = auth_claims(&state, &headers)?;
    require_admin(&claims)?;
    let rows = state.pm.db.list_groups().await.map_err(internal_err)?;
    Ok(Json(
        rows.into_iter()
            .map(|g| GroupDto {
                id: g.id,
                name: g.name,
                created_at: g.created_at,
            })
            .collect(),
    ))
}

async fn create_group(
    Extension(state): Extension<Arc<AppState>>,
    headers: HeaderMap,
    Json(req): Json<CreateGroupReq>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<ApiError>)> {
    let claims = auth_claims(&state, &headers)?;
    require_admin(&claims)?;
    let name = req.name.trim();
    if name.len() < 2 {
        return Err(bad_req("Group name must be at least 2 chars"));
    }
    state.pm.db.create_group(name).await.map_err(internal_err)?;
    Ok(Json(serde_json::json!({ "ok": true })))
}

async fn delete_group(
    Extension(state): Extension<Arc<AppState>>,
    headers: HeaderMap,
    Path(group_id): Path<i64>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<ApiError>)> {
    let claims = auth_claims(&state, &headers)?;
    require_admin(&claims)?;
    state.pm.db.delete_group(group_id).await.map_err(internal_err)?;
    Ok(Json(serde_json::json!({ "ok": true })))
}

async fn list_group_peers(
    Extension(state): Extension<Arc<AppState>>,
    headers: HeaderMap,
    Path(group_id): Path<i64>,
) -> Result<Json<Vec<String>>, (StatusCode, Json<ApiError>)> {
    let claims = auth_claims(&state, &headers)?;
    require_admin(&claims)?;
    let rows = state
        .pm
        .db
        .list_group_peers(group_id)
        .await
        .map_err(internal_err)?;
    Ok(Json(rows))
}

async fn add_group_peer(
    Extension(state): Extension<Arc<AppState>>,
    headers: HeaderMap,
    Path((group_id, peer_id)): Path<(i64, String)>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<ApiError>)> {
    let claims = auth_claims(&state, &headers)?;
    require_admin(&claims)?;
    state
        .pm
        .db
        .add_group_peer(group_id, peer_id.trim())
        .await
        .map_err(internal_err)?;
    Ok(Json(serde_json::json!({ "ok": true })))
}

async fn remove_group_peer(
    Extension(state): Extension<Arc<AppState>>,
    headers: HeaderMap,
    Path((group_id, peer_id)): Path<(i64, String)>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<ApiError>)> {
    let claims = auth_claims(&state, &headers)?;
    require_admin(&claims)?;
    state
        .pm
        .db
        .remove_group_peer(group_id, peer_id.trim())
        .await
        .map_err(internal_err)?;
    Ok(Json(serde_json::json!({ "ok": true })))
}
async fn list_conn_audits(
    Extension(state): Extension<Arc<AppState>>,
    headers: HeaderMap,
    Query(q): Query<AuditQuery>,
) -> Result<Json<Vec<AuditDto>>, (StatusCode, Json<ApiError>)> {
    let claims = auth_claims(&state, &headers)?;
    require_admin(&claims)?;
    let rows = list_punch_req_audits(q.offset.unwrap_or(0), q.limit.unwrap_or(50)).await;
    Ok(Json(
        rows
            .into_iter()
            .map(|x| AuditDto {
                timestamp: x.timestamp,
                from_ip: x.from_ip,
                to_ip: x.to_ip,
                to_id: x.to_id,
            })
            .collect(),
    ))
}

fn auth_claims(
    state: &AppState,
    headers: &HeaderMap,
) -> Result<Claims, (StatusCode, Json<ApiError>)> {
    let token = headers
        .get("token")
        .or_else(|| headers.get(header::AUTHORIZATION))
        .and_then(|v| v.to_str().ok())
        .map(|v| v.strip_prefix("Bearer ").unwrap_or(v))
        .ok_or_else(|| auth_err("Missing token"))?;
    let data = decode::<Claims>(
        token,
        &DecodingKey::from_secret(state.jwt_secret.as_bytes()),
        &Validation::default(),
    )
    .map_err(|_| auth_err("Invalid token"))?;
    Ok(data.claims)
}

fn require_admin(claims: &Claims) -> Result<(), (StatusCode, Json<ApiError>)> {
    if claims.role != "admin" {
        return Err(auth_err("Admin only"));
    }
    Ok(())
}

fn internal_err<E: std::fmt::Display>(err: E) -> (StatusCode, Json<ApiError>) {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(ApiError {
            message: err.to_string(),
        }),
    )
}

fn auth_err(msg: &str) -> (StatusCode, Json<ApiError>) {
    (
        StatusCode::UNAUTHORIZED,
        Json(ApiError {
            message: msg.to_owned(),
        }),
    )
}

fn bad_req(msg: &str) -> (StatusCode, Json<ApiError>) {
    (
        StatusCode::BAD_REQUEST,
        Json(ApiError {
            message: msg.to_owned(),
        }),
    )
}

fn not_found(msg: &str) -> (StatusCode, Json<ApiError>) {
    (
        StatusCode::NOT_FOUND,
        Json(ApiError {
            message: msg.to_owned(),
        }),
    )
}

const ADMIN_LOGIN_HTML: &str = r##"<!doctype html>
<html lang="en">
<head>
  <meta charset="UTF-8" />
  <meta name="viewport" content="width=device-width, initial-scale=1.0" />
  <title>RustDesk Console Login</title>
  <link rel="stylesheet" href="/admin/style.css" />
</head>
<body class="bg-grid">
  <main class="login-wrap">
    <section class="glass card login-card">
      <h1>RustDesk Console</h1>
      <p class="subtle">Use your admin or delegated user account.</p>
      <div class="row">
        <label>Username</label>
        <input id="username" placeholder="Enter username" value="admin" />
      </div>
      <div class="row">
        <label>Password</label>
        <input id="password" placeholder="Enter password" type="password" />
      </div>
      <div class="row">
        <button id="loginBtn" class="btn-primary w-full">Sign in</button>
      </div>
      <div id="loginMsg" class="hint"></div>
      <p class="subtle tiny">Default account: admin / admin123456</p>
    </section>
  </main>
  <script src="/admin/login.js"></script>
</body>
</html>
"##;

const ADMIN_DASHBOARD_HTML: &str = r##"<!doctype html>
<html lang="zh-CN">
<head>
  <meta charset="UTF-8" />
  <meta name="viewport" content="width=device-width, initial-scale=1.0" />
  <title>RustDesk Workplace</title>
  <link rel="stylesheet" href="/admin/style.css" />
</head>
<body class="work-bg">
  <header class="topbar glass">
    <div>
      <h1 class="brand">RustDesk Workplace</h1>
      <p class="subtle" id="welcomeText">加载中...</p>
    </div>
    <div class="row-inline">
      <button id="refreshAllBtn" class="btn-primary">刷新全局</button>
      <button id="refreshAuditsBtn" class="btn-ghost">刷新审计</button>
      <button id="logoutBtn" class="btn-ghost">退出登录</button>
    </div>
  </header>

  <main class="workplace">
    <section class="hero-card glass card">
      <div>
        <h2>工作台</h2>
        <p class="subtle">用户、设备、连接审计统一看板</p>
      </div>
      <div class="hero-meta">
        <div class="meta-item"><span>当前时间</span><strong id="clockText">--:--:--</strong></div>
        <div class="meta-item"><span>API</span><strong>/api</strong></div>
      </div>
    </section>

    <section class="kpi-grid">
      <article class="kpi-card glass">
        <p>用户总数</p>
        <h3 id="kpiUsers">0</h3>
      </article>
      <article class="kpi-card glass">
        <p>启用用户</p>
        <h3 id="kpiUsersEnabled">0</h3>
      </article>
      <article class="kpi-card glass">
        <p>设备总数</p>
        <h3 id="kpiPeers">0</h3>
      </article>
      <article class="kpi-card glass">
        <p>在线设备</p>
        <h3 id="kpiPeersOnline">0</h3>
      </article>
    </section>

    <section class="panel-grid">
      <section class="card glass panel" id="groupCard">
        <div class="panel-head">
          <h2>设备组管理</h2>
          <span class="subtle tiny">创建组、添加设备、授权给用户</span>
        </div>
        <div class="row row-inline">
          <input id="newGroup" placeholder="设备组名称" />
          <button id="createGroupBtn" class="btn-primary">创建设备组</button>
        </div>
        <div class="table-wrap">
          <table id="groupsTbl">
            <thead><tr><th>ID</th><th>名称</th><th>成员设备</th><th>创建时间</th><th>操作</th></tr></thead>
            <tbody></tbody>
          </table>
        </div>
      </section>
      <section class="card glass panel" id="userCard">
        <div class="panel-head">
          <h2>用户管理</h2>
          <span class="subtle tiny">创建、启停、删除、授权</span>
        </div>
        <div class="row row-inline">
          <input id="newUser" placeholder="新用户名" />
          <input id="newPass" placeholder="新密码" type="password" />
          <select id="newRole">
            <option value="user">user</option>
            <option value="admin">admin</option>
          </select>
          <button id="createUserBtn" class="btn-primary">创建用户</button>
        </div>
        <div class="table-wrap">
          <table id="usersTbl">
            <thead><tr><th>ID</th><th>用户名</th><th>角色</th><th>状态</th><th>创建时间</th><th>设备授权</th><th>设备组授权</th><th>操作</th></tr></thead>
            <tbody></tbody>
          </table>
        </div>
      </section>

      <section class="card glass panel">
        <div class="panel-head">
          <h2>设备管理</h2>
          <span class="subtle tiny">在线态 + DB 状态</span>
        </div>
        <div class="table-wrap">
          <table id="clientsTbl">
            <thead><tr><th>设备ID</th><th>在线状态</th><th>DB状态</th><th>最近心跳(秒)</th><th>IP</th><th>创建时间</th><th>操作</th></tr></thead>
            <tbody></tbody>
          </table>
        </div>
      </section>

      <section class="card glass panel" id="auditCard">
        <div class="panel-head">
          <h2>连接审计</h2>
          <span class="subtle tiny">最近打洞请求记录</span>
        </div>
        <div class="table-wrap">
          <table id="auditTbl">
            <thead><tr><th>时间(UTC)</th><th>来源IP</th><th>目标设备ID</th><th>目标IP</th></tr></thead>
            <tbody></tbody>
          </table>
        </div>
      </section>
    </section>
  </main>
  <script src="/admin/dashboard.js"></script>
</body>
</html>
"##;

const ADMIN_STYLE_CSS: &str = r##"
:root {
  --bg: #f3f6fb;
  --ink: #1f2a37;
  --muted: #6b7280;
  --line: #e5eaf3;
  --card: rgba(255,255,255,0.9);
  --ok: #0f766e;
  --warn: #b45309;
  --danger: #dc2626;
  --accent: #1677ff;
  --accent-2: #4096ff;
}
* { box-sizing: border-box; }
body {
  margin: 0;
  color: var(--ink);
  font: 14px/1.6 "Segoe UI", "PingFang SC", "Microsoft YaHei", sans-serif;
}
.work-bg {
  background:
    radial-gradient(800px 340px at 90% -10%, rgba(22,119,255,.17), transparent 60%),
    radial-gradient(680px 280px at -10% -20%, rgba(34,197,94,.09), transparent 60%),
    var(--bg);
  min-height: 100vh;
}
.glass {
  background: var(--card);
  border: 1px solid rgba(255,255,255,.7);
  box-shadow: 0 10px 28px rgba(15,23,42,.08);
  backdrop-filter: blur(5px);
}
.card { border-radius: 14px; padding: 16px; }
.brand { margin: 0; font-size: 24px; letter-spacing: .2px; }
h2 { margin: 0; font-size: 18px; }
h3 { margin: 0; font-size: 28px; line-height: 1.2; }
.subtle { margin: 0; color: var(--muted); }
.tiny { font-size: 12px; }
.row { margin-bottom: 12px; }
.row-inline { display: flex; flex-wrap: wrap; gap: 8px; }
input, select, button {
  border-radius: 10px;
  border: 1px solid var(--line);
  padding: 8px 10px;
  font-size: 14px;
  background: #fff;
}
input, select { min-width: 120px; }
button { cursor: pointer; }
.btn-primary {
  border: none;
  color: #fff;
  background: linear-gradient(135deg, var(--accent), var(--accent-2));
}
.btn-ghost {
  background: #fff;
  color: var(--ink);
}
.btn-danger {
  border: none;
  color: #fff;
  background: linear-gradient(135deg, #ef4444, #dc2626);
}
.topbar {
  margin: 16px;
  padding: 14px 16px;
  border-radius: 14px;
  display: flex;
  align-items: center;
  justify-content: space-between;
}
.workplace {
  margin: 16px;
  display: grid;
  gap: 16px;
}
.hero-card {
  display: flex;
  align-items: center;
  justify-content: space-between;
}
.hero-meta {
  display: grid;
  grid-template-columns: repeat(2, minmax(120px, 1fr));
  gap: 10px;
}
.meta-item {
  background: #f8fbff;
  border: 1px solid var(--line);
  border-radius: 10px;
  padding: 8px 10px;
}
.meta-item span { display: block; color: var(--muted); font-size: 12px; }
.meta-item strong { font-size: 14px; }
.kpi-grid {
  display: grid;
  grid-template-columns: repeat(5, minmax(0, 1fr));
  gap: 12px;
}
.kpi-card {
  border-radius: 14px;
  padding: 14px;
}
.kpi-card p { margin: 0 0 8px; color: var(--muted); }
.panel-grid {
  display: grid;
  grid-template-columns: 1fr;
  gap: 16px;
}
.panel-head {
  margin-bottom: 12px;
  display: flex;
  align-items: center;
  justify-content: space-between;
}
.table-wrap { overflow: auto; border: 1px solid var(--line); border-radius: 10px; background: #fff; }
table {
  width: 100%;
  border-collapse: collapse;
  min-width: 860px;
}
th, td {
  text-align: left;
  border-bottom: 1px solid var(--line);
  padding: 10px 8px;
}
th {
  position: sticky;
  top: 0;
  z-index: 1;
  background: #f8fbff;
  font-weight: 600;
  color: #334155;
}
.status-pill {
  display: inline-block;
  padding: 2px 8px;
  border-radius: 99px;
  font-size: 12px;
  font-weight: 700;
}
.status-online { background: #dcfce7; color: #166534; }
.status-offline { background: #fef3c7; color: #92400e; }
@media (max-width: 1160px) {
  .kpi-grid { grid-template-columns: repeat(2, minmax(0, 1fr)); }
}
@media (max-width: 840px) {
  .topbar, .hero-card { flex-direction: column; align-items: flex-start; gap: 10px; }
  .kpi-grid { grid-template-columns: 1fr; }
  .hero-meta { width: 100%; grid-template-columns: 1fr 1fr; }
}
"##;

const ADMIN_LOGIN_JS: &str = r##"(() => {
  const q = (s) => document.querySelector(s);
  const msg = (text, cls) => { const el = q("#loginMsg"); el.className = `hint ${cls || ""}`; el.textContent = text || ""; };
  localStorage.removeItem("adminToken");
  localStorage.removeItem("adminUser");
  q("#loginBtn").onclick = async () => {
    try {
      const res = await fetch("/api/admin/login", {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({
          username: q("#username").value.trim(),
          password: q("#password").value
        })
      });
      const data = await res.json().catch(() => ({}));
      if (!res.ok) throw new Error(data.message || `HTTP ${res.status}`);
      localStorage.setItem("adminToken", data.token || data.access_token || "");
      localStorage.setItem("adminUser", JSON.stringify({ user_id: data.user_id, username: data.username, role: data.role }));
      msg("Login successful, redirecting...", "ok");
      location.href = "/admin/dashboard";
    } catch (e) {
      msg(`Login failed: ${e.message}`, "err");
    }
  };
})();
"##;

const ADMIN_DASHBOARD_JS: &str = r##"(() => {
  const q = (s) => document.querySelector(s);
  const token = localStorage.getItem("adminToken") || "";
  const me = JSON.parse(localStorage.getItem("adminUser") || "{}");
  if (!token) location.href = "/admin/login";

  q("#welcomeText").textContent = `${me.username || "unknown"} (${me.role || "-"})`;

  function tickClock() {
    const now = new Date();
    q("#clockText").textContent = now.toLocaleTimeString();
  }
  tickClock();
  setInterval(tickClock, 1000);

  async function api(url, method = "GET", body) {
    const res = await fetch(url, {
      method,
      headers: {
        "Content-Type": "application/json",
        token,
      },
      body: body ? JSON.stringify(body) : undefined,
    });
    const data = await res.json().catch(() => ({}));
    if (res.status === 401) {
      localStorage.removeItem("adminToken");
      localStorage.removeItem("adminUser");
      location.href = "/admin/login";
      throw new Error("Unauthorized");
    }
    if (!res.ok) throw new Error(data.message || `HTTP ${res.status}`);
    return data;
  }

  const esc = (s) => String(s || "").replace(/[&<>"']/g, (c) => ({ "&":"&amp;","<":"&lt;",">":"&gt;","\"":"&quot;","'":"&#39;" }[c]));

  let usersCache = [];
  let peersCache = [];
  let groupsCache = [];

  function renderKpis() {
    const enabledUsers = usersCache.filter((u) => Number(u.status) !== 0).length;
    const onlinePeers = peersCache.filter((p) => !!p.online).length;
    q("#kpiUsers").textContent = String(usersCache.length);
    q("#kpiUsersEnabled").textContent = String(enabledUsers);
    q("#kpiPeers").textContent = String(peersCache.length);
    q("#kpiPeersOnline").textContent = String(onlinePeers);
    const g = q("#kpiGroups");
    if (g) g.textContent = String(groupsCache.length);
  }

  async function loadGroups() {
    if (me.role !== "admin") {
      const card = q("#groupCard");
      if (card) card.style.display = "none";
      groupsCache = [];
      renderKpis();
      return;
    }
    groupsCache = await api("/api/groups").catch(() => []);
    const tbody = q("#groupsTbl tbody");
    if (!tbody) {
      renderKpis();
      return;
    }
    tbody.innerHTML = "";
    for (const g of groupsCache) {
      const members = await api(`/api/groups/${g.id}/peers`).catch(() => []);
      const tr = document.createElement("tr");
      tr.innerHTML = `<td>${g.id}</td><td>${esc(g.name)}</td><td>${esc(members.join(", "))}</td><td>${esc(g.created_at)}</td><td>
        <input data-group="${g.id}" class="groupPeer" placeholder="设备ID" style="width:120px" />
        <button class="btn-primary" data-group="${g.id}" data-act="groupAddPeer">加设备</button>
        <button data-group="${g.id}" data-act="groupRmPeer">移设备</button>
        <button class="btn-danger" data-group="${g.id}" data-act="groupDelete">删组</button>
      </td>`;
      tbody.appendChild(tr);
    }
    renderKpis();
  }

  async function loadUsers() {
    if (me.role !== "admin") {
      q("#userCard").style.display = "none";
      usersCache = [];
      renderKpis();
      return;
    }
    usersCache = await api("/api/users").catch(() => []);
    const tbody = q("#usersTbl tbody");
    tbody.innerHTML = "";
    for (const u of usersCache) {
      const acl = await api(`/api/users/${u.id}/peers`).catch(() => []);
      const gAcl = await api(`/api/users/${u.id}/groups`).catch(() => []);
      const tr = document.createElement("tr");
      tr.innerHTML = `<td>${u.id}</td><td>${esc(u.username)}</td><td>${esc(u.role)}</td><td>${Number(u.status) === 0 ? "disabled" : "enabled"}</td><td>${esc(u.created_at)}</td><td>${esc(acl.join(", "))}</td><td>${esc(gAcl.join(", "))}</td><td>
        <input data-user="${u.id}" class="aclPeer" placeholder="设备ID" style="width:110px" />
        <button class="btn-primary" data-user="${u.id}" data-act="grant">授权设备</button>
        <button data-user="${u.id}" data-act="revoke">撤销设备</button>
        <input data-user="${u.id}" class="aclGroup" placeholder="组ID" style="width:80px" />
        <button class="btn-primary" data-user="${u.id}" data-act="grantGroup">授权组</button>
        <button data-user="${u.id}" data-act="revokeGroup">撤销组</button>
        <button data-user="${u.id}" data-act="enable">启用</button>
        <button data-user="${u.id}" data-act="disable">禁用</button>
        <button data-user="${u.id}" data-act="delete" class="btn-danger">删除</button>
      </td>`;
      tbody.appendChild(tr);
    }
    renderKpis();
  }

  async function loadClients() {
    peersCache = await api("/api/peers").catch(() => []);
    const tbody = q("#clientsTbl tbody");
    tbody.innerHTML = "";
    for (const c of peersCache) {
      const statusClass = c.online ? "status-online" : "status-offline";
      const statusText = c.online ? "online" : "offline";
      const dbStatus = Number(c.status) === 0 ? "disabled" : "enabled";
      const actions = me.role === "admin"
        ? `<button data-peer="${esc(c.id)}" data-act="peer-enable">启用</button>
           <button data-peer="${esc(c.id)}" data-act="peer-disable">禁用</button>
           <button data-peer="${esc(c.id)}" data-act="peer-delete" class="btn-danger">删除</button>`
        : "-";
      const tr = document.createElement("tr");
      tr.innerHTML = `<td>${esc(c.id)}</td><td><span class="status-pill ${statusClass}">${statusText}</span></td><td>${dbStatus}</td><td>${c.last_seen_secs ?? "-"}</td><td>${esc(c.ip || "-")}</td><td>${esc(c.created_at)}</td><td>${actions}</td>`;
      tbody.appendChild(tr);
    }
    renderKpis();
  }

  async function loadAudits() {
    if (me.role !== "admin") {
      q("#auditCard").style.display = "none";
      return;
    }
    const rows = await api("/api/audits/conn?limit=100").catch(() => []);
    const tbody = q("#auditTbl tbody");
    tbody.innerHTML = "";
    for (const a of rows) {
      const tr = document.createElement("tr");
      tr.innerHTML = `<td>${esc(a.timestamp)}</td><td>${esc(a.from_ip)}</td><td>${esc(a.to_id)}</td><td>${esc(a.to_ip)}</td>`;
      tbody.appendChild(tr);
    }
  }

  async function refreshAll() {
    await loadGroups();
    await loadUsers();
    await loadClients();
    await loadAudits();
  }

  q("#createGroupBtn").onclick = async () => {
    try {
      await api("/api/groups", "POST", { name: q("#newGroup").value.trim() });
      q("#newGroup").value = "";
      await loadGroups();
      await loadUsers();
    } catch (e) {
      alert(e.message);
    }
  };

  q("#groupsTbl").onclick = async (e) => {
    const btn = e.target.closest("button[data-act]");
    if (!btn) return;
    const groupId = btn.getAttribute("data-group");
    const act = btn.getAttribute("data-act");
    try {
      if (act === "groupDelete") {
        if (!confirm(`确认删除设备组 ${groupId} 吗？`)) return;
        await api(`/api/groups/${groupId}`, "DELETE");
      } else {
        const input = q(`input.groupPeer[data-group="${groupId}"]`);
        const peerId = (input?.value || "").trim();
        if (!peerId) return alert("请填写设备ID");
        if (act === "groupAddPeer") {
          await api(`/api/groups/${groupId}/peers/${encodeURIComponent(peerId)}`, "POST");
        } else if (act === "groupRmPeer") {
          await api(`/api/groups/${groupId}/peers/${encodeURIComponent(peerId)}`, "DELETE");
        }
      }
      await loadGroups();
      await loadUsers();
      await loadClients();
      await loadGroups();
      await loadUsers();
    } catch (e) {
      alert(e.message);
    }
  };
  q("#createUserBtn").onclick = async () => {
    try {
      await api("/api/users", "POST", {
        username: q("#newUser").value.trim(),
        password: q("#newPass").value,
        role: q("#newRole").value,
      });
      await loadUsers();
    } catch (e) {
      alert(e.message);
    }
  };

  q("#usersTbl").onclick = async (e) => {
    const btn = e.target.closest("button[data-act]");
    if (!btn) return;
    const userId = btn.getAttribute("data-user");
    const act = btn.getAttribute("data-act");
    try {
      if (act === "grant" || act === "revoke") {
        const input = q(`input.aclPeer[data-user="${userId}"]`);
        const peerId = (input?.value || "").trim();
        if (!peerId) return alert("请填写设备ID");
        await api(`/api/users/${userId}/peers/${encodeURIComponent(peerId)}`, act === "grant" ? "POST" : "DELETE");
      } else if (act === "grantGroup" || act === "revokeGroup") {
        const input = q(`input.aclGroup[data-user="${userId}"]`);
        const groupId = (input?.value || "").trim();
        if (!groupId) return alert("请填写组ID");
        await api(`/api/users/${userId}/groups/${encodeURIComponent(groupId)}`, act === "grantGroup" ? "POST" : "DELETE");
      } else if (act === "enable") {
        await api(`/api/users/${userId}/enable`, "POST");
      } else if (act === "disable") {
        await api(`/api/users/${userId}/disable`, "POST");
      } else if (act === "delete") {
        if (!confirm(`确认删除用户 ${userId} 吗？`)) return;
        await api(`/api/users/${userId}`, "DELETE");
      }
      await loadUsers();
      await loadGroups();
      await loadClients();
    } catch (e) {
      alert(e.message);
    }
  };

  q("#clientsTbl").onclick = async (e) => {
    const btn = e.target.closest("button[data-act]");
    if (!btn) return;
    const peerId = btn.getAttribute("data-peer");
    const act = btn.getAttribute("data-act");
    try {
      if (act === "peer-enable") await api(`/api/peers/${encodeURIComponent(peerId)}/enable`, "POST");
      if (act === "peer-disable") await api(`/api/peers/${encodeURIComponent(peerId)}/disable`, "POST");
      if (act === "peer-delete") {
        if (!confirm(`确认删除设备 ${peerId} 吗？`)) return;
        await api(`/api/peers/${encodeURIComponent(peerId)}`, "DELETE");
      }
      await loadClients();
      await loadGroups();
      await loadUsers();
    } catch (e) {
      alert(e.message);
    }
  };

  q("#refreshAllBtn").onclick = refreshAll;
  q("#refreshAuditsBtn").onclick = loadAudits;

  q("#logoutBtn").onclick = () => {
    localStorage.removeItem("adminToken");
    localStorage.removeItem("adminUser");
    location.href = "/admin/login";
  };

  (async () => {
    await refreshAll();
    setInterval(loadClients, 5000);
  })();
})();
"##;





































