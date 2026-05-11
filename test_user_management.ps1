# 用户管理功能测试脚本
# PowerShell脚本用于测试NAT Server的用户管理API

Write-Host "=== NAT Server 用户管理功能测试 ===" -ForegroundColor Green

# 基础URL
$baseUrl = "http://localhost:8080"

# 测试函数
function Test-Endpoint($method, $url, $body = $null) {
    try {
        $headers = @{
            "Content-Type" = "application/json"
        }
        
        if ($method -eq "GET") {
            $response = Invoke-RestMethod -Uri $url -Method $method -Headers $headers -TimeoutSec 10
        } else {
            $response = Invoke-RestMethod -Uri $url -Method $method -Headers $headers -Body $body -TimeoutSec 10
        }
        
        return @{
            Success = $true
            Data = $response
        }
    } catch {
        return @{
            Success = $false
            Error = $_.Exception.Message
        }
    }
}

# 1. 测试根端点
Write-Host "`n1. 测试根端点..." -ForegroundColor Yellow
$result = Test-Endpoint "GET" "$baseUrl/"
if ($result.Success) {
    Write-Host "✓ 根端点访问成功" -ForegroundColor Green
} else {
    Write-Host "✗ 根端点访问失败: $($result.Error)" -ForegroundColor Red
}

# 2. 测试用户注册
Write-Host "`n2. 测试用户注册..." -ForegroundColor Yellow
$registerBody = @{
    username = "testuser"
    email = "test@example.com"
    password = "password123"
    confirm_password = "password123"
} | ConvertTo-Json

$result = Test-Endpoint "POST" "$baseUrl/api/register" $registerBody
if ($result.Success) {
    Write-Host "✓ 用户注册成功" -ForegroundColor Green
    Write-Host "  用户信息: $($result.Data.data.username) ($($result.Data.data.email))" -ForegroundColor Cyan
} else {
    Write-Host "✗ 用户注册失败: $($result.Error)" -ForegroundColor Red
}

# 3. 测试用户登录
Write-Host "`n3. 测试用户登录..." -ForegroundColor Yellow
$loginBody = @{
    username = "testuser"
    password = "password123"
} | ConvertTo-Json

$result = Test-Endpoint "POST" "$baseUrl/api/login" $loginBody
if ($result.Success) {
    Write-Host "✓ 用户登录成功" -ForegroundColor Green
    Write-Host "  Token: $($result.Data.data.token.Substring(0, 20))..." -ForegroundColor Cyan
    $token = $result.Data.data.token
} else {
    Write-Host "✗ 用户登录失败: $($result.Error)" -ForegroundColor Red
}

# 4. 测试获取用户列表
Write-Host "`n4. 测试获取用户列表..." -ForegroundColor Yellow
$result = Test-Endpoint "GET" "$baseUrl/api/users"
if ($result.Success) {
    Write-Host "✓ 获取用户列表成功" -ForegroundColor Green
    Write-Host "  用户数量: $($result.Data.data.Count)" -ForegroundColor Cyan
} else {
    Write-Host "✗ 获取用户列表失败: $($result.Error)" -ForegroundColor Red
}

# 5. 测试密码重置请求
Write-Host "`n5. 测试密码重置请求..." -ForegroundColor Yellow
$resetBody = @{
    email = "test@example.com"
} | ConvertTo-Json

$result = Test-Endpoint "POST" "$baseUrl/api/forgot-password" $resetBody
if ($result.Success) {
    Write-Host "✓ 密码重置请求成功" -ForegroundColor Green
} else {
    Write-Host "✗ 密码重置请求失败: $($result.Error)" -ForegroundColor Red
}

Write-Host "`n=== 测试完成 ===" -ForegroundColor Green
Write-Host "注意: 请确保NAT Server正在运行在端口8080上" -ForegroundColor Yellow
