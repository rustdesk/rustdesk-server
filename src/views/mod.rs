// Views module
use askama::Template;
use serde::Serialize;

#[derive(Serialize, Clone)]
pub struct UserInfo {
    pub id: i64,
    pub username: String,
    pub email: String,
    pub role: String,
}

#[derive(Template)]
#[template(path = "layout.html")]
pub struct LayoutTemplate {
    pub title: String,
    pub current_user: Option<UserInfo>,
}

#[derive(Template)]
#[template(path = "login.html")]
pub struct LoginTemplate {
    pub title: String,
    pub current_user: Option<UserInfo>,
}

#[derive(Template)]
#[template(path = "register.html")]
pub struct RegisterTemplate {
    pub title: String,
    pub current_user: Option<UserInfo>,
}

#[derive(Template)]
#[template(path = "forgot_password.html")]
pub struct ForgotPasswordTemplate {
    pub title: String,
    pub current_user: Option<UserInfo>,
}

#[derive(Template)]
#[template(path = "reset_password.html")]
pub struct ResetPasswordTemplate {
    pub title: String,
    pub current_user: Option<UserInfo>,
    pub token: String,
}

#[derive(Template)]
#[template(path = "dashboard.html")]
pub struct DashboardTemplate {
    pub title: String,
    pub current_user: Option<UserInfo>,
}

#[derive(Template)]
#[template(path = "devices.html")]
pub struct DevicesTemplate {
    pub title: String,
    pub current_user: Option<UserInfo>,
}

#[derive(Template)]
#[template(path = "users.html")]
pub struct UsersTemplate {
    pub title: String,
    pub current_user: Option<UserInfo>,
}

#[derive(Template)]
#[template(path = "monitor.html")]
pub struct MonitorTemplate {
    pub title: String,
    pub current_user: Option<UserInfo>,
}
