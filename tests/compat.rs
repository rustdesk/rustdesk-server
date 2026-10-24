// Differential test against the original hbbs: the same old-style message
// sequences (no new fields, as clients up to 1.5.0 send them with UDP, IPv6
// and WebRTC off) go to the original binary and to this one, and the observed
// replies must be the same. The tests need an hbbs built from the commit
// before these connection types (08cd442) and are therefore ignored by a
// plain `cargo test`:
//   git worktree add ../hbbs-original 08cd442
//   (cd ../hbbs-original && git submodule update --init --recursive \
//     && cargo build --release)
//   HBBS_BASELINE_BIN=../hbbs-original/target/release/hbbs \
//     cargo test --test compat -- --ignored
// They pin that commit's behaviour, so an intended later change to the
// old-style replies makes them fail; update or drop them then.
mod common;

use common::*;
use hbb_common::{protobuf::Message as _, rendezvous_proto::*, tokio};
use sodiumoxide::crypto::sign;
use std::net::SocketAddr;

fn baseline_bin() -> String {
    let bin = std::env::var("HBBS_BASELINE_BIN")
        .expect("set HBBS_BASELINE_BIN to the original hbbs binary");
    // hbbs runs in a temporary directory, where a relative path would not resolve
    let path = std::fs::canonicalize(&bin).unwrap_or_else(|err| panic!("{}: {}", bin, err));
    path.to_string_lossy().into_owned()
}

/// What a client could observe, with addresses replaced by roles and
/// signatures by their verified content, so that two servers compare.
struct Observer<'a> {
    hbbs: &'a Hbbs,
    roles: Vec<(SocketAddr, &'static str)>,
    lines: Vec<String>,
}

impl<'a> Observer<'a> {
    fn addr(&self, bytes: &[u8]) -> String {
        if bytes.is_empty() {
            return "-".to_owned();
        }
        let addr = demangle(bytes);
        self.roles
            .iter()
            .find(|(a, _)| *a == addr)
            .map(|(_, r)| r.to_string())
            .unwrap_or_else(|| format!("other({})", bytes.len()))
    }

    fn pk(&self, signed: &[u8]) -> String {
        if signed.is_empty() {
            return "-".to_owned();
        }
        match sign::verify(signed, &self.hbbs.pk)
            .ok()
            .and_then(|b| IdPk::parse_from_bytes(&b).ok())
        {
            Some(id_pk) => format!("signed({}, {} bytes)", id_pk.id, id_pk.pk.len()),
            None => "bad signature".to_owned(),
        }
    }

    fn note(&mut self, what: &str, m: Option<rendezvous_message::Union>) {
        use rendezvous_message::Union::*;
        let line = match m {
            None => "nothing".to_owned(),
            Some(PunchHole(p)) => format!(
                "PunchHole addr={} nat={:?} relay={} udp_port={} v6={} offer={} force_relay={}",
                self.addr(&p.socket_addr),
                p.nat_type.enum_value(),
                p.relay_server,
                p.udp_port,
                p.socket_addr_v6.len(),
                p.webrtc_sdp_offer.len(),
                p.force_relay
            ),
            Some(FetchLocalAddr(f)) => format!(
                "FetchLocalAddr addr={} relay={} v6={}",
                self.addr(&f.socket_addr),
                f.relay_server,
                f.socket_addr_v6.len()
            ),
            Some(PunchHoleResponse(r)) => format!(
                "PunchHoleResponse addr={} pk={} relay={} nat={:?} local={:?} udp={} v6={} answer={} failure={:?} other={}",
                self.addr(&r.socket_addr),
                self.pk(&r.pk),
                r.relay_server,
                r.nat_type(),
                r.is_local(),
                r.is_udp,
                r.socket_addr_v6.len(),
                r.webrtc_sdp_answer.len(),
                r.failure.enum_value(),
                r.other_failure
            ),
            Some(RelayResponse(r)) => format!(
                "RelayResponse addr={} uuid={} relay={} pk={} v6={} answer={}",
                self.addr(&r.socket_addr),
                r.uuid,
                r.relay_server,
                self.pk(r.pk()),
                r.socket_addr_v6.len(),
                r.webrtc_sdp_answer.len()
            ),
            Some(RequestRelay(r)) => format!(
                "RequestRelay addr={} uuid={} relay={} id={}",
                self.addr(&r.socket_addr),
                r.uuid,
                r.relay_server,
                r.id
            ),
            Some(TestNatResponse(r)) => format!("TestNatResponse port_ok={} cu={}", r.port > 0, r.cu.is_some()),
            Some(RegisterPkResponse(r)) => format!("RegisterPkResponse {:?}", r.result.enum_value()),
            Some(RegisterPeerResponse(r)) => format!("RegisterPeerResponse request_pk={}", r.request_pk),
            Some(other) => format!("{:?}", other),
        };
        self.lines.push(format!("{}: {}", what, line));
    }
}

const RELAY: &str = "relay.example:21117";

async fn old_style_transcript(
    hbbs: &Hbbs,
    version: &str,
    extra: fn(&mut PunchHoleRequest),
) -> Vec<String> {
    let mut o = Observer {
        hbbs,
        roles: vec![],
        lines: vec![],
    };
    let peer = Peer::register(hbbs, "peer01").await;
    peer.send(&msg(|m| {
        m.set_register_peer(RegisterPeer {
            id: "peer01".to_owned(),
            ..Default::default()
        })
    }))
    .await;
    o.note("register peer", peer.recv(2000).await.and_then(|m| m.union));
    let req = |id: &str| {
        let mut r = punch_hole_request(id, &hbbs.key);
        r.version = version.to_owned();
        extra(&mut r);
        r
    };

    // unknown id
    let mut ctl = Tcp::connect(hbbs, CONTROLLER_IP, Kx::None).await;
    ctl.send(&msg(|m| m.set_punch_hole_request(req("nobody1"))))
        .await;
    o.note("unknown id", ctl.recv(3000).await.and_then(|m| m.union));

    // TCP punch, the controlled side answers on a new plaintext connection
    let mut ctl = Tcp::connect(hbbs, CONTROLLER_IP, Kx::None).await;
    o.roles.push((ctl.local_addr(), "controller"));
    ctl.send(&msg(|m| m.set_punch_hole_request(req("peer01"))))
        .await;
    let ph = peer.expect(3000, pick_punch_hole).await;
    o.note(
        "punch hole",
        ph.clone().map(rendezvous_message::Union::PunchHole),
    );
    if let Some(ph) = ph {
        let mut reply = Tcp::connect(hbbs, PEER_IP, Kx::None).await;
        o.roles.push((reply.local_addr(), "peer tcp"));
        reply
            .send(&msg(|m| {
                m.set_punch_hole_sent(PunchHoleSent {
                    socket_addr: ph.socket_addr.clone(),
                    id: "peer01".to_owned(),
                    relay_server: RELAY.to_owned(),
                    nat_type: NatType::ASYMMETRIC.into(),
                    version: version.to_owned(),
                    ..Default::default()
                })
            }))
            .await;
        drop(reply);
        o.note(
            "punch hole response",
            ctl.recv(3000).await.and_then(|m| m.union),
        );
    }

    // same intranet: LocalAddr
    let mut ctl = Tcp::connect(hbbs, PEER_IP, Kx::None).await;
    o.roles.push((ctl.local_addr(), "controller lan"));
    ctl.send(&msg(|m| m.set_punch_hole_request(req("peer01"))))
        .await;
    let fla = peer.expect(3000, pick_fetch_local_addr).await;
    o.note(
        "fetch local addr",
        fla.clone().map(rendezvous_message::Union::FetchLocalAddr),
    );
    if let Some(fla) = fla {
        let mut reply = Tcp::connect(hbbs, PEER_IP, Kx::None).await;
        let local = reply.local_addr();
        o.roles.push((local, "peer local"));
        reply
            .send(&msg(|m| {
                m.set_local_addr(LocalAddr {
                    id: "peer01".to_owned(),
                    socket_addr: mangle(demangle(&fla.socket_addr)),
                    local_addr: mangle(local),
                    relay_server: RELAY.to_owned(),
                    version: version.to_owned(),
                    ..Default::default()
                })
            }))
            .await;
        drop(reply);
        o.note(
            "local addr response",
            ctl.recv(3000).await.and_then(|m| m.union),
        );
    }

    // the controlled side relays instead (create_relay with initiate)
    let mut ctl = Tcp::connect(hbbs, CONTROLLER_IP, Kx::None).await;
    o.roles.push((ctl.local_addr(), "controller 2"));
    ctl.send(&msg(|m| m.set_punch_hole_request(req("peer01"))))
        .await;
    if let Some(ph) = peer.expect(3000, pick_punch_hole).await {
        let mut reply = Tcp::connect(hbbs, PEER_IP, Kx::None).await;
        let mut rr = RelayResponse {
            socket_addr: ph.socket_addr,
            uuid: "uuid-1".to_owned(),
            relay_server: RELAY.to_owned(),
            version: version.to_owned(),
            ..Default::default()
        };
        rr.set_id("peer01".to_owned());
        reply.send(&msg(|m| m.set_relay_response(rr))).await;
        drop(reply);
        o.note("relay response", ctl.recv(3000).await.and_then(|m| m.union));
    }

    // the controller asks for a relay (request_relay)
    let mut ctl = Tcp::connect(hbbs, CONTROLLER_IP, Kx::None).await;
    o.roles.push((ctl.local_addr(), "controller 3"));
    ctl.send(&msg(|m| {
        m.set_request_relay(RequestRelay {
            id: "peer01".to_owned(),
            uuid: "uuid-2".to_owned(),
            relay_server: RELAY.to_owned(),
            secure: true,
            ..Default::default()
        })
    }))
    .await;
    o.note(
        "request relay",
        peer.expect(3000, |u| match u {
            rendezvous_message::Union::RequestRelay(r) => {
                Some(rendezvous_message::Union::RequestRelay(r))
            }
            _ => None,
        })
        .await,
    );

    // NAT type test over TCP, as test_nat_type_ sends it
    let mut tcp = Tcp::connect(hbbs, CONTROLLER_IP, Kx::None).await;
    tcp.send(&msg(|m| {
        m.set_test_nat_request(TestNatRequest {
            serial: 0,
            ..Default::default()
        })
    }))
    .await;
    o.note("tcp nat test", tcp.recv(3000).await.and_then(|m| m.union));

    // a client that reads before it sends sees at most one KeyExchange first,
    // which every client skips (get_next_nonkeyexchange_msg)
    let mut slow = Tcp::connect(hbbs, CONTROLLER_IP, Kx::None).await;
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;
    slow.send(&msg(|m| m.set_punch_hole_request(req("nobody1"))))
        .await;
    o.note("slow client", slow.recv(3000).await.and_then(|m| m.union));
    o.lines
}

async fn compare(
    baseline: &str,
    modern_envs: &[(&str, &str)],
    version: &str,
    extra: fn(&mut PunchHoleRequest),
) {
    let a = Hbbs::start_bin(baseline, &[], &["-r", RELAY]).await;
    let old = old_style_transcript(&a, version, extra).await;
    drop(a);
    let b = Hbbs::start(modern_envs, &["-r", RELAY]).await;
    let new = old_style_transcript(&b, version, extra).await;
    for (x, y) in old.iter().zip(new.iter()) {
        println!("original: {}\nmodern:   {}", x, y);
    }
    assert_eq!(
        old, new,
        "modern {:?} differs from the original",
        modern_envs
    );
}

const ALL_OFF: [(&str, &str); 4] = [
    ("SECURE_TCP", "N"),
    ("IPV6_PUNCH", "N"),
    ("WEBRTC", "N"),
    ("UDP_PUNCH", "N"),
];

#[tokio::test]
#[ignore = "needs HBBS_BASELINE_BIN, see the top of this file"]
async fn old_style_requests_match_the_original() {
    let baseline = baseline_bin();
    for version in ["1.4.0", "1.4.9", "1.5.0"] {
        compare(&baseline, &[], version, |_| {}).await;
        compare(&baseline, &ALL_OFF, version, |_| {}).await;
    }
}

// 1.4.9 sends udp_port and socket_addr_v6 with UDP/IPv6 on; with every switch
// off they are dropped like the original does.
#[tokio::test]
#[ignore = "needs HBBS_BASELINE_BIN, see the top of this file"]
async fn new_fields_with_switches_off_match_the_original() {
    let baseline = baseline_bin();
    fn new_fields(r: &mut PunchHoleRequest) {
        r.udp_port = 40000;
        r.socket_addr_v6 = mangle("[2001:db8::a]:40001".parse().unwrap());
        r.webrtc_sdp_offer = "webrtc://e30=".to_owned();
        r.force_relay = true;
    }
    for version in ["1.4.9", "1.5.0"] {
        compare(&baseline, &ALL_OFF, version, new_fields).await;
    }
}

// 1.4.x controllers send udp_port and socket_addr_v6 when UDP/IPv6 is on; with
// the default switches they still get exactly what the original server did.
#[tokio::test]
#[ignore = "needs HBBS_BASELINE_BIN, see the top of this file"]
async fn new_fields_from_1_4_controllers_match_the_original() {
    let baseline = baseline_bin();
    fn udp_and_v6(r: &mut PunchHoleRequest) {
        r.udp_port = 40000;
        r.socket_addr_v6 = mangle("[2001:db8::a]:40001".parse().unwrap());
    }
    for version in ["1.4.0", "1.4.1", "1.4.9"] {
        compare(&baseline, &[], version, udp_and_v6).await;
    }
}
