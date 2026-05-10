// https://tools.ietf.org/rfc/rfc5128.txt
// https://blog.csdn.net/bytxl/article/details/44344855

use flexi_logger::*;
use hbb_common::{bail, config::RENDEZVOUS_PORT, ResultType, log};
use hbbs::common::{init_args, get_arg, get_arg_or};
use hbbs::{web::create_web_router, api::create_api_router, RendezvousServer};
use tokio::net::TcpListener;
use tokio::runtime::Runtime;

const RMEM: usize = 0;
const API_PORT: i32 = 8080;

#[tokio::main]
async fn main() -> ResultType<()> {
    let _logger = Logger::try_with_env_or_str("info")?
        .log_to_stdout()
        .format(opt_format)
        .write_mode(WriteMode::Async)
        .start()?;
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
        let mut db = "db_v2.sqlite3".to_owned();
        #[cfg(all(windows, not(debug_assertions)))]
        {
            if let Some(path) = hbb_common::config::Config::icon_path().parent() {
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
    let api_addr = format!("0.0.0.0:{}", API_PORT).parse()?;
    log::info!("API 服务器启动在端口: {}", API_PORT);
    
    // 启动 API 和 Rendezvous 服务器
    let rt = Runtime::new().unwrap();
    let api_server = rt.spawn(async move {
        axum::Server::bind(&api_addr)
            .serve(web_router.into_make_service())
            .await
    });
    let rendezvous_server = rt.spawn_blocking(move || {
        RendezvousServer::start(port, serial, &get_arg_or("key", "-".to_owned()), rmem)
    });
    
    tokio::select! {
        result = api_server => {
            match result {
                Ok(_) => log::info!("API 服务器正常关闭"),
                Err(e) => log::error!("API 服务器错误: {}", e),
            }
        }
        result = rendezvous_server => {
            match result {
                Ok(Ok(_)) => log::info!("Rendezvous 服务器正常关闭"),
                Ok(Err(e)) => log::error!("Rendezvous 服务器错误: {}", e),
                Err(e) => log::error!("Rendezvous 服务器任务错误: {}", e),
            }
        }
    }
    
    Ok(())
}
