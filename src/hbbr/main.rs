use clap::App;
mod relay_server;
use hbb_common::{env_logger::*, tokio, ResultType};
use relay_server::start;

const DEFAULT_PORT: &'static str = "21117";

#[tokio::main]
async fn main() -> ResultType<()> {
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
    start(matches.value_of("port").unwrap_or(DEFAULT_PORT)).await?;
    Ok(())
}
