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
