//! Telemetry and metrics collection for Gestura.app
//! Provides performance monitoring, usage analytics, and system health metrics

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::{Mutex, RwLock};

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
