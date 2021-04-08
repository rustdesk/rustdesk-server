use clap::App;
mod relay_server;
use hbb_common::{env_logger::*, ResultType};
use relay_server::*;
use std::sync::{Arc, Mutex};
mod lic;

fn main() -> ResultType<()> {
    init_from_env(Env::default().filter_or(DEFAULT_FILTER_ENV, "info"));
    let args = format!(
        "-p, --port=[NUMBER(default={})] 'Sets the listening port'
        -k, --key=[KEY] 'Only allow the client with the same key'
        {}
        ",
        DEFAULT_PORT,
        lic::EMAIL_ARG
    );
    let matches = App::new("hbbr")
        .version(hbbs::VERSION)
        .author("CarrieZ Studio<info@rustdesk.com>")
        .about("RustDesk Relay Server")
        .args_from_usage(&args)
        .get_matches();
    if !lic::check_lic(matches.value_of("email").unwrap_or("")) {
        return Ok(());
    }
    let stop: Arc<Mutex<bool>> = Default::default();
    start(
        matches.value_of("port").unwrap_or(DEFAULT_PORT),
        matches.value_of("key").unwrap_or(""),
        stop,
    )?;
    Ok(())
}
