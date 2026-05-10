// 简化的性能基准测试
// 测试当前系统的基本性能指标

use criterion::{black_box, criterion_group, criterion_main, Criterion};
use std::time::Instant;

fn benchmark_basic_operations(c: &mut Criterion) {
    // 测试字符串操作性能
    c.bench_function("string_operations", |b| {
        b.iter(|| {
            let mut s = String::new();
            for i in 0..1000 {
                s.push_str(&format!("test_{}", i));
            }
            black_box(s)
        })
    });

    // 测试向量操作性能
    c.bench_function("vector_operations", |b| {
        b.iter(|| {
            let mut v = Vec::new();
            for i in 0..1000 {
                v.push(i);
            }
            black_box(v)
        })
    });

    // 测试哈希映射操作性能
    c.bench_function("hashmap_operations", |b| {
        b.iter(|| {
            let mut map = std::collections::HashMap::new();
            for i in 0..1000 {
                map.insert(i, format!("value_{}", i));
            }
            black_box(map)
        })
    });

    // 测试网络地址解析性能
    c.bench_function("socket_addr_parsing", |b| {
        let addr_str = "192.168.1.100:59000";
        b.iter(|| {
            black_box(addr_str.parse::<std::net::SocketAddr>())
        })
    });

    // 测试JSON序列化性能
    c.bench_function("json_serialization", |b| {
        use serde_json;
        let data = serde_json::json!({
            "id": "test_peer",
            "serial": 12345,
            "nat_type": "symmetric",
            "socket_addr": "192.168.1.100:59000"
        });
        b.iter(|| {
            black_box(serde_json::to_string(&data).unwrap())
        })
    });

    // 测试JSON反序列化性能
    c.bench_function("json_deserialization", |b| {
        use serde_json;
        let json_str = r#"{"id":"test_peer","serial":12345,"nat_type":"symmetric","socket_addr":"192.168.1.100:59000"}"#;
        b.iter(|| {
            black_box(serde_json::from_str::<serde_json::Value>(json_str).unwrap())
        })
    });
}

fn benchmark_memory_allocation(c: &mut Criterion) {
    // 测试内存分配模式
    c.bench_function("memory_allocation_small", |b| {
        b.iter(|| {
            let mut allocations = Vec::new();
            for i in 0..100 {
                allocations.push(Box::new(i));
            }
            black_box(allocations)
        })
    });

    c.bench_function("memory_allocation_large", |b| {
        b.iter(|| {
            let mut allocations = Vec::new();
            for i in 0..10000 {
                allocations.push(Box::new([i; 1024]));
            }
            black_box(allocations)
        })
    });
}

fn benchmark_network_operations(c: &mut Criterion) {
    // 测试网络相关操作
    c.bench_function("ip_address_creation", |b| {
        b.iter(|| {
            black_box(std::net::IpAddr::V4(std::net::Ipv4Addr::new(192, 168, 1, 100)))
        })
    });

    c.bench_function("tcp_listener_creation", |b| {
        b.iter(|| {
            black_box(std::net::TcpListener::bind("127.0.0.1:0"))
        })
    });
}

fn benchmark_concurrent_operations(c: &mut Criterion) {
    // 测试并发操作
    c.bench_function("concurrent_counter", |b| {
        use std::sync::atomic::{AtomicUsize, Ordering};
        let counter = AtomicUsize::new(0);
        b.iter(|| {
            for _ in 0..1000 {
                counter.fetch_add(1, Ordering::SeqCst);
            }
            black_box(counter.load(Ordering::SeqCst))
        })
    });

    c.bench_function("mutex_operations", |b| {
        use std::sync::Mutex;
        let mutex = Mutex::new(0);
        b.iter(|| {
            for _ in 0..100 {
                let mut guard = mutex.lock().unwrap();
                *guard += 1;
            }
            black_box(*mutex.lock().unwrap())
        })
    });
}

criterion_group!(
    benches,
    benchmark_basic_operations,
    benchmark_memory_allocation,
    benchmark_network_operations,
    benchmark_concurrent_operations
);

criterion_main!(benches);
