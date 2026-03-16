//! Telemetry and metrics collection for Gestura.app.
//!
//! This module provides two complementary observability layers:
//! - an in-memory metric store used by the GUI/API for local inspection
//! - optional OTLP trace export for correlated request tracing in tools such as
//!   SigNoz

use crate::error::AppError;
use opentelemetry::{KeyValue, global, trace::TracerProvider as _};
use opentelemetry_otlp::{Protocol, SpanExporter, WithExportConfig};
use opentelemetry_sdk::{Resource, trace::SdkTracerProvider};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::{Mutex, RwLock};
use tracing_subscriber::{EnvFilter, fmt::MakeWriter, prelude::*};

/// Default OTLP/HTTP endpoint for local trace collection.
///
/// SigNoz exposes this path via the embedded OpenTelemetry collector.
pub const DEFAULT_OTLP_HTTP_TRACE_ENDPOINT: &str = "http://127.0.0.1:4318/v1/traces";

/// Default OTLP trace endpoint for the current default transport.
pub const DEFAULT_OTLP_TRACE_ENDPOINT: &str = DEFAULT_OTLP_GRPC_TRACE_ENDPOINT;

/// Default OTLP/gRPC endpoint for local trace collection.
pub const DEFAULT_OTLP_GRPC_TRACE_ENDPOINT: &str = "http://127.0.0.1:4317";

/// Supported OTLP transport protocols for trace export.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TraceExportProtocol {
    /// Export traces via OTLP over HTTP using protobuf binary payloads.
    Http,
    /// Export traces via OTLP over gRPC using tonic.
    Grpc,
}

impl TraceExportProtocol {
    /// Returns the stable lowercase config value for this protocol.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Http => "http",
            Self::Grpc => "grpc",
        }
    }
}

/// Runtime configuration for OTLP trace export.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TraceExportConfig {
    /// Whether OTLP export should be attached to the tracing subscriber.
    pub enabled: bool,
    /// OTLP transport protocol used to reach the collector.
    pub protocol: TraceExportProtocol,
    /// Collector endpoint that receives OTLP trace payloads.
    pub endpoint: String,
    /// Logical service name shown in the observability backend.
    pub service_name: String,
    /// Service version attached as a resource attribute.
    pub service_version: String,
}

/// Summary payload for recent in-memory metrics.
#[derive(Debug, Clone, serde::Serialize)]
pub struct MetricsSummary {
    pub timestamp: chrono::DateTime<chrono::Utc>,
    pub total_metrics: usize,
    pub counters: HashMap<String, f64>,
    pub gauges: HashMap<String, f64>,
    pub histograms: HashMap<String, serde_json::Value>,
    pub system_health: SystemHealth,
}

/// Metric types for telemetry
#[derive(Debug, Clone, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum MetricType {
    Counter,
    Gauge,
    Histogram,
    Timer,
}

/// Telemetry metric
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Metric {
    pub name: String,
    pub metric_type: MetricType,
    pub value: f64,
    pub timestamp: chrono::DateTime<chrono::Utc>,
    pub tags: HashMap<String, String>,
}

/// Performance timer for measuring operation duration
pub struct Timer {
    name: String,
    start_time: Instant,
    tags: HashMap<String, String>,
}

impl Timer {
    pub fn new(name: String) -> Self {
        Self {
            name,
            start_time: Instant::now(),
            tags: HashMap::new(),
        }
    }

    pub fn with_tag(mut self, key: String, value: String) -> Self {
        self.tags.insert(key, value);
        self
    }

    pub async fn finish(self) -> Duration {
        let duration = self.start_time.elapsed();

        // Record the timing metric
        let telemetry = get_telemetry_manager().await;
        telemetry
            .record_timer(&self.name, duration, self.tags)
            .await;

        duration
    }
}

/// System health metrics
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct SystemHealth {
    pub cpu_usage: f64,
    pub memory_usage: f64,
    pub disk_usage: f64,
    pub network_latency: Option<f64>,
    pub active_agents: usize,
    pub active_connections: usize,
    pub error_rate: f64,
    pub uptime_seconds: u64,
}

/// Telemetry manager
pub struct TelemetryManager {
    metrics: Arc<RwLock<Vec<Metric>>>,
    counters: Arc<RwLock<HashMap<String, f64>>>,
    gauges: Arc<RwLock<HashMap<String, f64>>>,
    histograms: Arc<RwLock<HashMap<String, Vec<f64>>>>,
    system_health: Arc<Mutex<SystemHealth>>,
    max_metrics: usize,
    start_time: Instant,
}

impl TelemetryManager {
    /// Create a new telemetry manager
    pub fn new(max_metrics: usize) -> Self {
        Self {
            metrics: Arc::new(RwLock::new(Vec::new())),
            counters: Arc::new(RwLock::new(HashMap::new())),
            gauges: Arc::new(RwLock::new(HashMap::new())),
            histograms: Arc::new(RwLock::new(HashMap::new())),
            system_health: Arc::new(Mutex::new(SystemHealth {
                cpu_usage: 0.0,
                memory_usage: 0.0,
                disk_usage: 0.0,
                network_latency: None,
                active_agents: 0,
                active_connections: 0,
                error_rate: 0.0,
                uptime_seconds: 0,
            })),
            max_metrics,
            start_time: Instant::now(),
        }
    }

    /// Record a counter metric
    pub async fn increment_counter(&self, name: &str, value: f64, tags: HashMap<String, String>) {
        let mut counters = self.counters.write().await;
        *counters.entry(name.to_string()).or_insert(0.0) += value;

        self.record_metric(Metric {
            name: name.to_string(),
            metric_type: MetricType::Counter,
            value,
            timestamp: chrono::Utc::now(),
            tags,
        })
        .await;
    }

    /// Record a gauge metric
    pub async fn set_gauge(&self, name: &str, value: f64, tags: HashMap<String, String>) {
        let mut gauges = self.gauges.write().await;
        gauges.insert(name.to_string(), value);

        self.record_metric(Metric {
            name: name.to_string(),
            metric_type: MetricType::Gauge,
            value,
            timestamp: chrono::Utc::now(),
            tags,
        })
        .await;
    }

    /// Record a histogram value
    pub async fn record_histogram(&self, name: &str, value: f64, tags: HashMap<String, String>) {
        let mut histograms = self.histograms.write().await;
        histograms
            .entry(name.to_string())
            .or_insert_with(Vec::new)
            .push(value);

        self.record_metric(Metric {
            name: name.to_string(),
            metric_type: MetricType::Histogram,
            value,
            timestamp: chrono::Utc::now(),
            tags,
        })
        .await;
    }

    /// Record a timer metric
    pub async fn record_timer(
        &self,
        name: &str,
        duration: Duration,
        tags: HashMap<String, String>,
    ) {
        let value = duration.as_secs_f64();

        self.record_metric(Metric {
            name: name.to_string(),
            metric_type: MetricType::Timer,
            value,
            timestamp: chrono::Utc::now(),
            tags,
        })
        .await;
    }

    /// Record a generic metric
    async fn record_metric(&self, metric: Metric) {
        let mut metrics = self.metrics.write().await;
        metrics.push(metric);

        // Trim metrics if needed
        if metrics.len() > self.max_metrics {
            metrics.remove(0);
        }
    }

    /// Update system health metrics
    pub async fn update_system_health(&self, health: SystemHealth) {
        let mut system_health = self.system_health.lock().await;
        *system_health = health;
    }

    /// Update active agent count without overwriting the rest of the health snapshot.
    pub async fn set_active_agents(&self, count: usize) {
        let mut health = self.system_health.lock().await;
        health.active_agents = count;
    }

    /// Update active connection count without overwriting the rest of the health snapshot.
    pub async fn set_active_connections(&self, count: usize) {
        let mut health = self.system_health.lock().await;
        health.active_connections = count;
    }

    /// Get current system health
    pub async fn get_system_health(&self) -> SystemHealth {
        let mut health = self.system_health.lock().await;
        health.uptime_seconds = self.start_time.elapsed().as_secs();
        health.clone()
    }

    /// Get recent metrics
    pub async fn get_recent_metrics(&self, limit: usize) -> Vec<Metric> {
        let metrics = self.metrics.read().await;
        metrics.iter().rev().take(limit).cloned().collect()
    }

    /// Get an aggregate snapshot of counters, gauges, histograms, and system health.
    pub async fn get_metrics_summary(&self) -> MetricsSummary {
        let metrics = self.metrics.read().await;
        let counters = self.counters.read().await;
        let gauges = self.gauges.read().await;
        let histograms = self.histograms.read().await;
        let health = self.get_system_health().await;

        let mut histogram_stats = HashMap::new();
        for (name, values) in histograms.iter() {
            if values.is_empty() {
                continue;
            }

            let mut sorted_values = values.clone();
            sorted_values.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));

            let len = sorted_values.len();
            let sum: f64 = sorted_values.iter().sum();
            let mean = sum / len as f64;
            let median = if len % 2 == 0 {
                (sorted_values[len / 2 - 1] + sorted_values[len / 2]) / 2.0
            } else {
                sorted_values[len / 2]
            };
            let p95_idx = ((len as f64) * 0.95).floor() as usize;
            let p95 = sorted_values
                .get(p95_idx.min(len.saturating_sub(1)))
                .copied()
                .unwrap_or(0.0);

            histogram_stats.insert(
                name.clone(),
                serde_json::json!({
                    "count": len,
                    "sum": sum,
                    "mean": mean,
                    "median": median,
                    "p95": p95,
                    "min": sorted_values.first().copied().unwrap_or(0.0),
                    "max": sorted_values.last().copied().unwrap_or(0.0),
                }),
            );
        }

        MetricsSummary {
            timestamp: chrono::Utc::now(),
            total_metrics: metrics.len(),
            counters: counters.clone(),
            gauges: gauges.clone(),
            histograms: histogram_stats,
            system_health: health,
        }
    }

    /// Clear all metrics
    pub async fn clear_metrics(&self) {
        let mut metrics = self.metrics.write().await;
        let mut counters = self.counters.write().await;
        let mut gauges = self.gauges.write().await;
        let mut histograms = self.histograms.write().await;

        metrics.clear();
        counters.clear();
        gauges.clear();
        histograms.clear();

        tracing::info!("Telemetry metrics cleared");
    }
}

/// Global telemetry manager instance
static TELEMETRY_MANAGER: tokio::sync::OnceCell<TelemetryManager> =
    tokio::sync::OnceCell::const_new();

/// Get the global telemetry manager
pub async fn get_telemetry_manager() -> &'static TelemetryManager {
    TELEMETRY_MANAGER
        .get_or_init(|| async { TelemetryManager::new(100000) })
        .await
}

/// Convenience function to start a timer
pub fn start_timer(name: &str) -> Timer {
    Timer::new(name.to_string())
}

/// Initialize the global tracing subscriber with optional OTLP trace export.
///
/// Call this exactly once from the process entrypoint. When `trace_export` is
/// enabled, spans emitted through `tracing` are exported to the configured OTLP
/// collector while still being written to the local log output.
pub fn init_tracing_subscriber<W>(
    filter: EnvFilter,
    writer: W,
    with_target: bool,
    trace_export: Option<TraceExportConfig>,
) -> Result<(), AppError>
where
    W: for<'writer> MakeWriter<'writer> + Send + Sync + 'static,
{
    let fmt_layer = tracing_subscriber::fmt::layer()
        .with_writer(writer)
        .with_target(with_target);

    if let Some(config) = trace_export.filter(|cfg| cfg.enabled) {
        let tracer = build_otlp_tracer(&config)?;
        tracing_subscriber::registry()
            .with(filter)
            .with(fmt_layer)
            .with(tracing_opentelemetry::layer().with_tracer(tracer))
            .try_init()
            .map_err(|error| {
                AppError::Config(format!("failed to initialize tracing subscriber: {error}"))
            })?;
        tracing::info!(
            protocol = config.protocol.as_str(),
            endpoint = %config.endpoint,
            service_name = %config.service_name,
            "OTLP trace export enabled"
        );
    } else {
        tracing_subscriber::registry()
            .with(filter)
            .with(fmt_layer)
            .try_init()
            .map_err(|error| {
                AppError::Config(format!("failed to initialize tracing subscriber: {error}"))
            })?;
    }

    Ok(())
}

/// Flush and shut down the global tracer provider.
pub fn shutdown_tracing() {
    // OTLP export uses the simple span processor, which exports on span end.
    // There is no global shutdown helper in `opentelemetry` 0.31, so the
    // remaining shutdown path is intentionally a no-op.
}

/// Convenience function to increment a counter
pub async fn increment_counter(name: &str, value: f64) {
    let telemetry = get_telemetry_manager().await;
    telemetry
        .increment_counter(name, value, HashMap::new())
        .await;
}

/// Convenience function to set a gauge
pub async fn set_gauge(name: &str, value: f64) {
    let telemetry = get_telemetry_manager().await;
    telemetry.set_gauge(name, value, HashMap::new()).await;
}

/// Convenience function to record a histogram value
pub async fn record_histogram(name: &str, value: f64) {
    let telemetry = get_telemetry_manager().await;
    telemetry
        .record_histogram(name, value, HashMap::new())
        .await;
}

fn build_otlp_tracer(
    config: &TraceExportConfig,
) -> Result<opentelemetry_sdk::trace::Tracer, AppError> {
    let exporter = match config.protocol {
        TraceExportProtocol::Http => SpanExporter::builder()
            .with_http()
            .with_protocol(Protocol::HttpBinary)
            .with_endpoint(config.endpoint.clone())
            .build(),
        TraceExportProtocol::Grpc => SpanExporter::builder()
            .with_tonic()
            .with_endpoint(config.endpoint.clone())
            .build(),
    }
    .map_err(|error| AppError::Config(format!("failed to build OTLP trace exporter: {error}")))?;

    let tracer_provider = SdkTracerProvider::builder()
        .with_simple_exporter(exporter)
        .with_resource(
            Resource::builder()
                .with_service_name(config.service_name.clone())
                .with_attributes([KeyValue::new(
                    "service.version",
                    config.service_version.clone(),
                )])
                .build(),
        )
        .build();

    let tracer = tracer_provider.tracer(config.service_name.clone());
    global::set_tracer_provider(tracer_provider);

    Ok(tracer)
}
