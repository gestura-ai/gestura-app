//! BLE integration for Haptic Harmony ring
//! Provides pairing, gesture detection, haptic feedback, and OTA updates

use crate::{AppConfig, AppError};
#[cfg(feature = "ble")]
use futures::StreamExt;
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

/// Real BLE ring manager using btleplug
#[cfg(feature = "ble")]
pub struct BtleRingManager {
    central: std::sync::Arc<tokio::sync::Mutex<Option<btleplug::platform::Manager>>>,
}

#[cfg(feature = "ble")]
impl Default for BtleRingManager {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(feature = "ble")]
impl BtleRingManager {
    pub fn new() -> Self {
        Self {
            central: std::sync::Arc::new(tokio::sync::Mutex::new(None)),
        }
    }

    async fn get_central(&self) -> Result<btleplug::platform::Manager, AppError> {
        let mut central_guard = self.central.lock().await;
        if central_guard.is_none() {
            let manager = btleplug::platform::Manager::new()
                .await
                .map_err(|e| AppError::Ble(e.to_string()))?;
            *central_guard = Some(manager);
        }
        Ok(central_guard.as_ref().unwrap().clone())
    }

    /// Scan for BLE devices (helper method)
    async fn scan_devices(&self, simulators_only: bool) -> Result<Vec<String>, AppError> {
        use btleplug::api::{Central as _, Manager as _, Peripheral as _, ScanFilter};

        let manager = self.get_central().await?;
        let adapters = manager
            .adapters()
            .await
            .map_err(|e| AppError::Ble(e.to_string()))?;

        if adapters.is_empty() {
            return Err(AppError::Ble("No Bluetooth adapters found".to_string()));
        }

        let central = &adapters[0];
        central
            .start_scan(ScanFilter::default())
            .await
            .map_err(|e| AppError::Ble(e.to_string()))?;

        // Scan for 5 seconds
        tokio::time::sleep(tokio::time::Duration::from_secs(5)).await;

        let peripherals = central
            .peripherals()
            .await
            .map_err(|e| AppError::Ble(e.to_string()))?;

        let mut devices = Vec::new();
        for peripheral in peripherals {
            if let Ok(Some(properties)) = peripheral.properties().await
                && let Some(name) = properties.local_name
            {
                let is_simulator = name.contains("Simulator");
                let is_haptic_ring = name.contains("Haptic") || name.contains("Ring");

                if is_haptic_ring {
                    if simulators_only && is_simulator {
                        devices.push(peripheral.id().to_string());
                        tracing::info!("Found simulator: {}", name);
                    } else if !simulators_only && !is_simulator {
                        devices.push(peripheral.id().to_string());
                        tracing::info!("Found ring: {}", name);
                    }
                }
            }
        }

        central
            .stop_scan()
            .await
            .map_err(|e| AppError::Ble(e.to_string()))?;

        Ok(devices)
    }

    /// Parse gesture data from BLE notification (helper method)
    fn parse_gesture_data(data: &[u8]) -> Option<GestureType> {
        if data.is_empty() {
            return None;
        }

        match data[0] {
            0x01 => Some(GestureType::Tap),
            0x02 => Some(GestureType::DoubleTap),
            0x03 => Some(GestureType::TiltLeft),
            0x04 => Some(GestureType::TiltRight),
            0x05 => Some(GestureType::TiltUp),
            0x06 => Some(GestureType::TiltDown),
            _ => None,
        }
    }
}

#[cfg(feature = "ble")]
#[async_trait::async_trait]
impl RingManager for BtleRingManager {
    async fn scan_for_rings(&self) -> Result<Vec<String>, AppError> {
        self.scan_devices(false).await
    }

    async fn scan_for_simulators(&self) -> Result<Vec<String>, AppError> {
        self.scan_devices(true).await
    }

    async fn pair_ring(&self, device_id: &str) -> Result<(), AppError> {
        use btleplug::api::{Central as _, Manager as _, Peripheral as _};

        tracing::info!("Pairing with ring: {}", device_id);

        let manager = self.get_central().await?;
        let adapters = manager
            .adapters()
            .await
            .map_err(|e| AppError::Ble(e.to_string()))?;

        if adapters.is_empty() {
            return Err(AppError::Ble("No Bluetooth adapters found".to_string()));
        }

        let central = &adapters[0];
        let peripherals = central
            .peripherals()
            .await
            .map_err(|e| AppError::Ble(e.to_string()))?;

        // Find the device by ID
        for peripheral in peripherals {
            if peripheral.id().to_string() == device_id {
                tracing::info!("Found device, connecting...");

                // Connect to the device
                peripheral
                    .connect()
                    .await
                    .map_err(|e| AppError::Ble(format!("Failed to connect: {}", e)))?;

                tracing::info!("Connected, discovering services...");

                // Discover services
                peripheral
                    .discover_services()
                    .await
                    .map_err(|e| AppError::Ble(format!("Failed to discover services: {}", e)))?;

                tracing::info!("Services discovered, pairing complete");
                return Ok(());
            }
        }

        Err(AppError::Ble(format!("Device not found: {}", device_id)))
    }

    async fn get_ring_status(&self, device_id: &str) -> Result<Option<RingStatus>, AppError> {
        use btleplug::api::{Central as _, Manager as _, Peripheral as _};

        let is_simulator = device_id.contains("simulator") || device_id.contains("Simulator");

        let manager = self.get_central().await?;
        let adapters = manager
            .adapters()
            .await
            .map_err(|e| AppError::Ble(e.to_string()))?;

        if adapters.is_empty() {
            return Ok(None);
        }

        let central = &adapters[0];
        let peripherals = central
            .peripherals()
            .await
            .map_err(|e| AppError::Ble(e.to_string()))?;

        // Find the device and check its status
        for peripheral in peripherals {
            if peripheral.id().to_string() == device_id {
                let is_connected = peripheral.is_connected().await.unwrap_or(false);
                let properties = peripheral.properties().await.ok().flatten();

                let mut battery_level = None;
                let mut firmware_version = None;

                // If connected, try to read characteristics
                if is_connected {
                    // Read battery level if available
                    if peripheral.characteristics().into_iter().next().is_some() {
                        // Battery level would be read from characteristic
                        battery_level = Some(75); // Placeholder until we read actual value
                    }
                    firmware_version =
                        Some(if is_simulator { "SIM-1.2.0" } else { "1.2.0" }.to_string());
                }

                return Ok(Some(RingStatus {
                    device_id: device_id.to_string(),
                    battery_level,
                    firmware_version,
                    is_connected,
                    last_seen: chrono::Utc::now(),
                    is_simulator,
                    simulator_status: if is_simulator {
                        Some(SimulatorStatus::Healthy)
                    } else {
                        None
                    },
                    connection_logs: vec![
                        format!(
                            "Connection status: {}",
                            if is_connected {
                                "connected"
                            } else {
                                "disconnected"
                            }
                        ),
                        format!(
                            "Device name: {}",
                            properties
                                .and_then(|p| p.local_name)
                                .unwrap_or_else(|| "Unknown".to_string())
                        ),
                    ],
                }));
            }
        }

        Ok(None)
    }

    async fn send_haptic(
        &self,
        device_id: &str,
        pattern: crate::haptics::HapticRequest,
    ) -> Result<(), AppError> {
        use btleplug::api::{Central as _, Manager as _, Peripheral as _, WriteType};
        use uuid::Uuid;

        tracing::info!("Sending haptic to {}: {:?}", device_id, pattern);

        let manager = self.get_central().await?;
        let adapters = manager
            .adapters()
            .await
            .map_err(|e| AppError::Ble(e.to_string()))?;

        if adapters.is_empty() {
            return Err(AppError::Ble("No Bluetooth adapters found".to_string()));
        }

        let central = &adapters[0];
        let peripherals = central
            .peripherals()
            .await
            .map_err(|e| AppError::Ble(e.to_string()))?;

        // Find the device
        for peripheral in peripherals {
            if peripheral.id().to_string() == device_id {
                // Ensure connected
                if !peripheral.is_connected().await.unwrap_or(false) {
                    peripheral
                        .connect()
                        .await
                        .map_err(|e| AppError::Ble(format!("Failed to connect: {}", e)))?;
                    peripheral.discover_services().await.map_err(|e| {
                        AppError::Ble(format!("Failed to discover services: {}", e))
                    })?;
                }

                // Find haptic command characteristic
                let haptic_uuid = Uuid::parse_str(ring_constants::HAPTIC_COMMAND_UUID)
                    .map_err(|e| AppError::Ble(format!("Invalid haptic UUID: {}", e)))?;

                for char in peripheral.characteristics() {
                    if char.uuid == haptic_uuid {
                        // Encode haptic pattern as bytes
                        let data = encode_haptic_pattern(&pattern);

                        peripheral
                            .write(&char, &data, WriteType::WithResponse)
                            .await
                            .map_err(|e| AppError::Ble(format!("Failed to write haptic: {}", e)))?;

                        tracing::info!("Haptic command sent successfully");
                        return Ok(());
                    }
                }

                return Err(AppError::Ble("Haptic characteristic not found".to_string()));
            }
        }

        Err(AppError::Ble(format!("Device not found: {}", device_id)))
    }

    async fn send_test_haptic(
        &self,
        device_id: &str,
        test_pattern: TestHapticPattern,
    ) -> Result<(), AppError> {
        tracing::info!("Sending test haptic to {}: {:?}", device_id, test_pattern);

        // Convert test pattern to haptic request
        let haptic_request = match test_pattern {
            TestHapticPattern::ConnectivityTest => crate::haptics::HapticRequest {
                pattern: crate::haptics::HapticPattern::Pulse,
                intensity: 0.5,
                duration_ms: 200,
                repeat_count: 0,
                repeat_delay_ms: 0,
            },
            TestHapticPattern::LatencyTest => crate::haptics::HapticRequest {
                pattern: crate::haptics::HapticPattern::Pulse,
                intensity: 1.0,
                duration_ms: 50,
                repeat_count: 0,
                repeat_delay_ms: 0,
            },
            TestHapticPattern::IntensityTest { min, max, steps: _ } => {
                crate::haptics::HapticRequest {
                    pattern: crate::haptics::HapticPattern::Ramp,
                    intensity: (min + max) / 2.0,
                    duration_ms: 500,
                    repeat_count: 0,
                    repeat_delay_ms: 0,
                }
            }
            TestHapticPattern::DurationTest { durations } => crate::haptics::HapticRequest {
                pattern: crate::haptics::HapticPattern::Pulse,
                intensity: 0.7,
                duration_ms: durations.first().copied().unwrap_or(100),
                repeat_count: 0,
                repeat_delay_ms: 0,
            },
            TestHapticPattern::ComplexPattern { pattern } => {
                let (intensity, duration) = pattern.first().copied().unwrap_or((0.5, 100));
                crate::haptics::HapticRequest {
                    pattern: crate::haptics::HapticPattern::Custom(0x00),
                    intensity,
                    duration_ms: duration,
                    repeat_count: 0,
                    repeat_delay_ms: 0,
                }
            }
        };

        self.send_haptic(device_id, haptic_request).await
    }

    async fn start_ota_update(
        &self,
        device_id: &str,
        firmware_data: Vec<u8>,
    ) -> Result<(), AppError> {
        use btleplug::api::{Central as _, Manager as _, Peripheral as _, WriteType};
        use uuid::Uuid;

        tracing::info!(
            "Starting OTA update for {}: {} bytes",
            device_id,
            firmware_data.len()
        );

        let manager = self.get_central().await?;
        let adapters = manager
            .adapters()
            .await
            .map_err(|e| AppError::Ble(e.to_string()))?;

        if adapters.is_empty() {
            return Err(AppError::Ble("No Bluetooth adapters found".to_string()));
        }

        let central = &adapters[0];
        let peripherals = central
            .peripherals()
            .await
            .map_err(|e| AppError::Ble(e.to_string()))?;

        for peripheral in peripherals {
            if peripheral.id().to_string() == device_id {
                // Ensure connected
                if !peripheral.is_connected().await.unwrap_or(false) {
                    peripheral
                        .connect()
                        .await
                        .map_err(|e| AppError::Ble(format!("Failed to connect: {}", e)))?;
                    peripheral.discover_services().await.map_err(|e| {
                        AppError::Ble(format!("Failed to discover services: {}", e))
                    })?;
                }

                // Find OTA characteristic
                let ota_uuid = Uuid::parse_str(ring_constants::OTA_UPDATE_UUID)
                    .map_err(|e| AppError::Ble(format!("Invalid OTA UUID: {}", e)))?;

                for char in peripheral.characteristics() {
                    if char.uuid == ota_uuid {
                        // Send firmware in chunks
                        const CHUNK_SIZE: usize = 512;
                        for (i, chunk) in firmware_data.chunks(CHUNK_SIZE).enumerate() {
                            peripheral
                                .write(&char, chunk, WriteType::WithResponse)
                                .await
                                .map_err(|e| {
                                    AppError::Ble(format!("Failed to write OTA chunk {}: {}", i, e))
                                })?;

                            tracing::debug!(
                                "OTA chunk {}/{} sent",
                                i + 1,
                                firmware_data.len().div_ceil(CHUNK_SIZE)
                            );
                        }

                        tracing::info!("OTA update completed successfully");
                        return Ok(());
                    }
                }

                return Err(AppError::Ble("OTA characteristic not found".to_string()));
            }
        }

        Err(AppError::Ble(format!("Device not found: {}", device_id)))
    }

    async fn start_gesture_monitoring(
        &self,
        device_id: &str,
        event_tx: tokio::sync::broadcast::Sender<BleEvent>,
    ) -> Result<(), AppError> {
        use btleplug::api::{Central as _, Manager as _, Peripheral as _};

        let manager = self.get_central().await?;
        let adapters = manager
            .adapters()
            .await
            .map_err(|e| AppError::Ble(e.to_string()))?;

        if adapters.is_empty() {
            return Err(AppError::Ble("No Bluetooth adapters found".to_string()));
        }

        let central = &adapters[0];
        let peripherals = central
            .peripherals()
            .await
            .map_err(|e| AppError::Ble(e.to_string()))?;

        // Find the specific device
        for peripheral in peripherals {
            if peripheral.id().to_string() == device_id {
                // Connect and subscribe to gesture characteristic
                peripheral
                    .connect()
                    .await
                    .map_err(|e| AppError::Ble(e.to_string()))?;
                peripheral
                    .discover_services()
                    .await
                    .map_err(|e| AppError::Ble(e.to_string()))?;

                // Find gesture characteristic
                let services = peripheral.services();
                for service in services {
                    for characteristic in &service.characteristics {
                        if characteristic.uuid.to_string() == ring_constants::GESTURE_EVENT_UUID {
                            // Subscribe to notifications
                            peripheral
                                .subscribe(characteristic)
                                .await
                                .map_err(|e| AppError::Ble(e.to_string()))?;

                            // Start notification handler
                            let event_tx_clone = event_tx.clone();
                            let peripheral_clone = peripheral.clone();
                            tokio::spawn(async move {
                                let mut notification_stream =
                                    peripheral_clone.notifications().await.unwrap();
                                while let Some(data) = notification_stream.next().await {
                                    if let Some(gesture) = Self::parse_gesture_data(&data.value) {
                                        let _ =
                                            event_tx_clone.send(BleEvent::GestureDetected(gesture));
                                    }
                                }
                            });

                            tracing::info!("Started gesture monitoring for {}", device_id);
                            return Ok(());
                        }
                    }
                }
            }
        }

        Err(AppError::Ble(format!(
            "Device {} not found or gesture characteristic not available",
            device_id
        )))
    }

    async fn stop_gesture_monitoring(&self, device_id: &str) -> Result<(), AppError> {
        use btleplug::api::{Central as _, Manager as _, Peripheral as _};

        tracing::info!("Stopping gesture monitoring for {}", device_id);

        let manager = self.get_central().await?;
        let adapters = manager
            .adapters()
            .await
            .map_err(|e| AppError::Ble(e.to_string()))?;

        if adapters.is_empty() {
            return Err(AppError::Ble("No Bluetooth adapters found".to_string()));
        }

        let central = &adapters[0];
        let peripherals = central
            .peripherals()
            .await
            .map_err(|e| AppError::Ble(e.to_string()))?;

        for peripheral in peripherals {
            if peripheral.id().to_string() == device_id {
                // Find and unsubscribe from gesture characteristic
                let services = peripheral.services();
                for service in services {
                    for characteristic in &service.characteristics {
                        if characteristic.uuid.to_string() == ring_constants::GESTURE_EVENT_UUID {
                            peripheral
                                .unsubscribe(characteristic)
                                .await
                                .map_err(|e| AppError::Ble(e.to_string()))?;

                            tracing::info!("Stopped gesture monitoring for {}", device_id);
                            return Ok(());
                        }
                    }
                }
            }
        }

        Ok(())
    }

    async fn reset_simulator(&self, device_id: &str) -> Result<(), AppError> {
        use btleplug::api::{Central as _, Manager as _, Peripheral as _};

        if !device_id.contains("simulator") && !device_id.contains("Simulator") {
            return Err(AppError::Ble("Device is not a simulator".to_string()));
        }

        tracing::info!("Resetting simulator: {}", device_id);

        let manager = self.get_central().await?;
        let adapters = manager
            .adapters()
            .await
            .map_err(|e| AppError::Ble(e.to_string()))?;

        if adapters.is_empty() {
            return Err(AppError::Ble("No Bluetooth adapters found".to_string()));
        }

        let central = &adapters[0];
        let peripherals = central
            .peripherals()
            .await
            .map_err(|e| AppError::Ble(e.to_string()))?;

        for peripheral in peripherals {
            if peripheral.id().to_string() == device_id {
                // Disconnect and reconnect to reset
                if peripheral.is_connected().await.unwrap_or(false) {
                    peripheral
                        .disconnect()
                        .await
                        .map_err(|e| AppError::Ble(format!("Failed to disconnect: {}", e)))?;
                }

                // Wait briefly before reconnecting
                tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;

                peripheral
                    .connect()
                    .await
                    .map_err(|e| AppError::Ble(format!("Failed to reconnect: {}", e)))?;

                peripheral
                    .discover_services()
                    .await
                    .map_err(|e| AppError::Ble(format!("Failed to discover services: {}", e)))?;

                tracing::info!("Simulator reset complete: {}", device_id);
                return Ok(());
            }
        }

        Err(AppError::Ble(format!("Simulator not found: {}", device_id)))
    }

    async fn get_simulator_health(&self, device_id: &str) -> Result<SimulatorStatus, AppError> {
        use btleplug::api::{Central as _, Manager as _, Peripheral as _};

        if !device_id.contains("simulator") && !device_id.contains("Simulator") {
            return Err(AppError::Ble("Device is not a simulator".to_string()));
        }

        let manager = self.get_central().await?;
        let adapters = manager
            .adapters()
            .await
            .map_err(|e| AppError::Ble(e.to_string()))?;

        if adapters.is_empty() {
            return Ok(SimulatorStatus::Offline);
        }

        let central = &adapters[0];
        let peripherals = central
            .peripherals()
            .await
            .map_err(|e| AppError::Ble(e.to_string()))?;

        for peripheral in peripherals {
            if peripheral.id().to_string() == device_id {
                let is_connected = peripheral.is_connected().await.unwrap_or(false);

                if !is_connected {
                    return Ok(SimulatorStatus::Offline);
                }

                // Check if services are available
                let services = peripheral.services();
                if services.is_empty() {
                    return Ok(SimulatorStatus::Degraded);
                }

                // Check for required characteristics
                let mut has_haptic = false;
                let mut has_gesture = false;

                for service in services {
                    for char in &service.characteristics {
                        if char.uuid.to_string() == ring_constants::HAPTIC_COMMAND_UUID {
                            has_haptic = true;
                        }
                        if char.uuid.to_string() == ring_constants::GESTURE_EVENT_UUID {
                            has_gesture = true;
                        }
                    }
                }

                if has_haptic && has_gesture {
                    return Ok(SimulatorStatus::Healthy);
                } else {
                    return Ok(SimulatorStatus::Degraded);
                }
            }
        }

        Ok(SimulatorStatus::Offline)
    }

    async fn get_connection_logs(&self, device_id: &str) -> Result<Vec<String>, AppError> {
        use btleplug::api::{Central as _, Manager as _, Peripheral as _};

        let mut logs = Vec::new();
        logs.push(format!("Connection log for {}", device_id));
        logs.push("BLE adapter initialized".to_string());

        let manager = self.get_central().await?;
        let adapters = manager
            .adapters()
            .await
            .map_err(|e| AppError::Ble(e.to_string()))?;

        if adapters.is_empty() {
            logs.push("No Bluetooth adapters found".to_string());
            return Ok(logs);
        }

        logs.push("Bluetooth adapter found".to_string());

        let central = &adapters[0];
        let peripherals = central
            .peripherals()
            .await
            .map_err(|e| AppError::Ble(e.to_string()))?;

        logs.push(format!("Found {} peripherals", peripherals.len()));

        for peripheral in peripherals {
            if peripheral.id().to_string() == device_id {
                logs.push("Device found in peripheral list".to_string());

                let is_connected = peripheral.is_connected().await.unwrap_or(false);
                logs.push(format!(
                    "Connection status: {}",
                    if is_connected {
                        "connected"
                    } else {
                        "disconnected"
                    }
                ));

                if let Ok(Some(props)) = peripheral.properties().await {
                    if let Some(name) = props.local_name {
                        logs.push(format!("Device name: {}", name));
                    }
                    if let Some(rssi) = props.rssi {
                        logs.push(format!("Signal strength (RSSI): {} dBm", rssi));
                    }
                }

                let services = peripheral.services();
                logs.push(format!("Discovered {} services", services.len()));

                return Ok(logs);
            }
        }

        logs.push("Device not found in peripheral list".to_string());
        Ok(logs)
    }
}

/// Encode a haptic pattern into bytes for BLE transmission
#[cfg(feature = "ble")]
fn encode_haptic_pattern(pattern: &crate::haptics::HapticRequest) -> Vec<u8> {
    let mut data = Vec::with_capacity(12);

    // Byte 0: Pattern type
    let pattern_byte = match pattern.pattern {
        crate::haptics::HapticPattern::Click => 0x00,
        crate::haptics::HapticPattern::Pulse => 0x01,
        crate::haptics::HapticPattern::Ramp => 0x02,
        crate::haptics::HapticPattern::Heartbeat => 0x03,
        crate::haptics::HapticPattern::Notification => 0x04,
        crate::haptics::HapticPattern::Alert => 0x05,
        crate::haptics::HapticPattern::Custom(code) => code,
    };
    data.push(pattern_byte);

    // Byte 1: Intensity (0-255 scaled from 0.0-1.0)
    let intensity_byte = (pattern.intensity.clamp(0.0, 1.0) * 255.0) as u8;
    data.push(intensity_byte);

    // Bytes 2-5: Duration in milliseconds (u32 little-endian)
    data.extend_from_slice(&pattern.duration_ms.to_le_bytes());

    // Byte 6: Repeat count
    data.push(pattern.repeat_count);

    // Bytes 7-10: Repeat delay in milliseconds (u32 little-endian)
    data.extend_from_slice(&pattern.repeat_delay_ms.to_le_bytes());

    data
}

/// Create appropriate ring manager based on features
pub fn create_ring_manager() -> Box<dyn RingManager> {
    #[cfg(feature = "ble")]
    {
        Box::new(BtleRingManager::new())
    }
    #[cfg(not(feature = "ble"))]
    {
        Box::new(MockRingManager)
    }
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
    /// Characteristic UUID for OTA updates
    pub const OTA_UPDATE_UUID: &str = "12345678-1234-5678-9abc-123456789ac0";
}
