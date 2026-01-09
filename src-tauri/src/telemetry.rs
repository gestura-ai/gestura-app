//! Telemetry and metrics collection for Gestura.app
//! Provides performance monitoring, usage analytics, and system health metrics

#[allow(unused_imports)]
use crate::AppError;
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

    /// Get metrics summary
    pub async fn get_metrics_summary(&self) -> serde_json::Value {
        let metrics = self.metrics.read().await;
        let counters = self.counters.read().await;
        let gauges = self.gauges.read().await;
        let histograms = self.histograms.read().await;
        let health = self.get_system_health().await;

        // Calculate histogram statistics
        let mut histogram_stats = HashMap::new();
        for (name, values) in histograms.iter() {
            if !values.is_empty() {
                let mut sorted_values = values.clone();
                sorted_values.sort_by(|a, b| a.partial_cmp(b).unwrap());

                let len = sorted_values.len();
                let sum: f64 = sorted_values.iter().sum();
                let mean = sum / len as f64;
                let median = if len % 2 == 0 {
                    (sorted_values[len / 2 - 1] + sorted_values[len / 2]) / 2.0
                } else {
                    sorted_values[len / 2]
                };
                let p95_idx = ((len as f64) * 0.95) as usize;
                let p95 = sorted_values.get(p95_idx).copied().unwrap_or(0.0);

                histogram_stats.insert(
                    name.clone(),
                    serde_json::json!({
                        "count": len,
                        "sum": sum,
                        "mean": mean,
                        "median": median,
                        "p95": p95,
                        "min": sorted_values.first().copied().unwrap_or(0.0),
                        "max": sorted_values.last().copied().unwrap_or(0.0)
                    }),
                );
            }
        }

        serde_json::json!({
            "timestamp": chrono::Utc::now(),
            "total_metrics": metrics.len(),
            "counters": *counters,
            "gauges": *gauges,
            "histograms": histogram_stats,
            "system_health": health
        })
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

    /// Start background system monitoring
    pub async fn start_system_monitoring(&self) {
        let system_health = self.system_health.clone();
        let start_time = self.start_time;

        tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(30));

            loop {
                interval.tick().await;

                // Collect system metrics (simplified)
                let health = SystemHealth {
                    cpu_usage: Self::get_cpu_usage(),
                    memory_usage: Self::get_memory_usage(),
                    disk_usage: Self::get_disk_usage(),
                    network_latency: Self::measure_network_latency().await,
                    active_agents: 0,      // TODO: Get from agent manager
                    active_connections: 0, // TODO: Get from connection manager
                    error_rate: 0.0,       // TODO: Calculate from error metrics
                    uptime_seconds: start_time.elapsed().as_secs(),
                };

                let mut system_health_guard = system_health.lock().await;
                *system_health_guard = health;
            }
        });

        tracing::info!("Started system monitoring");
    }

    /// Get CPU usage (simplified)
    fn get_cpu_usage() -> f64 {
        // In a real implementation, this would use system APIs
        // For now, return a mock value
        rand::random::<f64>() * 100.0
    }

    /// Get memory usage (simplified)
    fn get_memory_usage() -> f64 {
        // In a real implementation, this would use system APIs
        // For now, return a mock value
        rand::random::<f64>() * 100.0
    }

    /// Get disk usage (simplified)
    fn get_disk_usage() -> f64 {
        // In a real implementation, this would check disk space
        // For now, return a mock value
        rand::random::<f64>() * 100.0
    }

    /// Measure network latency (simplified)
    async fn measure_network_latency() -> Option<f64> {
        // In a real implementation, this would ping a known server
        // For now, return a mock value
        Some(rand::random::<f64>() * 100.0)
    }
}

/// Global telemetry manager instance
static TELEMETRY_MANAGER: tokio::sync::OnceCell<TelemetryManager> =
    tokio::sync::OnceCell::const_new();

/// Get the global telemetry manager
pub async fn get_telemetry_manager() -> &'static TelemetryManager {
    TELEMETRY_MANAGER
        .get_or_init(|| async {
            let manager = TelemetryManager::new(100000);
            manager.start_system_monitoring().await;
            manager
        })
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

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_telemetry_manager() {
        let manager = TelemetryManager::new(1000);

        // Test counter
        manager
            .increment_counter("test.counter", 1.0, HashMap::new())
            .await;
        manager
            .increment_counter("test.counter", 2.0, HashMap::new())
            .await;

        let counters = manager.counters.read().await;
        assert_eq!(counters.get("test.counter"), Some(&3.0));

        // Test gauge
        manager.set_gauge("test.gauge", 42.0, HashMap::new()).await;
        let gauges = manager.gauges.read().await;
        assert_eq!(gauges.get("test.gauge"), Some(&42.0));

        // Test histogram
        manager
            .record_histogram("test.histogram", 1.0, HashMap::new())
            .await;
        manager
            .record_histogram("test.histogram", 2.0, HashMap::new())
            .await;
        manager
            .record_histogram("test.histogram", 3.0, HashMap::new())
            .await;

        let histograms = manager.histograms.read().await;
        assert_eq!(histograms.get("test.histogram").unwrap().len(), 3);
    }

    #[tokio::test]
    async fn test_timer() {
        let timer = Timer::new("test.timer".to_string());
        tokio::time::sleep(Duration::from_millis(10)).await;
        let duration = timer.finish().await;

        assert!(duration >= Duration::from_millis(10));
    }
}
