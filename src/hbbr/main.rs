use clap::App;
use hbb_common::{
    env_logger::*,
    log,
    protobuf::Message as _,
    rendezvous_proto::*,
    sleep,
    tcp::{new_listener, FramedStream},
    tokio, ResultType,
};
use std::{
    collections::HashMap,
    net::SocketAddr,
    sync::{Arc, Mutex},
};

lazy_static::lazy_static! {
    static ref PEERS: Arc<Mutex<HashMap<String, FramedStream>>> = Arc::new(Mutex::new(HashMap::new()));
}

const DEFAULT_PORT: &'static str = "21117";

#[tokio::main]
async fn main() -> ResultType<()> {
    init_from_env(Env::default().filter_or(DEFAULT_FILTER_ENV, "info"));
    let args = format!(
        "-p, --port=[NUMBER(default={})] 'Sets the listening port'",
        DEFAULT_PORT
    );
    let matches = App::new("hbbr")
        .version("1.0")
        .author("Zhou Huabing <info@rustdesk.com>")
        .about("RustDesk Relay Server")
        .args_from_usage(&args)
        .get_matches();
    let addr = format!(
        "0.0.0.0:{}",
        matches.value_of("port").unwrap_or(DEFAULT_PORT)
    );
    log::info!("Listening on {}", addr);
    let mut listener = new_listener(addr, false).await?;
    loop {
        tokio::select! {
            Ok((stream, addr)) = listener.accept() => {
                tokio::spawn(async move {
                    make_pair(FramedStream::from(stream), addr).await.ok();
                });
            }
        }
    }
}

async fn make_pair(stream: FramedStream, addr: SocketAddr) -> ResultType<()> {
    let mut stream = stream;
    if let Some(Ok(bytes)) = stream.next_timeout(30_000).await {
        if let Ok(msg_in) = RendezvousMessage::parse_from_bytes(&bytes) {
            if let Some(rendezvous_message::Union::request_relay(rf)) = msg_in.union {
                if !rf.uuid.is_empty() {
                    let peer = PEERS.lock().unwrap().remove(&rf.uuid);
                    if let Some(peer) = peer {
                        log::info!("Forward request {} from {} got paired", rf.uuid, addr);
                        return relay(stream, peer).await;
                    } else {
                        log::info!("New relay request {} from {}", rf.uuid, addr);
                        PEERS.lock().unwrap().insert(rf.uuid.clone(), stream);
                        sleep(30.).await;
                        PEERS.lock().unwrap().remove(&rf.uuid);
                    }
                }
            }
        }
    }
    Ok(())
}

async fn relay(stream: FramedStream, peer: FramedStream) -> ResultType<()> {
    let mut peer = peer;
    let mut stream = stream;
    peer.set_raw();
    stream.set_raw();
    loop {
        tokio::select! {
            res = peer.next() => {
                if let Some(Ok(bytes)) = res {
                    stream.send_bytes(bytes.into()).await?;
                } else {
                    break;
                }
            },
            res = stream.next() => {
                if let Some(Ok(bytes)) = res {
                    peer.send_bytes(bytes.into()).await?;
                } else {
                    break;
                }
            },
        }
    }
    Ok(())
}
