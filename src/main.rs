// https://tools.ietf.org/rfc/rfc5128.txt
// https://blog.csdn.net/bytxl/article/details/44344855

use clap::App;
use hbb_common::{env_logger::*, log, tokio, ResultType};
use hbbs::*;
use ini::Ini;
const DEFAULT_PORT: &'static str = "21116";

#[tokio::main]
async fn main() -> ResultType<()> {
    init_from_env(Env::default().filter_or(DEFAULT_FILTER_ENV, "info"));
    let args = format!(
        "-c --config=[FILE] +takes_value 'Sets a custom config file'
        -p, --port=[NUMBER(default={})] 'Sets the listening port'
        -s, --serial=[NUMBER(default=0)] 'Sets configure update serial number'
        -R, --rendezvous-servers=[HOSTS] 'Sets rendezvous servers, seperated by colon'
        -u, --software-url=[URL] 'Sets download url of RustDesk software of newest version'
    -r, --relay-server=[HOST] 'Sets the default relay server'",
        DEFAULT_PORT
    );
    let matches = App::new("hbbs")
        .version(crate::VERSION)
        .author("CarrieZ Studio<info@rustdesk.com>")
        .about("RustDesk ID/Rendezvous Server")
        .args_from_usage(&args)
        .get_matches();
    let mut section = None;
    let conf; // for holding section
    if let Some(config) = matches.value_of("config") {
        if let Ok(v) = Ini::load_from_file(config) {
            conf = v;
            section = conf.section(None::<String>);
        }
    }
    let get_arg = |name: &str, default: &str| -> String {
        if let Some(v) = matches.value_of(name) {
            return v.to_owned();
        } else if let Some(section) = section {
            if let Some(v) = section.get(name) {
                return v.to_owned();
            }
        }
        return default.to_owned();
    };
    let port = get_arg("port", DEFAULT_PORT);
    let mut relay_server = get_arg("relay-server", "");
    if let Err(err) = test_if_valid_server(&relay_server) {
        relay_server = "".to_owned();
        log::error!("Invalid relay-server: {}", err);
    }
    let serial: i32 = get_arg("serial", "").parse().unwrap_or(0);
    let rendezvous_servers: Vec<String> = get_arg("rendezvous-servers", "")
        .split(",")
        .filter(|x| test_if_valid_server(x).is_ok())
        .map(|x| x.to_owned())
        .collect();
    let addr = format!("0.0.0.0:{}", port);
    log::info!("Listening on {}", addr);
    let addr2 = format!("0.0.0.0:{}", port.parse::<i32>().unwrap_or(0) - 1);
    log::info!("Listening on {}, extra port for NAT test", addr2);
    log::info!("relay-server={}", relay_server);
    log::info!("serial={}", serial);
    log::info!("rendezvous-servers={:?}", rendezvous_servers);
    RendezvousServer::start(
        &addr,
        &addr2,
        relay_server,
        serial,
        rendezvous_servers,
        get_arg("software-url", ""),
    )
    .await?;
    Ok(())
}
