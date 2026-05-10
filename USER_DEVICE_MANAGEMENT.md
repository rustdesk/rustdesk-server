# RustDesk 服务器用户和设备管理功能

本文档介绍了为 RustDesk 服务器添加的用户和设备管理功能。

## 功能概述

新增的功能包括：

1. **用户管理** - 创建、查看、更新、删除用户账户
2. **设备管理** - 将设备分配给用户，每个用户最多可拥有 10 个设备
3. **Web 管理界面** - 简单易用的 Web 管理后台
4. **API 接口** - RESTful API 支持第三方集成
5. **认证授权** - 基于 JWT 的用户认证

## 数据库结构

### 用户表 (users)
- `id` - 用户唯一标识
- `username` - 用户名（唯一）
- `email` - 邮箱地址（唯一）
- `password_hash` - 密码哈希值
- `created_at` - 创建时间
- `updated_at` - 更新时间
- `is_active` - 账户状态

### 设备关系表 (user_devices)
- `id` - 关系记录 ID
- `user_id` - 所属用户 ID
- `device_id` - RustDesk 设备 ID
- `device_name` - 设备名称（可选）
- `created_at` - 添加时间
- `is_active` - 设备状态

## API 接口

### 认证接口
- `POST /api/login` - 用户登录

### 用户管理
- `GET /api/users` - 获取用户列表
- `POST /api/users` - 创建新用户
- `GET /api/users/{id}` - 获取用户信息
- `PUT /api/users/{id}` - 更新用户信息
- `DELETE /api/users/{id}` - 删除用户

### 设备管理
- `GET /api/users/{id}/devices` - 获取用户的设备列表
- `POST /api/users/{id}/devices` - 为用户添加设备
- `DELETE /api/users/{id}/devices/{device_id}` - 移除用户的设备
- `GET /api/devices/{device_id}/owner` - 查询设备所有者

## 环境变量

- `DB_URL` - 数据库文件路径（默认：`./db_v2.sqlite3`）
- `JWT_SECRET` - JWT 密钥（生产环境必须设置）
- `MAX_DATABASE_CONNECTIONS` - 数据库连接池大小（默认：1）

## 启动服务

1. 编译项目：
```bash
cargo build --release
```

2. 设置环境变量：
```bash
export JWT_SECRET="your-very-secret-key-here"
export DB_URL="./data/db.sqlite3"
```

3. 启动服务：
```bash
./target/release/hbbs
```

服务启动后：
- Rendezvous 服务器运行在默认端口（21115）
- API 服务器运行在端口 8080
- Web 管理界面可通过 `http://localhost:8080` 访问

## Web 管理界面

访问 `http://localhost:8080` 即可使用 Web 管理界面：

### 登录
使用已创建的用户账户登录系统。

### 用户管理
- 查看所有用户列表
- 创建新用户（需要用户名、邮箱、密码）
- 编辑用户信息
- 删除用户（同时删除其所有设备）

### 设备管理
- 查看所有设备列表
- 为用户添加设备（每个用户最多 10 个）
- 移除用户的设备
- 搜索设备

### 系统设置
- 查看当前登录用户
- 查看API端点信息
- 退出登录

## 使用示例

### 1. 创建管理员用户
```bash
curl -X POST http://localhost:8080/api/users \
  -H "Content-Type: application/json" \
  -d '{
    "username": "admin",
    "email": "admin@example.com",
    "password": "admin123"
  }'
```

### 2. 用户登录
```bash
curl -X POST http://localhost:8080/api/login \
  -H "Content-Type: application/json" \
  -d '{
    "username": "admin",
    "password": "admin123"
  }'
```

### 3. 为用户添加设备
```bash
curl -X POST http://localhost:8080/api/users/1/devices \
  -H "Content-Type: application/json" \
  -H "Authorization: Bearer YOUR_JWT_TOKEN" \
  -d '{
    "user_id": 1,
    "device_id": "1234567890",
    "device_name": "办公电脑"
  }'
```

## 安全注意事项

1. **生产环境必须设置 JWT_SECRET**
2. 使用强密码策略
3. 定期备份数据库
4. 考虑添加 HTTPS 支持
5. 限制 API 访问频率

## 扩展建议

1. 添加设备在线状态监控
2. 实现用户角色权限管理
3. 添加设备使用统计
4. 支持批量操作
5. 添加审计日志
6. 集成邮件通知

## 故障排除

### 数据库连接失败
- 检查 `DB_URL` 环境变量
- 确保数据库文件路径存在且有写入权限

### JWT 认证失败
- 检查 `JWT_SECRET` 环境变量
- 确认 token 未过期

### Web 界面无法访问
- 检查端口 8080 是否被占用
- 检查防火墙设置

## 技术栈

- **后端**: Rust + Axum + SQLx + SQLite
- **前端**: HTML + CSS + JavaScript (原生)
- **认证**: JWT (JSON Web Tokens)
- **数据库**: SQLite (可扩展到 PostgreSQL/MySQL)

## 许可证

本扩展功能遵循与原 RustDesk 服务器相同的开源许可证。
