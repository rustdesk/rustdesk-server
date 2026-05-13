// 导入异步速度限制器
use async_speed_limit::Limiter;
// 导入异步trait支持
use async_trait::async_trait;
// 导入核心公共模块
use core_common::{
    allow_err, bail,
    bytes::{Bytes, BytesMut},
    futures_util::{sink::SinkExt, stream::StreamExt},
    log,
    protobuf::Message as _,
    rendezvous_proto::*,
    sleep,
    tcp::{listen_any, FramedStream},
    timeout,
    tokio::{
        self,
        io::{AsyncReadExt, AsyncWriteExt},
        net::{TcpListener, TcpStream},
        sync::{Mutex, RwLock},
        time::{interval, Duration},
    },
    ResultType,
};
// 导入Ed25519签名算法
use sodiumoxide::crypto::sign;
// 导入标准库模块
use std::{
    collections::{HashMap, HashSet},
    io::prelude::*,
    io::Error,
    net::SocketAddr,
    sync::atomic::{AtomicUsize, Ordering},
};
/// 连接使用情况类型定义
/// 包含运行时间、总流量、最高速度和当前速度统计
type Usage = (usize, usize, usize, usize);
// 全局静态变量定义
lazy_static::lazy_static! {
    /// 存储所有连接的Peer信息
    static ref PEERS: Mutex<HashMap<String, Box<dyn StreamTrait>>> = Default::default();
    /// 存储每个连接的使用情况
    static ref USAGE: RwLock<HashMap<String, Usage>> = Default::default();
    /// 黑名单IP地址集合
    static ref BLACKLIST: RwLock<HashSet<String>> = Default::default();
    /// 阻止名单IP地址集合
    static ref BLOCKLIST: RwLock<HashSet<String>> = Default::default();
}
/// 服务器配置常量
static DOWNGRADE_THRESHOLD_100: AtomicUsize = AtomicUsize::new(66); // 0.66 // 降级阈值百分比（0.66）
static DOWNGRADE_START_CHECK: AtomicUsize = AtomicUsize::new(1_800_000); // in ms // 降级检查开始时间（毫秒）
static LIMIT_SPEED: AtomicUsize = AtomicUsize::new(32 * 1024 * 1024); // in bit/s // 速度限制（比特/秒）
static TOTAL_BANDWIDTH: AtomicUsize = AtomicUsize::new(1024 * 1024 * 1024); // in bit/s // 总带宽限制（比特/秒）
static SINGLE_BANDWIDTH: AtomicUsize = AtomicUsize::new(128 * 1024 * 1024); // in bit/s // 单连接带宽限制（比特/秒）
const BLACKLIST_FILE: &str = "blacklist.txt"; // 黑名单文件路径
const BLOCKLIST_FILE: &str = "blocklist.txt"; // 阻止名单文件路径

/// 中继服务器主启动函数
/// # 参数
/// * `port` - 监听端口
/// * `key` - 服务器密钥文件路径
/// # 返回值
/// 返回服务器运行结果
#[tokio::main(flavor = "multi_thread")]
pub async fn start(port: &str, key: &str) -> ResultType<()> {
    // 获取服务器密钥对
    let key = get_server_sk(key);
    // 加载黑名单文件
    if let Ok(mut file) = std::fs::File::open(BLACKLIST_FILE) {
        let mut contents = String::new();
        if file.read_to_string(&mut contents).is_ok() {
            // 按行解析黑名单
            for x in contents.split('\n') {
                if let Some(ip) = x.trim().split(' ').next() {
                    BLACKLIST.write().await.insert(ip.to_owned());
                }
            }
        }
    }
    log::info!(
        "#blacklist({}): {}",
        BLACKLIST_FILE,
        BLACKLIST.read().await.len()
    );
    // 加载阻止名单文件
    if let Ok(mut file) = std::fs::File::open(BLOCKLIST_FILE) {
        let mut contents = String::new();
        if file.read_to_string(&mut contents).is_ok() {
            // 按行解析阻止名单
            for x in contents.split('\n') {
                if let Some(ip) = x.trim().split(' ').next() {
                    BLOCKLIST.write().await.insert(ip.to_owned());
                }
            }
        }
    }
    // 记录阻止名单加载结果
    log::info!(
        "#blocklist({}): {}",
        BLOCKLIST_FILE,
        BLOCKLIST.read().await.len()
    );
    // 解析端口号
    let port: u16 = port.parse()?;
    // 记录TCP监听端口
    log::info!("Listening on tcp :{}", port);
    // WebSocket端口（TCP端口+2）
    let port2 = port + 2;
    // 记录WebSocket监听端口
    log::info!("Listening on websocket :{}", port2);
    // 主服务循环任务
    let main_task = async move {
        loop {
            log::info!("Start");
            // 启动TCP和WebSocket监听
            io_loop(listen_any(port).await?, listen_any(port2).await?, &key).await;
        }
    };
    // 监听系统信号
    let listen_signal = crate::common::listen_signal();
    tokio::select!(
        res = main_task => res,
        res = listen_signal => res,
    )
}
/// 检查并加载环境参数
/// 从环境变量中读取降级阈值、速度限制等配置
fn check_params() {
    // 读取降级阈值参数
    let tmp = std::env::var("DOWNGRADE_THRESHOLD")
        .map(|x| x.parse::<f64>().unwrap_or(0.))
        .unwrap_or(0.);
    if tmp > 0. {
        // 转换为百分比存储
        DOWNGRADE_THRESHOLD_100.store((tmp * 100.) as _, Ordering::SeqCst);
    }
    // 记录当前降级阈值
    log::info!(
        "DOWNGRADE_THRESHOLD: {}",
        DOWNGRADE_THRESHOLD_100.load(Ordering::SeqCst) as f64 / 100.
    );
    // 读取降级检查开始时间
    let tmp = std::env::var("DOWNGRADE_START_CHECK")
        .map(|x| x.parse::<usize>().unwrap_or(0))
        .unwrap_or(0);
    if tmp > 0 {
        // 转换为毫秒存储
        DOWNGRADE_START_CHECK.store(tmp * 1000, Ordering::SeqCst);
    }
    // 记录降级检查开始时间
    log::info!(
        "DOWNGRADE_START_CHECK: {}s",
        DOWNGRADE_START_CHECK.load(Ordering::SeqCst) / 1000
    );
    // 读取速度限制参数
    let tmp = std::env::var("LIMIT_SPEED")
        .map(|x| x.parse::<f64>().unwrap_or(0.))
        .unwrap_or(0.);
    if tmp > 0. {
        // 转换为字节存储
        LIMIT_SPEED.store((tmp * 1024. * 1024.) as usize, Ordering::SeqCst);
    }
    // 记录当前速度限制
    log::info!(
        "LIMIT_SPEED: {}Mb/s",
        LIMIT_SPEED.load(Ordering::SeqCst) as f64 / 1024. / 1024.
    );
    // 读取总带宽参数
    let tmp = std::env::var("TOTAL_BANDWIDTH")
        .map(|x| x.parse::<f64>().unwrap_or(0.))
        .unwrap_or(0.);
    if tmp > 0. {
        // 转换为字节存储
        TOTAL_BANDWIDTH.store((tmp * 1024. * 1024.) as usize, Ordering::SeqCst);
    }
    // 记录当前总带宽限制
    log::info!(
        "TOTAL_BANDWIDTH: {}Mb/s",
        TOTAL_BANDWIDTH.load(Ordering::SeqCst) as f64 / 1024. / 1024.
    );
    // 读取单用户带宽参数
    let tmp = std::env::var("SINGLE_BANDWIDTH")
        .map(|x| x.parse::<f64>().unwrap_or(0.))
        .unwrap_or(0.);
    if tmp > 0. {
        // 转换为字节存储
        SINGLE_BANDWIDTH.store((tmp * 1024. * 1024.) as usize, Ordering::SeqCst);
    }
    // 记录当前单用户带宽限制
    log::info!(
        "SINGLE_BANDWIDTH: {}Mb/s",
        SINGLE_BANDWIDTH.load(Ordering::SeqCst) as f64 / 1024. / 1024.
    )
}
/// 处理命令
/// 
async fn check_cmd(cmd: &str, limiter: Limiter) -> String {
    use std::fmt::Write;

    let mut res = "".to_owned();
    let mut fds = cmd.trim().split(' ');
    match fds.next() {
        Some("h") => {
            res = format!(
                "{}\n{}\n{}\n{}\n{}\n{}\n{}\n{}\n{}\n{}\n{}\n{}\n",
                "blacklist-add(ba) <ip>",
                "blacklist-remove(br) <ip>",
                "blacklist(b) <ip>",
                "blocklist-add(Ba) <ip>",
                "blocklist-remove(Br) <ip>",
                "blocklist(B) <ip>",
                "downgrade-threshold(dt) [value]",
                "downgrade-start-check(t) [value(second)]",
                "limit-speed(ls) [value(Mb/s)]",
                "total-bandwidth(tb) [value(Mb/s)]",
                "single-bandwidth(sb) [value(Mb/s)]",
                "usage(u)"
            )
        }
        Some("blacklist-add" | "ba") => {
            if let Some(ip) = fds.next() {
                for ip in ip.split('|') {
                    BLACKLIST.write().await.insert(ip.to_owned());
                }
            }
        }
        Some("blacklist-remove" | "br") => {
            if let Some(ip) = fds.next() {
                if ip == "all" {
                    BLACKLIST.write().await.clear();
                } else {
                    for ip in ip.split('|') {
                        BLACKLIST.write().await.remove(ip);
                    }
                }
            }
        }
        Some("blacklist" | "b") => {
            if let Some(ip) = fds.next() {
                res = format!("{}\n", BLACKLIST.read().await.get(ip).is_some());
            } else {
                for ip in BLACKLIST.read().await.clone().into_iter() {
                    let _ = writeln!(res, "{ip}");
                }
            }
        }
        Some("blocklist-add" | "Ba") => {
            if let Some(ip) = fds.next() {
                for ip in ip.split('|') {
                    BLOCKLIST.write().await.insert(ip.to_owned());
                }
            }
        }
        Some("blocklist-remove" | "Br") => {
            if let Some(ip) = fds.next() {
                if ip == "all" {
                    BLOCKLIST.write().await.clear();
                } else {
                    for ip in ip.split('|') {
                        BLOCKLIST.write().await.remove(ip);
                    }
                }
            }
        }
        Some("blocklist" | "B") => {
            if let Some(ip) = fds.next() {
                res = format!("{}\n", BLOCKLIST.read().await.get(ip).is_some());
            } else {
                for ip in BLOCKLIST.read().await.clone().into_iter() {
                    let _ = writeln!(res, "{ip}");
                }
            }
        }
        Some("downgrade-threshold" | "dt") => {
            if let Some(v) = fds.next() {
                if let Ok(v) = v.parse::<f64>() {
                    if v > 0. {
                        DOWNGRADE_THRESHOLD_100.store((v * 100.) as _, Ordering::SeqCst);
                    }
                }
            } else {
                res = format!(
                    "{}\n",
                    DOWNGRADE_THRESHOLD_100.load(Ordering::SeqCst) as f64 / 100.
                );
            }
        }
        Some("downgrade-start-check" | "t") => {
            if let Some(v) = fds.next() {
                if let Ok(v) = v.parse::<usize>() {
                    if v > 0 {
                        DOWNGRADE_START_CHECK.store(v * 1000, Ordering::SeqCst);
                    }
                }
            } else {
                res = format!("{}s\n", DOWNGRADE_START_CHECK.load(Ordering::SeqCst) / 1000);
            }
        }
        Some("limit-speed" | "ls") => {
            if let Some(v) = fds.next() {
                if let Ok(v) = v.parse::<f64>() {
                    if v > 0. {
                        LIMIT_SPEED.store((v * 1024. * 1024.) as _, Ordering::SeqCst);
                    }
                }
            } else {
                res = format!(
                    "{}Mb/s\n",
                    LIMIT_SPEED.load(Ordering::SeqCst) as f64 / 1024. / 1024.
                );
            }
        }
        Some("total-bandwidth" | "tb") => {
            if let Some(v) = fds.next() {
                if let Ok(v) = v.parse::<f64>() {
                    if v > 0. {
                        TOTAL_BANDWIDTH.store((v * 1024. * 1024.) as _, Ordering::SeqCst);
                        limiter.set_speed_limit(TOTAL_BANDWIDTH.load(Ordering::SeqCst) as _);
                    }
                }
            } else {
                res = format!(
                    "{}Mb/s\n",
                    TOTAL_BANDWIDTH.load(Ordering::SeqCst) as f64 / 1024. / 1024.
                );
            }
        }
        Some("single-bandwidth" | "sb") => {
            if let Some(v) = fds.next() {
                if let Ok(v) = v.parse::<f64>() {
                    if v > 0. {
                        SINGLE_BANDWIDTH.store((v * 1024. * 1024.) as _, Ordering::SeqCst);
                    }
                }
            } else {
                res = format!(
                    "{}Mb/s\n",
                    SINGLE_BANDWIDTH.load(Ordering::SeqCst) as f64 / 1024. / 1024.
                );
            }
        }
        Some("usage" | "u") => {
            let mut tmp: Vec<(String, Usage)> = USAGE
                .read()
                .await
                .iter()
                .map(|x| (x.0.clone(), *x.1))
                .collect();
            tmp.sort_by(|a, b| ((b.1).1).partial_cmp(&(a.1).1).unwrap());
            for (ip, (elapsed, total, highest, speed)) in tmp {
                if elapsed == 0 {
                    continue;
                }
                let _ = writeln!(
                    res,
                    "{}: {}s {:.2}MB {}kb/s {}kb/s {}kb/s",
                    ip,
                    elapsed / 1000,
                    total as f64 / 1024. / 1024. / 8.,
                    highest,
                    total / elapsed,
                    speed
                );
            }
        }
        // 未知命令
        _ => {}
    }
    res
}

/// 主循环监听TCP和WebSocket连接
/// 
/// # Arguments
/// * `listener` - TCP监听器
/// * `listener2` - WebSocket监听器
/// * `key` - 密钥
async fn io_loop(listener: TcpListener, listener2: TcpListener, key: &str) {
    // 检查并加载环境参数
    check_params();
    // 创建总带宽限制器
    let limiter = <Limiter>::new(TOTAL_BANDWIDTH.load(Ordering::SeqCst) as _);
    // 主循环监听TCP和WebSocket连接
    loop {
        tokio::select! {
            // 处理TCP连接
            res = listener.accept() => {
                match res {
                    Ok((stream, addr))  => {
                        // 设置TCP_NODELAY优化性能
                        stream.set_nodelay(true).ok();
                        // 处理TCP连接
                        handle_connection(stream, addr, &limiter, key, false).await;
                    }
                    Err(err) => {
                        // 记录TCP监听错误
                       log::error!("listener.accept failed: {}", err);
                       break;
                    }
                }
            }
            // 处理WebSocket连接
            res = listener2.accept() => {
                match res {
                    Ok((stream, addr))  => {
                        // 设置TCP_NODELAY优化性能
                        stream.set_nodelay(true).ok();
                        // 处理WebSocket连接
                        handle_connection(stream, addr, &limiter, key, true).await;
                    }
                    Err(err) => {
                        // 记录WebSocket监听错误
                       log::error!("listener2.accept failed: {}", err);
                       break;
                    }
                }
            }
        }
    }
}
/// 处理连接函数
/// # 参数
/// * `stream` - TCP流
/// * `addr` - 客户端地址
/// * `limiter` - 速度限制器
/// * `key` - 服务器密钥
/// * `ws` - 是否为WebSocket连接
async fn handle_connection(
    stream: TcpStream,
    addr: SocketAddr,
    limiter: &Limiter,
    key: &str,
    ws: bool,
) {
    // 获取客户端IP地址
    let ip = core_common::try_into_v4(addr).ip();
    // 处理本地回环连接（管理接口）
    if !ws && ip.is_loopback() {
        let limiter = limiter.clone();
        tokio::spawn(async move {
            let mut stream = stream;
            let mut buffer = [0; 1024];
            // 读取管理命令
            if let Ok(Ok(n)) = timeout(1000, stream.read(&mut buffer[..])).await {
                if let Ok(data) = std::str::from_utf8(&buffer[..n]) {
                    // 执行管理命令
                    let res = check_cmd(data, limiter).await;
                    // 返回命令执行结果
                    stream.write(res.as_bytes()).await.ok();
                }
            }
        });
        return;
    }
    // 检查IP是否在阻止名单中
    let ip = ip.to_string();
    if BLOCKLIST.read().await.get(&ip).is_some() {
        log::info!("{} blocked", ip);
        return;
    }
    // 复制密钥和限制器
    let key = key.to_owned();
    let limiter = limiter.clone();
    // 异步处理连接配对
    tokio::spawn(async move {
        allow_err!(make_pair(stream, addr, &key, limiter, ws).await);
    });
}

/// 建立连接配对函数
/// # 参数
/// * `stream` - TCP流
/// * `addr` - 客户端地址
/// * `key` - 服务器密钥
/// * `limiter` - 速度限制器
/// * `ws` - 是否为WebSocket连接
/// # 返回值
/// 返回连接配对结果
async fn make_pair(
    stream: TcpStream,
    mut addr: SocketAddr,
    key: &str,
    limiter: Limiter,
    ws: bool,
) -> ResultType<()> {
    // 处理WebSocket连接
    if ws {
        use tokio_tungstenite::tungstenite::handshake::server::{Request, Response}; 
        // WebSocket握手回调函数
        let callback = |req: &Request, response: Response| {
            // 获取请求头
            let headers = req.headers();
            // 获取真实IP地址
            let real_ip = headers
                .get("X-Real-IP")
                .or_else(|| headers.get("X-Forwarded-For"))
                .and_then(|header_value| header_value.to_str().ok());
                // 如果有真实IP，更新地址
            if let Some(ip) = real_ip {
                if ip.contains('.') {
                    // IPv4地址格式
                    addr = format!("{ip}:0").parse().unwrap_or(addr);
                } else {
                    // IPv6地址格式
                    addr = format!("[{ip}]:0").parse().unwrap_or(addr);
                }
            }
            Ok(response)
        };
        // 接受WebSocket连接
        let ws_stream = tokio_tungstenite::accept_hdr_async(stream, callback).await?;
        // 处理WebSocket流
        make_pair_(ws_stream, addr, key, limiter).await;
    } else {
        // 处理TCP连接
        make_pair_(FramedStream::from(stream, addr), addr, key, limiter).await;
    }
    Ok(())
}
/// 处理连接配对的内部函数
/// # 参数
/// * `stream` - 流
/// * `addr` - 客户端地址
/// * `key` - 服务器密钥
/// * `limiter` - 速度限制器
async fn make_pair_(stream: impl StreamTrait, addr: SocketAddr, key: &str, limiter: Limiter) {
    let mut stream = stream;
    // 等待客户端发送消息
    if let Ok(Some(Ok(bytes))) = timeout(30_000, stream.recv()).await {
        // 解析消息
        if let Ok(msg_in) = RendezvousMessage::parse_from_bytes(&bytes) {
            // 检查是否是请求中继消息
            if let Some(rendezvous_message::Union::RequestRelay(rf)) = msg_in.union {
                // 验证密钥
                if !key.is_empty() && rf.licence_key != key {
                    log::warn!("Relay authentication failed from {} - invalid key", addr);
                    return;
                }
                // 检查UUID是否为空
                if !rf.uuid.is_empty() {
                    // 从PEERS中移除UUID对应的连接
                    let mut peer = PEERS.lock().await.remove(&rf.uuid);
                    // 如果找到了配对的连接
                    if let Some(peer) = peer.as_mut() {
                        log::info!("Relayrequest {} from {} got paired", rf.uuid, addr);
                        let id = format!("{}:{}", addr.ip(), addr.port());
                        // 插入使用记录
                        USAGE.write().await.insert(id.clone(), Default::default());
                        // 如果都不是WebSocket，设置为原始模式
                        if !stream.is_ws() && !peer.is_ws() {
                            peer.set_raw();
                            stream.set_raw();
                            log::info!("Both are raw");
                        }
                        // 开始中继
                        if let Err(err) = relay(addr, &mut stream, peer, limiter, id.clone()).await
                        {
                            log::info!("Relay of {} closed: {}", addr, err);
                        } else {
                            log::info!("Relay of {} closed", addr);
                        }
                        // 移除使用记录
                        USAGE.write().await.remove(&id);
                    } else {
                        // 没有找到配对的连接，等待30秒后移除
                        log::info!("New relay request {} from {}", rf.uuid, addr);
                        // 插入到PEERS中
                        PEERS.lock().await.insert(rf.uuid.clone(), Box::new(stream));
                        // 等待30秒
                        sleep(30.).await;
                        // 移除PEERS中的连接
                        PEERS.lock().await.remove(&rf.uuid);
                    }
                }
            }
        }
    }
}
/// 中继数据传输
/// # 参数
/// * `addr` - 客户端地址
/// * `stream` - 客户端流
/// * `peer` - 对等端流
/// * `total_limiter` - 总速度限制器
/// * `id` - 客户端ID
async fn relay(
    addr: SocketAddr,
    stream: &mut impl StreamTrait,
    peer: &mut Box<dyn StreamTrait>,
    total_limiter: Limiter,
    id: String,
) -> ResultType<()> {
    // 获取客户端IP
    let ip = addr.ip().to_string();
    // 初始化时间
    let mut tm = std::time::Instant::now();
    // 初始化变量
    let mut elapsed = 0;
    // 总传输量
    let mut total = 0;
    // 每秒传输量
    let mut total_s = 0;
    // 最高每秒传输量
    let mut highest_s = 0;
    // 是否降级
    let mut downgrade: bool = false;
    // 是否被拉黑
    let mut blacked: bool = false;
    // 单个连接带宽
    let sb = SINGLE_BANDWIDTH.load(Ordering::SeqCst) as f64;
    // 单个连接限流器
    let limiter = <Limiter>::new(sb);
    // 黑名单限流器
    let blacklist_limiter = <Limiter>::new(LIMIT_SPEED.load(Ordering::SeqCst) as _);
    // 降级阈值
    let downgrade_threshold =
        (sb * DOWNGRADE_THRESHOLD_100.load(Ordering::SeqCst) as f64 / 100. / 1000.) as usize; // in bit/ms
    // 定时器
    let mut timer = interval(Duration::from_secs(3));
    // 最后接收时间
    let mut last_recv_time = std::time::Instant::now();
    loop {
        tokio::select! {
            // 接收对等端数据
            res = peer.recv() => {
                // 检查是否有数据
                if let Some(Ok(bytes)) = res {
                    // 更新最后接收时间
                    last_recv_time = std::time::Instant::now();
                    // 计算字节数
                    let nb = bytes.len() * 8;
                    // 如果被拉黑或降级，使用黑名单限流器
                    if blacked || downgrade {
                        blacklist_limiter.consume(nb).await;
                    } else {
                        limiter.consume(nb).await;
                    }
                    // 总限流器
                    total_limiter.consume(nb).await;
                    // 总传输量
                    total += nb;
                    // 每秒传输量
                    total_s += nb;
                    // 如果数据不为空，发送给客户端
                    if !bytes.is_empty() {
                        stream.send_raw(bytes.into()).await?;
                    }
                } else {
                    break;
                }
            },
            // 接收客户端数据
            res = stream.recv() => {
                // 检查是否有数据
                if let Some(Ok(bytes)) = res {
                    // 更新最后接收时间
                    last_recv_time = std::time::Instant::now();
                    // 计算字节数
                    let nb = bytes.len() * 8;
                    // 如果被拉黑或降级，使用黑名单限流器
                    if blacked || downgrade {
                        blacklist_limiter.consume(nb).await;
                    } else {
                        limiter.consume(nb).await;
                    }
                    // 总限流器
                    total_limiter.consume(nb).await;
                    // 总传输量
                    total += nb;
                    // 每秒传输量
                    total_s += nb;
                    // 如果数据不为空，发送给对等端
                    if !bytes.is_empty() {
                        peer.send_raw(bytes.into()).await?;
                    }
                } else {
                    break;
                }
            },
            // 定时器
            _ = timer.tick() => {
                // 检查是否超时
                if last_recv_time.elapsed().as_secs() > 30 {
                    bail!("Timeout");
                }
            }
        }
        // 计算时间
        let n = tm.elapsed().as_millis() as usize;
        if n >= 1_000 {
            // 检查是否被拉黑
            if BLOCKLIST.read().await.get(&ip).is_some() {
                log::info!("{} blocked", ip);
                break;
            }
            // 检查是否在黑名单中
            blacked = BLACKLIST.read().await.get(&ip).is_some();
            // 重置时间
            tm = std::time::Instant::now();
            // 计算速度
            let speed = total_s / n;
            // 更新最高速度
            if speed > highest_s {
                highest_s = speed;
            }
            // 累加时间
            elapsed += n;
            // 更新使用情况
            USAGE.write().await.insert(
                id.clone(),
                (elapsed as _, total as _, highest_s as _, speed as _),
            );
            // 重置每秒传输量
            total_s = 0;
            // 检查是否需要降级
            if elapsed > DOWNGRADE_START_CHECK.load(Ordering::SeqCst)
                && !downgrade
                && total > elapsed * downgrade_threshold
            {
                downgrade = true;
                log::info!(
                    "Downgrade {}, exceed downgrade threshold {}bit/ms in {}ms",
                    id,
                    downgrade_threshold,
                    elapsed
                );
            }
        }
    }
    Ok(())
}
/// 获取服务器私钥
/// # 参数
/// * `key` - 密钥
/// # 返回
/// * `String` - 服务器私钥
fn get_server_sk(key: &str) -> String {
    let mut key = key.to_owned();
    if let Ok(sk) = base64::decode(&key) {
        if sk.len() == sign::SECRETKEYBYTES {
            log::info!("The key is a crypto private key");
            key = base64::encode(&sk[(sign::SECRETKEYBYTES / 2)..]);
        }
    }

    if key == "-" || key == "_" {
        let (pk, _) = crate::common::gen_sk(300);
        key = pk;
    }

    if !key.is_empty() {
        log::info!("Key: {}", key);
    }

    key
}
/// 流trait
/// # 功能
/// * `recv` - 接收数据
/// * `send_raw` - 发送原始数据
/// * `is_ws` - 是否是WebSocket
/// * `set_raw` - 设置为原始模式
#[async_trait]
trait StreamTrait: Send + Sync + 'static {
    /// 接收数据
    async fn recv(&mut self) -> Option<Result<BytesMut, Error>>;
    /// 发送原始数据
    async fn send_raw(&mut self, bytes: Bytes) -> ResultType<()>;
    /// 是否是WebSocket
    fn is_ws(&self) -> bool;
    fn set_raw(&mut self);
}
/// 实现StreamTrait trait
/// # 类型
/// * `FramedStream` - 帧流
#[async_trait]
impl StreamTrait for FramedStream {
    /// 接收数据
    async fn recv(&mut self) -> Option<Result<BytesMut, Error>> {
        self.next().await
    }

    /// 发送原始数据
    async fn send_raw(&mut self, bytes: Bytes) -> ResultType<()> {
        self.send_bytes(bytes).await
    }

    /// 是否是WebSocket
    fn is_ws(&self) -> bool {
        false
    }

    /// 设置为原始模式
    fn set_raw(&mut self) {
        self.set_raw();
    }
}
/// WebSocketStream的StreamTrait实现
/// 为WebSocket流实现流特征
/// 实现StreamTrait trait
/// # 类型
/// * `tokio_tungstenite::WebSocketStream<TcpStream>` - WebSocket流
#[async_trait]
impl StreamTrait for tokio_tungstenite::WebSocketStream<TcpStream> {
    /// 接收数据
    async fn recv(&mut self) -> Option<Result<BytesMut, Error>> {
        if let Some(msg) = self.next().await {
            match msg {
                Ok(msg) => {
                    match msg {
                        // WebSocket二进制消息
                        tungstenite::Message::Binary(bytes) => {
                            Some(Ok(bytes[..].into())) // to-do: poor performance
                        }
                        _ => Some(Ok(BytesMut::new())),
                    }
                }
                Err(err) => Some(Err(Error::new(std::io::ErrorKind::Other, err.to_string()))),
            }
        } else {
            None
        }
    }

    /// 发送原始数据
    async fn send_raw(&mut self, bytes: Bytes) -> ResultType<()> {
        Ok(self
            .send(tungstenite::Message::Binary(bytes.to_vec()))
            .await?) // to-do: poor performance
    }

    /// 是否是WebSocket
    fn is_ws(&self) -> bool {
        true
    }

    /// 设置为原始模式
    fn set_raw(&mut self) {}
}
