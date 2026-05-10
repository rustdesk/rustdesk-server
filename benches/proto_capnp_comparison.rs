// Protocol Buffers vs Cap'n Proto 性能对比基准测试
// 测试序列化/反序列化性能和内存使用

use criterion::{black_box, criterion_group, criterion_main, Criterion, BenchmarkId};
use prost::Message;
use bytes::Bytes;

// 导入proto3和capnp模块
use hbb_common::protos::rendezvous_proto;
use hbb_common::protos::rendezvous_capnp;
use crate::capnp_serialization::{CapnpSerializer, CapnpDeserializer};

// 创建测试数据
fn create_test_register_peer() -> rendezvous_proto::RegisterPeer {
    let mut register_peer = rendezvous_proto::RegisterPeer::default();
    register_peer.id = "test_peer_12345".to_string();
    register_peer.serial = 54321;
    register_peer
}

fn create_test_punch_hole_request() -> rendezvous_proto::PunchHoleRequest {
    let mut request = rendezvous_proto::PunchHoleRequest::default();
    request.id = "target_peer_67890".to_string();
    request.nat_type = rendezvous_proto::NatType::Symmetric as i32;
    request.licence_key = "test_license_key".to_string();
    request.conn_type = rendezvous_proto::ConnType::DefaultConn as i32;
    request.token = "test_token_abc123".to_string();
    request.version = "1.2.3".to_string();
    request
}

fn create_test_punch_hole_response() -> rendezvous_proto::PunchHoleResponse {
    let mut response = rendezvous_proto::PunchHoleResponse::default();
    response.socket_addr = b"192.168.1.100:59000".to_vec();
    response.pk = b"test_public_key_32bytes".to_vec();
    response.nat_type = Some(rendezvous_proto::NatType::Symmetric as i32);
    response.relay_server = "relay.test.com".to_string();
    response.is_local = Some(false);
    response
}

// Protocol Buffers 基准测试
fn bench_prost_serialization(c: &mut Criterion) {
    let register_peer = create_test_register_peer();
    let punch_request = create_test_punch_hole_request();
    let punch_response = create_test_punch_hole_response();

    c.bench_function("prost_serialize_register_peer", |b| {
        b.iter(|| {
            black_box(register_peer.encode_to_vec().as_slice())
        })
    });

    c.bench_function("prost_serialize_punch_hole_request", |b| {
        b.iter(|| {
            black_box(punch_request.encode_to_vec().as_slice())
        })
    });

    c.bench_function("prost_serialize_punch_hole_response", |b| {
        b.iter(|| {
            black_box(punch_response.encode_to_vec().as_slice())
        })
    });
}

fn bench_prost_deserialization(c: &mut Criterion) {
    let register_peer_bytes = create_test_register_peer().encode_to_vec();
    let punch_request_bytes = create_test_punch_hole_request().encode_to_vec();
    let punch_response_bytes = create_test_punch_hole_response().encode_to_vec();

    c.bench_function("prost_deserialize_register_peer", |b| {
        b.iter(|| {
            black_box(rendezvous_proto::RegisterPeer::decode(black_box(&register_peer_bytes)).unwrap())
        })
    });

    c.bench_function("prost_deserialize_punch_hole_request", |b| {
        b.iter(|| {
            black_box(rendezvous_proto::PunchHoleRequest::decode(black_box(&punch_request_bytes)).unwrap())
        })
    });

    c.bench_function("prost_deserialize_punch_hole_response", |b| {
        b.iter(|| {
            black_box(rendezvous_proto::PunchHoleResponse::decode(black_box(&punch_response_bytes)).unwrap())
        })
    });
}

// Cap'n Proto 基准测试
fn bench_capnp_serialization(c: &mut Criterion) {
    // 创建Cap'n Proto版本的测试数据
    let mut register_peer = rendezvous_capnp::RendezvousMessage::new_default();
    let mut register_peer_data = rendezvous_capnp::RegisterPeer::new_default();
    register_peer_data.set_id("test_peer_12345");
    register_peer_data.set_serial(54321);
    register_peer.set_register_peer(register_peer_data);

    let mut punch_request = rendezvous_capnp::RendezvousMessage::new_default();
    let mut punch_request_data = rendezvous_capnp::PunchHoleRequest::new_default();
    punch_request_data.set_id("target_peer_67890");
    punch_request_data.set_nat_type(rendezvous_capnp::NatType::SymmetricNat);
    punch_request_data.set_licence_key("test_license_key");
    punch_request_data.set_conn_type(rendezvous_capnp::ConnType::DefaultConn);
    punch_request_data.set_token("test_token_abc123");
    punch_request_data.set_version("1.2.3");
    punch_request.set_punch_hole_request(punch_request_data);

    let mut punch_response = rendezvous_capnp::RendezvousMessage::new_default();
    let mut punch_response_data = rendezvous_capnp::PunchHoleResponse::new_default();
    punch_response_data.set_failure(rendezvous_capnp::Failure::Offline);
    
    // 设置socket地址数据
    let mut socket_addr = rendezvous_capnp::Data::new_default();
    let addr_bytes = b"192.168.1.100:59000";
    socket_addr.init_data(addr_bytes.len() as u32);
    socket_addr.get_data().copy_from_slice(addr_bytes);
    
    // 设置公钥数据
    let mut pk = rendezvous_capnp::Data::new_default();
    let pk_bytes = b"test_public_key_32bytes";
    pk.init_data(pk_bytes.len() as u32);
    pk.get_data().copy_from_slice(pk_bytes);
    
    punch_response_data.set_socket_addr(socket_addr);
    punch_response_data.set_pk(pk);
    punch_response_data.set_relay_server("relay.test.com");
    punch_response_data.set_nat_type(rendezvous_capnp::NatType::SymmetricNat);
    punch_response.set_punch_hole_response(punch_response_data);

    c.bench_function("capnp_serialize_register_peer", |b| {
        b.iter(|| {
            black_box(CapnpSerializer::serialize_message(&register_peer).unwrap())
        })
    });

    c.bench_function("capnp_serialize_punch_hole_request", |b| {
        b.iter(|| {
            black_box(CapnpSerializer::serialize_message(&punch_request).unwrap())
        })
    });

    c.bench_function("capnp_serialize_punch_hole_response", |b| {
        b.iter(|| {
            black_box(CapnpSerializer::serialize_message(&punch_response).unwrap())
        })
    });
}

fn bench_capnp_deserialization(c: &mut Criterion) {
    let register_peer_bytes = CapnpSerializer::serialize_message(&create_capnp_register_peer()).unwrap();
    let punch_request_bytes = CapnpSerializer::serialize_message(&create_capnp_punch_hole_request()).unwrap();
    let punch_response_bytes = CapnpSerializer::serialize_message(&create_capnp_punch_hole_response()).unwrap();

    c.bench_function("capnp_deserialize_register_peer", |b| {
        b.iter(|| {
            black_box(CapnpDeserializer::deserialize_message::<rendezvous_capnp::RendezvousMessage>(black_box(&register_peer_bytes)).unwrap())
        })
    });

    c.bench_function("capnp_deserialize_punch_hole_request", |b| {
        b.iter(|| {
            black_box(CapnpDeserializer::deserialize_message::<rendezvous_capnp::RendezvousMessage>(black_box(&punch_request_bytes)).unwrap())
        })
    });

    c.bench_function("capnp_deserialize_punch_hole_response", |b| {
        b.iter(|| {
            black_box(CapnpDeserializer::deserialize_message::<rendezvous_capnp::RendezvousMessage>(black_box(&punch_response_bytes)).unwrap())
        })
    });
}

// 辅助函数：创建Cap'n Proto测试数据
fn create_capnp_register_peer() -> rendezvous_capnp::RendezvousMessage {
    let mut message = rendezvous_capnp::RendezvousMessage::new_default();
    let mut register_peer = rendezvous_capnp::RegisterPeer::new_default();
    register_peer.set_id("test_peer_12345");
    register_peer.set_serial(54321);
    message.set_register_peer(register_peer);
    message
}

fn create_capnp_punch_hole_request() -> rendezvous_capnp::RendezvousMessage {
    let mut message = rendezvous_capnp::RendezvousMessage::new_default();
    let mut punch_request = rendezvous_capnp::PunchHoleRequest::new_default();
    punch_request.set_id("target_peer_67890");
    punch_request.set_nat_type(rendezvous_capnp::NatType::SymmetricNat);
    punch_request.set_licence_key("test_license_key");
    punch_request.set_conn_type(rendezvous_capnp::ConnType::DefaultConn);
    punch_request.set_token("test_token_abc123");
    punch_request.set_version("1.2.3");
    message.set_punch_hole_request(punch_request);
    message
}

fn create_capnp_punch_hole_response() -> rendezvous_capnp::RendezvousMessage {
    let mut message = rendezvous_capnp::RendezvousMessage::new_default();
    let mut punch_response = rendezvous_capnp::PunchHoleResponse::new_default();
    punch_response.set_failure(rendezvous_capnp::Failure::Offline);
    
    // 设置socket地址数据
    let mut socket_addr = rendezvous_capnp::Data::new_default();
    let addr_bytes = b"192.168.1.100:59000";
    socket_addr.init_data(addr_bytes.len() as u32);
    socket_addr.get_data().copy_from_slice(addr_bytes);
    
    // 设置公钥数据
    let mut pk = rendezvous_capnp::Data::new_default();
    let pk_bytes = b"test_public_key_32bytes";
    pk.init_data(pk_bytes.len() as u32);
    pk.get_data().copy_from_slice(pk_bytes);
    
    punch_response.set_socket_addr(socket_addr);
    punch_response.set_pk(pk);
    punch_response.set_relay_server("relay.test.com");
    punch_response.set_nat_type(rendezvous_capnp::NatType::SymmetricNat);
    message.set_punch_hole_response(punch_response);
    message
}

// 内存使用对比测试
fn bench_memory_usage(c: &mut Criterion) {
    c.bench_function("prost_memory_usage", |b| {
        b.iter(|| {
            // 创建大量proto3消息
            let mut messages = Vec::new();
            for i in 0..1000 {
                let register_peer = create_test_register_peer();
                let bytes = register_peer.encode_to_vec();
                messages.push(bytes);
            }
            black_box(messages.len())
        })
    });

    c.bench_function("capnp_memory_usage", |b| {
        b.iter(|| {
            // 创建大量capnp消息
            let mut messages = Vec::new();
            for i in 0..1000 {
                let register_peer = create_capnp_register_peer();
                let bytes = CapnpSerializer::serialize_message(&register_peer).unwrap();
                messages.push(bytes);
            }
            black_box(messages.len())
        })
    });
}

criterion_group!(
    benches,
    prost_serialization,
    prost_deserialization,
    capnp_serialization,
    capnp_deserialization,
    memory_usage_comparison = bench_memory_usage
);

criterion_main!(benches);
