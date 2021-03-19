// https://tools.ietf.org/rfc/rfc5128.txt
// https://blog.csdn.net/bytxl/article/details/44344855

use clap::App;
use hbb_common::{env_logger::*, log, ResultType};
use hbbs::*;
use ini::Ini;
use std::sync::{Arc, Mutex};

fn main() -> ResultType<()> {
    init_from_env(Env::default().filter_or(DEFAULT_FILTER_ENV, "info"));
    let args = format!(
        "-c --config=[FILE] +takes_value 'Sets a custom config file'
        -p, --port=[NUMBER(default={})] 'Sets the listening port'
        -s, --serial=[NUMBER(default=0)] 'Sets configure update serial number'
        -R, --rendezvous-servers=[HOSTS] 'Sets rendezvous servers, seperated by colon'
        -u, --software-url=[URL] 'Sets download url of RustDesk software of newest version'
        -r, --relay-server{}=[HOST] 'Sets the default relay server{}'",
        DEFAULT_PORT,
        if LICENSE_KEY.is_empty() { "" } else { "s" },
        if LICENSE_KEY.is_empty() {
            ""
        } else {
            "s, seperated by colon, only available for licensed users"
        }
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
    let mut relay_servers: Vec<String> = get_arg(
        &format!(
            "relay-server{}",
            if LICENSE_KEY.is_empty() { "" } else { "s" }
        ),
        "",
    )
    .split(",")
    .filter(|x| !x.is_empty() && test_if_valid_server(x, "relay-server").is_ok())
    .map(|x| x.to_owned())
    .collect();
    if relay_servers.len() > 1 && LICENSE_KEY.is_empty() {
        log::error!("Only support multiple relay servers for licenced users");
        relay_servers = vec![relay_servers[0].clone()];
    }
    let serial: i32 = get_arg("serial", "").parse().unwrap_or(0);
    let rendezvous_servers: Vec<String> = get_arg("rendezvous-servers", "")
        .split(",")
        .filter(|x| !x.is_empty() && test_if_valid_server(x, "rendezvous-server").is_ok())
        .map(|x| x.to_owned())
        .collect();
    let addr = format!("0.0.0.0:{}", port);
    let addr2 = format!("0.0.0.0:{}", port.parse::<i32>().unwrap_or(0) - 1);
    log::info!("relay-servers={:?}", relay_servers);
    log::info!("serial={}", serial);
    log::info!("rendezvous-servers={:?}", rendezvous_servers);
    let stop: Arc<Mutex<bool>> = Default::default();
    RendezvousServer::start(
        &addr,
        &addr2,
        relay_servers,
        serial,
        rendezvous_servers,
        get_arg("software-url", ""),
        stop,
    )?;
    Ok(())
}
