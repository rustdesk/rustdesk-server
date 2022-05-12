// https://tools.ietf.org/rfc/rfc5128.txt
// https://blog.csdn.net/bytxl/article/details/44344855

use flexi_logger::*;
use hbb_common::{bail, config::RENDEZVOUS_PORT, ResultType};
use hbbs::{common::*, *};

const RMEM: usize = 0;

fn main() -> ResultType<()> {
    let _logger = Logger::try_with_env_or_str("info")?
        .log_to_stdout()
        .format(opt_format)
        .write_mode(WriteMode::Async)
        .start()?;
    let args = format!(
        "-c --config=[FILE] +takes_value 'Sets a custom config file'
        -p, --port=[NUMBER(default={})] 'Sets the listening port'
        -s, --serial=[NUMBER(default=0)] 'Sets configure update serial number'
        -R, --rendezvous-servers=[HOSTS] 'Sets rendezvous servers, seperated by colon'
        -u, --software-url=[URL] 'Sets download url of RustDesk software of newest version'
        -r, --relay-servers=[HOST] 'Sets the default relay servers, seperated by colon'
        -C, --change-id=[BOOL(default=Y)] 'Sets if support to change id'
        -M, --rmem=[NUMBER(default={})] 'Sets UDP recv buffer size, set system rmem_max first, e.g., sudo sysctl -w net.core.rmem_max=52428800. vi /etc/sysctl.conf, net.core.rmem_max=52428800, sudo sysctl –p'
        -k, --key=[KEY] 'Only allow the client with the same key'",
        RENDEZVOUS_PORT,
        RMEM,
    );
    init_args(&args, "hbbs", "RustDesk ID/Rendezvous Server");
    let port = get_arg_or("port", RENDEZVOUS_PORT.to_string()).parse::<i32>()?;
    if port < 3 {
        bail!("Invalid port");
    }
    let rmem = get_arg("rmem").parse::<usize>().unwrap_or(RMEM);
    let serial: i32 = get_arg("serial").parse().unwrap_or(0);
    let id_change_support: bool = get_arg_or("change-id", "Y".to_owned()).to_uppercase() == "Y";
    RendezvousServer::start(port, serial, &get_arg("key"), id_change_support, rmem)?;
    Ok(())
}
