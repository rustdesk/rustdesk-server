///
/// hbbr - RustDesk Heartbeat Bridge Server
/// 
/// This is a Rust implementation of the hbbr (heartbeat bridge) server
/// that handles NAT traversal and connection establishment for RustDesk.
/// 
/// The hbbr server is responsible for:
/// - Maintaining NAT mappings through periodic heartbeat messages
/// - Facilitating direct peer-to-peer connections between RustDesk clients
/// - Acting as a bridge when direct connections cannot be established
/// 

use clap::App;
mod common;
mod relay_server;
use flexi_logger::*;
use core_common::{config::RELAY_PORT, ResultType};
use relay_server::*;
mod version;

fn main() -> ResultType<()> {
    let _logger = Logger::try_with_env_or_str("info")?
        .log_to_stdout()
        .format(opt_format)
        .write_mode(WriteMode::Async)
        .start()?;
    let args = format!(
        "-p, --port=[NUMBER(default={RELAY_PORT})] 'Sets the listening port'
        -k, --key=[KEY] 'Only allow the client with the same key'
        ",
    );
    let matches = App::new("hbbr")
        .version(version::VERSION)
        .author("Purslane Ltd. <info@rustdesk.com>")
        .about("RustDesk Relay Server")
        .args_from_usage(&args)
        .get_matches();
    if let Ok(v) = ini::Ini::load_from_file(".env") {
        if let Some(section) = v.section(None::<String>) {
            section.iter().for_each(|(k, v)| std::env::set_var(k, v));
        }
    }
    let mut port = RELAY_PORT;
    if let Ok(v) = std::env::var("PORT") {
        let v: i32 = v.parse().unwrap_or_default();
        if v > 0 {
            port = v + 1;
        }
    }
    start(
        matches.value_of("port").unwrap_or(&port.to_string()),
        matches
            .value_of("key")
            .unwrap_or(&std::env::var("KEY").unwrap_or_default()),
    )?;
    Ok(())
}
