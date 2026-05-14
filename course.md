已实现端到端绑定与认证，说明如下。

1. 协议：RegisterPk.user_token（字段 6）
在 libs/core-common/protos/rendezvous.proto 与 libs/hbb_common/protos/rendezvous.proto 中为 RegisterPk 增加了：

string user_token = 6 — 携带与 Web /api/login 相同的 JWT（与 JWT_SECRET 一致）。
注意：官方 RustDesk 客户端若未改代码，不会填该字段；未设置强制校验时仍可像以前一样注册。

2. 登录 JWT：udid + 可选 device_id
Claims 增加可选字段 udid（对应 user_devices 表主键 id）。
LoginRequest 增加可选 device_id（与 user_devices.device_id 一致，一般为 RustDesk 的 9 位 ID）。
在 api.rs 与 web.rs 的登录逻辑中：若传了 device_id，会校验归属并在 JWT 里写入 udid；不传则仅靠 RegisterPk.id 与库中 user_devices.device_id 匹配 完成绑定（JWT 里无 udid 时由 resolve_peer_device_binding 按 peer id 查表）。
3. PeerMap::update_pk（src/peer.rs）
环境变量 REQUIRE_PEER_JWT=1 或 true：必须提供非空 user_token，否则返回 SERVER_ERROR。
未设置时：无 token 则行为与旧版一致（不强制账号绑定）。
校验 JWT：sub 为用户 id（支持数字或字符串），可选 udid；再调用 Database::resolve_peer_device_binding：
有 udid：校验该行的 user_id、device_id 必须与当前 RegisterPk.id 一致；
无 udid：用 user_id + RegisterPk.id 在 user_devices 中查行。
与 main.rs / api 对齐，默认密钥使用 your-secret-key-change-in-production（仍优先读 JWT_SECRET）。
从 DB 加载 peer 时：在 get_peer 中读出 user_id/device_id，并在 PeerMap::get 写回内存中的 user_id / device_id（对应 user_devices.id）。
4. Rendezvous：TCP / UDP
handle_tcp 与 UDP RegisterPk 分支会把 user_token 从消息里取出并传入 update_pk（不再写死 None）。
5. 数据库（src/database.rs）
get_peer：查询并填充 bound_user_id / bound_device_row_id（来自 peer.user_id、peer.device_id）。
resolve_peer_device_binding、get_user_device_row_id：支撑上述校验与登录选设备。
使用流程简述

用户在 user_devices 里登记设备的 device_id = RustDesk ID`（与现有 API 一致）。
登录：POST /api/login，可带 "device_id": "<RustDesk ID>" 以在 JWT 中写入 udid。
客户端在 RegisterPk 里带上 user_token（上述 JWT）；若开启 REQUIRE_PEER_JWT，则必须带且校验通过。
服务端把 users.id 与 user_devices.id 写入 peer 表对应列，并与内存 Peer 一致。
cargo build 已通过。若你需要下一步在 RustDesk 客户端 里自动把 JWT 填进 RegisterPk，需要单独改客户端并重新生成其 protobuf。

已做的改动
1. 设备页真正走「用户 → 多设备」API（templates/devices.html）
GET /api/users/{当前用户 id}/devices：拉取该用户在 user_devices 里的全部记录，用 allDevices 保存一对多列表。
POST /api/devices：添加设备（请求体 device_id + 可选 device_name）。
DELETE /api/users/{id}/devices/{device_id}：从该用户下移除设备。
所有请求带 Authorization: Bearer <jwt_token>（与登录写入的 localStorage 一致）。
搜索只过滤展示，不再破坏 allDevices；去掉原来的假数据与「编辑」按钮（暂无改名接口）。
登录后若服务端未渲染用户名，会用 localStorage 里的 user_info 填充右上角用户名 + 退出。
2. 后端鉴权（与前端配套）
api.rs：jwt_user_id_from_headers；get_user_devices / remove_device 必须带合法 JWT，且 路径里的 user_id 必须与 JWT 的 sub 一致，否则 403。
device_api.rs：add_device / remove_device_by_id 不再写死 user_id = 1，改为从 Bearer JWT 解析 sub 作为当前用户。
3. 设备页服务端模板数据（web.rs）
devices_page 的 current_user 改为 None（真实身份以浏览器里的 JWT/user_info 为准），避免再显示假的 test_user。
未改动的 device_pages.rs 里那份内嵌「设备管理」大段 HTML 仍是演示用；当前 main 走的是 web + Askama 模板，实际生效的是上面的 devices.html。若你以后会挂载 create_api_router，再考虑删掉或同步那份内嵌页即可。

已接上用户管理页面，改动如下。

1. 路由与处理器（src/web.rs）
新增 users_page，用 Askama 渲染用户管理页。
注册 GET /users，与控制台里「用户管理」链接一致。
2. 模板类型（src/views/mod.rs）
增加 UsersTemplate，对应 templates/users.html。
3. 页面（templates/users.html）
继承 layout.html，导航与设备页一致（控制台 / 设备 / 用户管理 高亮）。
登录校验与设备页相同：未登录跳转 /login，顶部显示当前用户名与退出。
列表：GET /api/users?limit=500&offset=0，表格展示 ID、用户名、邮箱、注册时间、启用状态。
筛选：按用户名、邮箱本地过滤。
新建用户：弹窗表单，调用已有 POST /api/register（与注册页同一套校验与创建逻辑）。
编辑：PUT /api/users/:id，提交用户名、邮箱；若改的是当前登录用户，会同步更新 localStorage 里的 user_info。
删除：DELETE /api/users/:id；若删的是当前账号，会执行退出并跳转登录。
本地已通过 cargo build --bin hbbs 编译。

说明： 新建用户走的是公开注册接口；当前后端对用户列表/改删也未做「仅管理员」限制，与现有 API 行为一致。若以后要区分管理员与普通用户，需要在 API 层加角色或权限后再收紧界面逻辑。

已实现基于 admin / user 双角色 的权限模型，并与 JWT、REST API 和 Web 页面对齐。要点如下。

数据库（src/database.rs + src/database_simple.rs）
表 users 增加列 role（TEXT NOT NULL DEFAULT 'user'），启动时自动 ALTER 迁移。
常量 USER_ROLE_ADMIN / USER_ROLE_USER。
若当前 没有任何 admin：
优先把环境变量 BOOTSTRAP_ADMIN_USERNAME 指定用户设为管理员；
否则把 id 最小 的用户提升为管理员（并打日志说明）。
新增 count_admins、set_user_role；所有读写 User 的 SQL 已带上 role。
JWT 与登录（src/api.rs、src/web.rs）
Claims 增加 role（旧令牌缺字段时反序列化为 user）。
登录颁发的 JWT 与 UserInfo 均带 role（UserInfo 增加 role 字段）。
Web 登录增加 is_active 校验，与 API 侧一致。
API 权限（src/api.rs）
接口	规则
GET /api/users
仅 管理员（JWT + 数据库校验 role）
POST /api/users
管理员创建用户（替代管理页里对 /api/register 的依赖）
GET /api/users/:id
本人或管理员
PUT /api/users/:id
本人或管理员
DELETE /api/users/:id
本人或管理员；删除 最后一个管理员 时拒绝
PUT /api/users/:id/role
仅管理员；把用户从 admin 改为 user 时若会撤销 最后一个管理员 则拒绝
GET /api/users/:id/devices
本人或管理员（管理员可查看他人设备列表）
辅助函数 db_user_is_admin；JWT 解析统一走 decode_claims::<Claims>（sub 为字符串数字）。

Web 路由（src/web.rs）
POST /api/users → admin_create_user
PUT /api/users/:id/role → update_user_role
前端（Askama 模板）
views::UserInfo 增加 role（供以后服务端渲染用）。
控制台 / 设备页：带 nav-admin-only 的「用户管理」入口与控制台卡片仅在 user.role === 'admin' 时显示（dashboard.html、devices.html）。
用户管理页：非管理员跳转控制台；列表增加 角色列；对他人可 下拉修改角色（调 PUT .../role）；新建用户改为 POST /api/users（需管理员 JWT）。
部署说明：已有数据库在首次启动新版本时会加列并自动保证至少一名管理员；生产环境建议设置 BOOTSTRAP_ADMIN_USERNAME 明确首个管理员账号。旧 JWT 无 role 字段时会按 普通用户 处理，管理员在升级后应 重新登录 以拿到带 role 的新令牌。




---

**问题总结（三层叠加）：**

1. **死锁（根本原因）**：`create_tables()` 持有连接池里唯一一个连接（`MAX_DATABASE_CONNECTIONS=1`），然后调用 `migrate_users_add_role_column()` 和 `bootstrap_admin_if_none()`，这两个函数也试图从同一个池获取连接，永远等待 → 程序挂死。修复：在调用它们之前显式 `drop(conn)`。

2. **嵌套 Runtime**：`main` 已有 `#[tokio::main]` runtime，代码里又 `Runtime::new()` 创建第二个，`RendezvousServer::start` 自身还有第三个 `#[tokio::main]`。修复：去掉多余的 `Runtime::new()`，API server 改用 `tokio::spawn`，RendezvousServer 改用 `std::thread::spawn` + `oneshot channel`。

3. **日志丢失**：`WriteMode::Async` 在程序异常退出时缓冲区来不及刷新。修复：改为 `WriteMode::Direct`。

现在日志正常输出，8080 端口正常监听，HTTP 返回 200。

我的建议

**现在不要做**。当前的 proto3 实现运行良好（刚修复的那套），capnp 带来的收益（序列化速度 +20-30%）与引入的风险和工作量不成比例。

如果你的目标是**性能**，更值得做的方向是：

- 把连接池 `MAX_DATABASE_CONNECTIONS` 适当调大
- 对热路径加内存缓存（peer 状态）
- 用 release 模式编译 (`cargo build --release`)

如果你确实想推进 capnp 迁移，我可以帮你从正确的方式一步步来——但需要作为一个独立的专项，而不是"开关一下就替换"。


能否跟据https://github.com/rustdesk/rustdesk.git项目源码的ipc.rs、lan.rs和rendezvous_mediator.rs
等源文件，实现一个与nat-server项目相结合的完整内网穿透工具完成？

实现概要
1. 共享编解码（libs/core-common）
增加 protos/rendezvous.capnp，在 build.rs 里与 protobuf 一起用 capnpc 生成 Rust。
根模块 rendezvous_capnp 放在 lib.rs（满足生成代码里的 crate::rendezvous_capnp）。
新增 rendezvous_codec：Protocol、detect、parse、serialize，与原先 hbbs 中逻辑一致，并带有单元测试。
2. nat-server（hbbs）
src/lib.rs 中 codec 改为 pub use core_common::rendezvous_codec::*。
删除重复的 src/codec.rs、src/proto/，根目录 build.rs 不再编译 capnp。
tcp_punch：HashMap<SocketAddr, (Sink, Protocol)>，在 PunchHoleRequest / RequestRelay 入库时写入该 TCP 上的协议；send_to_tcp / send_to_tcp_sync 按映射回包（不再写死 Proto3）。
UDP 打洞错误：handle_udp_punch_hole_request 在只回给请求方时用请求方的 proto，不再误用 target_proto。
PunchHoleSent / LocalAddr 经 UDP 回给对端：用 PeerMap::protocol_for_addr(addr_a) 选编码。
PeerMap::protocol_for_addr：按 try_into_v4(socket_addr) 匹配已注册 peer 的 protocol 字段。
3. nat-client
配置项 rendezvous_wire_protocol（proto3 / capnp，默认 proto3），ClientConfig::get_rendezvous_wire_protocol()。
gui / daemon 增加 --rendezvous-protocol proto3|capnp，传入 config::init(..., Option<RendezvousWireProtocol>) 写入配置。
rendezvous_mediator：收包统一 rendezvous_codec::parse；发包用 serialize + send_bytes / send_raw；connect_to_peer 同样按配置编解码。
使用说明
默认仍为 proto3，与现有客户端兼容。
若要用 capnp：客户端 config.toml 中设 rendezvous_wire_protocol = "capnp"，或启动时加 --rendezvous-protocol capnp；须与对端/服务器实际帧格式一致（服务器仍按首字节 0x00 自动识别入站协议，并按 peer 记录与 tcp_punch 映射回包）。
局域网 lan.rs 的 PeerDiscovery 仍为 protobuf，与本次 rendezvous/hbbs 双协议无关。

1. libs/core-common/src/rendezvous_codec.rs
capnp → proto3：增加 W::PeerDiscovery，把 capnp 的 PeerDiscovery 填进 RendezvousMessage::peer_discovery。
proto3 → capnp：增加 Union::PeerDiscovery，init_peer_discovery() 并写入各字段。
测试：新增 peer_discovery_capnp_roundtrip。
2. nat-client/src/lan.rs
serialize_lan_message：用 rendezvous_codec::serialize（失败时退回 write_to_bytes）。
start_listening：入站用 rendezvous_codec::parse；pong 用 rendezvous_codec::detect 得到的协议编码，与对端 ping 格式一致（proto3 / capnp 可混用）。
discover：广播 ping 使用 ClientConfig::get_rendezvous_wire_protocol()（与 hbbs / rendezvous_mediator 同一配置）。
collect_responses：入站用 rendezvous_codec::parse，可收两种格式。
模块注释已说明上述行为。
构建与 peer_discovery_capnp_roundtrip 测试已通过。
