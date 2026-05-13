// https://tools.ietf.org/rfc/rfc5128.txt
// https://blog.csdn.net/bytxl/article/details/44344855

use flexi_logger::*;
use core_common::{bail, config::RENDEZVOUS_PORT, ResultType, log};
use hbbs::common::{init_args, get_arg, get_arg_or};
use hbbs::{web::create_web_router, RendezvousServer};

const RMEM: usize = 0;
/// 默认 Web/API 端口；可用环境变量 `API_PORT` 覆盖（例如与其它服务冲突时）。
const DEFAULT_API_PORT: u16 = 8080;

#[tokio::main]
async fn main() -> ResultType<()> {
    let _logger = Logger::try_with_env_or_str("info")?
        .log_to_stdout()
        .format(opt_format)
        // Async 模式下若进程很快出错，控制台可能看不到日志；阻塞写更易在终端看到启动信息
        .write_mode(WriteMode::BufferAndFlush)
        .start()?;
    eprintln!("[hbbs] 已初始化日志（RUST_LOG={}）", std::env::var("RUST_LOG").unwrap_or_else(|_| "info".into()));
    let args = format!(
        "-c --config=[FILE] +takes_value 'Sets a custom config file'
        -p, --port=[NUMBER(default={RENDEZVOUS_PORT})] 'Sets the listening port'
        -s, --serial=[NUMBER(default=0)] 'Sets configure update serial number'
        -R, --rendezvous-servers=[HOSTS] 'Sets rendezvous servers, separated by comma'
        -u, --software-url=[URL] 'Sets download url of RustDesk software of newest version'
        -r, --relay-servers=[HOST] 'Sets the default relay servers, separated by comma'
        -M, --rmem=[NUMBER(default={RMEM})] 'Sets UDP recv buffer size, set system rmem_max first, e.g., sudo sysctl -w net.core.rmem_max=52428800. vi /etc/sysctl.conf, net.core.rmem_max=52428800, sudo sysctl –p'
        , --mask=[MASK] 'Determine if the connection comes from LAN, e.g. 192.168.0.0/16'
        -k, --key=[KEY] 'Only allow the client with the same key'",
    );
    init_args(&args, "hbbs", "RustDesk ID/Rendezvous Server");
    let port = get_arg_or("port", RENDEZVOUS_PORT.to_string()).parse::<i32>()?;
    if port < 3 {
        bail!("Invalid port");
    }
    let rmem = get_arg("rmem").parse::<usize>().unwrap_or(RMEM);
    let serial: i32 = get_arg("serial").parse().unwrap_or(0);
    
    // 获取数据库路径
    let db_url = std::env::var("DB_URL").unwrap_or_else(|_| {
        let db = "db_v2.sqlite3".to_owned();
        #[cfg(all(windows, not(debug_assertions)))]
        {
            if let Some(path) = core_common::config::Config::icon_path().parent() {
                db = format!("{}\\{}", path.to_str().unwrap_or("."), db);
            }
        }
        #[cfg(not(windows))]
        {
            db = format!("./{db}");
        }
        db
    });
    
    // 初始化数据库
    let db = hbbs::database::Database::new(&db_url).await?;
    log::info!("数据库初始化完成: {}", db_url);
    
    // 启动 API 服务器
    let jwt_secret = std::env::var("JWT_SECRET").unwrap_or_else(|_| {
        log::warn!("JWT_SECRET 环境变量未设置，使用默认密钥（生产环境请设置）");
        "your-secret-key-change-in-production".to_string()
    });
    
    let web_router = create_web_router(db.clone(), jwt_secret);

    let api_port: u16 = std::env::var("API_PORT")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(DEFAULT_API_PORT);
    let api_addr = std::net::SocketAddr::from(([0, 0, 0, 0], api_port));

    eprintln!(
        "[hbbs] 正在绑定 Web/API 端口 {}（占用时可设置环境变量 API_PORT）…",
        api_port
    );
    let tcp = tokio::net::TcpListener::bind(api_addr).await.map_err(|e| {
        eprintln!(
            "[hbbs] 无法绑定 {} — {}\n\
             Windows 错误 10048 表示端口已被占用：请结束其它 hbbs 或占用该端口的程序，或例如: set API_PORT=18080",
            api_addr, e
        );
        core_common::anyhow::anyhow!("API 端口绑定失败: {}", e)
    })?;
    let std_listener = tcp.into_std().map_err(|e| {
        eprintln!("[hbbs] TcpListener::into_std 失败: {}", e);
        core_common::anyhow::anyhow!("into_std: {}", e)
    })?;
    std_listener
        .set_nonblocking(true)
        .map_err(|e| core_common::anyhow::anyhow!("set_nonblocking: {}", e))?;

    log::info!("API 监听 http://0.0.0.0:{}/", api_port);

    // 必须在 `#[tokio::main]` 的同一运行时上调度任务；预先 bind 可避免 `Server::bind` 在占用时 panic 且不易见日志。
    let api_server = tokio::spawn(async move {
        let server = match axum::Server::from_tcp(std_listener) {
            Ok(s) => s,
            Err(e) => {
                log::error!("API Server::from_tcp: {}", e);
                return;
            }
        };
        if let Err(e) = server.serve(web_router.into_make_service()).await {
            log::error!("API 服务异常结束: {}", e);
        }
    });

    let key = get_arg_or("key", "-".to_owned());
    let rendezvous_server = tokio::task::spawn_blocking(move || {
        match RendezvousServer::start(port, serial, &key, rmem) {
            Ok(()) => log::info!("Rendezvous 服务器正常关闭"),
            Err(e) => log::error!("Rendezvous 服务器错误: {}", e),
        }
    });

    tokio::select! {
        result = api_server => {
            match result {
                Ok(()) => {}
                Err(e) => log::error!("API 服务器任务错误: {}", e),
            }
        }
        result = rendezvous_server => {
            match result {
                Ok(()) => {}
                Err(e) => log::error!("Rendezvous 服务器任务错误: {}", e),
            }
        }
    }
    
    Ok(())
}
