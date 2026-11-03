//! End-to-end check of the rendezvous key exchange against a real hbbs.
//!
//! The client half is a transcription of what RustDesk does in
//! `secure_tcp_impl`/`create_symmetric_key_msg` (rustdesk/src/common.rs) when
//! it is signed in to an API account, so this exercises the wire protocol the
//! actual client speaks rather than the server's own idea of it.

use hbb_common::{
    bytes::Bytes,
    protobuf::Message as _,
    rendezvous_proto::*,
    tcp::FramedStream,
    tokio,
};
use sodiumoxide::crypto::{box_, secretbox, sign};
use std::net::{IpAddr, Ipv4Addr, SocketAddr};

// hbbs binds three ports around this one: PORT - 1 (NAT type test), PORT (the
// rendezvous listener this test speaks to) and PORT + 2 (the websocket
// listener). 41115, 41116 and 41118 are the defaults shifted by 20000, kept
// clear of anything a developer machine or a CI runner is likely to use; if the
// test cannot bind, that whole range is what has to be free.
const PORT: i32 = 41116;

async fn secure_tcp(conn: &mut FramedStream, server_pk: &sign::PublicKey) {
    let bytes = hbb_common::timeout(5_000, conn.next())
        .await
        .expect("server must offer a key exchange before the read timeout")
        .expect("stream stays open")
        .expect("frame decodes");
    let msg_in = RendezvousMessage::parse_from_bytes(&bytes).expect("offer parses");
    let Some(rendezvous_message::Union::KeyExchange(ex)) = msg_in.union else {
        panic!("first frame is not a key exchange: {msg_in:?}");
    };
    assert_eq!(ex.keys.len(), 1);
    let their_pk_b = sign::verify(&ex.keys[0], server_pk).expect("offer is signed by the server");
    let mut pk_ = [0u8; box_::PUBLICKEYBYTES];
    pk_.copy_from_slice(&their_pk_b);
    let their_pk_b = box_::PublicKey(pk_);

    let (our_pk_b, our_sk_b) = box_::gen_keypair();
    let key = secretbox::gen_key();
    let nonce = box_::Nonce([0u8; box_::NONCEBYTES]);
    let sealed_key = box_::seal(&key.0, &nonce, &their_pk_b, &our_sk_b);
    let mut msg_out = RendezvousMessage::new();
    msg_out.set_key_exchange(KeyExchange {
        keys: vec![Bytes::from(our_pk_b.0.to_vec()), Bytes::from(sealed_key)],
        ..Default::default()
    });
    conn.send(&msg_out).await.expect("answer is sent in clear");
    conn.set_key(key);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn signed_in_client_completes_the_exchange_and_keeps_talking_encrypted() {
    let dir = std::env::temp_dir().join(format!("hbbs-key-exchange-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("scratch dir");
    std::env::set_var("DB_URL", dir.join("db.sqlite3"));
    std::env::set_var("TEST_HBBS", "no");

    let (pk, sk) = sign::gen_keypair();
    let key = base64::encode(sk.0);
    // `start_with_bind` is `#[tokio::main]`, i.e. it builds its own runtime and
    // blocks, so it gets a thread of its own. Its error travels back over a
    // channel: panicking on that thread would leave this one retrying the
    // connection for five seconds and then failing with the wrong reason.
    let (failed_tx, failed_rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        if let Err(err) = hbbs::RendezvousServer::start_with_bind(
            Some(IpAddr::V4(Ipv4Addr::LOCALHOST)),
            PORT,
            0,
            &key,
            0,
        ) {
            let _ = failed_tx.send(err.to_string());
        }
    });

    let addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), PORT as u16);
    let mut conn = None;
    for _ in 0..50 {
        if let Ok(err) = failed_rx.try_recv() {
            panic!("hbbs did not start: {err}");
        }
        if let Ok(c) = FramedStream::new(addr, None, 1_000).await {
            conn = Some(c);
            break;
        }
        hbb_common::sleep(0.1).await;
    }
    // Check the channel once more before blaming the connection: if the server
    // never came up, that is the failure worth reporting.
    let mut conn = match conn {
        Some(conn) => conn,
        None => match failed_rx.try_recv() {
            Ok(err) => panic!("hbbs did not start: {err}"),
            Err(_) => panic!("hbbs accepts tcp connections"),
        },
    };

    secure_tcp(&mut conn, &pk).await;

    // Everything from here on is encrypted in both directions.
    let mut msg_out = RendezvousMessage::new();
    msg_out.set_punch_hole_request(PunchHoleRequest {
        id: "123456789".to_owned(),
        licence_key: base64::encode(pk.0),
        ..Default::default()
    });
    conn.send(&msg_out).await.expect("request is sent");

    let bytes = hbb_common::timeout(5_000, conn.next())
        .await
        .expect("server answers the encrypted request")
        .expect("stream stays open")
        .expect("frame decrypts with the negotiated key");
    let msg_in = RendezvousMessage::parse_from_bytes(&bytes).expect("response parses");
    match msg_in.union {
        Some(rendezvous_message::Union::PunchHoleResponse(ph)) => {
            assert_eq!(ph.failure.enum_value_or_default(), punch_hole_response::Failure::ID_NOT_EXIST);
        }
        other => panic!("unexpected response: {other:?}"),
    }

    // A client that is not signed in never answers the offer: it just sends
    // its request in the clear, and must still be served.
    let mut conn = FramedStream::new(addr, None, 1_000)
        .await
        .expect("second connection");
    let _offer = hbb_common::timeout(5_000, conn.next())
        .await
        .expect("offer arrives")
        .expect("stream stays open")
        .expect("frame decodes");
    conn.send(&msg_out).await.expect("plain request is sent");
    let bytes = hbb_common::timeout(5_000, conn.next())
        .await
        .expect("server answers the plain request")
        .expect("stream stays open")
        .expect("frame decodes");
    let msg_in = RendezvousMessage::parse_from_bytes(&bytes).expect("response parses");
    match msg_in.union {
        Some(rendezvous_message::Union::PunchHoleResponse(ph)) => {
            assert_eq!(
                ph.failure.enum_value_or_default(),
                punch_hole_response::Failure::ID_NOT_EXIST
            );
        }
        other => panic!("unexpected response for the plain client: {other:?}"),
    }

    std::fs::remove_dir_all(&dir).ok();
}
