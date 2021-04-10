use hbb_common::{bail, log, ResultType};
use serde_derive::{Deserialize, Serialize};
use std::io::prelude::*;
use std::path::Path;

#[derive(Debug, PartialEq, Default, Serialize, Deserialize, Clone)]
pub struct Machine {
    #[serde(default)]
    hostname: String,
    #[serde(default)]
    uid: String,
    #[serde(default)]
    mac: String,
}

#[derive(Debug, PartialEq, Default, Serialize, Deserialize, Clone)]
pub struct Post {
    #[serde(default)]
    machine: String,
    #[serde(default)]
    email: String,
    #[serde(default)]
    status: String,
}

const LICENSE_FILE: &'static str = ".license.txt";

pub fn check_lic(email: &str) -> bool {
    let machine = get_lic();
    let path = Path::new(LICENSE_FILE);
    if Path::is_file(&path) {
        let contents = std::fs::read_to_string(&path).unwrap_or("".to_owned());
        if verify(&contents, &machine) {
            return true;
        }
    }

    if email.is_empty() {
        log::error!("Registered email required.");
        return false;
    }

    match check_email(machine, email.to_owned()) {
        Ok(v) => {
            return v;
        }
        Err(err) => {
            log::error!("{}", err);
            return false;
        }
    }
}

fn write_lic(lic: &str) {
    if let Ok(mut f) = std::fs::File::create(LICENSE_FILE) {
        f.write_all(lic.as_bytes()).ok();
        f.sync_all().ok();
    }
}

fn check_email(machine: String, email: String) -> ResultType<bool> {
    log::info!("Checking email with the server ...");
    use reqwest::blocking::Client;
    let resp = Client::new()
        .post("http://rustdesk.com/api/check-email")
        .json(&Post {
            machine: machine.clone(),
            email,
            ..Default::default()
        })
        .send()?;
    if resp.status().is_success() {
        let p: Post = resp.json()?;
        if !verify(&p.machine, &machine) {
            bail!("Verification failure");
        }
        if !p.status.is_empty() {
            bail!("{}", p.status);
        }
        write_lic(&p.machine);
    } else {
        bail!("Server error: {}", resp.status());
    }
    Ok(true)
}

fn get_lic() -> String {
    let hostname = whoami::hostname();
    let uid = machine_uid::get().unwrap_or("".to_owned());
    let mac = if let Ok(Some(ma)) = mac_address::get_mac_address() {
        base64::encode(ma.bytes())
    } else {
        "".to_owned()
    };
    serde_json::to_string(&Machine { hostname, uid, mac }).unwrap()
}

fn verify(enc_str: &str, msg: &str) -> bool {
    if let Ok(data) = base64::decode(enc_str) {
        let key =
            b"\xf1T\xc0\x1c\xffee\x86,S*\xd9.\x91\xcd\x85\x12:\xec\xa9 \x99:\x8a\xa2S\x1f Yy\x93R";
        cryptoxide::ed25519::verify(msg.as_bytes(), &key[..], &data)
    } else {
        false
    }
}

pub const EMAIL_ARG: &'static str =
    "-m, --email=[EMAIL] 'Sets your email address registered with RustDesk'";
