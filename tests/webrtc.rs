// WebRTC signalling as the 1.5.0 client does it: the offer in PunchHoleRequest
// (client.rs _start_inner), the answer in PunchHoleSent or RelayResponse
// (rendezvous_mediator.rs handle_punch_hole/create_relay), trickle ICE from the
// controller on its punch connection (spawn_webrtc_ice_bridge) and from the
// controlled side on a connection of its own (rendezvous_mediator.rs ~930).
mod common;

use common::*;
use hbb_common::{bytes::Bytes, rendezvous_proto::*, tokio};
use std::time::Duration;

const FP: &str = "sha-256 97:52:D6:1F:1E:87:6C:DA:B8:21:95:64:A5:85:89:FA:02:71:C7:4D:B3:FD:25:92:40:FB:6B:65:24:3C:79:88";
const CANDIDATE: &str = r#"{"candidate":"candidate:1 1 udp 2130706431 192.168.1.5 50000 typ host","sdpMid":"0","sdpMLineIndex":0,"usernameFragment":"abcd"}"#;

fn endpoint(kind: &str, fp: &str, pad: usize) -> String {
    let sdp = format!(
        "v=0\r\no=- 1 2 IN IP4 0.0.0.0\r\ns=-\r\nt=0 0\r\na=fingerprint:{}\r\na=group:BUNDLE 0\r\nm=application 9 UDP/DTLS/SCTP webrtc-datachannel\r\nc=IN IP4 0.0.0.0\r\na=ice-ufrag:abcd\r\na=ice-pwd:{}\r\na=setup:actpass\r\na=mid:0\r\na=sctp-port:5000\r\n",
        fp,
        "p".repeat(24 + pad)
    );
    let json = serde_json::json!({"type": kind, "sdp": sdp, "ice_policy": "all"});
    format!("webrtc://{}", base64::encode(json.to_string()))
}

fn offer() -> String {
    endpoint("offer", FP, 0)
}

fn answer() -> String {
    endpoint("answer", "sha-256 AA:BB", 0)
}

fn ice(session_key: &str, candidate: &str) -> IceCandidate {
    IceCandidate {
        session_key: session_key.to_owned(),
        candidate: candidate.to_owned(),
        ..Default::default()
    }
}

async fn offer_request(hbbs: &Hbbs, ctl: &mut Tcp, id: &str, offer: String) {
    let mut req = punch_hole_request(id, &hbbs.key);
    req.webrtc_sdp_offer = offer;
    ctl.send(&msg(|m| m.set_punch_hole_request(req))).await;
}

/// The controlled side's reply without UDP: PunchHoleSent on a new connection
/// after `secure_tcp_required`.
async fn answer_over_tcp(hbbs: &Hbbs, ph: &PunchHole, answer: String) -> Tcp {
    let mut reply = Tcp::connect(hbbs, PEER_IP, Kx::V1).await;
    assert!(reply.secured);
    reply
        .send(&msg(|m| {
            m.set_punch_hole_sent(PunchHoleSent {
                socket_addr: ph.socket_addr.clone(),
                id: "peer01".to_owned(),
                nat_type: NatType::ASYMMETRIC.into(),
                version: "1.5.0".to_owned(),
                webrtc_sdp_answer: answer,
                ..Default::default()
            })
        }))
        .await;
    reply
}

/// The controlled side's dedicated, encrypted ICE connection.
async fn ice_connection(hbbs: &Hbbs) -> Tcp {
    let conn = Tcp::connect(hbbs, PEER_IP, Kx::V1).await;
    assert!(conn.secured);
    conn
}

async fn send_ice_from_peer(
    conn: &mut Tcp,
    ph_socket_addr: &Bytes,
    session_key: &str,
    candidate: &str,
) {
    let mut c = ice(session_key, candidate);
    c.socket_addr = ph_socket_addr.clone();
    conn.send(&msg(|m| m.set_ice_candidate(c))).await;
}

async fn send_ice_from_controller(ctl: &mut Tcp, id: &str, session_key: &str, candidate: &str) {
    let mut c = ice(session_key, candidate);
    c.id = id.to_owned();
    ctl.send(&msg(|m| m.set_ice_candidate(c))).await;
}

struct Session {
    hbbs: Hbbs,
    peer: Peer,
    ctl: Tcp,
    ph: PunchHole,
}

async fn start_session(envs: &[(&str, &str)]) -> Session {
    let hbbs = Hbbs::start(envs, &[]).await;
    let peer = Peer::register(&hbbs, "peer01").await;
    let mut ctl = Tcp::connect(&hbbs, CONTROLLER_IP, Kx::V1).await;
    assert!(ctl.secured);
    offer_request(&hbbs, &mut ctl, "peer01", offer()).await;
    let ph = peer
        .expect(3000, pick_punch_hole)
        .await
        .expect("no PunchHole");
    Session {
        hbbs,
        peer,
        ctl,
        ph,
    }
}

#[tokio::test]
async fn offer_answer_and_trickle_ice() {
    let Session {
        hbbs,
        peer,
        mut ctl,
        ph,
    } = start_session(&[]).await;
    assert_eq!(ph.webrtc_sdp_offer, offer());
    assert!(!ph.force_relay);
    assert_eq!(ph.udp_port, 0);

    // the controlled side's candidates may arrive before its answer
    let mut ice_conn = ice_connection(&hbbs).await;
    send_ice_from_peer(&mut ice_conn, &ph.socket_addr, FP, CANDIDATE).await;
    let _reply = answer_over_tcp(&hbbs, &ph, answer()).await;
    send_ice_from_peer(&mut ice_conn, &ph.socket_addr, FP, "c2").await;

    let mut got_answer = None;
    let mut candidates = vec![];
    while got_answer.is_none() || candidates.len() < 2 {
        let m = ctl.recv(3000).await.expect("controller got too little");
        match m.union {
            Some(rendezvous_message::Union::PunchHoleResponse(r)) => got_answer = Some(r),
            Some(rendezvous_message::Union::IceCandidate(c)) => candidates.push(c),
            other => panic!("unexpected {:?}", other),
        }
    }
    let res = got_answer.unwrap();
    assert_eq!(res.webrtc_sdp_answer, answer());
    assert!(!res.socket_addr.is_empty());
    assert!(!res.pk.is_empty());
    assert_eq!(candidates[0].session_key, FP);
    assert_eq!(candidates[0].candidate, CANDIDATE);
    assert!(candidates[0].id.is_empty() && candidates[0].socket_addr.is_empty());
    assert_eq!(candidates[1].candidate, "c2");

    // the controller's candidates, each sent twice, reach the peer over UDP
    for _ in 0..2 {
        send_ice_from_controller(&mut ctl, "peer01", FP, CANDIDATE).await;
    }
    for _ in 0..2 {
        let c = peer
            .expect(3000, pick_ice)
            .await
            .expect("peer got no candidate");
        assert_eq!(c.session_key, FP);
        assert_eq!(c.candidate, CANDIDATE);
    }
    // both trickle connections stay open
    assert!(!ctl.closed_within(500).await);
    assert!(!ice_conn.closed_within(10).await);

    // nothing sensitive in the debug log
    let log = hbbs.log();
    assert!(log.contains("WebRTC offer"));
    for secret in [FP, "a=ice-pwd", "typ host", "webrtc://"] {
        assert!(!log.contains(secret), "log contains {}", secret);
    }
}

#[tokio::test]
async fn answer_in_relay_response() {
    let Session {
        hbbs, mut ctl, ph, ..
    } = start_session(&[]).await;
    // create_relay(initiate = true) with the answer, after secure_tcp_required
    let mut reply = Tcp::connect(&hbbs, PEER_IP, Kx::V1).await;
    let mut rr = RelayResponse {
        socket_addr: ph.socket_addr.clone(),
        uuid: "uuid-relay-1".to_owned(),
        relay_server: "relay.example:21117".to_owned(),
        version: "1.5.0".to_owned(),
        webrtc_sdp_answer: answer(),
        ..Default::default()
    };
    rr.set_id("peer01".to_owned());
    reply.send(&msg(|m| m.set_relay_response(rr))).await;
    let rr = ctl.expect(3000, pick_relay_response).await.unwrap();
    assert_eq!(rr.webrtc_sdp_answer, answer());
    assert_eq!(rr.uuid, "uuid-relay-1");
    // the controller keeps its connection for ICE
    let mut ice_conn = ice_connection(&hbbs).await;
    send_ice_from_peer(&mut ice_conn, &ph.socket_addr, FP, CANDIDATE).await;
    assert_eq!(
        ctl.expect(3000, pick_ice).await.unwrap().candidate,
        CANDIDATE
    );
}

// Anybody who knows the controller's address can send a reply; the answer only
// counts from the host the peer registered from.
#[tokio::test]
async fn answer_only_from_the_peer_host() {
    let stranger_ip = "127.0.0.3".parse().unwrap();
    let Session {
        hbbs,
        peer,
        mut ctl,
        ph,
    } = start_session(&[]).await;
    let mut stranger = Tcp::connect(&hbbs, stranger_ip, Kx::V1).await;
    stranger
        .send(&msg(|m| {
            m.set_punch_hole_sent(PunchHoleSent {
                socket_addr: ph.socket_addr.clone(),
                id: "peer01".to_owned(),
                version: "1.5.0".to_owned(),
                webrtc_sdp_answer: answer(),
                ..Default::default()
            })
        }))
        .await;
    let res = ctl.expect(3000, pick_punch_hole_response).await.unwrap();
    assert!(res.webrtc_sdp_answer.is_empty());

    // the same in a RelayResponse
    let mut ctl = Tcp::connect(&hbbs, CONTROLLER_IP, Kx::V1).await;
    offer_request(&hbbs, &mut ctl, "peer01", offer()).await;
    let ph = peer.expect(3000, pick_punch_hole).await.unwrap();
    let mut stranger = Tcp::connect(&hbbs, stranger_ip, Kx::V1).await;
    let mut rr = RelayResponse {
        socket_addr: ph.socket_addr,
        uuid: "uuid-1".to_owned(),
        relay_server: "relay.example:21117".to_owned(),
        version: "1.5.0".to_owned(),
        webrtc_sdp_answer: answer(),
        ..Default::default()
    };
    rr.set_id("peer01".to_owned());
    stranger.send(&msg(|m| m.set_relay_response(rr))).await;
    let rr = ctl.expect(3000, pick_relay_response).await.unwrap();
    assert!(rr.webrtc_sdp_answer.is_empty());
    assert_eq!(rr.uuid, "uuid-1");
}

#[tokio::test]
async fn same_intranet_offer_gets_a_punch_hole() {
    let hbbs = Hbbs::start(&[], &[]).await;
    let peer = Peer::register(&hbbs, "peer01").await;
    // the preferred round carries the offer: PunchHole, so WebRTC can use LAN candidates
    let mut ctl = Tcp::connect(&hbbs, PEER_IP, Kx::V1).await;
    offer_request(&hbbs, &mut ctl, "peer01", offer()).await;
    let ph = peer
        .expect(3000, pick_punch_hole)
        .await
        .expect("no PunchHole");
    assert_eq!(ph.webrtc_sdp_offer, offer());
    // the offer-less fallback round still gets FetchLocalAddr
    let mut fallback = Tcp::connect(&hbbs, PEER_IP, Kx::None).await;
    fallback
        .send(&msg(|m| {
            m.set_punch_hole_request(punch_hole_request("peer01", &hbbs.key))
        }))
        .await;
    assert!(peer.expect(3000, pick_fetch_local_addr).await.is_some());
}

#[tokio::test]
async fn foreign_candidates_are_dropped() {
    let Session {
        hbbs,
        peer,
        mut ctl,
        ph,
    } = start_session(&[]).await;
    let mut ice_conn = ice_connection(&hbbs).await;
    // wrong session key, unknown controller address, garbage address
    send_ice_from_peer(&mut ice_conn, &ph.socket_addr, "sha-256 00:11", CANDIDATE).await;
    let stranger = mangle("127.0.0.9:4000".parse().unwrap());
    send_ice_from_peer(&mut ice_conn, &stranger, FP, CANDIDATE).await;
    send_ice_from_peer(&mut ice_conn, &Bytes::from(vec![9u8; 17]), FP, CANDIDATE).await;
    assert!(ctl.expect(700, pick_ice).await.is_none());
    // the controller cannot address another peer or session
    let _other = Peer::register(&hbbs, "peer02").await;
    send_ice_from_controller(&mut ctl, "peer02", FP, CANDIDATE).await;
    send_ice_from_controller(&mut ctl, "peer01", "sha-256 00:11", CANDIDATE).await;
    assert!(peer.expect(700, pick_ice).await.is_none());
    // a connection without a WebRTC route cannot send candidates to anyone
    let mut plain = Tcp::connect(&hbbs, CONTROLLER_IP, Kx::None).await;
    send_ice_from_controller(&mut plain, "peer01", FP, CANDIDATE).await;
    assert!(peer.expect(700, pick_ice).await.is_none());
}

#[tokio::test]
async fn too_many_misses_close_the_connection() {
    let Session { hbbs, ph, .. } = start_session(&[]).await;
    let mut ice_conn = ice_connection(&hbbs).await;
    for _ in 0..17 {
        send_ice_from_peer(&mut ice_conn, &ph.socket_addr, "sha-256 00", CANDIDATE).await;
    }
    assert!(ice_conn.closed_within(3000).await);
}

#[tokio::test]
async fn ice_is_bounded_per_route() {
    let Session {
        hbbs,
        peer,
        mut ctl,
        ph,
    } = start_session(&[]).await;
    let mut ice_conn = ice_connection(&hbbs).await;
    for i in 0..140 {
        send_ice_from_controller(&mut ctl, "peer01", FP, &format!("c{}", i)).await;
    }
    let mut to_peer = 0;
    while peer.expect(500, pick_ice).await.is_some() {
        to_peer += 1;
    }
    assert_eq!(to_peer, 128);
    for i in 0..140 {
        send_ice_from_peer(&mut ice_conn, &ph.socket_addr, FP, &format!("c{}", i)).await;
    }
    let mut to_controller = 0;
    while ctl.expect(500, pick_ice).await.is_some() {
        to_controller += 1;
    }
    assert_eq!(to_controller, 128);
}

#[tokio::test]
async fn oversized_sdp_and_ice_are_dropped() {
    let hbbs = Hbbs::start(&[], &[]).await;
    let peer = Peer::register(&hbbs, "peer01").await;
    let mut ctl = Tcp::connect(&hbbs, CONTROLLER_IP, Kx::V1).await;
    offer_request(&hbbs, &mut ctl, "peer01", endpoint("offer", FP, 4000)).await;
    let ph = peer.expect(3000, pick_punch_hole).await.unwrap();
    assert!(ph.webrtc_sdp_offer.is_empty());

    let Session {
        hbbs,
        mut ctl,
        ph,
        peer,
    } = start_session(&[]).await;
    let _reply = answer_over_tcp(&hbbs, &ph, endpoint("answer", FP, 4000)).await;
    let res = ctl.expect(3000, pick_punch_hole_response).await.unwrap();
    assert!(res.webrtc_sdp_answer.is_empty());
    let mut ice_conn = ice_connection(&hbbs).await;
    send_ice_from_peer(&mut ice_conn, &ph.socket_addr, FP, &"x".repeat(2049)).await;
    send_ice_from_peer(&mut ice_conn, &ph.socket_addr, &"k".repeat(257), CANDIDATE).await;
    assert!(ctl.expect(700, pick_ice).await.is_none());
    send_ice_from_controller(&mut ctl, "peer01", FP, &"x".repeat(2049)).await;
    assert!(peer.expect(700, pick_ice).await.is_none());
}

#[tokio::test]
async fn unreadable_offer_is_dropped() {
    let hbbs = Hbbs::start(&[], &[]).await;
    let peer = Peer::register(&hbbs, "peer01").await;
    for bad in ["webrtc://%%%", "rtc://abc", "webrtc://e30="] {
        let mut ctl = Tcp::connect(&hbbs, CONTROLLER_IP, Kx::V1).await;
        offer_request(&hbbs, &mut ctl, "peer01", bad.to_owned()).await;
        let ph = peer.expect(3000, pick_punch_hole).await.unwrap();
        assert!(ph.webrtc_sdp_offer.is_empty());
    }
}

#[tokio::test]
async fn switched_off_behaves_like_before() {
    for (envs, args) in [
        (vec![("WEBRTC", "N")], vec![]),
        (vec![], vec!["--webrtc", "N"]),
    ] {
        let hbbs = Hbbs::start(&envs, &args).await;
        let peer = Peer::register(&hbbs, "peer01").await;
        let mut ctl = Tcp::connect(&hbbs, CONTROLLER_IP, Kx::V1).await;
        offer_request(&hbbs, &mut ctl, "peer01", offer()).await;
        let ph = peer.expect(3000, pick_punch_hole).await.unwrap();
        assert!(ph.webrtc_sdp_offer.is_empty());
        let _reply = answer_over_tcp(&hbbs, &ph, answer()).await;
        let res = ctl.expect(3000, pick_punch_hole_response).await.unwrap();
        assert!(res.webrtc_sdp_answer.is_empty());
        // an IceCandidate is an unknown message, which closes the connection as before
        send_ice_from_controller(&mut ctl, "peer01", FP, CANDIDATE).await;
        assert!(ctl.closed_within(3000).await);
    }
}

#[tokio::test]
async fn forced_relay_drops_the_offer() {
    let hbbs = Hbbs::start(&[("ALWAYS_USE_RELAY", "Y")], &[]).await;
    let peer = Peer::register(&hbbs, "peer01").await;
    let mut ctl = Tcp::connect(&hbbs, CONTROLLER_IP, Kx::V1).await;
    offer_request(&hbbs, &mut ctl, "peer01", offer()).await;
    let ph = peer.expect(3000, pick_punch_hole).await.unwrap();
    assert!(ph.webrtc_sdp_offer.is_empty());
    assert_eq!(ph.nat_type.enum_value(), Ok(NatType::SYMMETRIC));
}

#[tokio::test]
async fn force_relay_travels_with_the_offer() {
    let hbbs = Hbbs::start(&[], &[]).await;
    let peer = Peer::register(&hbbs, "peer01").await;
    let mut ctl = Tcp::connect(&hbbs, CONTROLLER_IP, Kx::V1).await;
    let mut req = punch_hole_request("peer01", &hbbs.key);
    req.webrtc_sdp_offer = offer();
    req.force_relay = true;
    req.nat_type = NatType::SYMMETRIC.into();
    ctl.send(&msg(|m| m.set_punch_hole_request(req))).await;
    let ph = peer.expect(3000, pick_punch_hole).await.unwrap();
    assert!(ph.force_relay);
    assert_eq!(ph.webrtc_sdp_offer, offer());
    // without an offer force_relay is not forwarded, like the original server
    let mut ctl = Tcp::connect(&hbbs, CONTROLLER_IP, Kx::None).await;
    let mut req = punch_hole_request("peer01", &hbbs.key);
    req.force_relay = true;
    ctl.send(&msg(|m| m.set_punch_hole_request(req))).await;
    assert!(
        !peer
            .expect(3000, pick_punch_hole)
            .await
            .unwrap()
            .force_relay
    );
}

#[tokio::test]
async fn websocket_offer_is_dropped() {
    use hbb_common::{futures_util::SinkExt, protobuf::Message as _};
    let hbbs = Hbbs::start(&[], &[]).await;
    let peer = Peer::register(&hbbs, "peer01").await;
    let (mut ws, _) = tokio_tungstenite::connect_async(format!("ws://{}", hbbs.ws_addr()))
        .await
        .unwrap();
    let mut req = punch_hole_request("peer01", &hbbs.key);
    req.webrtc_sdp_offer = offer();
    ws.send(tungstenite::Message::Binary(
        msg(|m| m.set_punch_hole_request(req))
            .write_to_bytes()
            .unwrap(),
    ))
    .await
    .unwrap();
    let ph = peer.expect(3000, pick_punch_hole).await.unwrap();
    assert!(ph.webrtc_sdp_offer.is_empty());
}

#[tokio::test]
async fn routes_end_with_the_controller_connection() {
    let Session { hbbs, ctl, ph, .. } = start_session(&[]).await;
    drop(ctl);
    tokio::time::sleep(Duration::from_millis(300)).await;
    let mut ice_conn = ice_connection(&hbbs).await;
    for _ in 0..17 {
        send_ice_from_peer(&mut ice_conn, &ph.socket_addr, FP, CANDIDATE).await;
    }
    // every candidate misses now
    assert!(ice_conn.closed_within(3000).await);
}

// Waits out the 60 s WebRTC route TTL while the controller keeps its connection.
#[tokio::test]
async fn routes_expire() {
    let Session {
        hbbs,
        peer,
        mut ctl,
        ph,
    } = start_session(&[]).await;
    for _ in 0..3 {
        tokio::time::sleep(Duration::from_secs(18)).await;
        // keeps the connection busy without refreshing the route
        send_ice_from_controller(&mut ctl, "peer01", FP, CANDIDATE).await;
        assert!(peer.expect(2000, pick_ice).await.is_some());
    }
    tokio::time::sleep(Duration::from_secs(8)).await;
    send_ice_from_controller(&mut ctl, "peer01", FP, CANDIDATE).await;
    assert!(peer.expect(1000, pick_ice).await.is_none());
    let mut ice_conn = ice_connection(&hbbs).await;
    send_ice_from_peer(&mut ice_conn, &ph.socket_addr, FP, CANDIDATE).await;
    assert!(ctl.expect(1000, pick_ice).await.is_none());
    // a late answer is not forwarded either
    let _reply = answer_over_tcp(&hbbs, &ph, answer()).await;
    let res = ctl.expect(3000, pick_punch_hole_response).await.unwrap();
    assert!(res.webrtc_sdp_answer.is_empty());
}

// A controlled side without the server's key cannot send its encrypted answer
// and replies nothing; the controller's re-sent request then goes without the
// offer, so the peer answers with a plain punch and the controller can relay.
#[tokio::test]
async fn unanswered_retries_lose_the_offer() {
    let Session {
        hbbs,
        peer,
        mut ctl,
        ph,
    } = start_session(&[]).await;
    assert!(!ph.webrtc_sdp_offer.is_empty());
    offer_request(&hbbs, &mut ctl, "peer01", offer()).await;
    let ph = peer.expect(3000, pick_punch_hole).await.unwrap();
    assert!(!ph.webrtc_sdp_offer.is_empty());
    offer_request(&hbbs, &mut ctl, "peer01", offer()).await;
    let ph = peer.expect(3000, pick_punch_hole).await.unwrap();
    assert!(ph.webrtc_sdp_offer.is_empty());
}

#[tokio::test]
async fn answered_retries_keep_the_offer() {
    let Session {
        hbbs,
        peer,
        mut ctl,
        ph,
    } = start_session(&[]).await;
    let _reply = answer_over_tcp(&hbbs, &ph, answer()).await;
    assert!(ctl.expect(3000, pick_punch_hole_response).await.is_some());
    for _ in 0..3 {
        offer_request(&hbbs, &mut ctl, "peer01", offer()).await;
        let ph = peer.expect(3000, pick_punch_hole).await.unwrap();
        assert_eq!(ph.webrtc_sdp_offer, offer());
    }
}

#[tokio::test]
async fn ipv6_controller_gets_answer_and_ice() {
    let hbbs = Hbbs::start(&[], &[]).await;
    let peer = Peer::register(&hbbs, "peer01").await;
    let loopback6: std::net::IpAddr = "::1".parse().unwrap();
    let addr = std::net::SocketAddr::new(loopback6, hbbs.port);
    let mut ctl = Tcp::connect_to(addr, &hbbs.pk, loopback6, Kx::V1).await;
    assert!(ctl.secured);
    offer_request(&hbbs, &mut ctl, "peer01", offer()).await;
    let ph = peer.expect(3000, pick_punch_hole).await.unwrap();
    assert_eq!(ph.socket_addr.len(), 18);
    assert_eq!(ph.webrtc_sdp_offer, offer());
    let _reply = answer_over_tcp(&hbbs, &ph, answer()).await;
    let mut ice_conn = ice_connection(&hbbs).await;
    send_ice_from_peer(&mut ice_conn, &ph.socket_addr, FP, CANDIDATE).await;
    let mut got_answer = false;
    let mut got_ice = false;
    while !(got_answer && got_ice) {
        match ctl
            .recv(3000)
            .await
            .expect("controller got too little")
            .union
        {
            Some(rendezvous_message::Union::PunchHoleResponse(r)) => {
                assert_eq!(r.webrtc_sdp_answer, answer());
                got_answer = true;
            }
            Some(rendezvous_message::Union::IceCandidate(c)) => {
                assert_eq!(c.candidate, CANDIDATE);
                got_ice = true;
            }
            other => panic!("unexpected {:?}", other),
        }
    }
    send_ice_from_controller(&mut ctl, "peer01", FP, CANDIDATE).await;
    assert!(peer.expect(3000, pick_ice).await.is_some());
}

#[tokio::test]
async fn websocket_ice_candidate_closes_as_before() {
    use hbb_common::{
        futures_util::{SinkExt, StreamExt},
        protobuf::Message as _,
    };
    let hbbs = Hbbs::start(&[], &[]).await;
    let (mut ws, _) = tokio_tungstenite::connect_async(format!("ws://{}", hbbs.ws_addr()))
        .await
        .unwrap();
    let mut c = ice(FP, CANDIDATE);
    c.id = "peer01".to_owned();
    ws.send(tungstenite::Message::Binary(
        msg(|m| m.set_ice_candidate(c)).write_to_bytes().unwrap(),
    ))
    .await
    .unwrap();
    let closed = tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            match ws.next().await {
                None | Some(Err(_)) | Some(Ok(tungstenite::Message::Close(_))) => return true,
                Some(Ok(_)) => continue,
            }
        }
    })
    .await;
    assert_eq!(closed, Ok(true));
}

#[tokio::test]
async fn switch_values_false_and_off_turn_features_off() {
    let hbbs = Hbbs::start(
        &[
            ("WEBRTC", "false"),
            ("IPV6_PUNCH", "off"),
            ("UDP_PUNCH", "0"),
        ],
        &[],
    )
    .await;
    let peer = Peer::register(&hbbs, "peer01").await;
    let mut ctl = Tcp::connect(&hbbs, CONTROLLER_IP, Kx::V1).await;
    let mut req = punch_hole_request("peer01", &hbbs.key);
    req.webrtc_sdp_offer = offer();
    req.udp_port = 40000;
    req.socket_addr_v6 = mangle("[2001:db8::a]:40001".parse().unwrap());
    ctl.send(&msg(|m| m.set_punch_hole_request(req))).await;
    let ph = peer.expect(3000, pick_punch_hole).await.unwrap();
    assert!(ph.webrtc_sdp_offer.is_empty());
    assert!(ph.socket_addr_v6.is_empty());
    assert_eq!(ph.udp_port, 0);
    for line in ["WEBRTC=N", "IPV6_PUNCH=N", "UDP_PUNCH=N"] {
        assert!(hbbs.log().contains(line), "{}", line);
    }
}

// A WebRTC route keeps the controller's connection for ICE. Replies for its
// address can be forged by anyone; they must not queue up behind a controller
// that stopped reading. So the kept connection serves one response; after it
// only the ICE candidates of the route.
#[tokio::test]
async fn a_kept_connection_serves_one_response() {
    let Session {
        hbbs, mut ctl, ph, ..
    } = start_session(&[]).await;
    let relay_response = || {
        let mut rr = RelayResponse {
            socket_addr: ph.socket_addr.clone(),
            uuid: "u".to_owned(),
            relay_server: "relay.example:21117".to_owned(),
            version: "1.5.0".to_owned(),
            webrtc_sdp_answer: answer(),
            ..Default::default()
        };
        rr.set_id("peer01".to_owned());
        msg(|m| m.set_relay_response(rr))
    };
    for _ in 0..5 {
        let mut forged = Tcp::connect(&hbbs, PEER_IP, Kx::None).await;
        forged.send(&relay_response()).await;
    }
    let _reply = answer_over_tcp(&hbbs, &ph, answer()).await;
    let mut responses = 0;
    while let Some(m) = ctl.recv(700).await {
        if matches!(
            m.union,
            Some(rendezvous_message::Union::RelayResponse(_))
                | Some(rendezvous_message::Union::PunchHoleResponse(_))
        ) {
            responses += 1;
        }
    }
    assert_eq!(responses, 1);
    // the candidates still get through
    let mut ice_conn = ice_connection(&hbbs).await;
    send_ice_from_peer(&mut ice_conn, &ph.socket_addr, FP, CANDIDATE).await;
    assert!(ctl.expect(3000, pick_ice).await.is_some());
}

// More candidates than the route's budget are dropped quietly; they do not
// count as misses and do not close the controller's signalling connection.
#[tokio::test]
async fn over_budget_candidates_keep_the_connection() {
    let Session {
        hbbs,
        peer,
        mut ctl,
        ph,
    } = start_session(&[]).await;
    for i in 0..160 {
        send_ice_from_controller(&mut ctl, "peer01", FP, &format!("c{}", i)).await;
    }
    let mut ice_conn = ice_connection(&hbbs).await;
    for i in 0..160 {
        send_ice_from_peer(&mut ice_conn, &ph.socket_addr, FP, &format!("c{}", i)).await;
    }
    while ctl.expect(300, pick_ice).await.is_some() {}
    while peer.expect(300, pick_ice).await.is_some() {}
    assert!(!ctl.closed_within(500).await);
    assert!(!ice_conn.closed_within(100).await);
}
