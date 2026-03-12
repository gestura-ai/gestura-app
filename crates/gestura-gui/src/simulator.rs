use crate::ble::{BleEvent, RingManager, SimulatorStatus, TestHapticPattern};
/// Haptic Harmony Ring Simulator Support Module
///
/// This module provides enhanced support for Haptic Harmony Ring simulators,
/// including auto-discovery, health monitoring, and development workflow integration.
use crate::{
    AppError,
    config::{DeveloperSettings, SimulatorSettings},
};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{Mutex, RwLock, broadcast};

/// Simulator discovery and management service
pub struct SimulatorManager {
    /// Current developer settings
    settings: Arc<RwLock<DeveloperSettings>>,
    /// Connected simulators
    simulators: Arc<RwLock<HashMap<String, SimulatorInfo>>>,
    /// BLE ring manager
    ring_manager: Arc<dyn RingManager>,
    /// Event broadcaster
    event_tx: broadcast::Sender<BleEvent>,
    /// Health check task handle
    health_check_handle: Arc<Mutex<Option<tokio::task::JoinHandle<()>>>>,
}

/// Information about a connected simulator
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SimulatorInfo {
    pub device_id: String,
    pub device_name: String,
    pub status: SimulatorStatus,
    pub last_health_check: chrono::DateTime<chrono::Utc>,
    pub connection_time: chrono::DateTime<chrono::Utc>,
    pub metrics: SimulatorMetrics,
}

/// Performance metrics for simulators
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SimulatorMetrics {
    pub latency_ms: Option<f64>,
    pub packet_loss_rate: f64,
    pub uptime_seconds: u64,
    pub haptic_commands_sent: u64,
    pub gestures_received: u64,
}

impl Default for SimulatorMetrics {
    fn default() -> Self {
        Self {
            latency_ms: None,
            packet_loss_rate: 0.0,
            uptime_seconds: 0,
            haptic_commands_sent: 0,
            gestures_received: 0,
        }
    }
}

impl SimulatorManager {
    /// Create a new simulator manager
    pub fn new(
        settings: Arc<RwLock<DeveloperSettings>>,
        ring_manager: Arc<dyn RingManager>,
        event_tx: broadcast::Sender<BleEvent>,
    ) -> Self {
        Self {
            settings,
            simulators: Arc::new(RwLock::new(HashMap::new())),
            ring_manager,
            event_tx,
            health_check_handle: Arc::new(Mutex::new(None)),
        }
    }

    /// Start the simulator manager
    pub async fn start(&self) -> Result<(), AppError> {
        let (enable_simulators, auto_discover) = {
            let settings = self.settings.read().await;
            (
                settings.enable_simulators,
                settings.auto_discover_simulators,
            )
        };

        if !enable_simulators {
            tracing::info!("Simulator support is disabled");
            return Ok(());
        }

        tracing::info!("Starting simulator manager");

        // Start auto-discovery if enabled
        if auto_discover {
            self.start_auto_discovery().await?;
        }

        // Start health check task
        self.start_health_check_task().await;

        Ok(())
    }

    /// Stop the simulator manager
    pub async fn stop(&self) {
        if let Some(handle) = self.health_check_handle.lock().await.take() {
            handle.abort();
        }
        tracing::info!("Simulator manager stopped");
    }

    /// Discover simulators on localhost
    async fn start_auto_discovery(&self) -> Result<(), AppError> {
        let settings = self.settings.read().await;
        let (start_port, end_port) = settings.simulator.discovery_port_range;

        tracing::info!(
            "Starting simulator auto-discovery on ports {}-{}",
            start_port,
            end_port
        );

        self.scan_for_simulators().await.map(|_| ())
    }

    /// Register a new simulator
    async fn register_simulator(&self, device_id: String) -> Result<(), AppError> {
        if self.simulators.read().await.contains_key(&device_id) {
            return Ok(());
        }

        let settings = self.settings.read().await;

        let simulator_info = SimulatorInfo {
            device_id: device_id.clone(),
            device_name: settings.simulator.device_name_pattern.clone(),
            status: SimulatorStatus::Healthy,
            last_health_check: chrono::Utc::now(),
            connection_time: chrono::Utc::now(),
            metrics: SimulatorMetrics::default(),
        };

        {
            let mut simulators = self.simulators.write().await;
            simulators.insert(device_id.clone(), simulator_info);
        }

        // Emit simulator found event
        let _ = self
            .event_tx
            .send(BleEvent::SimulatorFound(device_id.clone()));

        // Auto-connect if enabled
        if settings.simulator.auto_connect {
            if let Err(e) = self.ring_manager.pair_ring(&device_id).await {
                tracing::warn!("Failed to auto-connect to simulator {}: {}", device_id, e);
            } else {
                let _ = self.event_tx.send(BleEvent::Connected(device_id));
            }
        }

        Ok(())
    }

    /// Start health check task
    async fn start_health_check_task(&self) {
        let mut handle_guard = self.health_check_handle.lock().await;
        if handle_guard
            .as_ref()
            .is_some_and(|handle| !handle.is_finished())
        {
            return;
        }

        let settings = self.settings.clone();
        let simulators = self.simulators.clone();
        let ring_manager = self.ring_manager.clone();
        let event_tx = self.event_tx.clone();

        let handle = tokio::spawn(async move {
            loop {
                let interval = {
                    let settings = settings.read().await;
                    settings.simulator.health_check_interval
                };

                tokio::time::sleep(tokio::time::Duration::from_secs(interval as u64)).await;

                let simulator_ids: Vec<String> = {
                    let simulators = simulators.read().await;
                    simulators.keys().cloned().collect()
                };

                for device_id in simulator_ids {
                    match ring_manager.get_simulator_health(&device_id).await {
                        Ok(status) => {
                            let mut simulators = simulators.write().await;
                            if let Some(info) = simulators.get_mut(&device_id) {
                                info.status = status.clone();
                                info.last_health_check = chrono::Utc::now();
                                info.metrics.uptime_seconds =
                                    (chrono::Utc::now() - info.connection_time).num_seconds()
                                        as u64;
                            }
                            let _ = event_tx.send(BleEvent::SimulatorStatus(device_id, status));
                        }
                        Err(e) => {
                            tracing::warn!(
                                "Health check failed for simulator {}: {}",
                                device_id,
                                e
                            );
                        }
                    }
                }
            }
        });

        *handle_guard = Some(handle);
    }

    /// Scan for simulators and register newly discovered instances.
    pub async fn scan_for_simulators(&self) -> Result<Vec<String>, AppError> {
        let simulator_ids = self.ring_manager.scan_for_simulators().await?;
        for simulator_id in &simulator_ids {
            self.register_simulator(simulator_id.clone()).await?;
        }
        Ok(simulator_ids)
    }

    /// Get all connected simulators
    pub async fn get_simulators(&self) -> HashMap<String, SimulatorInfo> {
        self.simulators.read().await.clone()
    }

    /// Reset a specific simulator
    pub async fn reset_simulator(&self, device_id: &str) -> Result<(), AppError> {
        self.ring_manager.reset_simulator(device_id).await?;

        // Update simulator info
        let mut simulators = self.simulators.write().await;
        if let Some(info) = simulators.get_mut(device_id) {
            info.connection_time = chrono::Utc::now();
            info.metrics = SimulatorMetrics::default();
        }

        Ok(())
    }

    /// Send test haptic pattern to simulator
    pub async fn send_test_haptic(
        &self,
        device_id: &str,
        pattern: TestHapticPattern,
    ) -> Result<(), AppError> {
        self.ring_manager
            .send_test_haptic(device_id, pattern)
            .await?;

        // Update metrics
        let mut simulators = self.simulators.write().await;
        if let Some(info) = simulators.get_mut(device_id) {
            info.metrics.haptic_commands_sent += 1;
        }

        Ok(())
    }

    /// Get connection logs for a simulator
    pub async fn get_connection_logs(&self, device_id: &str) -> Result<Vec<String>, AppError> {
        self.ring_manager.get_connection_logs(device_id).await
    }

    /// Get simulator health status.
    pub async fn get_simulator_health(&self, device_id: &str) -> Result<SimulatorStatus, AppError> {
        self.ring_manager.get_simulator_health(device_id).await
    }

    /// Check if developer mode is enabled
    pub async fn is_developer_mode_enabled(&self) -> bool {
        self.settings.read().await.developer_mode
    }

    /// Return the current simulator settings snapshot.
    pub async fn get_simulator_config(&self) -> SimulatorSettings {
        self.settings.read().await.simulator.clone()
    }

    /// Return the current developer settings snapshot.
    pub async fn get_settings(&self) -> DeveloperSettings {
        self.settings.read().await.clone()
    }

    /// Update simulator settings used by the runtime manager.
    pub async fn update_simulator_config(&self, config: SimulatorSettings) {
        self.settings.write().await.simulator = config;
    }

    /// Update the developer mode flag used by simulator features.
    pub async fn set_developer_mode(&self, enabled: bool) {
        self.settings.write().await.developer_mode = enabled;
    }

    /// Enable or disable simulator support at runtime.
    pub async fn set_simulator_support(&self, enabled: bool) -> Result<(), AppError> {
        {
            let mut settings = self.settings.write().await;
            settings.enable_simulators = enabled;
        }

        if enabled {
            self.start().await
        } else {
            self.stop().await;
            self.simulators.write().await.clear();
            Ok(())
        }
    }

    /// Update the health monitoring interval for future checks.
    pub async fn set_health_check_interval(&self, interval_seconds: u32) {
        self.settings.write().await.simulator.health_check_interval = interval_seconds;
    }

    /// Get current metrics for a simulator.
    pub async fn get_simulator_metrics(
        &self,
        device_id: &str,
    ) -> Result<SimulatorMetrics, AppError> {
        self.simulators
            .read()
            .await
            .get(device_id)
            .map(|info| info.metrics.clone())
            .ok_or_else(|| AppError::Ble(format!("Simulator not found: {device_id}")))
    }

    /// Update simulator metrics
    pub async fn update_metrics(&self, device_id: &str, latency_ms: Option<f64>) {
        let mut simulators = self.simulators.write().await;
        if let Some(info) = simulators.get_mut(device_id) {
            if let Some(latency) = latency_ms {
                info.metrics.latency_ms = Some(latency);
            }
            info.metrics.uptime_seconds =
                (chrono::Utc::now() - info.connection_time).num_seconds() as u64;
        }
    }

    /// Record gesture received from simulator
    pub async fn record_gesture_received(&self, device_id: &str) {
        let mut simulators = self.simulators.write().await;
        if let Some(info) = simulators.get_mut(device_id) {
            info.metrics.gestures_received += 1;
        }
    }
}

/// Simulator testing utilities
pub struct SimulatorTester {
    manager: Arc<SimulatorManager>,
}

impl SimulatorTester {
    pub fn new(manager: Arc<SimulatorManager>) -> Self {
        Self { manager }
    }

    /// Run comprehensive simulator tests
    pub async fn run_comprehensive_test(&self, device_id: &str) -> Result<TestResults, AppError> {
        let results = TestResults {
            connectivity: self.test_connectivity(device_id).await?,
            latency_ms: self.test_latency(device_id).await?,
            haptic_tests: self.test_haptic_patterns(device_id).await?,
        };

        Ok(results)
    }

    async fn test_connectivity(&self, device_id: &str) -> Result<bool, AppError> {
        self.manager
            .send_test_haptic(device_id, TestHapticPattern::ConnectivityTest)
            .await?;
        Ok(true)
    }

    async fn test_latency(&self, device_id: &str) -> Result<f64, AppError> {
        let start = std::time::Instant::now();
        self.manager
            .send_test_haptic(device_id, TestHapticPattern::LatencyTest)
            .await?;
        let latency = start.elapsed().as_millis() as f64;

        self.manager.update_metrics(device_id, Some(latency)).await;
        Ok(latency)
    }

    async fn test_haptic_patterns(
        &self,
        device_id: &str,
    ) -> Result<Vec<HapticTestResult>, AppError> {
        let mut results = Vec::new();

        // Test intensity range
        let intensity_test = TestHapticPattern::IntensityTest {
            min: 0.1,
            max: 1.0,
            steps: 10,
        };
        match self
            .manager
            .send_test_haptic(device_id, intensity_test)
            .await
        {
            Ok(_) => results.push(HapticTestResult {
                pattern: "IntensityTest".to_string(),
                success: true,
                error: None,
            }),
            Err(e) => results.push(HapticTestResult {
                pattern: "IntensityTest".to_string(),
                success: false,
                error: Some(e.to_string()),
            }),
        }

        // Test duration patterns
        let duration_test = TestHapticPattern::DurationTest {
            durations: vec![100, 200, 500, 1000],
        };
        match self
            .manager
            .send_test_haptic(device_id, duration_test)
            .await
        {
            Ok(_) => results.push(HapticTestResult {
                pattern: "DurationTest".to_string(),
                success: true,
                error: None,
            }),
            Err(e) => results.push(HapticTestResult {
                pattern: "DurationTest".to_string(),
                success: false,
                error: Some(e.to_string()),
            }),
        }

        Ok(results)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TestResults {
    pub connectivity: bool,
    pub latency_ms: f64,
    pub haptic_tests: Vec<HapticTestResult>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ble::RingManager;
    use crate::device_simulator::{SimulatedRingManager, create_test_simulator};

    async fn build_manager() -> Arc<SimulatorManager> {
        let (device_simulator, _event_rx) = create_test_simulator().await;
        let ring_manager: Arc<dyn RingManager> =
            Arc::new(SimulatedRingManager::new(device_simulator));
        let (event_tx, _) = broadcast::channel(32);
        Arc::new(SimulatorManager::new(
            Arc::new(RwLock::new(DeveloperSettings::default())),
            ring_manager,
            event_tx,
        ))
    }

    #[tokio::test]
    async fn scan_for_simulators_registers_runtime_devices() {
        let manager = build_manager().await;

        let discovered = manager.scan_for_simulators().await.unwrap();
        let simulators = manager.get_simulators().await;

        assert!(!discovered.is_empty());
        assert_eq!(discovered.len(), simulators.len());
        assert!(
            discovered
                .iter()
                .all(|device_id| simulators.contains_key(device_id))
        );
    }

    #[tokio::test]
    async fn update_simulator_config_changes_runtime_settings() {
        let manager = build_manager().await;
        let mut updated = manager.get_simulator_config().await;
        updated.health_check_interval = 5;
        updated.enable_metrics = false;

        manager.update_simulator_config(updated.clone()).await;

        assert_eq!(manager.get_simulator_config().await, updated);
    }
}

impl Default for TestResults {
    fn default() -> Self {
        Self {
            connectivity: false,
            latency_ms: 0.0,
            haptic_tests: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HapticTestResult {
    pub pattern: String,
    pub success: bool,
    pub error: Option<String>,
}
