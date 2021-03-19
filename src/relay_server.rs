use hbb_common::{
    log,
    protobuf::Message as _,
    rendezvous_proto::*,
    sleep,
    tcp::{new_listener, FramedStream},
    tokio::{
        self,
        time::{interval, Duration},
    },
    ResultType,
};
use std::{
    collections::HashMap,
    net::SocketAddr,
    sync::{Arc, Mutex},
};

lazy_static::lazy_static! {
    static ref PEERS: Arc<Mutex<HashMap<String, FramedStream>>> = Arc::new(Mutex::new(HashMap::new()));
}

pub const DEFAULT_PORT: &'static str = "21117";

#[tokio::main(basic_scheduler)]
pub async fn start(port: &str, license: &str, stop: Arc<Mutex<bool>>) -> ResultType<()> {
    let addr = format!("0.0.0.0:{}", port);
    log::info!("Listening on {}", addr);
    let mut timer = interval(Duration::from_millis(300));
    let mut listener = new_listener(addr, false).await?;
    loop {
        tokio::select! {
            Ok((stream, addr)) = listener.accept() => {
                let license = license.to_owned();
                tokio::spawn(async move {
                    make_pair(FramedStream::from(stream), addr, &license).await.ok();
                });
            }
            _ = timer.tick() => {
                if *stop.lock().unwrap() {
                    log::info!("Stopped");
                    break;
                }
            }
        }
    }
    Ok(())
}

async fn make_pair(stream: FramedStream, addr: SocketAddr, license: &str) -> ResultType<()> {
    let mut stream = stream;
    if let Some(Ok(bytes)) = stream.next_timeout(30_000).await {
        if let Ok(msg_in) = RendezvousMessage::parse_from_bytes(&bytes) {
            if let Some(rendezvous_message::Union::request_relay(rf)) = msg_in.union {
                if !license.is_empty() && rf.licence_key != license {
                    return Ok(());
                }
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
