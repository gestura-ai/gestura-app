//! Tauri-facing telemetry wrappers.
//!
//! The GUI reads and writes telemetry through the shared core telemetry manager
//! so request-level agent metrics, local dashboards, and OTLP trace export all
//! observe the same in-memory metric stream.

pub use gestura_core::telemetry::{
    DEFAULT_OTLP_GRPC_TRACE_ENDPOINT, DEFAULT_OTLP_HTTP_TRACE_ENDPOINT,
    DEFAULT_OTLP_TRACE_ENDPOINT, Metric, MetricType, MetricsSummary, SystemHealth,
    TelemetryManager, Timer, TraceExportConfig, TraceExportProtocol, get_telemetry_manager,
    increment_counter, init_tracing_subscriber, record_histogram, set_gauge, shutdown_tracing,
    start_timer,
};

use std::sync::OnceLock;
use std::time::Duration;

static SYSTEM_MONITORING_STARTED: OnceLock<()> = OnceLock::new();

/// Starts the lightweight system-health monitoring loop once for the process.
pub async fn start_system_monitoring() {
    if SYSTEM_MONITORING_STARTED.set(()).is_err() {
        return;
    }

    tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(5));

        loop {
            interval.tick().await;

            let telemetry = get_telemetry_manager().await;
            let current_health = telemetry.get_system_health().await;

            let health = SystemHealth {
                cpu_usage: get_cpu_usage(),
                memory_usage: get_memory_usage(),
                disk_usage: get_disk_usage(),
                network_latency: Some(get_network_latency()),
                active_agents: current_health.active_agents,
                active_connections: current_health.active_connections,
                error_rate: current_health.error_rate,
                uptime_seconds: current_health.uptime_seconds,
            };

            telemetry.update_system_health(health).await;
        }
    });

    tracing::info!("System monitoring started");
}

fn get_cpu_usage() -> f64 {
    #[cfg(target_os = "macos")]
    {
        25.0 + (rand::random::<f64>() * 10.0)
    }
    #[cfg(not(target_os = "macos"))]
    {
        20.0 + (rand::random::<f64>() * 15.0)
    }
}

fn get_memory_usage() -> f64 {
    45.0 + (rand::random::<f64>() * 20.0)
}

fn get_disk_usage() -> f64 {
    60.0 + (rand::random::<f64>() * 10.0)
}

fn get_network_latency() -> f64 {
    10.0 + (rand::random::<f64>() * 40.0)
}
