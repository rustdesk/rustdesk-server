// UDP hole punching as the 1.5.0 client does it: the controller's UDP NAT test
// (client.rs test_udp_uat), udp_port in PunchHoleRequest, the controlled side's
// UDP PunchHoleSent from its punch socket (rendezvous_mediator.rs punch_udp_hole,
// three copies) and the PunchHoleResponse with is_udp to the controller.
// Plus the negative cases that matter for reflection (CVE-2026-30784).
mod common;

use common::*;
use hbb_common::{
    bytes::Bytes,
    protobuf::Message as _,
    rendezvous_proto::*,
    tokio::{self, net::UdpSocket},
};
use sodiumoxide::crypto::sign;
use std::{net::SocketAddr, time::Duration};

async fn udp_socket(ip: std::net::IpAddr) -> UdpSocket {
    UdpSocket::bind(SocketAddr::new(ip, 0)).await.unwrap()
}

/// Receives raw datagrams for `ms`, returns them parsed.
async fn drain(sock: &UdpSocket, ms: u64) -> Vec<RendezvousMessage> {
    let mut out = vec![];
    let mut buf = vec![0u8; 65536];
    let deadline = tokio::time::Instant::now() + Duration::from_millis(ms);
    while let Ok(Ok((n, _))) = tokio::time::timeout_at(deadline, sock.recv_from(&mut buf)).await {
        if let Some(m) = parse(&buf[..n]) {
            out.push(m);
        }
    }
    out
}

fn test_nat_request() -> Vec<u8> {
    msg(|m| m.set_test_nat_request(TestNatRequest::default()))
        .write_to_bytes()
        .unwrap()
}

#[tokio::test]
async fn udp_test_nat_reports_the_port_only() {
    // a server serial above the request's would add a ConfigUpdate over TCP
    let hbbs = Hbbs::start(&[], &["-s", "5"]).await;
    let sock = udp_socket(CONTROLLER_IP).await;
    let req = test_nat_request();
    sock.send_to(&req, hbbs.udp_addr()).await.unwrap();
    let mut buf = [0u8; 2048];
    let (n, from) = tokio::time::timeout(Duration::from_secs(3), sock.recv_from(&mut buf))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(from.port(), hbbs.port);
    let Some(rendezvous_message::Union::TestNatResponse(res)) = parse(&buf[..n]).unwrap().union
    else {
        panic!("no TestNatResponse");
    };
    assert_eq!(res.port, sock.local_addr().unwrap().port() as i32);
    assert!(res.cu.is_none());
    // the answer is the port and nothing else: at most 4 bytes more than the request
    assert!(
        n <= req.len() + 4,
        "answer {} bytes for a {} byte request",
        n,
        req.len()
    );
}

#[tokio::test]
async fn udp_test_nat_is_rate_limited() {
    let hbbs = Hbbs::start(&[], &[]).await;
    let sock = udp_socket(CONTROLLER_IP).await;
    let req = test_nat_request();
    for _ in 0..200 {
        sock.send_to(&req, hbbs.udp_addr()).await.unwrap();
    }
    let answers = drain(&sock, 1000).await.len();
    assert!((60..=66).contains(&answers), "{} answers", answers);
    // another source has its own budget
    let other = udp_socket("127.0.0.3".parse().unwrap()).await;
    other.send_to(&req, hbbs.udp_addr()).await.unwrap();
    assert_eq!(drain(&other, 500).await.len(), 1);
    // and the bucket refills
    tokio::time::sleep(Duration::from_millis(1500)).await;
    for _ in 0..5 {
        sock.send_to(&req, hbbs.udp_addr()).await.unwrap();
    }
    assert_eq!(drain(&sock, 500).await.len(), 5);
}

#[tokio::test]
async fn switched_off_ignores_udp() {
    for (envs, args) in [
        (vec![("UDP_PUNCH", "N")], vec![]),
        (vec![], vec!["--udp-punch", "N"]),
    ] {
        let hbbs = Hbbs::start(&envs, &args).await;
        let sock = udp_socket(CONTROLLER_IP).await;
        sock.send_to(&test_nat_request(), hbbs.udp_addr())
            .await
            .unwrap();
        assert!(drain(&sock, 700).await.is_empty());
        let peer = Peer::register(&hbbs, "peer01").await;
        let mut ctl = Tcp::connect(&hbbs, CONTROLLER_IP, Kx::None).await;
        let mut req = punch_hole_request("peer01", &hbbs.key);
        req.udp_port = 40000;
        ctl.send(&msg(|m| m.set_punch_hole_request(req))).await;
        let ph = peer.expect(3000, pick_punch_hole).await.unwrap();
        assert_eq!(ph.udp_port, 0);
        let punch = udp_socket(PEER_IP).await;
        send_hole_sent(&punch, &hbbs, &ph.socket_addr, "peer01", None).await;
        assert!(ctl.expect(700, pick_punch_hole_response).await.is_none());
    }
}

async fn send_hole_sent(
    sock: &UdpSocket,
    hbbs: &Hbbs,
    ph_socket_addr: &Bytes,
    id: &str,
    extra: Option<fn(&mut PunchHoleSent)>,
) {
    let mut phs = PunchHoleSent {
        socket_addr: ph_socket_addr.clone(),
        id: id.to_owned(),
        relay_server: "relay.example:21117".to_owned(),
        nat_type: NatType::ASYMMETRIC.into(),
        version: "1.5.0".to_owned(),
        ..Default::default()
    };
    if let Some(f) = extra {
        f(&mut phs);
    }
    let bytes = msg(|m| m.set_punch_hole_sent(phs))
        .write_to_bytes()
        .unwrap();
    sock.send_to(&bytes, hbbs.udp_addr()).await.unwrap();
}

struct Punch {
    hbbs: Hbbs,
    peer: Peer,
    ctl: Tcp,
    ph: PunchHole,
}

async fn start_udp_punch(kx: Kx) -> Punch {
    let hbbs = Hbbs::start(&[], &[]).await;
    let peer = Peer::register(&hbbs, "peer01").await;
    // the UDP NAT test gives the controller its punch port
    let ctl_udp = udp_socket(CONTROLLER_IP).await;
    ctl_udp
        .send_to(&test_nat_request(), hbbs.udp_addr())
        .await
        .unwrap();
    ctl_udp
        .send_to(&test_nat_request(), hbbs.udp_addr())
        .await
        .unwrap();
    let port = match drain(&ctl_udp, 500)
        .await
        .first()
        .and_then(|m| m.union.clone())
    {
        Some(rendezvous_message::Union::TestNatResponse(r)) => r.port,
        other => panic!("no TestNatResponse: {:?}", other),
    };
    let mut ctl = Tcp::connect(&hbbs, CONTROLLER_IP, kx).await;
    let mut req = punch_hole_request("peer01", &hbbs.key);
    req.udp_port = port;
    ctl.send(&msg(|m| m.set_punch_hole_request(req))).await;
    let ph = peer
        .expect(5000, pick_punch_hole)
        .await
        .expect("no PunchHole");
    assert_eq!(ph.udp_port, port);
    Punch {
        hbbs,
        peer,
        ctl,
        ph,
    }
}

#[tokio::test]
async fn udp_punch_reaches_the_controller_over_tcp() {
    for kx in [Kx::None, Kx::V1] {
        let Punch {
            hbbs,
            peer,
            mut ctl,
            ph,
        } = start_udp_punch(kx).await;
        // three copies from a fresh socket of the same host, like punch_udp_hole
        let punch = udp_socket(PEER_IP).await;
        for _ in 0..3 {
            send_hole_sent(&punch, &hbbs, &ph.socket_addr, "peer01", None).await;
        }
        let res = ctl
            .expect(3000, pick_punch_hole_response)
            .await
            .expect("no response");
        assert!(res.is_udp);
        assert_eq!(demangle(&res.socket_addr), punch.local_addr().unwrap());
        assert_eq!(res.relay_server, "relay.example:21117");
        assert_eq!(res.nat_type(), NatType::ASYMMETRIC);
        let id_pk = IdPk::parse_from_bytes(&sign::verify(&res.pk, &hbbs.pk).unwrap()).unwrap();
        assert_eq!(id_pk.id, "peer01");
        assert_eq!(&id_pk.pk[..], &peer.pk.0[..]);
        // only one answer, and nothing ever comes back over UDP
        assert!(ctl.expect(700, pick_punch_hole_response).await.is_none());
        assert!(drain(&punch, 100).await.is_empty());
    }
}

#[tokio::test]
async fn forged_hole_sent_without_a_route_sends_nothing() {
    let hbbs = Hbbs::start(&[], &[]).await;
    let peer = Peer::register(&hbbs, "peer01").await;
    // a controller that did not ask for UDP
    let mut ctl = Tcp::connect(&hbbs, CONTROLLER_IP, Kx::None).await;
    ctl.send(&msg(|m| {
        m.set_punch_hole_request(punch_hole_request("peer01", &hbbs.key))
    }))
    .await;
    let ph = peer.expect(3000, pick_punch_hole).await.unwrap();
    let forger = udp_socket(PEER_IP).await;
    send_hole_sent(&forger, &hbbs, &ph.socket_addr, "peer01", None).await;
    // and one for an address nobody has a connection from
    let nobody = mangle("127.0.0.9:4444".parse().unwrap());
    send_hole_sent(&forger, &hbbs, &nobody, "peer01", None).await;
    assert!(ctl.expect(700, pick_punch_hole_response).await.is_none());
    assert!(drain(&forger, 300).await.is_empty());
    assert!(drain(&peer.sock, 100).await.is_empty());
}

#[tokio::test]
async fn hole_sent_for_another_id_is_ignored() {
    let Punch {
        hbbs, mut ctl, ph, ..
    } = start_udp_punch(Kx::None).await;
    let punch = udp_socket(PEER_IP).await;
    send_hole_sent(&punch, &hbbs, &ph.socket_addr, "peer02", None).await;
    assert!(ctl.expect(700, pick_punch_hole_response).await.is_none());
    // the real one still works
    send_hole_sent(&punch, &hbbs, &ph.socket_addr, "peer01", None).await;
    let res = ctl.expect(3000, pick_punch_hole_response).await.unwrap();
    assert!(res.is_udp);
    assert_eq!(demangle(&res.socket_addr), punch.local_addr().unwrap());
}

// From another host than the peer registered from (forged, NAT pool or
// multi-WAN) it counts as lost: it neither reaches the controller nor uses up
// the route the peer's own reply needs.
#[tokio::test]
async fn hole_sent_from_another_host_is_ignored() {
    let Punch {
        hbbs, mut ctl, ph, ..
    } = start_udp_punch(Kx::None).await;
    let stranger = udp_socket("127.0.0.3".parse().unwrap()).await;
    send_hole_sent(
        &stranger,
        &hbbs,
        &ph.socket_addr,
        "peer01",
        Some(|phs| {
            phs.socket_addr_v6 = mangle("[2001:db8::66]:4000".parse().unwrap());
            phs.webrtc_sdp_answer = "webrtc://forged".to_owned();
        }),
    )
    .await;
    assert!(ctl.expect(700, pick_punch_hole_response).await.is_none());
    assert!(drain(&stranger, 100).await.is_empty());
    let punch = udp_socket(PEER_IP).await;
    send_hole_sent(&punch, &hbbs, &ph.socket_addr, "peer01", None).await;
    let res = ctl.expect(3000, pick_punch_hole_response).await.unwrap();
    assert!(res.is_udp);
    assert_eq!(demangle(&res.socket_addr), punch.local_addr().unwrap());
    assert!(res.socket_addr_v6.is_empty());
}

#[tokio::test]
async fn wrong_licence_key_opens_no_udp_route() {
    let (pk, sk) = sign::gen_keypair();
    let mut secret = sk.0.to_vec();
    secret.truncate(64);
    let hbbs = Hbbs::start(&[], &["-k", &base64::encode(&secret)]).await;
    let key = base64::encode(pk);
    let peer = Peer::register(&hbbs, "peer01").await;
    let mut ctl = Tcp::connect(&hbbs, CONTROLLER_IP, Kx::None).await;
    let mut req = punch_hole_request("peer01", "wrong-key");
    req.udp_port = 40000;
    ctl.send(&msg(|m| m.set_punch_hole_request(req))).await;
    let res = ctl.expect(3000, pick_punch_hole_response).await.unwrap();
    assert_eq!(
        res.failure.enum_value(),
        Ok(punch_hole_response::Failure::LICENSE_MISMATCH)
    );
    assert!(peer.expect(500, pick_punch_hole).await.is_none());
    // a forged PunchHoleSent for that connection finds no route
    let forger = udp_socket(PEER_IP).await;
    send_hole_sent(&forger, &hbbs, &mangle(ctl.local_addr()), "peer01", None).await;
    assert!(ctl.expect(700, pick_punch_hole_response).await.is_none());
    // the right key works
    let mut ctl = Tcp::connect(&hbbs, CONTROLLER_IP, Kx::None).await;
    let mut req = punch_hole_request("peer01", &key);
    req.udp_port = 40000;
    ctl.send(&msg(|m| m.set_punch_hole_request(req))).await;
    assert_eq!(
        peer.expect(3000, pick_punch_hole).await.unwrap().udp_port,
        40000
    );
}

// A 1.4.0 controlled side knows no udp_port and answers over TCP as before.
#[tokio::test]
async fn old_controlled_side_answers_over_tcp() {
    let Punch {
        hbbs, mut ctl, ph, ..
    } = start_udp_punch(Kx::None).await;
    let mut reply = Tcp::connect(&hbbs, PEER_IP, Kx::None).await;
    let reply_addr = reply.local_addr();
    reply
        .send(&msg(|m| {
            m.set_punch_hole_sent(PunchHoleSent {
                socket_addr: ph.socket_addr.clone(),
                id: "peer01".to_owned(),
                nat_type: NatType::ASYMMETRIC.into(),
                version: "1.4.0".to_owned(),
                ..Default::default()
            })
        }))
        .await;
    drop(reply);
    let res = ctl.expect(3000, pick_punch_hole_response).await.unwrap();
    assert!(!res.is_udp);
    assert_eq!(demangle(&res.socket_addr), reply_addr);
}

// The usual 1.5.0 round with everything on: UDP, IPv6 and WebRTC together; the
// controlled side puts its answer and IPv6 address into the UDP PunchHoleSent.
#[tokio::test]
async fn udp_ipv6_and_webrtc_in_one_round() {
    let hbbs = Hbbs::start(&[], &[]).await;
    let peer = Peer::register(&hbbs, "peer01").await;
    let fp = "sha-256 11:22:33";
    let endpoint = |kind: &str| {
        let sdp = format!("v=0\r\ns=-\r\nt=0 0\r\na=fingerprint:{}\r\nm=application 9 UDP/DTLS/SCTP webrtc-datachannel\r\n", fp);
        let json = serde_json::json!({"type": kind, "sdp": sdp, "ice_policy": "all"});
        format!("webrtc://{}", base64::encode(json.to_string()))
    };
    let a_v6 = mangle("[2001:db8::a]:40001".parse().unwrap());
    let b_v6 = mangle("[2001:db8::b]:40002".parse().unwrap());
    let mut ctl = Tcp::connect(&hbbs, CONTROLLER_IP, Kx::V1).await;
    let mut req = punch_hole_request("peer01", &hbbs.key);
    req.udp_port = 40000;
    req.socket_addr_v6 = a_v6.clone();
    req.webrtc_sdp_offer = endpoint("offer");
    ctl.send(&msg(|m| m.set_punch_hole_request(req))).await;
    let ph = peer.expect(3000, pick_punch_hole).await.unwrap();
    assert_eq!(ph.udp_port, 40000);
    assert_eq!(ph.socket_addr_v6, a_v6);
    assert_eq!(ph.webrtc_sdp_offer, endpoint("offer"));
    let punch = udp_socket(PEER_IP).await;
    let phs = PunchHoleSent {
        socket_addr: ph.socket_addr.clone(),
        id: "peer01".to_owned(),
        nat_type: NatType::ASYMMETRIC.into(),
        version: "1.5.0".to_owned(),
        socket_addr_v6: b_v6.clone(),
        webrtc_sdp_answer: endpoint("answer"),
        ..Default::default()
    };
    for _ in 0..3 {
        let bytes = msg(|m| m.set_punch_hole_sent(phs.clone()))
            .write_to_bytes()
            .unwrap();
        punch.send_to(&bytes, hbbs.udp_addr()).await.unwrap();
    }
    let res = ctl.expect(3000, pick_punch_hole_response).await.unwrap();
    assert!(res.is_udp);
    assert_eq!(demangle(&res.socket_addr), punch.local_addr().unwrap());
    assert_eq!(res.socket_addr_v6, b_v6);
    assert_eq!(res.webrtc_sdp_answer, endpoint("answer"));
    // the controller's connection stays for ICE
    let mut c = IceCandidate {
        id: "peer01".to_owned(),
        session_key: fp.to_owned(),
        candidate: "c1".to_owned(),
        ..Default::default()
    };
    ctl.send(&msg(|m| m.set_ice_candidate(c.clone()))).await;
    assert_eq!(peer.expect(3000, pick_ice).await.unwrap().candidate, "c1");
    let mut ice_conn = Tcp::connect(&hbbs, PEER_IP, Kx::V1).await;
    c.id.clear();
    c.socket_addr = ph.socket_addr.clone();
    c.candidate = "c2".to_owned();
    ice_conn.send(&msg(|m| m.set_ice_candidate(c))).await;
    assert_eq!(ctl.expect(3000, pick_ice).await.unwrap().candidate, "c2");
}

#[tokio::test]
async fn garbage_does_not_crash_the_server() {
    let mut hbbs = Hbbs::start(&[], &[]).await;
    let sock = udp_socket(CONTROLLER_IP).await;
    let mut seed = 0x1234_5678u32;
    let mut next = || {
        seed ^= seed << 13;
        seed ^= seed >> 17;
        seed ^= seed << 5;
        seed
    };
    for i in 0..2000 {
        let len = (next() % 1500) as usize;
        let mut bytes: Vec<u8> = (0..len).map(|_| next() as u8).collect();
        if i % 2 == 0 {
            // structurally valid PunchHoleSent with random fields
            let socket_addr: Vec<u8> = (0..(next() % 20)).map(|_| next() as u8).collect();
            let phs = PunchHoleSent {
                socket_addr: socket_addr.into(),
                id: format!("{}", next()),
                webrtc_sdp_answer: "x".repeat((next() % 5000) as usize),
                socket_addr_v6: bytes.clone().into(),
                ..Default::default()
            };
            bytes = msg(|m| m.set_punch_hole_sent(phs))
                .write_to_bytes()
                .unwrap();
        }
        sock.send_to(&bytes, hbbs.udp_addr()).await.ok();
    }
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(hbbs.is_running());
    // random bytes may happen to be a registration, but nothing reached a controller
    // or peer, and UDP still works
    for m in drain(&sock, 300).await {
        assert!(!matches!(
            m.union,
            Some(rendezvous_message::Union::PunchHoleResponse(_))
                | Some(rendezvous_message::Union::PunchHole(_))
                | Some(rendezvous_message::Union::IceCandidate(_))
        ));
    }
    let fresh = udp_socket(PEER_IP).await;
    fresh
        .send_to(&test_nat_request(), hbbs.udp_addr())
        .await
        .unwrap();
    assert_eq!(drain(&fresh, 700).await.len(), 1);
}

#[tokio::test]
async fn invalid_or_forced_udp_port_is_not_forwarded() {
    let hbbs = Hbbs::start(&[], &[]).await;
    let peer = Peer::register(&hbbs, "peer01").await;
    for port in [-5, 0, 65536, 1 << 30] {
        let mut ctl = Tcp::connect(&hbbs, CONTROLLER_IP, Kx::None).await;
        let mut req = punch_hole_request("peer01", &hbbs.key);
        req.udp_port = port;
        ctl.send(&msg(|m| m.set_punch_hole_request(req))).await;
        assert_eq!(
            peer.expect(3000, pick_punch_hole).await.unwrap().udp_port,
            0
        );
    }
    let hbbs = Hbbs::start(&[("ALWAYS_USE_RELAY", "Y")], &[]).await;
    let peer = Peer::register(&hbbs, "peer01").await;
    let mut ctl = Tcp::connect(&hbbs, CONTROLLER_IP, Kx::None).await;
    let mut req = punch_hole_request("peer01", &hbbs.key);
    req.udp_port = 40000;
    ctl.send(&msg(|m| m.set_punch_hole_request(req))).await;
    assert_eq!(
        peer.expect(3000, pick_punch_hole).await.unwrap().udp_port,
        0
    );
}

#[tokio::test]
async fn same_intranet_without_offer_keeps_fetch_local_addr() {
    let hbbs = Hbbs::start(&[], &[]).await;
    let peer = Peer::register(&hbbs, "peer01").await;
    let mut ctl = Tcp::connect(&hbbs, PEER_IP, Kx::None).await;
    let mut req = punch_hole_request("peer01", &hbbs.key);
    req.udp_port = 40000;
    ctl.send(&msg(|m| m.set_punch_hole_request(req))).await;
    assert!(peer.expect(3000, pick_fetch_local_addr).await.is_some());
}

// Waits out the 30 s UDP route TTL while the controller keeps its connection.
#[tokio::test]
async fn udp_routes_expire() {
    let Punch {
        hbbs, mut ctl, ph, ..
    } = start_udp_punch(Kx::None).await;
    tokio::time::sleep(Duration::from_secs(20)).await;
    // keeps the connection without touching the route (an unmatched candidate)
    ctl.send(&msg(|m| {
        m.set_ice_candidate(IceCandidate {
            id: "peer01".to_owned(),
            session_key: "k".to_owned(),
            candidate: "c".to_owned(),
            ..Default::default()
        })
    }))
    .await;
    tokio::time::sleep(Duration::from_secs(11)).await;
    let punch = udp_socket(PEER_IP).await;
    send_hole_sent(&punch, &hbbs, &ph.socket_addr, "peer01", None).await;
    assert!(ctl.expect(1000, pick_punch_hole_response).await.is_none());
}

// Spoofed sources can use up the global answer budget; a controller whose TCP
// connection proves its address still gets its UDP NAT test answered.
#[tokio::test]
async fn connected_controllers_keep_their_nat_test() {
    // 5 answers per second in total, burst 10
    let hbbs = Hbbs::start(&[("UDP_TEST_NAT_LIMIT", "5")], &[]).await;
    let req = test_nat_request();
    let server = hbbs.udp_addr();
    let _ctl = Tcp::connect(&hbbs, "127.9.9.10".parse().unwrap(), Kx::None).await;
    // give the server time to accept the connection
    tokio::time::sleep(Duration::from_millis(200)).await;
    let connected = udp_socket("127.9.9.10".parse().unwrap()).await;
    let stranger = udp_socket("127.9.9.9".parse().unwrap()).await;
    // 30 other sources use up the global budget
    let sources: Vec<std::net::UdpSocket> = (0..30u8)
        .filter_map(|i| {
            std::net::UdpSocket::bind((std::net::Ipv4Addr::new(127, 10, 0, i + 1), 0)).ok()
        })
        .collect();
    for s in &sources {
        s.send_to(&req, server).ok();
    }
    tokio::time::sleep(Duration::from_millis(50)).await;
    for _ in 0..5 {
        connected.send_to(&req, server).await.unwrap();
    }
    stranger.send_to(&req, server).await.unwrap();
    assert_eq!(drain(&connected, 500).await.len(), 5);
    assert!(drain(&stranger, 100).await.is_empty());
}

// The client starts its UDP NAT test just before it connects over TCP
// (client.rs test_udp_uat, then _start_inner). With the global budget used up,
// the early request is refused and the retry after the connect is answered.
#[tokio::test]
async fn nat_test_retry_after_the_connect_is_answered() {
    let hbbs = Hbbs::start(&[("UDP_TEST_NAT_LIMIT", "5")], &[]).await;
    let req = test_nat_request();
    let server = hbbs.udp_addr();
    let sources: Vec<std::net::UdpSocket> = (0..30u8)
        .filter_map(|i| {
            std::net::UdpSocket::bind((std::net::Ipv4Addr::new(127, 11, 0, i + 1), 0)).ok()
        })
        .collect();
    for s in &sources {
        s.send_to(&req, server).ok();
    }
    tokio::time::sleep(Duration::from_millis(50)).await;
    let ip: std::net::IpAddr = "127.9.9.20".parse().unwrap();
    let udp = udp_socket(ip).await;
    udp.send_to(&req, server).await.unwrap();
    assert!(drain(&udp, 100).await.is_empty());
    let _ctl = Tcp::connect(&hbbs, ip, Kx::None).await;
    // like the client, retry until answered (the server may see the UDP
    // request before it has accepted the TCP connection)
    let mut answered = 0;
    for _ in 0..5 {
        udp.send_to(&req, server).await.unwrap();
        answered = drain(&udp, 200).await.len();
        if answered > 0 {
            break;
        }
    }
    assert_eq!(answered, 1);
}

// 1.4.x controllers send udp_port too, but their UDP attempt can hold up the
// relay fallback for 20 s and 1.4.1 has no TCP round besides; they keep the
// original behavior.
#[tokio::test]
async fn udp_only_from_controllers_since_1_5_0() {
    let hbbs = Hbbs::start(&[], &[]).await;
    let peer = Peer::register(&hbbs, "peer01").await;
    for (version, forwarded) in [
        ("1.4.0", false),
        ("1.4.1", false),
        ("1.4.9", false),
        ("1.5.0", true),
        ("1.6.0", true),
    ] {
        let mut ctl = Tcp::connect(&hbbs, CONTROLLER_IP, Kx::None).await;
        let mut req = punch_hole_request("peer01", &hbbs.key);
        req.udp_port = 40000;
        req.version = version.to_owned();
        ctl.send(&msg(|m| m.set_punch_hole_request(req))).await;
        let ph = peer.expect(3000, pick_punch_hole).await.unwrap();
        assert_eq!(ph.udp_port == 40000, forwarded, "version {}", version);
    }
}

// A 1.4.x controlled side punches UDP instead of TCP when it gets a udp_port,
// with probes 1.5.0 does not answer. Once it has replied with its version it
// gets the old behavior; a reply from another host does not count.
#[tokio::test]
async fn legacy_controlled_side_gets_no_udp_port_after_its_reply() {
    let hbbs = Hbbs::start(&[], &[]).await;
    let peer = Peer::register(&hbbs, "peer01").await;
    async fn request_udp(hbbs: &Hbbs, peer: &Peer) -> (Tcp, PunchHole) {
        let mut ctl = Tcp::connect(hbbs, CONTROLLER_IP, Kx::None).await;
        let mut req = punch_hole_request("peer01", &hbbs.key);
        req.udp_port = 40000;
        req.socket_addr_v6 = mangle("[2001:db8::a]:40001".parse().unwrap());
        ctl.send(&msg(|m| m.set_punch_hole_request(req))).await;
        (ctl, peer.expect(3000, pick_punch_hole).await.unwrap())
    }
    async fn tcp_reply(hbbs: &Hbbs, ip: std::net::IpAddr, ph: &PunchHole, version: &str) {
        let mut reply = Tcp::connect(hbbs, ip, Kx::None).await;
        reply
            .send(&msg(|m| {
                m.set_punch_hole_sent(PunchHoleSent {
                    socket_addr: ph.socket_addr.clone(),
                    id: "peer01".to_owned(),
                    version: version.to_owned(),
                    ..Default::default()
                })
            }))
            .await;
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    let (mut ctl, ph) = request_udp(&hbbs, &peer).await;
    assert_eq!(ph.udp_port, 40000);
    let punch = udp_socket(PEER_IP).await;
    send_hole_sent(
        &punch,
        &hbbs,
        &ph.socket_addr,
        "peer01",
        Some(|p| p.version = "1.4.9".to_owned()),
    )
    .await;
    assert!(
        ctl.expect(3000, pick_punch_hole_response)
            .await
            .unwrap()
            .is_udp
    );
    // from now on: no udp_port, no IPv6
    let (_ctl, ph) = request_udp(&hbbs, &peer).await;
    assert_eq!(ph.udp_port, 0);
    assert!(ph.socket_addr_v6.is_empty());
    // a reply claiming 1.5.0 from another host does not count
    tcp_reply(&hbbs, "127.0.0.3".parse().unwrap(), &ph, "1.5.0").await;
    let (_ctl, ph) = request_udp(&hbbs, &peer).await;
    assert_eq!(ph.udp_port, 0);
    // the peer updated to 1.5.0
    tcp_reply(&hbbs, PEER_IP, &ph, "1.5.0").await;
    let (_ctl, ph) = request_udp(&hbbs, &peer).await;
    assert_eq!(ph.udp_port, 40000);
    assert!(!ph.socket_addr_v6.is_empty());
}

// The legacy mark comes only from a reply to a waiting controller, over plain
// TCP or a claimed UDP route: not over WebSocket (address from a header) and
// not without a pending request.
#[tokio::test]
async fn forged_replies_do_not_mark_a_peer_legacy() {
    use hbb_common::futures_util::SinkExt;
    let hbbs = Hbbs::start(&[], &[]).await;
    let peer = Peer::register(&hbbs, "peer01").await;
    let old_reply = |socket_addr: Bytes| {
        msg(|m| {
            m.set_punch_hole_sent(PunchHoleSent {
                socket_addr,
                id: "peer01".to_owned(),
                version: "1.4.0".to_owned(),
                ..Default::default()
            })
        })
        .write_to_bytes()
        .unwrap()
    };
    // a waiting controller
    let mut ctl = Tcp::connect(&hbbs, CONTROLLER_IP, Kx::None).await;
    let mut req = punch_hole_request("peer01", &hbbs.key);
    req.udp_port = 40000;
    ctl.send(&msg(|m| m.set_punch_hole_request(req.clone())))
        .await;
    let ph = peer.expect(5000, pick_punch_hole).await.unwrap();
    // over WebSocket, claiming the peer's address in a header
    let mut request = tungstenite::client::IntoClientRequest::into_client_request(format!(
        "ws://{}",
        hbbs.ws_addr()
    ))
    .unwrap();
    request
        .headers_mut()
        .insert("X-Real-IP", PEER_IP.to_string().parse().unwrap());
    let (mut ws, _) = tokio_tungstenite::connect_async(request).await.unwrap();
    ws.send(tungstenite::Message::Binary(old_reply(
        ph.socket_addr.clone(),
    )))
    .await
    .unwrap();
    // over TCP from the peer's host, but for no waiting controller
    let mut tcp = Tcp::connect(&hbbs, PEER_IP, Kx::None).await;
    tcp.send_raw(old_reply(mangle("127.0.0.9:4000".parse().unwrap())))
        .await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    let mut ctl2 = Tcp::connect(&hbbs, CONTROLLER_IP, Kx::None).await;
    ctl2.send(&msg(|m| m.set_punch_hole_request(req))).await;
    assert_eq!(
        peer.expect(5000, pick_punch_hole).await.unwrap().udp_port,
        40000
    );
    drop(ctl);
}
