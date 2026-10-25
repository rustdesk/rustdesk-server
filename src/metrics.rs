//! Prometheus metrics and health check exporter for RustDesk Server (`hbbs` and `hbbr`).
//!
//! Provides zero-lock atomic metrics counters and an embedded Axum HTTP server
//! exposing `/metrics` (OpenMetrics format) and `/health` / `/livez` endpoints.

use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::get,
    Router,
};
use lazy_static::lazy_static;
use std::{
    net::{IpAddr, Ipv4Addr, SocketAddr},
    sync::atomic::{AtomicU64, AtomicUsize, Ordering},
    time::Instant,
};

lazy_static! {
    static ref SERVER_START_TIME: Instant = Instant::now();
}

// ========================================================
// ZERO-LOCK ATOMIC COUNTERS (High Performance / No Lock Contention)
// ========================================================
pub static ACTIVE_PEERS: AtomicUsize = AtomicUsize::new(0);
pub static TOTAL_CONNECTIONS: AtomicU64 = AtomicU64::new(0);
pub static TOTAL_DISCONNECTIONS: AtomicU64 = AtomicU64::new(0);
pub static TOTAL_BYTES_RELAYED: AtomicU64 = AtomicU64::new(0);
pub static TOTAL_HEARTBEATS: AtomicU64 = AtomicU64::new(0);
pub static TOTAL_PACKET_DROPS: AtomicU64 = AtomicU64::new(0);

// ========================================================
// ATOMIC COUNTER HELPERS
// ========================================================

/// Adds relayed payload size in bytes to the total forwarded bytes counter.
#[inline]
pub fn add_relayed_bytes(bytes: usize) {
    TOTAL_BYTES_RELAYED.fetch_add(bytes as u64, Ordering::Relaxed);
}

/// Increments total heartbeat / registration packets received.
#[inline]
pub fn inc_heartbeat() {
    TOTAL_HEARTBEATS.fetch_add(1, Ordering::Relaxed);
}

/// Increments total packet drop counter on send failure.
#[inline]
pub fn inc_packet_drop() {
    TOTAL_PACKET_DROPS.fetch_add(1, Ordering::Relaxed);
}

/// RAII guard ensuring active peer gauge and connection counters are tracked with exact counts.
/// Decrements active peers and increments disconnections upon drop.
pub struct ActivePeerGuard(usize);

impl ActivePeerGuard {
    pub fn new(count: usize) -> Self {
        ACTIVE_PEERS.fetch_add(count, Ordering::Relaxed);
        TOTAL_CONNECTIONS.fetch_add(count as u64, Ordering::Relaxed);
        Self(count)
    }
}

impl Drop for ActivePeerGuard {
    fn drop(&mut self) {
        ACTIVE_PEERS.fetch_sub(self.0, Ordering::Relaxed);
        TOTAL_DISCONNECTIONS.fetch_add(self.0 as u64, Ordering::Relaxed);
    }
}

// ========================================================
// PROMETHEUS / OPENMETRICS EXPOSITION FORMAT
// ========================================================

/// Formats the current in-memory counters into OpenMetrics text format tailored to the active service.
pub fn generate_prometheus_metrics(service_name: &str) -> String {
    let uptime = SERVER_START_TIME.elapsed().as_secs();

    if service_name == "hbbr" {
        let active = ACTIVE_PEERS.load(Ordering::Relaxed);
        let total_conn = TOTAL_CONNECTIONS.load(Ordering::Relaxed);
        let total_disc = TOTAL_DISCONNECTIONS.load(Ordering::Relaxed);
        let total_bytes = TOTAL_BYTES_RELAYED.load(Ordering::Relaxed);
        let total_drops = TOTAL_PACKET_DROPS.load(Ordering::Relaxed);

        format!(
            "# HELP rustdesk_server_uptime_seconds Total server uptime in seconds.\n\
             # TYPE rustdesk_server_uptime_seconds counter\n\
             rustdesk_server_uptime_seconds{{service=\"{service_name}\"}} {uptime}\n\n\
             # HELP rustdesk_active_peers Current active peer sessions in relay.\n\
             # TYPE rustdesk_active_peers gauge\n\
             rustdesk_active_peers{{service=\"{service_name}\"}} {active}\n\n\
             # HELP rustdesk_connections_total Total peer connections accepted for relay.\n\
             # TYPE rustdesk_connections_total counter\n\
             rustdesk_connections_total{{service=\"{service_name}\"}} {total_conn}\n\n\
             # HELP rustdesk_disconnections_total Total peer connections closed from relay.\n\
             # TYPE rustdesk_disconnections_total counter\n\
             rustdesk_disconnections_total{{service=\"{service_name}\"}} {total_disc}\n\n\
             # HELP rustdesk_bytes_relayed_total Total bytes forwarded by relay.\n\
             # TYPE rustdesk_bytes_relayed_total counter\n\
             rustdesk_bytes_relayed_total{{service=\"{service_name}\"}} {total_bytes}\n\n\
             # HELP rustdesk_packet_drops_total Total relay packets dropped on send failure.\n\
             # TYPE rustdesk_packet_drops_total counter\n\
             rustdesk_packet_drops_total{{service=\"{service_name}\"}} {total_drops}\n",
            service_name = service_name,
            uptime = uptime,
            active = active,
            total_conn = total_conn,
            total_disc = total_disc,
            total_bytes = total_bytes,
            total_drops = total_drops
        )
    } else {
        let heartbeats = TOTAL_HEARTBEATS.load(Ordering::Relaxed);

        format!(
            "# HELP rustdesk_server_uptime_seconds Total server uptime in seconds.\n\
             # TYPE rustdesk_server_uptime_seconds counter\n\
             rustdesk_server_uptime_seconds{{service=\"{service_name}\"}} {uptime}\n\n\
             # HELP rustdesk_heartbeats_total Total heartbeat and registration pings processed.\n\
             # TYPE rustdesk_heartbeats_total counter\n\
             rustdesk_heartbeats_total{{service=\"{service_name}\"}} {heartbeats}\n",
            service_name = service_name,
            uptime = uptime,
            heartbeats = heartbeats
        )
    }
}

// ========================================================
// HTTP ROUTE HANDLERS
// ========================================================

async fn metrics_handler(service_name: String) -> Response {
    let body = generate_prometheus_metrics(&service_name);
    (
        StatusCode::OK,
        [("Content-Type", "text/plain; version=0.0.4; charset=utf-8")],
        body,
    )
        .into_response()
}

async fn health_handler() -> Response {
    (
        StatusCode::OK,
        [("Content-Type", "application/json")],
        r#"{"status":"healthy","service":"rustdesk-server"}"#,
    )
        .into_response()
}

// ========================================================
// ASYNC HTTP SERVER LAUNCHER
// ========================================================

/// Spawns an asynchronous Axum HTTP server on the specified bind address and port.
pub async fn start_metrics_server(
    bind_addr: Option<IpAddr>,
    port: u16,
    service_name: &'static str,
) {
    // Eagerly initialize start time on daemon boot so uptime counts from startup
    lazy_static::initialize(&SERVER_START_TIME);

    let app = Router::new()
        .route(
            "/metrics",
            get({
                let svc = service_name.to_string();
                move || metrics_handler(svc)
            }),
        )
        .route("/health", get(health_handler))
        .route("/livez", get(health_handler));

    let ip = bind_addr.unwrap_or_else(|| IpAddr::V4(Ipv4Addr::new(0, 0, 0, 0)));
    let addr = SocketAddr::new(ip, port);

    match axum::Server::try_bind(&addr) {
        Ok(builder) => {
            hbb_common::log::info!("Prometheus metrics server listening on http://{addr}/metrics");
            hbb_common::tokio::spawn(async move {
                if let Err(e) = builder.serve(app.into_make_service()).await {
                    hbb_common::log::error!("Prometheus metrics server error on {addr}: {e}");
                }
            });
        }
        Err(e) => {
            hbb_common::log::error!("Failed to bind Prometheus metrics server on {addr}: {e}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_prometheus_formatting_and_atomics() {
        let initial_active = ACTIVE_PEERS.load(Ordering::Relaxed);
        {
            let _guard = ActivePeerGuard::new(2);
            assert_eq!(ACTIVE_PEERS.load(Ordering::Relaxed), initial_active + 2);
            add_relayed_bytes(1024);
            inc_packet_drop();

            let output_hbbr = generate_prometheus_metrics("hbbr");
            assert!(output_hbbr.contains("rustdesk_active_peers{service=\"hbbr\"}"));
            assert!(output_hbbr.contains("rustdesk_bytes_relayed_total{service=\"hbbr\"}"));
            assert!(output_hbbr.contains("rustdesk_packet_drops_total{service=\"hbbr\"}"));
            assert!(output_hbbr.contains("rustdesk_server_uptime_seconds{service=\"hbbr\"}"));
        }
        assert_eq!(ACTIVE_PEERS.load(Ordering::Relaxed), initial_active);

        inc_heartbeat();
        let output_hbbs = generate_prometheus_metrics("hbbs");
        assert!(output_hbbs.contains("rustdesk_heartbeats_total{service=\"hbbs\"}"));
        assert!(output_hbbs.contains("rustdesk_server_uptime_seconds{service=\"hbbs\"}"));
        assert!(!output_hbbs.contains("rustdesk_bytes_relayed_total"));
        assert!(!output_hbbs.contains("rustdesk_packet_drops_total"));
    }
}
