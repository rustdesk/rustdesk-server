// src/codec.rs
// 双协议编解码层：Proto3 ↔ Cap'n Proto

use core_common::bytes::Bytes;
use core_common::{
    protobuf::Message as _,
    rendezvous_proto::{self, rendezvous_message, *},
};

use crate::rendezvous_capnp;

/// 协议类型
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Protocol {
    /// 原始 proto3（protobuf crate）
    Proto3,
    /// Cap'n Proto
    Capnp,
}

/// 通过第一字节判断协议。
/// capnp framed 消息首 4 字节为 segment_count(u32le)，最小值 0 即 0x00。
/// proto3 字段 tag = (field_num << 3 | wire_type)，field_num ≥ 1 → 首字节 ≥ 8，不可能为 0x00。
pub fn detect(bytes: &[u8]) -> Protocol {
    if bytes.first() == Some(&0x00) {
        Protocol::Capnp
    } else {
        Protocol::Proto3
    }
}

/// 解析字节 → RendezvousMessage（自动判断协议）
pub fn parse(bytes: &[u8]) -> Option<RendezvousMessage> {
    match detect(bytes) {
        Protocol::Proto3 => RendezvousMessage::parse_from_bytes(bytes).ok(),
        Protocol::Capnp => from_capnp(bytes),
    }
}

/// 序列化 RendezvousMessage → 字节（指定目标协议）
pub fn serialize(msg: &RendezvousMessage, proto: Protocol) -> Option<Bytes> {
    match proto {
        Protocol::Proto3 => msg.write_to_bytes().ok().map(Bytes::from),
        Protocol::Capnp => to_capnp(msg).map(Bytes::from),
    }
}

// ── capnp → proto3 ──────────────────────────────────────────────────────────

fn from_capnp(bytes: &[u8]) -> Option<RendezvousMessage> {
    let mut slice = bytes;
    let reader =
        capnp::serialize::read_message(&mut slice, capnp::message::ReaderOptions::new()).ok()?;
    let msg = reader
        .get_root::<rendezvous_capnp::rendezvous_message::Reader>()
        .ok()?;
    capnp_to_proto3(msg)
}

fn capnp_to_proto3(msg: rendezvous_capnp::rendezvous_message::Reader) -> Option<RendezvousMessage> {
    // Alias the generated Which enum to avoid name conflicts with rendezvous_proto structs
    use rendezvous_capnp::rendezvous_message::Which as W;
    let mut out = RendezvousMessage::new();

    match msg.which().ok()? {
        W::RegisterPeer(rp) => {
            let rp = rp.ok()?;
            out.set_register_peer(RegisterPeer {
                id: rp.get_id().ok()?.to_str().ok()?.to_string(),
                serial: rp.get_serial(),
                ..Default::default()
            });
        }
        W::RegisterPeerResponse(rpr) => {
            let rpr = rpr.ok()?;
            out.set_register_peer_response(RegisterPeerResponse {
                request_pk: rpr.get_request_pk(),
                ..Default::default()
            });
        }
        W::PunchHoleRequest(phr) => {
            let phr = phr.ok()?;
            out.set_punch_hole_request(PunchHoleRequest {
                id: phr.get_id().ok()?.to_str().ok()?.to_string(),
                nat_type: capnp_nat_type(phr.get_nat_type().ok()?).into(),
                licence_key: phr.get_licence_key().ok()?.to_str().ok()?.to_string(),
                conn_type: capnp_conn_type(phr.get_conn_type().ok()?).into(),
                token: phr.get_token().ok()?.to_str().ok()?.to_string(),
                version: phr.get_version().ok()?.to_str().ok()?.to_string(),
                udp_port: phr.get_udp_port(),
                force_relay: phr.get_force_relay(),
                upnp_port: phr.get_upnp_port(),
                socket_addr_v6: phr.get_socket_addr_v6().ok()?.to_vec().into(),
                ..Default::default()
            });
        }
        W::RegisterPk(rk) => {
            let rk = rk.ok()?;
            out.set_register_pk(RegisterPk {
                id: rk.get_id().ok()?.to_str().ok()?.to_string(),
                uuid: rk.get_uuid().ok()?.to_vec().into(),
                pk: rk.get_pk().ok()?.to_vec().into(),
                old_id: rk.get_old_id().ok()?.to_str().ok()?.to_string(),
                no_register_device: rk.get_no_register_device(),
                user_token: rk.get_user_token().ok()?.to_str().ok()?.to_string(),
                ..Default::default()
            });
        }
        W::TestNatRequest(tnr) => {
            let tnr = tnr.ok()?;
            out.set_test_nat_request(TestNatRequest {
                serial: tnr.get_serial(),
                ..Default::default()
            });
        }
        W::FetchLocalAddr(fla) => {
            let fla = fla.ok()?;
            out.set_fetch_local_addr(FetchLocalAddr {
                socket_addr: fla.get_socket_addr().ok()?.to_vec().into(),
                relay_server: fla.get_relay_server().ok()?.to_str().ok()?.to_string(),
                socket_addr_v6: fla.get_socket_addr_v6().ok()?.to_vec().into(),
                ..Default::default()
            });
        }
        W::OnlineRequest(or_) => {
            let or_ = or_.ok()?;
            let peers: Vec<String> = or_
                .get_peers()
                .ok()?
                .iter()
                .filter_map(|p| p.ok()?.to_str().ok().map(|s| s.to_string()))
                .collect();
            out.set_online_request(OnlineRequest {
                id: or_.get_id().ok()?.to_str().ok()?.to_string(),
                peers,
                ..Default::default()
            });
        }
        W::RequestRelay(rr) => {
            let rr = rr.ok()?;
            out.set_request_relay(RequestRelay {
                id: rr.get_id().ok()?.to_str().ok()?.to_string(),
                uuid: rr.get_uuid().ok()?.to_str().ok()?.to_string(),
                socket_addr: rr.get_socket_addr().ok()?.to_vec().into(),
                relay_server: rr.get_relay_server().ok()?.to_str().ok()?.to_string(),
                secure: rr.get_secure(),
                licence_key: rr.get_licence_key().ok()?.to_str().ok()?.to_string(),
                conn_type: capnp_conn_type(rr.get_conn_type().ok()?).into(),
                token: rr.get_token().ok()?.to_str().ok()?.to_string(),
                ..Default::default()
            });
        }
        W::PunchHoleSent(phs) => {
            let phs = phs.ok()?;
            out.set_punch_hole_sent(PunchHoleSent {
                socket_addr: phs.get_socket_addr().ok()?.to_vec().into(),
                id: phs.get_id().ok()?.to_str().ok()?.to_string(),
                relay_server: phs.get_relay_server().ok()?.to_str().ok()?.to_string(),
                nat_type: capnp_nat_type(phs.get_nat_type().ok()?).into(),
                version: phs.get_version().ok()?.to_str().ok()?.to_string(),
                upnp_port: phs.get_upnp_port(),
                socket_addr_v6: phs.get_socket_addr_v6().ok()?.to_vec().into(),
                ..Default::default()
            });
        }
        W::KeyExchange(ke) => {
            let ke = ke.ok()?;
            let keys: Vec<Bytes> = ke
                .get_keys()
                .ok()?
                .iter()
                .filter_map(|k| k.ok().map(|b| Bytes::copy_from_slice(b)))
                .collect();
            out.set_key_exchange(KeyExchange {
                keys,
                ..Default::default()
            });
        }
        W::HealthCheck(hc) => {
            let hc = hc.ok()?;
            out.set_hc(HealthCheck {
                token: hc.get_token().ok()?.to_str().ok()?.to_string(),
                ..Default::default()
            });
        }
        // ── server→client 方向消息（双向 capnp 测试 / relay 场景需要）──
        W::RegisterPkResponse(rpr) => {
            let rpr = rpr.ok()?;
            use rendezvous_proto::register_pk_response::Result as R;
            let result = match rpr.get_result().ok()? {
                rendezvous_capnp::RegisterResult::Ok => R::OK,
                rendezvous_capnp::RegisterResult::UuidMismatch => R::UUID_MISMATCH,
                rendezvous_capnp::RegisterResult::IdExists => R::ID_EXISTS,
                rendezvous_capnp::RegisterResult::TooFrequent => R::TOO_FREQUENT,
                rendezvous_capnp::RegisterResult::InvalidIdFormat => R::INVALID_ID_FORMAT,
                rendezvous_capnp::RegisterResult::NotSupport => R::NOT_SUPPORT,
                rendezvous_capnp::RegisterResult::ServerError => R::SERVER_ERROR,
            };
            out.set_register_pk_response(RegisterPkResponse {
                result: result.into(),
                keep_alive: rpr.get_keep_alive(),
                ..Default::default()
            });
        }
        W::PunchHole(ph) => {
            let ph = ph.ok()?;
            out.set_punch_hole(PunchHole {
                socket_addr: ph.get_socket_addr().ok()?.to_vec().into(),
                relay_server: ph.get_relay_server().ok()?.to_str().ok()?.to_string(),
                nat_type: capnp_nat_type(ph.get_nat_type().ok()?).into(),
                udp_port: ph.get_udp_port(),
                force_relay: ph.get_force_relay(),
                upnp_port: ph.get_upnp_port(),
                socket_addr_v6: ph.get_socket_addr_v6().ok()?.to_vec().into(),
                ..Default::default()
            });
        }
        W::PunchHoleResponse(phr) => {
            let phr = phr.ok()?;
            use rendezvous_proto::punch_hole_response::Failure as F;
            let failure = match phr.get_failure().ok()? {
                rendezvous_capnp::PunchHoleFailure::IdNotExist => F::ID_NOT_EXIST,
                rendezvous_capnp::PunchHoleFailure::Offline => F::OFFLINE,
                rendezvous_capnp::PunchHoleFailure::LicenseMismatch => F::LICENSE_MISMATCH,
                rendezvous_capnp::PunchHoleFailure::LicenseOveruse => F::LICENSE_OVERUSE,
            };
            let mut p = PunchHoleResponse {
                socket_addr: phr.get_socket_addr().ok()?.to_vec().into(),
                pk: phr.get_pk().ok()?.to_vec().into(),
                failure: failure.into(),
                relay_server: phr.get_relay_server().ok()?.to_str().ok()?.to_string(),
                other_failure: phr.get_other_failure().ok()?.to_str().ok()?.to_string(),
                feedback: phr.get_feedback(),
                is_udp: phr.get_is_udp(),
                upnp_port: phr.get_upnp_port(),
                socket_addr_v6: phr.get_socket_addr_v6().ok()?.to_vec().into(),
                ..Default::default()
            };
            if phr.get_is_local() {
                p.set_is_local(true);
            } else {
                p.set_nat_type(capnp_nat_type(phr.get_nat_type().ok()?).into());
            }
            out.set_punch_hole_response(p);
        }
        W::ConfigureUpdate(cu) => {
            let cu = cu.ok()?;
            let servers: Vec<String> = cu
                .get_rendezvous_servers()
                .ok()?
                .iter()
                .filter_map(|s| s.ok()?.to_str().ok().map(|s| s.to_string()))
                .collect();
            out.set_configure_update(ConfigUpdate {
                serial: cu.get_serial(),
                rendezvous_servers: servers,
                ..Default::default()
            });
        }
        W::OnlineResponse(or_) => {
            let or_ = or_.ok()?;
            out.set_online_response(OnlineResponse {
                states: or_.get_states().ok()?.to_vec().into(),
                ..Default::default()
            });
        }
        W::RelayResponse(rr) => {
            let rr = rr.ok()?;
            let id_bytes = rr.get_id().ok()?.to_str().ok()?.to_string();
            let pk_bytes = rr.get_pk().ok()?.to_vec();
            let mut r = RelayResponse {
                socket_addr: rr.get_socket_addr().ok()?.to_vec().into(),
                uuid: rr.get_uuid().ok()?.to_str().ok()?.to_string(),
                relay_server: rr.get_relay_server().ok()?.to_str().ok()?.to_string(),
                refuse_reason: rr.get_refuse_reason().ok()?.to_str().ok()?.to_string(),
                version: rr.get_version().ok()?.to_str().ok()?.to_string(),
                feedback: rr.get_feedback(),
                socket_addr_v6: rr.get_socket_addr_v6().ok()?.to_vec().into(),
                upnp_port: rr.get_upnp_port(),
                ..Default::default()
            };
            if !pk_bytes.is_empty() {
                r.set_pk(pk_bytes.into());
            } else {
                r.set_id(id_bytes);
            }
            out.set_relay_response(r);
        }
        W::LocalAddr(la) => {
            let la = la.ok()?;
            out.set_local_addr(LocalAddr {
                socket_addr: la.get_socket_addr().ok()?.to_vec().into(),
                local_addr: la.get_local_address().ok()?.to_vec().into(),
                relay_server: la.get_relay_server().ok()?.to_str().ok()?.to_string(),
                id: la.get_id().ok()?.to_str().ok()?.to_string(),
                version: la.get_version().ok()?.to_str().ok()?.to_string(),
                socket_addr_v6: la.get_socket_addr_v6().ok()?.to_vec().into(),
                ..Default::default()
            });
        }
        W::SoftwareUpdate(su) => {
            let su = su.ok()?;
            out.set_software_update(SoftwareUpdate {
                url: su.get_url().ok()?.to_str().ok()?.to_string(),
                ..Default::default()
            });
        }
        W::TestNatResponse(tnr) => {
            let tnr = tnr.ok()?;
            let servers: Vec<String> = tnr
                .get_rendezvous_servers()
                .ok()?
                .iter()
                .filter_map(|s| s.ok()?.to_str().ok().map(|s| s.to_string()))
                .collect();
            out.set_test_nat_response(TestNatResponse {
                port: tnr.get_port(),
                cu: Some(ConfigUpdate {
                    serial: tnr.get_config_serial(),
                    rendezvous_servers: servers,
                    ..Default::default()
                })
                .into(),
                ..Default::default()
            });
        }
        _ => {
            core_common::log::debug!("codec: unknown capnp variant, dropping");
            return None;
        }
    }
    Some(out)
}

// ── proto3 → capnp ──────────────────────────────────────────────────────────

fn to_capnp(msg: &RendezvousMessage) -> Option<Vec<u8>> {
    let mut builder = capnp::message::Builder::new_default();
    let root = builder.init_root::<rendezvous_capnp::rendezvous_message::Builder>();

    match msg.union.as_ref()? {
        rendezvous_message::Union::RegisterPeerResponse(rpr) => {
            let mut r = root.init_register_peer_response();
            r.set_request_pk(rpr.request_pk);
        }
        rendezvous_message::Union::RegisterPkResponse(rpr) => {
            let mut r = root.init_register_pk_response();
            r.set_result(proto3_register_result(rpr.result.enum_value_or_default()));
            r.set_keep_alive(rpr.keep_alive);
        }
        rendezvous_message::Union::PunchHoleResponse(phr) => {
            let mut r = root.init_punch_hole_response();
            r.set_socket_addr(&phr.socket_addr);
            r.set_pk(&phr.pk);
            r.set_failure(proto3_punch_failure(phr.failure.enum_value_or_default()));
            r.set_relay_server(&phr.relay_server);
            r.set_is_local(phr.is_local());
            r.set_nat_type(proto3_nat_type(phr.nat_type()));
            r.set_other_failure(&phr.other_failure);
            r.set_feedback(phr.feedback);
            r.set_is_udp(phr.is_udp);
            r.set_upnp_port(phr.upnp_port);
            r.set_socket_addr_v6(&phr.socket_addr_v6);
        }
        rendezvous_message::Union::PunchHole(ph) => {
            let mut r = root.init_punch_hole();
            r.set_socket_addr(&ph.socket_addr);
            r.set_relay_server(&ph.relay_server);
            r.set_nat_type(proto3_nat_type(ph.nat_type.enum_value_or_default()));
            r.set_udp_port(ph.udp_port);
            r.set_force_relay(ph.force_relay);
            r.set_upnp_port(ph.upnp_port);
            r.set_socket_addr_v6(&ph.socket_addr_v6);
        }
        rendezvous_message::Union::ConfigureUpdate(cu) => {
            let mut r = root.init_configure_update();
            r.set_serial(cu.serial);
            let mut list = r.init_rendezvous_servers(cu.rendezvous_servers.len() as u32);
            for (i, s) in cu.rendezvous_servers.iter().enumerate() {
                list.set(i as u32, s.as_str());
            }
        }
        rendezvous_message::Union::TestNatResponse(tnr) => {
            let mut r = root.init_test_nat_response();
            r.set_port(tnr.port);
            if let Some(cu) = tnr.cu.as_ref() {
                r.set_config_serial(cu.serial);
                let mut list = r.init_rendezvous_servers(cu.rendezvous_servers.len() as u32);
                for (i, s) in cu.rendezvous_servers.iter().enumerate() {
                    list.set(i as u32, s.as_str());
                }
            }
        }
        rendezvous_message::Union::OnlineResponse(or_) => {
            let mut r = root.init_online_response();
            r.set_states(&or_.states);
        }
        rendezvous_message::Union::RelayResponse(rr) => {
            let mut r = root.init_relay_response();
            r.set_socket_addr(&rr.socket_addr);
            r.set_uuid(&rr.uuid);
            r.set_relay_server(&rr.relay_server);
            if let Some(rendezvous_proto::relay_response::Union::Id(id)) = &rr.union {
                r.set_id(id.as_str());
            } else if let Some(rendezvous_proto::relay_response::Union::Pk(pk)) = &rr.union {
                r.set_pk(pk);
            }
            r.set_refuse_reason(&rr.refuse_reason);
            r.set_version(&rr.version);
            r.set_feedback(rr.feedback);
            r.set_socket_addr_v6(&rr.socket_addr_v6);
            r.set_upnp_port(rr.upnp_port);
        }
        rendezvous_message::Union::LocalAddr(la) => {
            let mut r = root.init_local_addr();
            r.set_socket_addr(&la.socket_addr);
            r.set_local_address(&la.local_addr);
            r.set_relay_server(&la.relay_server);
            r.set_id(&la.id);
            r.set_version(&la.version);
            r.set_socket_addr_v6(&la.socket_addr_v6);
        }
        rendezvous_message::Union::SoftwareUpdate(su) => {
            let mut r = root.init_software_update();
            r.set_url(&su.url);
        }
        _ => {
            core_common::log::debug!("codec: unsupported proto3→capnp variant");
            return None;
        }
    }

    let mut buf = Vec::new();
    capnp::serialize::write_message(&mut buf, &builder).ok()?;
    Some(buf)
}

// ── Enum 转换辅助 ────────────────────────────────────────────────────────────

fn capnp_nat_type(n: rendezvous_capnp::NatType) -> rendezvous_proto::NatType {
    match n {
        rendezvous_capnp::NatType::Asymmetric => rendezvous_proto::NatType::ASYMMETRIC,
        rendezvous_capnp::NatType::Symmetric => rendezvous_proto::NatType::SYMMETRIC,
        _ => rendezvous_proto::NatType::UNKNOWN_NAT,
    }
}

fn proto3_nat_type(n: rendezvous_proto::NatType) -> rendezvous_capnp::NatType {
    match n {
        rendezvous_proto::NatType::ASYMMETRIC => rendezvous_capnp::NatType::Asymmetric,
        rendezvous_proto::NatType::SYMMETRIC => rendezvous_capnp::NatType::Symmetric,
        _ => rendezvous_capnp::NatType::UnknownNat,
    }
}

fn capnp_conn_type(c: rendezvous_capnp::ConnType) -> rendezvous_proto::ConnType {
    match c {
        rendezvous_capnp::ConnType::FileTransfer => rendezvous_proto::ConnType::FILE_TRANSFER,
        rendezvous_capnp::ConnType::PortForward => rendezvous_proto::ConnType::PORT_FORWARD,
        rendezvous_capnp::ConnType::Rdp => rendezvous_proto::ConnType::RDP,
        rendezvous_capnp::ConnType::ViewCamera => rendezvous_proto::ConnType::VIEW_CAMERA,
        rendezvous_capnp::ConnType::Terminal => rendezvous_proto::ConnType::TERMINAL,
        _ => rendezvous_proto::ConnType::DEFAULT_CONN,
    }
}

fn proto3_register_result(
    r: rendezvous_proto::register_pk_response::Result,
) -> rendezvous_capnp::RegisterResult {
    use rendezvous_proto::register_pk_response::Result as R;
    match r {
        R::OK => rendezvous_capnp::RegisterResult::Ok,
        R::UUID_MISMATCH => rendezvous_capnp::RegisterResult::UuidMismatch,
        R::ID_EXISTS => rendezvous_capnp::RegisterResult::IdExists,
        R::TOO_FREQUENT => rendezvous_capnp::RegisterResult::TooFrequent,
        R::INVALID_ID_FORMAT => rendezvous_capnp::RegisterResult::InvalidIdFormat,
        R::NOT_SUPPORT => rendezvous_capnp::RegisterResult::NotSupport,
        R::SERVER_ERROR => rendezvous_capnp::RegisterResult::ServerError,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use core_common::rendezvous_proto::*;

    /// proto3 字节第一字节永远不为 0x00
    #[test]
    fn detect_proto3() {
        let mut msg = RendezvousMessage::new();
        msg.set_register_peer(RegisterPeer {
            id: "abc".into(),
            serial: 1,
            ..Default::default()
        });
        let bytes = core_common::protobuf::Message::write_to_bytes(&msg).unwrap();
        assert_ne!(bytes[0], 0x00);
        assert_eq!(detect(&bytes), Protocol::Proto3);
    }

    /// capnp framed 字节第一字节固定为 0x00
    #[test]
    fn detect_capnp() {
        use capnp::message;
        let mut builder = message::Builder::new_default();
        {
            let mut root =
                builder.init_root::<crate::rendezvous_capnp::rendezvous_message::Builder>();
            let mut rpr = root.init_register_peer_response();
            rpr.set_request_pk(true);
        }
        let mut buf = Vec::new();
        capnp::serialize::write_message(&mut buf, &builder).unwrap();
        assert_eq!(buf[0], 0x00);
        assert_eq!(detect(&buf), Protocol::Capnp);
    }

    /// capnp RegisterPk 能正确转换为 proto3
    #[test]
    fn round_trip_register_pk() {
        use capnp::message;
        // 构造 capnp RegisterPk
        let mut builder = message::Builder::new_default();
        {
            let mut root =
                builder.init_root::<crate::rendezvous_capnp::rendezvous_message::Builder>();
            let mut rk = root.init_register_pk();
            rk.set_id("peer-001");
            rk.set_uuid(b"uuid-bytes");
            rk.set_pk(b"pk-bytes");
            rk.set_user_token("jwt-token");
        }
        let mut capnp_bytes = Vec::new();
        capnp::serialize::write_message(&mut capnp_bytes, &builder).unwrap();

        // 解析
        let msg = parse(&capnp_bytes).expect("parse failed");
        match msg.union {
            Some(rendezvous_message::Union::RegisterPk(rk)) => {
                assert_eq!(rk.id, "peer-001");
                assert_eq!(rk.user_token, "jwt-token");
            }
            other => panic!("unexpected variant: {:?}", other),
        }
    }

    /// proto3 RegisterPkResponse 能正确序列化为 capnp
    #[test]
    fn serialize_register_pk_response_capnp() {
        use core_common::rendezvous_proto::register_pk_response;
        let mut msg = RendezvousMessage::new();
        msg.set_register_pk_response(RegisterPkResponse {
            result: register_pk_response::Result::OK.into(),
            keep_alive: 30,
            ..Default::default()
        });
        let bytes = serialize(&msg, Protocol::Capnp).expect("serialize failed");
        assert_eq!(bytes[0], 0x00, "capnp output must start with 0x00");
        // 反解析验证
        let back = from_capnp(&bytes).expect("round-trip failed");
        match back.union {
            Some(rendezvous_message::Union::RegisterPkResponse(rpr)) => {
                assert_eq!(rpr.keep_alive, 30);
            }
            other => panic!("unexpected: {:?}", other),
        }
    }
}

fn proto3_punch_failure(
    f: rendezvous_proto::punch_hole_response::Failure,
) -> rendezvous_capnp::PunchHoleFailure {
    use rendezvous_proto::punch_hole_response::Failure as F;
    match f {
        F::ID_NOT_EXIST => rendezvous_capnp::PunchHoleFailure::IdNotExist,
        F::OFFLINE => rendezvous_capnp::PunchHoleFailure::Offline,
        F::LICENSE_MISMATCH => rendezvous_capnp::PunchHoleFailure::LicenseMismatch,
        F::LICENSE_OVERUSE => rendezvous_capnp::PunchHoleFailure::LicenseOveruse,
    }
}
