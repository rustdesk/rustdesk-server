use clap::App;
mod relay_server;
use hbb_common::{env_logger::*, ResultType};
use relay_server::*;
use std::sync::{Arc, Mutex};

fn main() -> ResultType<()> {
    init_from_env(Env::default().filter_or(DEFAULT_FILTER_ENV, "info"));
    let args = format!(
        "-p, --port=[NUMBER(default={})] 'Sets the listening port'",
        DEFAULT_PORT
    );
    let matches = App::new("hbbr")
        .version(hbbs::VERSION)
        .author("CarrieZ Studio<info@rustdesk.com>")
        .about("RustDesk Relay Server")
        .args_from_usage(&args)
        .get_matches();
    let stop: Arc<Mutex<bool>> = Default::default();
    start(matches.value_of("port").unwrap_or(DEFAULT_PORT), "", stop)?;
    Ok(())
}
