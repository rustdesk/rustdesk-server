use crate::api::{ApiState, ApiResponse, PasswordResetRequest, PasswordResetConfirmRequest, ChangePasswordRequest};
use axum::{
    extract::Extension,
    http::StatusCode,
    response::{Json, IntoResponse, Html},
};
use hbb_common::log;

// 密码重置相关API函数
pub async fn forgot_password(
    Extension(state): Extension<ApiState>,
    Json(request): Json<PasswordResetRequest>,
) -> Result<Json<ApiResponse<()>>, StatusCode> {
    if request.email.trim().is_empty() {
        return Ok(Json(ApiResponse::error("邮箱不能为空".to_string())));
    }
    
    match state.db.get_user_by_email(&request.email).await {
        Ok(Some(user)) => {
            // 创建密码重置令牌
            match state.db.create_password_reset_token(user.id).await {
                Ok(token) => {
                    // 在实际应用中，这里应该发送邮件
                    // 为了演示，我们直接返回成功信息
                    log::info!("Password reset token created for user {}: {}", user.email, token);
                    Ok(Json(ApiResponse::success(())))
                }
                Err(e) => Ok(Json(ApiResponse::error(format!("创建重置令牌失败: {}", e)))),
            }
        }
        Ok(None) => {
            // 为了安全，即使用户不存在也返回成功
            Ok(Json(ApiResponse::success(())))
        }
        Err(_) => Ok(Json(ApiResponse::error("数据库错误".to_string()))),
    }
}

pub async fn reset_password(
    Extension(state): Extension<ApiState>,
    Json(request): Json<PasswordResetConfirmRequest>,
) -> Result<Json<ApiResponse<()>>, StatusCode> {
    // 验证输入
    if request.token.trim().is_empty() {
        return Ok(Json(ApiResponse::error("重置令牌不能为空".to_string())));
    }
    
    if request.new_password.len() < 6 {
        return Ok(Json(ApiResponse::error("密码长度至少6位".to_string())));
    }
    
    if request.new_password != request.confirm_password {
        return Ok(Json(ApiResponse::error("两次输入的密码不一致".to_string())));
    }
    
    // 验证重置令牌
    match state.db.validate_password_reset_token(&request.token).await {
        Ok(Some(user_id)) => {
            // 重置密码
            match state.db.reset_password(user_id, &request.new_password).await {
                Ok(_) => Ok(Json(ApiResponse::success(()))),
                Err(e) => Ok(Json(ApiResponse::error(format!("重置密码失败: {}", e)))),
            }
        }
        Ok(None) => Ok(Json(ApiResponse::error("重置令牌无效或已过期".to_string()))),
        Err(e) => Ok(Json(ApiResponse::error(format!("验证令牌失败: {}", e)))),
    }
}

pub async fn change_password(
    Extension(state): Extension<ApiState>,
    Json(request): Json<ChangePasswordRequest>,
) -> Result<Json<ApiResponse<()>>, StatusCode> {
    // 验证输入
    if request.new_password.len() < 6 {
        return Ok(Json(ApiResponse::error("密码长度至少6位".to_string())));
    }
    
    if request.new_password != request.confirm_password {
        return Ok(Json(ApiResponse::error("两次输入的密码不一致".to_string())));
    }
    
    // 这里需要从JWT token中获取用户ID，为了简化，我们暂时从请求中获取
    // 在实际应用中，应该从认证中间件中获取用户信息
    
    // 由于这是一个简化版本，我们需要先实现JWT认证中间件
    // 暂时返回一个提示信息
    Ok(Json(ApiResponse::error("需要先实现JWT认证中间件".to_string())))
}

// 密码重置页面
pub async fn forgot_password_page() -> impl IntoResponse {
    let html = r#"<!DOCTYPE html>
<html lang="zh-CN">
<head>
    <meta charset="UTF-8">
    <meta name="viewport" content="width=device-width, initial-scale=1.0">
    <title>忘记密码 - NAT Server</title>
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
        
        .forgot-container {
            background: white;
            padding: 2rem;
            border-radius: 10px;
            box-shadow: 0 15px 35px rgba(0, 0, 0, 0.1);
            width: 100%;
            max-width: 400px;
        }
        
        .forgot-header {
            text-align: center;
            margin-bottom: 2rem;
        }
        
        .forgot-header h1 {
            color: #333;
            font-size: 2rem;
            margin-bottom: 0.5rem;
        }
        
        .forgot-header p {
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
        
        .submit-btn {
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
        
        .submit-btn:hover {
            transform: translateY(-2px);
        }
        
        .submit-btn:active {
            transform: translateY(0);
        }
        
        .back-link {
            text-align: center;
            margin-top: 1.5rem;
            color: #666;
        }
        
        .back-link a {
            color: #667eea;
            text-decoration: none;
            font-weight: 500;
        }
        
        .back-link a:hover {
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
    <div class="forgot-container">
        <div class="forgot-header">
            <h1>忘记密码</h1>
            <p>请输入您的邮箱地址以重置密码</p>
        </div>
        
        <div class="error-message" id="error-message"></div>
        <div class="success-message" id="success-message"></div>
        
        <form id="forgot-form">
            <div class="form-group">
                <label for="email">邮箱地址</label>
                <input type="email" id="email" name="email" required>
            </div>
            
            <button type="submit" class="submit-btn">发送重置链接</button>
        </form>
        
        <div class="back-link">
            <a href="/login">返回登录</a>
        </div>
    </div>
    
    <script>
        document.getElementById('forgot-form').addEventListener('submit', async function(e) {
            e.preventDefault();
            
            const email = document.getElementById('email').value;
            const errorDiv = document.getElementById('error-message');
            const successDiv = document.getElementById('success-message');
            
            errorDiv.style.display = 'none';
            successDiv.style.display = 'none';
            
            try {
                const response = await fetch('/api/forgot-password', {
                    method: 'POST',
                    headers: {
                        'Content-Type': 'application/json',
                    },
                    body: JSON.stringify({ email }),
                });
                
                const result = await response.json();
                
                if (result.success) {
                    successDiv.textContent = '重置链接已发送到您的邮箱，请查收';
                    successDiv.style.display = 'block';
                    
                    // 清空表单
                    document.getElementById('email').value = '';
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

    Html(html).into_response()
}

// 密码重置确认页面
pub async fn reset_password_page() -> impl IntoResponse {
    let html = r#"<!DOCTYPE html>
<html lang="zh-CN">
<head>
    <meta charset="UTF-8">
    <meta name="viewport" content="width=device-width, initial-scale=1.0">
    <title>重置密码 - NAT Server</title>
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
        
        .reset-container {
            background: white;
            padding: 2rem;
            border-radius: 10px;
            box-shadow: 0 15px 35px rgba(0, 0, 0, 0.1);
            width: 100%;
            max-width: 400px;
        }
        
        .reset-header {
            text-align: center;
            margin-bottom: 2rem;
        }
        
        .reset-header h1 {
            color: #333;
            font-size: 2rem;
            margin-bottom: 0.5rem;
        }
        
        .reset-header p {
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
        
        .reset-btn {
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
        
        .reset-btn:hover {
            transform: translateY(-2px);
        }
        
        .reset-btn:active {
            transform: translateY(0);
        }
        
        .back-link {
            text-align: center;
            margin-top: 1.5rem;
            color: #666;
        }
        
        .back-link a {
            color: #667eea;
            text-decoration: none;
            font-weight: 500;
        }
        
        .back-link a:hover {
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
    <div class="reset-container">
        <div class="reset-header">
            <h1>重置密码</h1>
            <p>请输入您的新密码</p>
        </div>
        
        <div class="error-message" id="error-message"></div>
        <div class="success-message" id="success-message"></div>
        
        <form id="reset-form">
            <div class="form-group">
                <label for="token">重置令牌</label>
                <input type="text" id="token" name="token" required placeholder="从邮件中获取的重置令牌">
            </div>
            
            <div class="form-group">
                <label for="new_password">新密码</label>
                <input type="password" id="new_password" name="new_password" required>
                <div class="password-requirements">密码至少6位字符</div>
            </div>
            
            <div class="form-group">
                <label for="confirm_password">确认新密码</label>
                <input type="password" id="confirm_password" name="confirm_password" required>
            </div>
            
            <button type="submit" class="reset-btn">重置密码</button>
        </form>
        
        <div class="back-link">
            <a href="/login">返回登录</a>
        </div>
    </div>
    
    <script>
        document.getElementById('reset-form').addEventListener('submit', async function(e) {
            e.preventDefault();
            
            const token = document.getElementById('token').value;
            const newPassword = document.getElementById('new_password').value;
            const confirmPassword = document.getElementById('confirm_password').value;
            const errorDiv = document.getElementById('error-message');
            const successDiv = document.getElementById('success-message');
            
            errorDiv.style.display = 'none';
            successDiv.style.display = 'none';
            
            // 客户端验证
            if (newPassword !== confirmPassword) {
                errorDiv.textContent = '两次输入的密码不一致';
                errorDiv.style.display = 'block';
                return;
            }
            
            if (newPassword.length < 6) {
                errorDiv.textContent = '密码长度至少6位';
                errorDiv.style.display = 'block';
                return;
            }
            
            try {
                const response = await fetch('/api/reset-password', {
                    method: 'POST',
                    headers: {
                        'Content-Type': 'application/json',
                    },
                    body: JSON.stringify({ 
                        token, 
                        new_password: newPassword, 
                        confirm_password: confirmPassword 
                    }),
                });
                
                const result = await response.json();
                
                if (result.success) {
                    successDiv.textContent = '密码重置成功！正在跳转到登录页面...';
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

    Html(html).into_response()
}
