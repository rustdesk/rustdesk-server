// https://tools.ietf.org/rfc/rfc5128.txt
// https://blog.csdn.net/bytxl/article/details/44344855

use clap::App;
use hbb_common::{env_logger::*, log, tokio, ResultType};
use hbbs::*;
const DEFAULT_PORT: &'static str = "21116";

#[tokio::main]
async fn main() -> ResultType<()> {
    init_from_env(Env::default().filter_or(DEFAULT_FILTER_ENV, "info"));
    let args = format!(
        "-p, --port=[default={}] 'Sets the listening port'
    -r, --relay-server=[] 'Sets the default relay server'",
        DEFAULT_PORT
    );
    let matches = App::new("hbbs")
        .version("1.0")
        .author("Zhou Huabing <info@rustdesk.com>")
        .about("RustDesk Rendezvous Server")
        .args_from_usage(&args)
        .get_matches();
    let addr = format!(
        "0.0.0.0:{}",
        matches.value_of("port").unwrap_or(DEFAULT_PORT)
    );
    log::info!("Listening on {}", addr);
    RendezvousServer::start(
        &addr,
        matches.value_of("relay-server").unwrap_or("").to_owned(),
    )
    .await?;
    Ok(())
}
