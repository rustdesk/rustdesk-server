# nat-client — 完整内网穿透客户端

> 与 **nat-server**（hbbs/hbbr）配合使用的 Rust 内网穿透客户端工具。  
> 参照 RustDesk 开源项目的 `ipc.rs`、`lan.rs`、`rendezvous_mediator.rs` 实现，  
> 并与 nat-server 的**用户系统**和**设备管理**完整集成。

---

## 目录

1. [项目概述](#1-项目概述)
2. [整体架构](#2-整体架构)
3. [工作原理](#3-工作原理)
   - [注册与公钥确认流程](#31-注册与公钥确认流程)
   - [用户登录与设备绑定流程](#32-用户登录与设备绑定流程)
   - [TCP 打洞流程](#33-tcp-打洞流程)
   - [中继连接流程](#34-中继连接流程)
   - [局域网发现流程](#35-局域网发现流程)
4. [网络端口说明](#4-网络端口说明)
5. [模块说明](#5-模块说明)
6. [系统要求与构建](#6-系统要求与构建)
7. [快速开始](#7-快速开始)
8. [命令行参考](#8-命令行参考)
   - [守护进程](#81-daemon--启动守护进程)
   - [基础查询](#82-基础查询命令)
   - [用户与设备管理](#83-用户与设备管理命令)
   - [隧道连接](#84-隧道连接命令)
   - [调试](#85-调试命令)
9. [IPC 控制接口参考](#9-ipc-控制接口参考)
   - [基础命令](#91-基础命令)
   - [认证命令](#92-认证命令)
   - [隧道命令](#93-隧道命令)
10. [配置文件说明](#10-配置文件说明)
11. [典型使用场景](#11-典型使用场景)
12. [与 nat-server 的关系](#12-与-nat-server-的关系)
13. [常见问题](#13-常见问题)

---

## 1. 项目概述

`nat-client` 是运行在**客户端主机**上的守护进程，通过连接 **nat-server**（hbbs 渲染同端服务器 + hbbr 中继服务器）实现两台均位于 NAT 后方主机的透明互联，无需手动配置端口转发或 VPN。

### 核心能力

| 能力 | 说明 |
|---|---|
| **自动注册** | 启动后自动生成 Peer ID 和 Ed25519 密钥，向 hbbs 注册 |
| **用户认证** | 登录 nat-server 账户，获取 JWT，在 `RegisterPk` 中携带 token 完成用户-设备绑定 |
| **设备管理** | 注册/查询/移除绑定到账户的设备，支持多设备多账户管理 |
| **TCP 打洞** | 利用 NAT 映射复用，在两端建立直接 TCP 连接 |
| **中继转发** | 对称 NAT 或打洞失败时自动经由 hbbr 中转 |
| **局域网发现** | UDP 广播自动发现同一局域网内的其他 nat-client 节点 |
| **本地 IPC** | 提供 JSON-over-TCP 控制接口，供脚本或应用程序编程控制 |
| **端口隧道** | 建连后在本地开放 TCP 端口，透明转发到对端任意服务 |
| **Token 监控** | 后台自动检测 JWT 过期，提醒用户重新登录 |

---

## 2. 整体架构

```
┌──────────────────────────────────────────────────────────────────────┐
│                         nat-client 进程                               │
│                                                                      │
│  ┌─────────────────────┐  ┌───────────────────┐  ┌────────────────┐ │
│  │  RendezvousMediator │  │   LAN Discovery   │  │  Auth Module   │ │
│  │  (TCP → hbbs:21116) │  │ (UDP 广播 :21119) │  │ (HTTP :8080)   │ │
│  │  ┌──────────────┐   │  └───────────────────┘  └───────┬────────┘ │
│  │  │ user_token ◄─┼───┼─────────────────────────────────┘         │ │
│  │  │ (JWT注入)    │   │        JWT 存储                            │ │
│  │  └──────────────┘   │                                           │ │
│  └──────────┬──────────┘                                           │ │
│             │ 注册/打洞/中继                                         │ │
│  ┌──────────▼──────────┐  ┌───────────────────────────────────┐   │ │
│  │  PortForwardManager │  │           IPC Server              │   │ │
│  │  (直连 / 中继隧道)   │  │  (127.0.0.1:21114 TCP JSON-RPC)  │   │ │
│  └─────────────────────┘  └───────────────────────────────────┘   │ │
└──────────────┬───────────────────────────────────────────────────────┘
               │ NAT
 ══════════════╪══════════════════════════════════════════════════════════
               │ 公网
      ┌────────▼────────────────────────────────┐
      │               nat-server                │
      │  hbbs  :21116  （渲染同端 / 协调打洞）    │
      │  hbbr  :21117  （中继转发）              │
      │  HTTP  :8080   （用户/设备 REST API）    │
      └────────┬────────────────────────────────┘
               │ NAT
 ══════════════╪══════════════════════════════════════════════════════════
               │
┌──────────────▼────────────────────────────────────────────────────────┐
│                         主机 B（nat-client）                           │
│                    （镜像结构，同样运行守护进程）                        │
└───────────────────────────────────────────────────────────────────────┘
```

---

## 3. 工作原理

### 3.1 注册与公钥确认流程

```
nat-client (首次启动)                hbbs (:21116)
      │                                   │
      │  本地自动生成：                    │
      │  ├─ Peer ID（9位数字）             │
      │  ├─ UUID（设备唯一标识）           │
      │  └─ Ed25519 密钥对（sk/pk）        │
      │                                   │
      │── TCP 连接 :21116 ────────────────►│
      │── RegisterPk {                    │
      │     id, uuid, pk,                 │
      │     user_token: ""（未登录时为空） │
      │   } ─────────────────────────────►│
      │                                   │  验证并存储公钥
      │◄── RegisterPkResponse {OK} ───────│  若 user_token 有效则绑定用户
      │                                   │
      │   （此后每 15 秒心跳）              │
      │── RegisterPeer {id} ─────────────►│
      │◄── RegisterPeerResponse ──────────│
```

---

### 3.2 用户登录与设备绑定流程

登录后 JWT 会在下次 `RegisterPk` 时自动携带，服务端完成用户-设备关联。

```
nat-client login                  nat-server HTTP API (:8080)      hbbs (:21116)
      │                                    │                             │
      │  ── 第1步：携带 device_id 登录 ──   │                             │
      │── POST /api/login ────────────────►│                             │
      │   {username, password,             │                             │
      │    device_id: "386742019"}         │                             │
      │                                    │                             │
      │  [设备已绑定] ──────────────────────│                             │
      │◄── {token:"eyJ...udid:42...", ...}─│                             │
      │  保存 token → config.toml           │                             │
      │                                    │                             │
      │  [设备未绑定] ──────────────────────│                             │
      │── POST /api/login (无device_id) ──►│                             │
      │◄── {token:"eyJ...", user:{...}} ───│  获取临时 token              │
      │── POST /api/devices ──────────────►│  注册本机到账户              │
      │   {device_id:"386742019",          │                             │
      │    device_name:"nat-client@host"}  │                             │
      │◄── {id:42, device_id:"..."} ───────│  device_row_id = 42         │
      │── POST /api/login (含device_id) ──►│                             │
      │◄── {token:"eyJ...udid:42..."} ─────│  JWT 含 udid=42             │
      │  保存 token → config.toml           │                             │
      │                                    │                             │
      │  触发中介重连：                      │                             │
      │── TCP 连接 ──────────────────────────────────────────────────────►│
      │── RegisterPk {                     │                             │
      │     id, uuid, pk,                  │                             │
      │     user_token: "eyJ...udid:42"    │                             │
      │   } ─────────────────────────────────────────────────────────────►│
      │                                    │  服务端验证 JWT：             │
      │                                    │  peer 绑定到                │
      │                                    │  users.id=5                 │
      │                                    │  user_devices.id=42         │
      │◄── RegisterPkResponse {OK} ─────────────────────────────────────│
```

---

### 3.3 TCP 打洞流程

```
主机 B (nat-client)        hbbs (nat-server)        主机 A (nat-client)
       │                         │                         │
       │── PunchHoleRequest ────►│                         │
       │   {id: "A的ID"}         │                         │
       │                         │── PunchHole ───────────►│
       │                         │   {B的NAT地址}           │
       │                         │                         │── 新建TCP连接获取出口地址
       │                         │                         │── 向B发SYN（打开NAT映射）
       │                         │◄── PunchHoleSent ───────│
       │◄── PunchHoleResponse ───│                         │
       │                         │                         │
       │─── TCP SYN ─────────────────────────────────────►│  直连建立！
       │◄── TCP SYN+ACK ─────────────────────────────────-│
       │◄══════════════ 双向 TCP 数据流（直连）══════════════►│
```

---

### 3.4 中继连接流程

当 NAT 类型为**对称 NAT** 或打洞失败时，自动回落到中继：

```
主机 B              hbbs              hbbr              主机 A
  │                  │                 │                   │
  │── 请求连接 A ────►│── RequestRelay ─────────────────────►│
  │◄── RelayResponse ─│◄── RelayResponse ───────────────────│
  │── TCP连接hbbr ─────────────────────►│◄── A已连入 ────────│
  │◄══════════════ 双向代理（经 hbbr 中转）══════════════════►│
```

---

### 3.5 局域网发现流程

```
nat-client (A)                       nat-client (B) [同一局域网]
      │                                      │
      │── UDP 广播 255.255.255.255:21119 ────►│
      │   PeerDiscovery {cmd:"ping", id:"A"} │
      │                                      │
      │◄── UDP 单播 ─────────────────────────│
           PeerDiscovery {cmd:"pong", id:"B",
             hostname, username, platform}
```

---

## 4. 网络端口说明

| 端口 | 协议 | 方向 | 用途 |
|---|---|---|---|
| **21116** | TCP | 出站 → hbbs | 渲染同端注册与打洞协调 |
| **21117** | TCP | 出站 → hbbr | 中继数据转发 |
| **21119** | UDP | 双向（局域网广播） | 局域网 Peer 发现 |
| **21114** | TCP | 入站（仅 127.0.0.1） | 本地 IPC 控制接口 |
| **8080** | TCP | 出站 → HTTP API | 用户注册/登录/设备管理 REST API |
| **随机** | TCP | 出站/入站 | NAT 打洞用临时端口 |
| **用户指定** | TCP | 入站（仅 127.0.0.1） | 隧道本地监听端口 |

> **防火墙说明**：只需放行出站 TCP 21116、TCP 21117、TCP 8080。  
> IPC 端口 21114 仅监听本机回环，无需对外开放。

---

## 5. 模块说明

### `auth.rs` — 用户认证与设备管理 ⭐ 新增

封装对 nat-server HTTP REST API（`:8080`）的所有调用。

| 函数 | 说明 |
|---|---|
| `login(username, password, device_name)` | 登录（4步自动流程：登录→注册设备→再登录获取udid JWT） |
| `register(username, email, password, device_name)` | 注册新账户后自动登录 |
| `logout()` | 清除本地 token |
| `change_password(old, new)` | 修改密码 |
| `list_devices()` | 查询当前用户绑定的设备列表 |
| `remove_device(device_id)` | 移除一台绑定设备 |
| `get_user_info()` | 获取用户资料（id, username, email, role） |
| `start_token_refresh_watcher()` | 后台 token 过期监控（每 5 分钟检查） |

**`AuthStatus` 结构**（IPC/CLI 响应）：
```json
{
  "logged_in": true,
  "user_id": 5,
  "username": "alice",
  "role": "user",
  "device_row_id": 42,
  "token_expires": 1720086400,
  "token_remaining_secs": 79200
}
```

**登录四步流程说明**：

```
步骤1: POST /api/login {device_id:"386742019"} → 若设备已绑定，直接返回含 udid 的 JWT ✓
步骤2: 若设备未绑定 → POST /api/login {无device_id} → 获取临时 token
步骤3: POST /api/devices {device_id, device_name} → 绑定本机到账户，得到 udid
步骤4: POST /api/login {device_id:"386742019"} → 返回含 udid 的完整 JWT ✓
```

---

### `config.rs` — 客户端配置管理

负责 Peer ID、UUID、Ed25519 密钥对及认证信息的生成与持久化。

**认证相关快捷方法**：

| 方法 | 说明 |
|---|---|
| `ClientConfig::is_logged_in()` | 是否已登录（token 有效且未过期） |
| `ClientConfig::get_auth_token()` | 获取有效 JWT（已过期返回 `None`） |
| `ClientConfig::save_login(...)` | 保存登录结果到配置文件 |
| `ClientConfig::clear_login()` | 清除所有认证信息 |
| `ClientConfig::get_api_url()` | 自动推导 API 地址（从 rendezvous_servers） |

**配置文件路径**：
- Linux/macOS：`~/.config/nat-client/config.toml`
- Windows：`%APPDATA%\nat-client\config.toml`

---

### `rendezvous_mediator.rs` — NAT 穿透中介

参照 RustDesk `src/rendezvous_mediator.rs` 实现。

**用户认证集成**（关键改动）：

`register_pk()` 函数在构造 `RegisterPk` 消息时，会自动读取 `ClientConfig::get_auth_token()`，若用户已登录则将 JWT 写入 `user_token` 字段：

```
RegisterPk {
    id:         "386742019",
    uuid:       <设备UUID字节>,
    pk:         <Ed25519公钥字节>,
    user_token: "eyJhbGciOiJIUzI1NiJ9..."   ← 已登录时自动注入
}
```

服务端（`nat-server/src/peer.rs`）收到后验证 JWT，将 peer 与用户账户绑定到数据库。

---

### `lan.rs` — 局域网节点发现

参照 RustDesk `src/lan.rs` 实现。

| 函数 | 说明 |
|---|---|
| `start_listening()` | 阻塞监听 UDP 21119，响应 ping（独立线程） |
| `discover()` | 发送广播 ping，等待 3 秒收集 pong，返回发现列表 |
| `get_peers()` | 返回全局缓存节点（60 秒未响应标记为离线） |

---

### `ipc.rs` — 本地 IPC 控制接口

参照 RustDesk `src/ipc.rs` 实现，监听 `127.0.0.1:21114`，使用**换行符分隔的 JSON** 协议。

新增认证相关命令：`auth_status`、`auth_login`、`auth_logout`、`auth_register`、`auth_change_password`、`auth_list_devices`、`auth_remove_device`、`auth_profile`。

完整命令表见 [第 9 节](#9-ipc-控制接口参考)。

---

### `port_forward.rs` — TCP 端口转发管理器

管理所有隧道连接的生命周期（直连 + 中继）。

| 方法 | 说明 |
|---|---|
| `register_inbound(local, peer, uuid)` | 注册入站直连等待（打洞被动侧） |
| `create_outbound_direct(port, peer)` | 建立出站直连隧道，监听本地端口 |
| `register_relay(uuid, peer, conn, ...)` | 注册中继连接（被动侧） |
| `create_outbound_relay(port, conn, uuid, ...)` | 建立出站中继隧道，监听本地端口 |
| `get_active_connections()` | 返回所有活跃连接快照 |

---

## 6. 系统要求与构建

### 系统要求

| 项目 | 要求 |
|---|---|
| Rust | 1.75+（推荐 stable 最新版） |
| 操作系统 | Linux / macOS / Windows |
| nat-server | hbbs（:21116）+ hbbr（:21117）+ HTTP API（:8080）均需运行 |

### 构建

```bash
# 进入 nat-server 项目根目录
cd nat-server

# 仅编译 nat-client（调试版）
cargo build -p nat-client

# 发布版（体积小、性能高）
cargo build -p nat-client --release

# 编译产物：
# 调试版：./target/debug/nat-client
# 发布版：./target/release/nat-client
```

---

## 7. 快速开始

### 步骤一：确认 nat-server 已运行

```bash
# 在服务器上（假设公网 IP 为 1.2.3.4）
./hbbs -p 21116 -r 1.2.3.4:21117
./hbbr -p 21117
# HTTP API 由 hbbs 自动启动在 :8080
```

### 步骤二：在主机 A 上启动守护进程

```bash
./nat-client daemon --server 1.2.3.4

# 首次启动日志：
# [INFO] === nat-client v0.1.0 启动 ===
# [INFO] 生成新 Peer ID: 386742019
# [INFO] 生成新 Ed25519 密钥对
# [INFO] [mediator] TCP 连接成功: 1.2.3.4:21116
# [INFO] [mediator] 公钥确认成功，已上线
# [INFO] 当前未登录，以匿名模式运行。可执行 `nat-client login` 登录以启用用户-设备绑定
```

### 步骤三：注册并登录账户（可选，但推荐）

```bash
# 注册新账户（首次使用）
nat-client register -u alice -e alice@example.com -p mypassword

# 输出示例：
# 注册用户 alice...
# {
#   "auth": {
#     "logged_in": true,
#     "user_id": 5,
#     "username": "alice",
#     "role": "user",
#     "device_row_id": 42,
#     "token_expires": 1720086400,
#     "token_remaining_secs": 86370
#   }
# }

# 或已有账户直接登录
nat-client login -u alice -p mypassword
```

登录成功后守护进程会自动重连，`RegisterPk` 中携带 JWT，服务端完成用户-设备绑定。

### 步骤四：在主机 B 上也启动并登录

```bash
./nat-client daemon --server 1.2.3.4

# 同账户或不同账户均可登录
nat-client login -u bob -p bobpassword
# [INFO] 本机 Peer ID: 749183026
```

### 步骤五：从主机 A 连接主机 B

```bash
nat-client connect --peer-id 749183026

# 输出：
# 正在连接对端 749183026...
# { "local_port": 54321 }
#
# ✅ 隧道已建立！
#    请连接 127.0.0.1:54321 即可访问对端服务

# SSH 穿透示例：
ssh -p 54321 user@127.0.0.1
```

---

## 8. 命令行参考

### 全局选项

```
--ipc-port <PORT>    与守护进程通信的 IPC 端口（默认 21114）
```

---

### 8.1 `daemon` — 启动守护进程

```bash
nat-client daemon --server <ADDR> [选项]
```

| 选项 | 必须 | 默认值 | 说明 |
|---|---|---|---|
| `-s, --server <ADDR>` | ✅ | — | hbbs 服务器地址（`host` 或 `host:port`） |
| `-r, --relay <ADDR>` | ❌ | server+1端口 | hbbr 中继服务器地址 |
| `--id <ID>` | ❌ | 自动生成 | 指定本机 Peer ID（9 位数字） |
| `--ipc-port <PORT>` | ❌ | 21114 | IPC 控制接口端口 |
| `--log-level <LEVEL>` | ❌ | info | 日志级别：`trace/debug/info/warn/error` |

```bash
# 示例
nat-client daemon --server 1.2.3.4
nat-client daemon --server myserver.com:21116 --log-level debug
```

---

### 8.2 基础查询命令

| 命令 | 说明 | 示例输出 |
|---|---|---|
| `nat-client id` | 查看本机 Peer ID | `{"id":"386742019"}` |
| `nat-client status` | 查看在线状态和 NAT 类型 | `{"online":true,"nat_type":0}` |
| `nat-client discover` | 扫描局域网（约 3 秒） | `{"peers":[...]}` |
| `nat-client peers` | 查看缓存的局域网节点 | `{"peers":[...]}` |

---

### 8.3 用户与设备管理命令 ⭐ 新增

#### `register` — 注册新账户

```bash
nat-client register -u <用户名> -e <邮箱> -p <密码> [--device-name <设备名>]
```

| 选项 | 必须 | 说明 |
|---|---|---|
| `-u, --username` | ✅ | 用户名 |
| `-e, --email` | ✅ | 邮箱地址 |
| `-p, --password` | ✅ | 密码（最少 6 位） |
| `--device-name` | ❌ | 设备名称，默认为 `nat-client@hostname` |

**输出示例（成功）**：
```json
{
  "auth": {
    "logged_in": true,
    "user_id": 5,
    "username": "alice",
    "role": "user",
    "device_row_id": 42,
    "token_expires": 1720086400,
    "token_remaining_secs": 86370
  }
}
```

---

#### `login` — 登录

```bash
nat-client login -u <用户名> -p <密码> [--device-name <设备名>]
```

| 选项 | 必须 | 说明 |
|---|---|---|
| `-u, --username` | ✅ | 用户名 |
| `-p, --password` | ✅ | 密码 |
| `--device-name` | ❌ | 本机设备名称（首次绑定时用） |

登录后守护进程自动重连，`RegisterPk.user_token` 携带 JWT，服务端完成绑定。

---

#### `logout` — 注销

```bash
nat-client logout
```

清除本地 token，守护进程自动重连切换回匿名模式。

---

#### `auth-status` — 查看认证状态

```bash
nat-client auth-status
```

**输出示例**：
```json
{
  "auth": {
    "logged_in": true,
    "user_id": 5,
    "username": "alice",
    "role": "user",
    "device_row_id": 42,
    "token_expires": 1720086400,
    "token_remaining_secs": 79200
  }
}
```

`token_remaining_secs` 为负数表示已过期，需重新登录。

---

#### `change-password` — 修改密码

```bash
nat-client change-password --old-password <旧密码> --new-password <新密码>
```

修改成功后自动清除本地 token，需重新登录。

---

#### `devices` — 查看绑定设备列表

```bash
nat-client devices
```

**输出示例**：
```json
{
  "devices": [
    {
      "id": 42,
      "device_id": "386742019",
      "device_name": "nat-client@desktop-alice",
      "is_active": true,
      "created_at": "2025-01-01T00:00:00Z"
    },
    {
      "id": 43,
      "device_id": "749183026",
      "device_name": "nat-client@laptop",
      "is_active": true,
      "created_at": "2025-01-02T00:00:00Z"
    }
  ]
}
```

---

#### `remove-device` — 移除绑定设备

```bash
nat-client remove-device --device-id <Peer ID>
```

**示例**：
```bash
nat-client remove-device --device-id 749183026
# 输出：{"ok": true}
```

> ⚠️ 移除的是目标设备对应的**绑定关系**，不影响对方守护进程运行，但该设备将无法再通过用户身份注册。

---

#### `profile` — 查看用户资料

```bash
nat-client profile
```

**输出示例**：
```json
{
  "user": {
    "id": 5,
    "username": "alice",
    "email": "alice@example.com",
    "role": "user",
    "is_active": true,
    "created_at": "2025-01-01T00:00:00Z"
  }
}
```

---

### 8.4 隧道连接命令

#### `connect` — 发起连接

```bash
nat-client connect --peer-id <ID> [--local-port <PORT>]
```

| 选项 | 必须 | 默认值 | 说明 |
|---|---|---|---|
| `-p, --peer-id <ID>` | ✅ | — | 目标 Peer ID |
| `-l, --local-port <PORT>` | ❌ | 0（自动分配） | 本地监听端口 |

#### `connections` — 查看活跃连接

```bash
nat-client connections
```

#### `close` — 关闭连接

```bash
nat-client close --uuid <UUID>
```

#### `restart` — 重启 rendezvous 中介

```bash
nat-client restart
```

---

### 8.5 调试命令

#### `send` — 发送原始 JSON 命令

```bash
nat-client send '{"cmd":"ping"}'
nat-client send '{"cmd":"auth_status"}'
```

---

## 9. IPC 控制接口参考

监听 `127.0.0.1:21114`，使用**换行符（`\n`）分隔的 JSON** 协议。直接测试：

```bash
echo '{"cmd":"ping"}' | nc 127.0.0.1 21114
```

---

### 9.1 基础命令

| 命令 | 请求示例 | 响应示例 |
|---|---|---|
| `ping` | `{"cmd":"ping"}` | `{"pong":true}` |
| `get_id` | `{"cmd":"get_id"}` | `{"id":"386742019"}` |
| `get_status` | `{"cmd":"get_status"}` | `{"online":true,"nat_type":0}` |
| `get_config` | `{"cmd":"get_config"}` | `{"ok":true,"id":"...","online":true}` |
| `get_peers` | `{"cmd":"get_peers"}` | `{"peers":[...]}` |
| `discover` | `{"cmd":"discover"}` | `{"peers":[...]}` ⚠️约3秒 |
| `get_connections` | `{"cmd":"get_connections"}` | `{"connections":[...]}` |
| `restart_mediator` | `{"cmd":"restart_mediator"}` | `{"ok":true}` |

---

### 9.2 认证命令 ⭐ 新增

#### `auth_status` — 查询认证状态

```json
请求：{"cmd": "auth_status"}

响应：{
  "auth": {
    "logged_in": true,
    "user_id": 5,
    "username": "alice",
    "role": "user",
    "device_row_id": 42,
    "token_expires": 1720086400,
    "token_remaining_secs": 79200
  }
}
```

---

#### `auth_login` — 登录

```json
请求：{
  "cmd": "auth_login",
  "username": "alice",
  "password": "mypassword",
  "device_name": "My Desktop"
}

响应（成功）：{ "auth": { "logged_in": true, "user_id": 5, ... } }
响应（失败）：{ "error": "登录失败: 密码错误" }
```

> 登录成功后自动触发 rendezvous 中介重连，新 token 立即生效。

---

#### `auth_logout` — 注销

```json
请求：{"cmd": "auth_logout"}
响应：{"ok": true}
```

---

#### `auth_register` — 注册新用户

```json
请求：{
  "cmd": "auth_register",
  "username": "bob",
  "email": "bob@example.com",
  "password": "securepass",
  "device_name": "Bob's Laptop"
}

响应（成功）：{ "auth": { "logged_in": true, "user_id": 6, ... } }
响应（失败）：{ "error": "注册失败: 用户名已存在" }
```

---

#### `auth_change_password` — 修改密码

```json
请求：{
  "cmd": "auth_change_password",
  "old_password": "oldpass",
  "new_password": "newpass"
}

响应（成功）：{"ok": true}
响应（失败）：{"error": "修改密码失败: 旧密码错误"}
```

> 修改成功后本地 token 自动清除，需重新登录。

---

#### `auth_list_devices` — 查看绑定设备

```json
请求：{"cmd": "auth_list_devices"}

响应：{
  "devices": [
    {
      "id": 42,
      "device_id": "386742019",
      "device_name": "nat-client@desktop",
      "is_active": true,
      "created_at": "2025-01-01T00:00:00Z"
    }
  ]
}
```

---

#### `auth_remove_device` — 移除设备

```json
请求：{"cmd": "auth_remove_device", "device_id": "749183026"}
响应（成功）：{"ok": true}
响应（失败）：{"error": "移除设备失败: 设备不存在"}
```

---

#### `auth_profile` — 获取用户资料

```json
请求：{"cmd": "auth_profile"}

响应：{
  "user": {
    "id": 5,
    "username": "alice",
    "email": "alice@example.com",
    "role": "user",
    "is_active": true,
    "created_at": "2025-01-01T00:00:00Z"
  }
}
```

---

### 9.3 隧道命令

#### `connect` — 发起连接

```json
请求：{
  "cmd": "connect",
  "peer_id": "749183026",
  "local_port": 0
}
响应：{"local_port": 54321}
```

#### `get_connections` — 获取活跃连接

```json
请求：{"cmd": "get_connections"}
响应：{
  "connections": [
    {
      "uuid": "550e8400-...",
      "conn_type": "Direct",
      "peer_addr": "1.2.3.4:54321",
      "local_port": 54321,
      "bytes_sent": 102400,
      "bytes_recv": 204800,
      "created_at": 1720000000
    }
  ]
}
```

#### `close_conn` — 关闭连接

```json
请求：{"cmd": "close_conn", "uuid": "550e8400-..."}
响应：{"ok": true}
```

---

### 错误响应格式

所有命令出错时返回：
```json
{"error": "错误描述"}
```

---

## 10. 配置文件说明

首次运行守护进程时**自动生成**，路径：
- Linux/macOS：`~/.config/nat-client/config.toml`
- Windows：`%APPDATA%\nat-client\config.toml`

### 完整字段说明

```toml
# ── 基础身份配置（自动生成） ───────────────────────────────────────────

# 本机 9 位数字 Peer ID（自动生成，通常不需手动修改）
id = "386742019"

# 设备 UUID，Base64 编码（自动生成）
uuid = "RhGwP3z6..."

# Ed25519 私钥，Base64 编码（自动生成，请勿泄露！）
sk = "AAABBBCCC..."

# Ed25519 公钥，Base64 编码（自动生成）
pk = "DDDEEEFFF..."

# 公钥是否已被 hbbs 服务器确认
key_confirmed = true

# 各服务器主机的公钥确认状态（自动维护）
[host_keys_confirmed]
"1" = true

# ── 服务器配置 ─────────────────────────────────────────────────────────

# hbbs 服务器地址列表（逗号分隔）
rendezvous_servers = "1.2.3.4:21116"

# hbbr 中继服务器地址（留空则使用 hbbs 地址+1 端口）
relay_server = ""

# nat-server HTTP API 地址（留空则自动推导为 http://hbbs主机:8080）
api_url = ""

# ── 运行配置 ───────────────────────────────────────────────────────────

# IPC 控制接口端口
ipc_port = 21114

# 直接访问端口（0 = 禁用）
direct_listen_port = 0

# NAT 类型缓存
nat_type = 0

# ── 用户认证信息（由 login 命令自动填写） ⭐ ──────────────────────────

# 当前登录用户的 JWT token（24小时有效期）
auth_token = "eyJhbGciOiJIUzI1NiJ9..."

# JWT 过期时间（Unix 秒级时间戳，0 = 未登录）
auth_token_expires = 1720086400

# 当前登录的用户 ID
auth_user_id = 5

# 当前登录的用户名
auth_username = "alice"

# 当前登录的角色（"user" 或 "admin"）
auth_role = "user"

# 本设备在服务器 user_devices 表中的行 ID（udid）；0 = 未绑定
# 此 ID 会被编码进 JWT，服务端通过它关联 peer 和用户
auth_device_row_id = 42
```

> **安全提示**：
> - `sk` 字段是 Ed25519 私钥，`auth_token` 是 JWT；两者均需保护。
> - 建议设置文件权限 `chmod 600 ~/.config/nat-client/config.toml`（Linux/macOS）
> - JWT 有效期 24 小时，过期后以匿名模式继续运行，执行 `nat-client login` 续期

---

## 11. 典型使用场景

### 场景一：注册账户后远程 SSH 访问内网主机

```bash
# === 服务器端（主机 B，内网）===
nat-client daemon --server 1.2.3.4

# 注册账户并登录（首次）
nat-client register -u alice -e alice@example.com -p mypassword
# 记下 Peer ID: 749183026

# === 客户端（主机 A）===
nat-client daemon --server 1.2.3.4
nat-client login -u alice -p mypassword   # 同一账户多设备登录

# 建立 SSH 隧道
nat-client connect --peer-id 749183026 --local-port 2222
# 输出：{"local_port": 2222}

ssh -p 2222 user@127.0.0.1
```

---

### 场景二：多设备账户管理

```bash
# 查看账户下所有绑定设备
nat-client devices
# 输出包含所有设备的 device_id 和 device_name

# 移除不再使用的设备
nat-client remove-device --device-id 123456789
```

---

### 场景三：匿名模式（无需登录）

不登录也可正常使用内网穿透，但 Peer 不与任何用户账户关联：

```bash
# 仅启动守护进程，不登录
nat-client daemon --server 1.2.3.4
nat-client connect --peer-id 749183026  # 直接连接，无身份验证
```

---

### 场景四：脚本自动化登录

```bash
#!/bin/bash
# 等待守护进程启动
until echo '{"cmd":"ping"}' | nc -q1 127.0.0.1 21114 | grep -q pong; do
  sleep 1
done

# 检查是否已登录，未登录则自动登录
LOGGED_IN=$(echo '{"cmd":"auth_status"}' | nc -q1 127.0.0.1 21114 \
  | python3 -c "import sys,json; print(json.load(sys.stdin)['auth']['logged_in'])")

if [ "$LOGGED_IN" != "True" ]; then
  echo '{"cmd":"auth_login","username":"alice","password":"mypassword"}' \
    | nc -q1 127.0.0.1 21114
fi

# 建立连接
PORT=$(echo '{"cmd":"connect","peer_id":"749183026","local_port":0}' \
  | nc -q1 127.0.0.1 21114 \
  | python3 -c "import sys,json; print(json.load(sys.stdin)['local_port'])")
echo "隧道端口: $PORT"
```

---

### 场景五：作为系统服务（Linux systemd）

```ini
# /etc/systemd/system/nat-client.service
[Unit]
Description=NAT Traversal Client
After=network-online.target
Wants=network-online.target

[Service]
Type=simple
User=nobody
ExecStart=/usr/local/bin/nat-client daemon --server 1.2.3.4
ExecStartPost=/bin/sh -c 'sleep 3 && /usr/local/bin/nat-client login -u alice -p mypassword'
Restart=always
RestartSec=5

[Install]
WantedBy=multi-user.target
```

```bash
sudo systemctl daemon-reload
sudo systemctl enable nat-client
sudo systemctl start nat-client
```

---

## 12. 与 nat-server 的关系

```
nat-server 项目
├── src/                         ← 服务端代码
│   ├── rendezvous_server.rs     ← hbbs（接收 RegisterPk，验证 user_token）
│   ├── relay_server.rs          ← hbbr（中继转发）
│   ├── peer.rs                  ← Peer 注册，validate_peer_token() 验证 JWT
│   ├── api.rs                   ← REST API（/api/login, /api/register...）
│   ├── device_api.rs            ← 设备 API（/api/devices）
│   ├── database.rs              ← users / user_devices / peer 表操作
│   └── web.rs                   ← Web 路由
│
└── nat-client/                  ← 客户端代码（本工具）
    └── src/
        ├── main.rs              ← CLI 入口（含 register/login/logout/devices 等子命令）
        ├── auth.rs              ← 用户认证与设备管理 ⭐ 新增
        ├── config.rs            ← 配置管理（含 auth_token 等认证字段）⭐ 更新
        ├── rendezvous_mediator.rs ← 连接 hbbs（RegisterPk 注入 user_token）⭐ 更新
        ├── ipc.rs               ← 本地控制（含 auth_* 命令）⭐ 更新
        ├── lan.rs               ← LAN 发现
        └── port_forward.rs      ← TCP 隧道管理
```

### 服务端数据库对应关系

```
nat-server SQLite 数据库
├── users 表
│   ├── id            ← auth_user_id（存入配置文件）
│   ├── username      ← auth_username
│   └── role          ← auth_role
│
├── user_devices 表
│   ├── id            ← auth_device_row_id（= JWT.udid）
│   ├── user_id       ← 关联 users.id
│   └── device_id     ← Peer ID（= ClientConfig.id）
│
└── peer 表
    ├── id            ← Peer ID（= ClientConfig.id）
    ├── user_id       ← 由 validate_peer_token() 填充（= users.id）
    └── device_id     ← 由 validate_peer_token() 填充（= user_devices.id）
```

### 协议兼容性

- ✅ 与标准 nat-server（hbbs/hbbr + REST API）完整兼容
- ✅ `RegisterPk.user_token` 字段符合服务端 proto 定义（field 6）
- ✅ JWT 格式与服务端 `Claims` 结构一致（`sub`, `username`, `exp`, `udid`, `role`）
- ✅ 匿名模式（不携带 token）完全向后兼容

---

## 13. 常见问题

### Q：守护进程启动后显示"当前未登录，以匿名模式运行"

这是**正常现象**，匿名模式下内网穿透仍可正常工作。  
若需用户-设备绑定，执行：
```bash
nat-client login -u <用户名> -p <密码>
```

---

### Q：登录时报"未配置服务器地址"

守护进程必须先以 `--server` 启动，auth 模块才能自动推导 API 地址：
```bash
nat-client daemon --server 1.2.3.4
# 然后再登录
nat-client login -u alice -p mypassword
```

也可在配置文件中显式指定：
```toml
api_url = "http://1.2.3.4:8080"
```

---

### Q：登录时报"指定的设备不属于当前用户或未激活"

原因：尝试携带 `device_id` 登录，但该设备未绑定到此账户。  
这是正常流程，客户端会自动转为匿名登录 → 注册设备 → 再携带 device_id 登录。  
若持续失败，检查 nat-server API 日志确认 `/api/devices` 是否返回错误。

---

### Q：JWT token 多久过期？

服务端默认 **24 小时**（在 `nat-server/src/api.rs` 中 `Duration::hours(24)` 设置）。  
nat-client 每 5 分钟检查一次过期状态：
- 剩余 < 1 小时：打印警告日志
- 已过期：自动清除 token，切换匿名模式，打印日志提醒重新登录

---

### Q：修改密码后守护进程还在运行吗？

是的，守护进程继续运行，但 token 被清除，切换为匿名模式。  
执行 `nat-client login` 用新密码重新登录即可恢复绑定状态。

---

### Q：能否同时登录多台设备？

可以。同一账户可以绑定多台设备，每台设备有独立的 `device_row_id`（udid）。  
查看所有绑定设备：`nat-client devices`。

---

### Q：服务器连接失败，一直重连

```bash
# 检查 hbbs 是否可达
telnet 1.2.3.4 21116

# 检查 HTTP API 是否可达
curl http://1.2.3.4:8080/

# 查看详细日志
nat-client daemon --server 1.2.3.4 --log-level debug
```

---

### Q：配置文件如何重置？

```bash
# Linux/macOS（删除后重启守护进程会重新生成 ID 和密钥）
rm ~/.config/nat-client/config.toml

# Windows
del %APPDATA%\nat-client\config.toml
```

> ⚠️ **重置后会生成新的 Peer ID 和密钥**，原有的连接关系和设备绑定需重新建立。

---

## 附录：完整通信时序图（含认证）

```
主机A (nat-client)   HTTP API(:8080)   hbbs(:21116)  hbbr(:21117)  主机B
       │                  │                 │               │           │
  [首次启动]               │                 │               │           │
       │── TCP 连接 ────────────────────────►│               │           │
       │── RegisterPk{id,pk,user_token:""} ─►│               │           │
       │◄── RegisterPkResponse{OK} ──────────│               │           │
       │                  │                 │               │           │
  [用户登录]               │                 │               │           │
       │── POST /login ──►│                 │               │           │
       │◄── JWT{udid:42} ─│                 │               │           │
       │ 触发重连           │                 │               │           │
       │── RegisterPk{user_token:"JWT"} ──────────────────► │           │
       │◄── RegisterPkResponse{OK} ──────────│               │           │
       │    （服务端完成 peer↔user 绑定）      │               │           │
       │                  │                 │               │           │
  [主机B也完成登录注册]      │                 │               │           │
  [A 发起连接 B]            │                 │               │           │
       │── PunchHoleRequest{id:"B的ID"} ──────────────────────────────── │
       │                  │── PunchHole ──────────────────────────────────►│
       │                  │                 │               │    [B处理打洞] │
       │◄── PunchHoleResponse ───────────────│               │           │
       │                  │                 │               │           │
  [直连成功]               │                 │               │           │
       │◄═══════════════════════════ 双向 TCP 数据流（直连）══════════════►│
       │                  │                 │               │           │
  [若打洞失败，走中继]       │                 │               │           │
       │── TCP 连接 hbbr ─────────────────────────────────────►│          │
       │◄══════════════════════════════ 双向代理（经 hbbr 中转）══════════►│
```

---

*文档版本：v0.2.0 | 最后更新：2025 年（新增用户认证与设备管理功能）*
