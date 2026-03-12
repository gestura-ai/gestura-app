//! BLE integration for Haptic Harmony ring
//! Provides pairing, gesture detection, haptic feedback, and OTA updates

use crate::{AppConfig, AppError};
use std::time::Duration;
use tokio::sync::broadcast;
use tokio::time;

/// Event emitted by BLE detector
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BleEvent {
    DeviceFound(String),
    SimulatorFound(String),
    Connected(String),
    Disconnected(String),
    GestureDetected(GestureType),
    BatteryLevel(u8),
    FirmwareVersion(String),
    SimulatorStatus(String, SimulatorStatus),
    ConnectionLog(String, String), // device_id, log_message
}

/// Gesture types detected from the ring
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GestureType {
    Tap,
    DoubleTap,
    TiltLeft,
    TiltRight,
    TiltUp,
    TiltDown,
}

/// Simulator status information
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum SimulatorStatus {
    Healthy,
    Degraded,
    Offline,
    Error(String),
}

/// Ring connection status and metadata
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct RingStatus {
    pub device_id: String,
    pub battery_level: Option<u8>,
    pub firmware_version: Option<String>,
    pub is_connected: bool,
    pub last_seen: chrono::DateTime<chrono::Utc>,
    pub is_simulator: bool,
    pub simulator_status: Option<SimulatorStatus>,
    pub connection_logs: Vec<String>,
}

/// Trait for BLE device detection and connection
#[async_trait::async_trait]
pub trait BleDetector: Send + Sync {
    /// Periodically detect devices and emit events to subscribers and NATS
    async fn auto_detect(
        &self,
        config: &AppConfig,
        tx: broadcast::Sender<BleEvent>,
        nats: Option<&crate::NatsConn>,
    ) -> Result<(), AppError>;
}

/// Trait for ring pairing and management
#[async_trait::async_trait]
pub trait RingManager: Send + Sync {
    /// Scan for available rings
    async fn scan_for_rings(&self) -> Result<Vec<String>, AppError>;
    /// Scan specifically for simulators
    async fn scan_for_simulators(&self) -> Result<Vec<String>, AppError>;
    /// Pair with a specific ring
    async fn pair_ring(&self, device_id: &str) -> Result<(), AppError>;
    /// Get current ring status
    async fn get_ring_status(&self, device_id: &str) -> Result<Option<RingStatus>, AppError>;
    /// Send haptic feedback to ring
    async fn send_haptic(
        &self,
        device_id: &str,
        pattern: crate::haptics::HapticRequest,
    ) -> Result<(), AppError>;
    /// Send test haptic pattern to simulator
    async fn send_test_haptic(
        &self,
        device_id: &str,
        test_pattern: TestHapticPattern,
    ) -> Result<(), AppError>;
    /// Start OTA update
    async fn start_ota_update(
        &self,
        device_id: &str,
        firmware_data: Vec<u8>,
    ) -> Result<(), AppError>;
    /// Start gesture monitoring
    async fn start_gesture_monitoring(
        &self,
        device_id: &str,
        event_tx: tokio::sync::broadcast::Sender<BleEvent>,
    ) -> Result<(), AppError>;
    /// Stop gesture monitoring
    async fn stop_gesture_monitoring(&self, device_id: &str) -> Result<(), AppError>;
    /// Reset simulator connection
    async fn reset_simulator(&self, device_id: &str) -> Result<(), AppError>;
    /// Get simulator health status
    async fn get_simulator_health(&self, device_id: &str) -> Result<SimulatorStatus, AppError>;
    /// Get connection logs for device
    async fn get_connection_logs(&self, device_id: &str) -> Result<Vec<String>, AppError>;
}

/// Test haptic patterns specifically for simulators
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub enum TestHapticPattern {
    /// Basic connectivity test
    ConnectivityTest,
    /// Latency measurement pattern
    LatencyTest,
    /// Intensity range test
    IntensityTest { min: f32, max: f32, steps: u8 },
    /// Duration test
    DurationTest { durations: Vec<u32> },
    /// Complex pattern test
    ComplexPattern { pattern: Vec<(f32, u32)> }, // (intensity, duration) pairs
}

/// A mock BLE detector for testing without hardware.
pub struct MockBleDetector;

#[async_trait::async_trait]
impl BleDetector for MockBleDetector {
    async fn auto_detect(
        &self,
        _config: &AppConfig,
        tx: broadcast::Sender<BleEvent>,
        _nats: Option<&crate::NatsConn>,
    ) -> Result<(), AppError> {
        let mut interval = time::interval(Duration::from_secs(30));
        loop {
            interval.tick().await;
            let evt = BleEvent::DeviceFound("mock-device".into());
            let _ = tx.send(evt.clone());
            #[cfg(feature = "nats")]
            {
                let _ = _nats; // avoid unused warnings when feature toggles
            }
        }
    }
}

/// Mock ring manager for testing
pub struct MockRingManager;

#[async_trait::async_trait]
impl RingManager for MockRingManager {
    async fn scan_for_rings(&self) -> Result<Vec<String>, AppError> {
        Ok(vec!["mock-ring-001".to_string()])
    }

    async fn scan_for_simulators(&self) -> Result<Vec<String>, AppError> {
        Ok(vec!["mock-simulator-001".to_string()])
    }

    async fn pair_ring(&self, _device_id: &str) -> Result<(), AppError> {
        Ok(())
    }

    async fn get_ring_status(&self, device_id: &str) -> Result<Option<RingStatus>, AppError> {
        let is_simulator = device_id.contains("simulator");
        Ok(Some(RingStatus {
            device_id: device_id.to_string(),
            battery_level: Some(85),
            firmware_version: Some(if is_simulator { "SIM-1.0.0" } else { "1.0.0" }.to_string()),
            is_connected: true,
            last_seen: chrono::Utc::now(),
            is_simulator,
            simulator_status: if is_simulator {
                Some(SimulatorStatus::Healthy)
            } else {
                None
            },
            connection_logs: vec![
                "Connected successfully".to_string(),
                "BLE services discovered".to_string(),
                if is_simulator {
                    "Simulator health check: OK"
                } else {
                    "Device paired"
                }
                .to_string(),
            ],
        }))
    }

    async fn send_haptic(
        &self,
        _device_id: &str,
        _pattern: crate::haptics::HapticRequest,
    ) -> Result<(), AppError> {
        Ok(())
    }

    async fn send_test_haptic(
        &self,
        device_id: &str,
        test_pattern: TestHapticPattern,
    ) -> Result<(), AppError> {
        tracing::info!("Sending test haptic to {}: {:?}", device_id, test_pattern);
        Ok(())
    }

    async fn start_ota_update(
        &self,
        _device_id: &str,
        _firmware_data: Vec<u8>,
    ) -> Result<(), AppError> {
        Ok(())
    }

    async fn start_gesture_monitoring(
        &self,
        device_id: &str,
        event_tx: tokio::sync::broadcast::Sender<BleEvent>,
    ) -> Result<(), AppError> {
        // Mock gesture simulation
        let device_id = device_id.to_string();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(tokio::time::Duration::from_secs(10));
            let gestures = [
                GestureType::Tap,
                GestureType::DoubleTap,
                GestureType::TiltLeft,
                GestureType::TiltRight,
            ];
            let mut gesture_idx = 0;

            loop {
                interval.tick().await;
                let gesture = gestures[gesture_idx % gestures.len()].clone();
                let _ = event_tx.send(BleEvent::GestureDetected(gesture));
                gesture_idx += 1;
            }
        });
        tracing::info!("Started mock gesture monitoring for {}", device_id);
        Ok(())
    }

    async fn stop_gesture_monitoring(&self, device_id: &str) -> Result<(), AppError> {
        tracing::info!("Stopped gesture monitoring for {}", device_id);
        Ok(())
    }

    async fn reset_simulator(&self, device_id: &str) -> Result<(), AppError> {
        tracing::info!("Resetting simulator: {}", device_id);
        Ok(())
    }

    async fn get_simulator_health(&self, device_id: &str) -> Result<SimulatorStatus, AppError> {
        if device_id.contains("simulator") {
            Ok(SimulatorStatus::Healthy)
        } else {
            Err(AppError::Ble("Not a simulator device".to_string()))
        }
    }

    async fn get_connection_logs(&self, device_id: &str) -> Result<Vec<String>, AppError> {
        Ok(vec![
            format!("Mock connection log for {}", device_id),
            "BLE scan started".to_string(),
            "Device discovered".to_string(),
            "Connection established".to_string(),
            "Services discovered".to_string(),
        ])
    }
}

/// Create appropriate ring manager (always uses mock — real BLE was never wired up)
pub fn create_ring_manager() -> Box<dyn RingManager> {
    Box::new(MockRingManager)
}

/// Haptic Harmony ring specific constants
pub mod ring_constants {
    /// Service UUID for Haptic Harmony ring
    pub const HAPTIC_SERVICE_UUID: &str = "12345678-1234-5678-9abc-123456789abc";
    /// Characteristic UUID for haptic commands
    pub const HAPTIC_COMMAND_UUID: &str = "12345678-1234-5678-9abc-123456789abd";
    /// Characteristic UUID for gesture events
    pub const GESTURE_EVENT_UUID: &str = "12345678-1234-5678-9abc-123456789abe";
    /// Characteristic UUID for battery level
    pub const BATTERY_LEVEL_UUID: &str = "12345678-1234-5678-9abc-123456789abf";
    /// Characteristic UUID for shared state snapshots
    pub const STATE_SNAPSHOT_UUID: &str = "12345678-1234-5678-9abc-123456789ac1";
    /// Characteristic UUID for OTA updates
    pub const OTA_UPDATE_UUID: &str = "12345678-1234-5678-9abc-123456789ac0";
}
