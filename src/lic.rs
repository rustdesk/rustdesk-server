use hbb_common::{
    bail, log,
    sodiumoxide::crypto::{
        secretbox::{self, Key, Nonce},
        sign,
    },
    ResultType,
};
use rand::Rng;
use serde_derive::{Deserialize, Serialize};
use std::io::prelude::*;
use std::path::Path;

#[derive(Debug, PartialEq, Default, Serialize, Deserialize, Clone)]
pub struct License {
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
    lic: License,
    #[serde(default)]
    email: String,
    #[serde(default)]
    status: String,
    #[serde(default)]
    nonce: usize,
}

const LICENSE_FILE: &'static str = ".license.txt";

pub fn check_lic(email: &str) -> bool {
    let lic = get_lic();
    let path = Path::new(LICENSE_FILE);
    if Path::is_file(&path) {
        let contents = std::fs::read_to_string(&path).unwrap_or("".to_owned());
        if let Ok(old_lic) = dec_lic(&contents) {
            if lic == old_lic {
                return true;
            }
        }
    }

    if email.is_empty() {
        log::error!("Registered email required.");
        return false;
    }

    match check_email(lic.clone(), email.to_owned()) {
        Ok(v) => {
            if v {
                write_lic(&lic);
            }
            return v;
        }
        Err(err) => {
            log::error!("{}", err);
            return false;
        }
    }
}

fn write_lic(lic: &License) {
    if let Ok(s) = enc_lic(&lic) {
        if let Ok(mut f) = std::fs::File::create(LICENSE_FILE) {
            f.write_all(s.as_bytes()).ok();
            f.sync_all().ok();
        }
    }
}

fn check_email(lic: License, email: String) -> ResultType<bool> {
    log::info!("Checking email with the server ...");
    let mut rng = rand::thread_rng();
    let nonce: usize = rng.gen();
    use reqwest::blocking::Client;
    let resp = Client::new()
        .post("http://rustdesk.com/api/check-email")
        .json(&Post {
            lic,
            email,
            nonce,
            ..Default::default()
        })
        .send()?;
    if resp.status().is_success() {
        let text = base64::decode(resp.text()?)?;
        let p = dec_data(&text, nonce)?;
        if !p.status.is_empty() {
            bail!("{}", p.status);
        }
    } else {
        bail!("Server error: {}", resp.status());
    }
    Ok(true)
}

fn get_lic() -> License {
    let hostname = whoami::hostname();
    let uid = machine_uid::get().unwrap_or("".to_owned());
    let mac = if let Ok(Some(ma)) = mac_address::get_mac_address() {
        base64::encode_config(ma.bytes(), base64::URL_SAFE_NO_PAD)
    } else {
        "".to_owned()
    };
    License { hostname, uid, mac }
}

fn enc_lic(lic: &License) -> ResultType<String> {
    let tmp = serde_json::to_vec::<License>(lic)?;
    const SK: &[u64] = &[
        139, 164, 88, 86, 6, 123, 221, 248, 96, 36, 106, 207, 99, 124, 27, 196, 5, 159, 58, 253,
        238, 94, 3, 184, 237, 236, 122, 59, 205, 95, 6, 189, 88, 168, 68, 104, 60, 5, 163, 198,
        165, 38, 12, 85, 114, 203, 96, 163, 70, 48, 0, 131, 57, 12, 46, 129, 83, 17, 84, 193, 119,
        197, 130, 103,
    ];
    let sk: Vec<u8> = SK.iter().map(|x| *x as u8).collect();
    let mut sk_ = [0u8; sign::SECRETKEYBYTES];
    sk_[..].copy_from_slice(&sk);
    let sk = sign::SecretKey(sk_);
    let tmp = base64::encode_config(sign::sign(&tmp, &sk), base64::URL_SAFE_NO_PAD);
    let tmp: String = tmp.chars().rev().collect();
    Ok(tmp)
}

fn dec_lic(s: &str) -> ResultType<License> {
    let tmp: String = s.chars().rev().collect();
    const PK: &[u64] = &[
        88, 168, 68, 104, 60, 5, 163, 198, 165, 38, 12, 85, 114, 203, 96, 163, 70, 48, 0, 131, 57,
        12, 46, 129, 83, 17, 84, 193, 119, 197, 130, 103,
    ];
    let pk: Vec<u8> = PK.iter().map(|x| *x as u8).collect();
    let mut pk_ = [0u8; sign::PUBLICKEYBYTES];
    pk_[..].copy_from_slice(&pk);
    let pk = sign::PublicKey(pk_);
    if let Ok(data) = sign::verify(&base64::decode_config(tmp, base64::URL_SAFE_NO_PAD)?, &pk) {
        Ok(serde_json::from_slice::<License>(&data)?)
    } else {
        bail!("sign:verify failed");
    }
}

fn dec_data(data: &[u8], n: usize) -> ResultType<Post> {
    let key = b"\xa94\xb4\xb4\xda\xf82\x96\x8b\xb0\x9d\x04d\"\x94T\xa6\xdb\xf6\xd5i=Y.\xf5\xf5i\xa9\x14\x91\xa7\xa9";
    let mut nonce = Nonce([0u8; secretbox::NONCEBYTES]);
    nonce.0[..std::mem::size_of_val(&n)].copy_from_slice(&n.to_le_bytes());
    let key = secretbox::Key(*key);
    if let Ok(res) = secretbox::open(&data, &nonce, &key) {
        return Ok(serde_json::from_slice::<Post>(&res)?);
    }
    bail!("Encryption error");
}

pub const EMAIL_ARG: &'static str =
    "-m, --email=[EMAIL] 'Sets your email address registered with RustDesk'";
