//! Device simulation for testing without real hardware
//! Provides virtual Haptic Harmony rings and other devices

use crate::AppError;
use crate::ble::{BleEvent, GestureType, RingManager, RingStatus};
use crate::haptics::HapticRequest;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{RwLock, broadcast};

/// Simulated Haptic Harmony ring device
#[derive(Debug, Clone)]
pub struct SimulatedRing {
    pub device_id: String,
    pub name: String,
    pub battery_level: u8,
    pub firmware_version: String,
    pub is_connected: bool,
    pub is_paired: bool,
    pub last_seen: chrono::DateTime<chrono::Utc>,
    pub gesture_monitoring: bool,
}

impl SimulatedRing {
    pub fn new(device_id: String, name: String) -> Self {
        Self {
            device_id,
            name,
            battery_level: 85,
            firmware_version: "1.2.3".to_string(),
            is_connected: false,
            is_paired: false,
            last_seen: chrono::Utc::now(),
            gesture_monitoring: false,
        }
    }

    /// Simulate battery drain over time
    pub fn update_battery(&mut self) {
        if self.is_connected && self.battery_level > 0 {
            // Simulate 1% drain every update (for testing)
            self.battery_level = self.battery_level.saturating_sub(1);
        }
    }

    /// Simulate connection status changes
    pub fn simulate_connection_change(&mut self) -> Option<BleEvent> {
        let should_disconnect = rand::random::<f32>() < 0.1; // 10% chance

        if self.is_connected && should_disconnect {
            self.is_connected = false;
            Some(BleEvent::Disconnected(self.device_id.clone()))
        } else if !self.is_connected && !should_disconnect {
            self.is_connected = true;
            self.last_seen = chrono::Utc::now();
            Some(BleEvent::Connected(self.device_id.clone()))
        } else {
            None
        }
    }

    /// Generate random gesture events
    pub fn generate_gesture(&self) -> Option<GestureType> {
        if !self.gesture_monitoring || !self.is_connected {
            return None;
        }

        let gesture_chance = rand::random::<f32>();
        if gesture_chance < 0.05 {
            // 5% chance per update
            let gestures = [
                GestureType::Tap,
                GestureType::DoubleTap,
                GestureType::TiltLeft,
                GestureType::TiltRight,
                GestureType::TiltUp,
                GestureType::TiltDown,
            ];
            let index = (rand::random::<f32>() * gestures.len() as f32) as usize;
            Some(gestures[index % gestures.len()].clone())
        } else {
            None
        }
    }
}

/// Device simulator that manages multiple simulated devices
pub struct DeviceSimulator {
    rings: Arc<RwLock<HashMap<String, SimulatedRing>>>,
    event_tx: broadcast::Sender<BleEvent>,
    simulation_running: Arc<RwLock<bool>>,
}

impl DeviceSimulator {
    /// Create a new device simulator
    pub fn new() -> (Self, broadcast::Receiver<BleEvent>) {
        let (event_tx, event_rx) = broadcast::channel(1000);

        let simulator = Self {
            rings: Arc::new(RwLock::new(HashMap::new())),
            event_tx,
            simulation_running: Arc::new(RwLock::new(false)),
        };

        (simulator, event_rx)
    }

    /// Add a simulated ring
    pub async fn add_ring(&self, device_id: String, name: String) {
        let ring = SimulatedRing::new(device_id.clone(), name.clone());
        let mut rings = self.rings.write().await;
        rings.insert(device_id.clone(), ring);

        // Emit device found event
        let _ = self.event_tx.send(BleEvent::DeviceFound(device_id));
        tracing::info!("Added simulated ring: {}", name);
    }

    /// Start the simulation loop
    pub async fn start_simulation(&self) {
        let mut running = self.simulation_running.write().await;
        if *running {
            return; // Already running
        }
        *running = true;
        drop(running);

        let rings = self.rings.clone();
        let event_tx = self.event_tx.clone();
        let simulation_running = self.simulation_running.clone();

        tokio::spawn(async move {
            let mut interval = tokio::time::interval(tokio::time::Duration::from_secs(5));

            while *simulation_running.read().await {
                interval.tick().await;

                let mut rings_guard = rings.write().await;
                for ring in rings_guard.values_mut() {
                    // Update battery
                    ring.update_battery();

                    // Simulate connection changes
                    if let Some(event) = ring.simulate_connection_change() {
                        let _ = event_tx.send(event);
                    }

                    // Generate gestures
                    if let Some(gesture) = ring.generate_gesture() {
                        let _ = event_tx.send(BleEvent::GestureDetected(gesture));
                    }

                    // Emit battery level updates occasionally
                    if rand::random::<f32>() < 0.2 {
                        // 20% chance
                        let _ = event_tx.send(BleEvent::BatteryLevel(ring.battery_level));
                    }
                }
            }
        });

        tracing::info!("Started device simulation");
    }

    /// Stop the simulation
    pub async fn stop_simulation(&self) {
        let mut running = self.simulation_running.write().await;
        *running = false;
        tracing::info!("Stopped device simulation");
    }

    /// Get all simulated rings
    pub async fn get_rings(&self) -> Vec<String> {
        let rings = self.rings.read().await;
        rings.keys().cloned().collect()
    }

    /// Get ring status
    pub async fn get_ring_status(&self, device_id: &str) -> Option<RingStatus> {
        let rings = self.rings.read().await;
        rings.get(device_id).map(|ring| RingStatus {
            device_id: ring.device_id.clone(),
            battery_level: Some(ring.battery_level),
            firmware_version: Some(ring.firmware_version.clone()),
            is_connected: ring.is_connected,
            last_seen: ring.last_seen,
            is_simulator: true,
            simulator_status: Some(crate::ble::SimulatorStatus::Healthy),
            connection_logs: vec![
                "Simulator connected".to_string(),
                "Virtual services initialized".to_string(),
                "Ready for commands".to_string(),
            ],
        })
    }

    /// Simulate pairing a ring
    pub async fn pair_ring(&self, device_id: &str) -> Result<(), AppError> {
        let mut rings = self.rings.write().await;
        if let Some(ring) = rings.get_mut(device_id) {
            ring.is_paired = true;
            ring.is_connected = true;
            ring.last_seen = chrono::Utc::now();

            let _ = self
                .event_tx
                .send(BleEvent::Connected(device_id.to_string()));
            tracing::info!("Simulated pairing for ring: {}", device_id);
            Ok(())
        } else {
            Err(AppError::Ble("Ring not found".to_string()))
        }
    }

    /// Simulate haptic feedback
    pub async fn send_haptic(
        &self,
        device_id: &str,
        request: HapticRequest,
    ) -> Result<(), AppError> {
        let rings = self.rings.read().await;
        if let Some(ring) = rings.get(device_id) {
            if ring.is_connected {
                tracing::info!(
                    "Simulated haptic feedback for {}: {:?} ({}ms, {}% intensity)",
                    device_id,
                    request.pattern,
                    request.duration_ms,
                    (request.intensity * 100.0) as u8
                );
                Ok(())
            } else {
                Err(AppError::Ble("Ring not connected".to_string()))
            }
        } else {
            Err(AppError::Ble("Ring not found".to_string()))
        }
    }

    /// Start gesture monitoring for a ring
    pub async fn start_gesture_monitoring(&self, device_id: &str) -> Result<(), AppError> {
        let mut rings = self.rings.write().await;
        if let Some(ring) = rings.get_mut(device_id) {
            ring.gesture_monitoring = true;
            tracing::info!(
                "Started gesture monitoring for simulated ring: {}",
                device_id
            );
            Ok(())
        } else {
            Err(AppError::Ble("Ring not found".to_string()))
        }
    }

    /// Stop gesture monitoring for a ring
    pub async fn stop_gesture_monitoring(&self, device_id: &str) -> Result<(), AppError> {
        let mut rings = self.rings.write().await;
        if let Some(ring) = rings.get_mut(device_id) {
            ring.gesture_monitoring = false;
            tracing::info!(
                "Stopped gesture monitoring for simulated ring: {}",
                device_id
            );
            Ok(())
        } else {
            Err(AppError::Ble("Ring not found".to_string()))
        }
    }
}

/// Simulated ring manager that implements RingManager trait
pub struct SimulatedRingManager {
    simulator: Arc<DeviceSimulator>,
}

impl SimulatedRingManager {
    pub fn new(simulator: Arc<DeviceSimulator>) -> Self {
        Self { simulator }
    }
}

#[async_trait::async_trait]
impl RingManager for SimulatedRingManager {
    async fn scan_for_rings(&self) -> Result<Vec<String>, AppError> {
        Ok(self.simulator.get_rings().await)
    }

    async fn scan_for_simulators(&self) -> Result<Vec<String>, AppError> {
        // Return all rings as simulators since this is the simulator manager
        Ok(self.simulator.get_rings().await)
    }

    async fn pair_ring(&self, device_id: &str) -> Result<(), AppError> {
        self.simulator.pair_ring(device_id).await
    }

    async fn get_ring_status(&self, device_id: &str) -> Result<Option<RingStatus>, AppError> {
        Ok(self.simulator.get_ring_status(device_id).await)
    }

    async fn send_haptic(&self, device_id: &str, pattern: HapticRequest) -> Result<(), AppError> {
        self.simulator.send_haptic(device_id, pattern).await
    }

    async fn send_test_haptic(
        &self,
        device_id: &str,
        test_pattern: crate::ble::TestHapticPattern,
    ) -> Result<(), AppError> {
        tracing::info!("Sending test haptic to {}: {:?}", device_id, test_pattern);
        // Convert test pattern to regular haptic request for simulation
        let haptic_request = match test_pattern {
            crate::ble::TestHapticPattern::ConnectivityTest => HapticRequest {
                pattern: crate::haptics::HapticPattern::Click,
                intensity: 0.5,
                duration_ms: 100,
                repeat_count: 1,
                repeat_delay_ms: 0,
            },
            crate::ble::TestHapticPattern::LatencyTest => HapticRequest {
                pattern: crate::haptics::HapticPattern::Pulse,
                intensity: 0.7,
                duration_ms: 50,
                repeat_count: 0,
                repeat_delay_ms: 0,
            },
            _ => HapticRequest {
                pattern: crate::haptics::HapticPattern::Click,
                intensity: 0.5,
                duration_ms: 100,
                repeat_count: 0,
                repeat_delay_ms: 0,
            },
        };
        self.simulator.send_haptic(device_id, haptic_request).await
    }

    async fn start_ota_update(
        &self,
        device_id: &str,
        firmware_data: Vec<u8>,
    ) -> Result<(), AppError> {
        tracing::info!(
            "Simulated OTA update for {}: {} bytes",
            device_id,
            firmware_data.len()
        );
        Ok(())
    }

    async fn start_gesture_monitoring(
        &self,
        device_id: &str,
        _event_tx: broadcast::Sender<BleEvent>,
    ) -> Result<(), AppError> {
        self.simulator.start_gesture_monitoring(device_id).await
    }

    async fn stop_gesture_monitoring(&self, device_id: &str) -> Result<(), AppError> {
        self.simulator.stop_gesture_monitoring(device_id).await
    }

    async fn reset_simulator(&self, device_id: &str) -> Result<(), AppError> {
        tracing::info!("Resetting simulator: {}", device_id);
        // For simulated devices, we can just log the reset
        Ok(())
    }

    async fn get_simulator_health(
        &self,
        device_id: &str,
    ) -> Result<crate::ble::SimulatorStatus, AppError> {
        // Check if the device exists in our simulator
        let rings = self.simulator.get_rings().await;
        if rings.contains(&device_id.to_string()) {
            Ok(crate::ble::SimulatorStatus::Healthy)
        } else {
            Err(AppError::Ble("Simulator not found".to_string()))
        }
    }

    async fn get_connection_logs(&self, device_id: &str) -> Result<Vec<String>, AppError> {
        Ok(vec![
            format!("Simulator connection log for {}", device_id),
            "Simulator initialized".to_string(),
            "Virtual BLE services started".to_string(),
            "Ready for connections".to_string(),
            "Health check: OK".to_string(),
        ])
    }
}

/// Create a pre-configured device simulator with sample devices
pub async fn create_test_simulator() -> (Arc<DeviceSimulator>, broadcast::Receiver<BleEvent>) {
    let (simulator, event_rx) = DeviceSimulator::new();
    let simulator = Arc::new(simulator);

    // Add some sample rings
    simulator
        .add_ring("sim-ring-001".to_string(), "Simulated Ring 1".to_string())
        .await;
    simulator
        .add_ring("sim-ring-002".to_string(), "Simulated Ring 2".to_string())
        .await;
    simulator
        .add_ring("sim-ring-003".to_string(), "Test Ring".to_string())
        .await;

    // Start simulation
    simulator.start_simulation().await;

    (simulator, event_rx)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_device_simulator() {
        let (simulator, mut event_rx) = DeviceSimulator::new();

        // Add a ring
        simulator
            .add_ring("test-ring".to_string(), "Test Ring".to_string())
            .await;

        // Should receive device found event
        let event = event_rx.recv().await.unwrap();
        assert!(matches!(event, BleEvent::DeviceFound(_)));

        // Get rings
        let rings = simulator.get_rings().await;
        assert_eq!(rings.len(), 1);
        assert_eq!(rings[0], "test-ring");
    }

    #[tokio::test]
    async fn test_simulated_ring_manager() {
        let (simulator, _) = DeviceSimulator::new();
        let simulator = Arc::new(simulator);
        let manager = SimulatedRingManager::new(simulator.clone());

        // Add a ring
        simulator
            .add_ring("test-ring".to_string(), "Test Ring".to_string())
            .await;

        // Test scanning
        let rings = manager.scan_for_rings().await.unwrap();
        assert_eq!(rings.len(), 1);

        // Test pairing
        manager.pair_ring("test-ring").await.unwrap();

        // Test status
        let status = manager.get_ring_status("test-ring").await.unwrap();
        assert!(status.is_some());
        assert!(status.unwrap().is_connected);
    }
}
