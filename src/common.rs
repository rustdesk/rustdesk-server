// 导入命令行参数解析库
use clap::App;
// 导入核心公共模块
use core_common::{
    allow_err, anyhow::{Context, Result}, get_version_number, log, tokio, ResultType
};
// 导入INI配置文件处理库
use ini::Ini;
// 导入Ed25519签名算法库
use sodiumoxide::crypto::sign;
// 导入标准库模块
use std::{
    io::prelude::*,
    io::Read,
    net::SocketAddr,
    time::{Instant, SystemTime},
};

#[allow(dead_code)]
/// 获取过期时间（当前时间减去1小时）
/// 返回一个Instant对象，表示1小时前的时间点
/// 如果计算溢出则返回当前时间
pub(crate) fn get_expired_time() -> Instant {
    let now = Instant::now();
    now.checked_sub(std::time::Duration::from_secs(3600))
        .unwrap_or(now)
}

#[allow(dead_code)]
/// 测试服务器地址是否有效
/// # 参数
/// * `host` - 服务器主机名或IP地址
/// * `name` - 服务器类型名称（用于日志）
/// # 返回值
/// 返回解析后的SocketAddr，如果解析失败则返回错误
pub(crate) fn test_if_valid_server(host: &str, name: &str) -> ResultType<SocketAddr> {
    use std::net::ToSocketAddrs;
    // 如果主机名包含端口号，直接解析
    let res = if host.contains(':') {
        host.to_socket_addrs()?.next().context("")
    } else {
        // 否则添加默认端口0进行解析
        format!("{}:{}", host, 0)
            .to_socket_addrs()?
            .next()
            .context("")
    };
    // 如果解析失败，记录错误日志
    if res.is_err() {
        log::error!("Invalid {} {}: {:?}", name, host, res);
    }
    res
}

#[allow(dead_code)]
/// 从逗号分隔的字符串中解析服务器列表
/// # 参数
/// * `s` - 逗号分隔的服务器字符串
/// * `tag` - 日志标签
/// # 返回值
/// 返回有效的服务器地址列表
pub(crate) fn get_servers(s: &str, tag: &str) -> Vec<String> {
    // 按逗号分割字符串，过滤空值和无效地址
    let servers: Vec<String> = s
        .split(',')
        .filter(|x| !x.is_empty() && test_if_valid_server(x, tag).is_ok())
        .map(|x| x.to_owned())
        .collect();
    // 记录解析结果
    log::info!("{}={:?}", tag, servers);
    servers
}

#[allow(dead_code)]
#[inline]
fn arg_name(name: &str) -> String {
    name.to_uppercase().replace('_', "-")
}

#[allow(dead_code)]
/// 初始化命令行参数和环境变量
/// 按优先级加载配置：命令行参数 > 配置文件 > .env文件
/// # 参数
/// * `args` - 命令行参数字符串
/// * `name` - 应用程序名
/// * `about` - 应用程序描述
pub fn init_args(args: &str, name: &str, about: &str) {
    // 解析命令行参数
    let matches = App::new(name)
        .version(crate::version::VERSION)
        .author("Purslane Ltd. <info@rustdesk.com>")
        .about(about)
        .args_from_usage(args)
        .get_matches();
    
    // 首先加载.env文件中的配置
    if let Ok(v) = Ini::load_from_file(".env") {
        if let Some(section) = v.section(None::<String>) {
            section
                .iter()
                .for_each(|(k, v)| std::env::set_var(arg_name(k), v));
        }
    }
    
    // 然后加载指定的配置文件
    if let Some(config) = matches.value_of("config") {
        if let Ok(v) = Ini::load_from_file(config) {
            if let Some(section) = v.section(None::<String>) {
                section
                    .iter()
                    .for_each(|(k, v)| std::env::set_var(arg_name(k), v));
            }
        }
    }
    
    // 最后加载命令行参数（优先级最高）
    for (k, v) in matches.args {
        if let Some(v) = v.vals.first() {
            std::env::set_var(arg_name(k), v.to_string_lossy().to_string());
        }
    }
}

#[allow(dead_code)]
/// 获取命令行参数或环境变量的值
/// # 参数
/// * `name` - 参数名
/// # 返回值
/// 返回参数值，如果不存在则返回空字符串
#[inline]
pub fn get_arg(name: &str) -> String {
    get_arg_or(name, "".to_owned())
}

#[allow(dead_code)]
/// 获取命令行参数或环境变量的值，带默认值
/// # 参数
/// * `name` - 参数名
/// * `default` - 默认值
/// # 返回值
/// 返回参数值，如果不存在则返回默认值
#[inline]
pub fn get_arg_or(name: &str, default: String) -> String {
    std::env::var(arg_name(name)).unwrap_or(default)
}

#[allow(dead_code)]
/// 获取当前Unix时间戳
/// # 返回值
/// 返回从1970年1月1日以来的秒数
#[inline]
pub fn now() -> u64 {
    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|x| x.as_secs())
        .unwrap_or_default()
}

/// 生成或加载Ed25519密钥对
/// # 参数
/// * `wait` - 等待时间（毫秒），如果密钥文件不存在则等待
/// # 返回值
/// 返回公钥字符串和私钥对象，如果生成失败则返回空值
pub fn gen_sk(wait: u64) -> (String, Option<sign::SecretKey>) {
    let sk_file = "id_ed25519";  // 私钥文件名
    
    // 如果需要等待且文件不存在，则等待指定时间
    if wait > 0 && !std::path::Path::new(sk_file).exists() {
        std::thread::sleep(std::time::Duration::from_millis(wait));
    }
    
    // 尝试从文件加载现有私钥
    if let Ok(mut file) = std::fs::File::open(sk_file) {
        let mut contents = String::new();
        if file.read_to_string(&mut contents).is_ok() {
            let contents = contents.trim();
            let sk = base64::decode(contents).unwrap_or_default();
            // 验证私钥长度是否正确
            if sk.len() == sign::SECRETKEYBYTES {
                let mut tmp = [0u8; sign::SECRETKEYBYTES];
                tmp[..].copy_from_slice(&sk);
                // 提取公钥部分（私钥前半部分是公钥）
                let pk = base64::encode(&tmp[sign::SECRETKEYBYTES / 2..]);
                log::info!("Private key comes from {}", sk_file);
                return (pk, Some(sign::SecretKey(tmp)));
            } else {
                // 私钥格式错误，致命错误退出
                // don't use log here, since it is async
                println!("Fatal error: malformed private key in {sk_file}.");
                std::process::exit(1);
            }
        }
    } else {
        // 生成新的密钥对
        let gen_func = || {
            let (tmp, sk) = sign::gen_keypair();
            (base64::encode(tmp), sk)
        };
        let (mut pk, mut sk) = gen_func();
        
        // 生成有效密钥，最多尝试300次
        for _ in 0..300 {
            if !pk.contains('/') && !pk.contains(':') {
                break;
            }
            (pk, sk) = gen_func();
        }
        
        // 保存公钥文件
        let pub_file = format!("{sk_file}.pub");
        if let Ok(mut f) = std::fs::File::create(&pub_file) {
            f.write_all(pk.as_bytes()).ok();
            // 保存私钥文件
            if let Ok(mut f) = std::fs::File::create(sk_file) {
                let s = base64::encode(&sk);
                if f.write_all(s.as_bytes()).is_ok() {
                    log::info!("Private/public key written to {}/{}", sk_file, pub_file);
                    log::debug!("Public key: {}", pk);
                    return (pk, Some(sk));
                }
            }
        }
    }
    // 如果所有操作都失败，返回空值
    ("".to_owned(), None)
}

#[cfg(unix)]
/// 监听Unix系统信号（仅Unix系统）
/// 监听SIGTERM、SIGINT、SIGQUIT信号
/// # 返回值
/// 返回异步任务结果
pub async fn listen_signal() -> Result<()> {
    use core_common::tokio;
    use core_common::tokio::signal::unix::{signal, SignalKind};

    // 在异步任务中监听信号
    tokio::spawn(async {
        // 监听终止信号（SIGTERM）
        let mut s = signal(SignalKind::terminate())?;
        let terminate = s.recv();
        
        // 监听中断信号（SIGINT，Ctrl+C）
        let mut s = signal(SignalKind::interrupt())?;
        let interrupt = s.recv();
        
        // 监听退出信号（SIGQUIT）
        let mut s = signal(SignalKind::quit())?;
        let quit = s.recv();

        // 使用tokio::select!等待任意一个信号
        tokio::select! {
            _ = terminate => {
                log::info!("signal terminate");
            }
            _ = interrupt => {
                log::info!("signal interrupt");
            }
            _ = quit => {
                log::info!("signal quit");
            }
        }
        Ok(())
    })
    .await?
}

#[cfg(not(unix))]
/// 监听系统信号（非Unix系统）
/// 在非Unix系统上，此函数为空实现
/// # 返回值
/// 永远不会完成，因为非Unix系统不支持信号监听
pub async fn listen_signal() -> Result<()> {
    let () = std::future::pending().await;
    unreachable!();
}


/// 启动软件更新检查任务
/// 在后台线程中每天检查一次软件更新
pub fn check_software_update() {
    const ONE_DAY_IN_SECONDS: u64 = 60 * 60 * 24;  // 一天的秒数
    
    // 在后台线程中启动定时检查循环
    std::thread::spawn(move || loop {
        // 在新线程中执行更新检查
        std::thread::spawn(move || allow_err!(check_software_update_()));
        // 等待一天
        std::thread::sleep(std::time::Duration::from_secs(ONE_DAY_IN_SECONDS));
    });
}

#[tokio::main(flavor = "current_thread")]
/// 执行软件更新检查的具体实现
/// 检查是否有新版本的RustDesk Server可用
/// # 返回值
/// 返回检查结果，如果出错则返回错误信息
async fn check_software_update_() -> core_common::ResultType<()> {
    // 构建版本检查请求
    let (request, url) = core_common::version_check_request(core_common::VER_TYPE_RUSTDESK_SERVER.to_string());
    
    // 发送HTTP POST请求到更新服务器
    let latest_release_response = reqwest::Client::builder().build()?
        .post(url)
        .json(&request)
        .send()
        .await?;

    // 读取响应数据
    let bytes = latest_release_response.bytes().await?;
    let resp: core_common::VersionCheckResponse = serde_json::from_slice(&bytes)?;
    
    // 从响应URL中提取版本号
    let response_url = resp.url;
    let latest_release_version = response_url.rsplit('/').next().unwrap_or_default();
    
    // 比较版本号
    if get_version_number(&latest_release_version) > get_version_number(crate::version::VERSION) {
       log::info!("new version is available: {}", latest_release_version);
    }
    
    Ok(())
}