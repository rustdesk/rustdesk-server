mod src;
pub use src::*;

use hbb_common::{allow_err, log};
use std::sync::{Arc, Mutex};

lazy_static::lazy_static! {
    static ref STOP: Arc<Mutex<bool>> = Arc::new(Mutex::new(true));
}

fn is_running() -> bool {
    !*STOP.lock().unwrap()
}

pub fn start(license: &str, host: &str) {
    if is_running() {
        return;
    }
    *STOP.lock().unwrap() = false;
    let port = rendezvous_server::DEFAULT_PORT;
    let addr = format!("0.0.0.0:{}", port);
    let addr2 = format!("0.0.0.0:{}", port.parse::<i32>().unwrap_or(0) - 1);
    let relay_servers: Vec<String> = vec![format!("{}:{}", host, relay_server::DEFAULT_PORT)];
    let tmp_license = license.to_owned();
    std::thread::spawn(move || {
        allow_err!(rendezvous_server::RendezvousServer::start(
            &addr,
            &addr2,
            relay_servers,
            0,
            Default::default(),
            Default::default(),
            &tmp_license,
            STOP.clone(),
        ));
    });
    let tmp_license = license.to_owned();
    std::thread::spawn(move || {
        allow_err!(relay_server::start(
            relay_server::DEFAULT_PORT,
            &tmp_license,
            STOP.clone()
        ));
    });
}

pub fn stop() {
    *STOP.lock().unwrap() = true;
}
