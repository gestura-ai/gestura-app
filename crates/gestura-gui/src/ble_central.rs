//! Real BLE central integration for Haptic Harmony rings and simulators.

use crate::AppError;
use crate::ble::{
    BleEvent, GestureType, RingManager, RingStatus, SimulatorStatus, TestHapticPattern,
    ring_constants,
};
use crate::haptics::{HapticPattern, HapticRequest};
use btleplug::api::{
    Central, Characteristic, Manager as _, Peripheral as _, ScanFilter, ValueNotification,
    WriteType,
};
use btleplug::platform::{Adapter, Manager, Peripheral};
use futures::StreamExt;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::RwLock;
use tokio::task::JoinHandle;
use uuid::Uuid;

const PROTOCOL_VERSION: &str = "0.1.0";
const EXTERNAL_SCAN_WINDOW: Duration = Duration::from_secs(2);

#[derive(Clone)]
struct CachedPeripheral {
    peripheral: Peripheral,
    name: Option<String>,
    is_simulator: bool,
    last_seen: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, Deserialize)]
struct BleGestureFrame {
    gesture_type: String,
    data: Vec<u8>,
}

#[derive(Debug, Deserialize)]
struct BleBatteryFrame {
    level: u8,
}

#[derive(Debug, Deserialize)]
struct StateSnapshotFrame {
    firmware_version: String,
    degraded_modes: Vec<serde_json::Value>,
    revocation_reason: Option<String>,
    privileged_actions_enabled: bool,
}

#[derive(Debug, Deserialize)]
struct ProtocolEnvelope<T> {
    payload: T,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "event_kind", content = "event", rename_all = "snake_case")]
enum SimulatorEventFrame {
    Gesture(SemanticGestureEventFrame),
}

#[derive(Debug, Deserialize)]
struct SemanticGestureEventFrame {
    gesture: SemanticGestureFrame,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "gesture_kind", rename_all = "snake_case")]
enum SemanticGestureFrame {
    Tap,
    DoubleTap,
    Hold { duration_ms: u64 },
    Slide { direction: SemanticSlideDirection },
    Tilt { angle_degrees: f32 },
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
enum SemanticSlideDirection {
    Up,
    Down,
    Left,
    Right,
}

#[derive(Debug, Serialize)]
struct CommandEnvelope<'a, T> {
    protocol_version: &'a str,
    message_kind: &'a str,
    message_id: String,
    sequence: u64,
    timestamp_ms: u64,
    payload: T,
}

#[derive(Debug, Serialize)]
#[serde(tag = "command_kind", content = "command", rename_all = "snake_case")]
enum SimulatorCommandPayload {
    Haptic(HapticCommandPayload),
}

#[derive(Debug, Serialize)]
struct HapticCommandPayload {
    pattern: SemanticHapticPattern,
}

#[derive(Debug, Serialize)]
#[serde(tag = "pattern_kind", rename_all = "snake_case")]
enum SemanticHapticPattern {
    Notify,
    Success,
    Error,
    Custom { intensity: f32, duration_ms: u64 },
}

/// BLE central manager backed by `btleplug`.
///
/// This manager scans for real BLE peripherals matching the Haptic Harmony UUIDs,
/// connects to them, and translates notifications into Gestura's existing BLE events.
pub struct ExternalBleRingManager {
    adapter: Adapter,
    devices: Arc<RwLock<HashMap<String, CachedPeripheral>>>,
    connection_logs: Arc<RwLock<HashMap<String, Vec<String>>>>,
    gesture_tasks: Arc<RwLock<HashMap<String, JoinHandle<()>>>>,
}

impl ExternalBleRingManager {
    /// Creates a new external BLE ring manager from the first available adapter.
    pub async fn new() -> Result<Self, AppError> {
        let manager = Manager::new().await.map_err(map_ble_error)?;
        let adapter = manager
            .adapters()
            .await
            .map_err(map_ble_error)?
            .into_iter()
            .next()
            .ok_or_else(|| AppError::Ble("No BLE adapter available".to_string()))?;

        Ok(Self {
            adapter,
            devices: Arc::new(RwLock::new(HashMap::new())),
            connection_logs: Arc::new(RwLock::new(HashMap::new())),
            gesture_tasks: Arc::new(RwLock::new(HashMap::new())),
        })
    }

    pub(crate) async fn can_handle_device(&self, device_id: &str) -> bool {
        if self.devices.read().await.contains_key(device_id) {
            return true;
        }

        self.adapter
            .peripherals()
            .await
            .map(|peripherals| peripherals.iter().any(|p| p.id().to_string() == device_id))
            .unwrap_or(false)
    }

    pub(crate) async fn refresh_device(&self, device_id: &str) -> Result<bool, AppError> {
        self.scan_devices(false).await?;
        Ok(self.devices.read().await.contains_key(device_id))
    }

    async fn scan_devices(&self, simulators_only: bool) -> Result<Vec<String>, AppError> {
        self.adapter
            .start_scan(ScanFilter::default())
            .await
            .map_err(map_ble_error)?;
        tokio::time::sleep(EXTERNAL_SCAN_WINDOW).await;

        let mut device_ids = Vec::new();
        for peripheral in self.adapter.peripherals().await.map_err(map_ble_error)? {
            let Some(properties) = peripheral.properties().await.map_err(map_ble_error)? else {
                continue;
            };

            let name = properties.local_name.clone();
            let has_service = properties
                .services
                .iter()
                .any(|uuid| *uuid == haptic_service_uuid());
            let is_gestura_named = name.as_deref().map(is_ring_name).unwrap_or(false);
            let is_simulator = name.as_deref().map(is_simulator_name).unwrap_or(false);

            if !(has_service || is_gestura_named) {
                continue;
            }

            if simulators_only && !is_simulator {
                continue;
            }

            let device_id = peripheral.id().to_string();
            self.record_log(
                &device_id,
                format!("Discovered {}", display_name(name.as_deref(), &device_id)),
            )
            .await;
            self.devices.write().await.insert(
                device_id.clone(),
                CachedPeripheral {
                    peripheral,
                    name,
                    is_simulator,
                    last_seen: chrono::Utc::now(),
                },
            );
            device_ids.push(device_id);
        }

        let _ = self.adapter.stop_scan().await;
        Ok(device_ids)
    }

    async fn get_cached_peripheral(
        &self,
        device_id: &str,
    ) -> Result<Option<CachedPeripheral>, AppError> {
        if let Some(device) = self.devices.read().await.get(device_id).cloned() {
            return Ok(Some(device));
        }

        let _ = self.refresh_device(device_id).await?;
        Ok(self.devices.read().await.get(device_id).cloned())
    }

    async fn ensure_connected(&self, device_id: &str) -> Result<CachedPeripheral, AppError> {
        let cached = self
            .get_cached_peripheral(device_id)
            .await?
            .ok_or_else(|| AppError::Ble(format!("Unknown BLE device: {device_id}")))?;

        let peripheral = cached.peripheral.clone();
        if !peripheral.is_connected().await.map_err(map_ble_error)? {
            self.record_log(device_id, "Connecting to BLE peripheral".to_string())
                .await;
            peripheral.connect().await.map_err(map_ble_error)?;
        }

        peripheral
            .discover_services()
            .await
            .map_err(map_ble_error)?;
        self.record_log(device_id, "Discovered BLE services".to_string())
            .await;

        Ok(cached)
    }

    async fn read_status_fields(
        &self,
        device_id: &str,
        peripheral: &Peripheral,
    ) -> Result<(Option<u8>, Option<String>, Option<SimulatorStatus>), AppError> {
        let battery_level = if let Some(characteristic) =
            find_characteristic(peripheral, ring_constants::BATTERY_LEVEL_UUID)
        {
            match peripheral.read(&characteristic).await {
                Ok(bytes) => parse_battery_level(&bytes),
                Err(error) => {
                    self.record_log(device_id, format!("Battery read failed: {error}"))
                        .await;
                    None
                }
            }
        } else {
            None
        };

        let state = if let Some(characteristic) =
            find_characteristic(peripheral, ring_constants::STATE_SNAPSHOT_UUID)
        {
            match peripheral.read(&characteristic).await {
                Ok(bytes) => parse_state_snapshot(&bytes),
                Err(error) => {
                    self.record_log(device_id, format!("State snapshot read failed: {error}"))
                        .await;
                    None
                }
            }
        } else {
            None
        };

        Ok((
            battery_level,
            state
                .as_ref()
                .map(|snapshot| snapshot.firmware_version.clone()),
            state.map(snapshot_to_status),
        ))
    }

    async fn write_haptic_request(
        &self,
        device_id: &str,
        request: HapticRequest,
    ) -> Result<(), AppError> {
        let cached = self.ensure_connected(device_id).await?;
        let characteristic =
            find_characteristic(&cached.peripheral, ring_constants::HAPTIC_COMMAND_UUID)
                .ok_or_else(|| {
                    AppError::Ble("Haptic command characteristic not found".to_string())
                })?;
        let payload = encode_haptic_request(&request)?;
        cached
            .peripheral
            .write(&characteristic, &payload, WriteType::WithResponse)
            .await
            .map_err(map_ble_error)?;
        self.record_log(
            device_id,
            format!("Sent haptic command ({} bytes)", payload.len()),
        )
        .await;
        Ok(())
    }
}

#[async_trait::async_trait]
impl RingManager for ExternalBleRingManager {
    async fn scan_for_rings(&self) -> Result<Vec<String>, AppError> {
        self.scan_devices(false).await
    }

    async fn scan_for_simulators(&self) -> Result<Vec<String>, AppError> {
        self.scan_devices(true).await
    }

    async fn pair_ring(&self, device_id: &str) -> Result<(), AppError> {
        let cached = self.ensure_connected(device_id).await?;
        self.record_log(
            device_id,
            format!(
                "Paired with {}",
                display_name(cached.name.as_deref(), device_id)
            ),
        )
        .await;
        Ok(())
    }

    async fn get_ring_status(&self, device_id: &str) -> Result<Option<RingStatus>, AppError> {
        let Some(cached) = self.get_cached_peripheral(device_id).await? else {
            return Ok(None);
        };

        let is_connected = cached
            .peripheral
            .is_connected()
            .await
            .map_err(map_ble_error)?;
        let (battery_level, firmware_version, simulator_status) = if is_connected {
            self.read_status_fields(device_id, &cached.peripheral)
                .await?
        } else {
            (None, None, None)
        };

        Ok(Some(RingStatus {
            device_id: device_id.to_string(),
            battery_level,
            firmware_version,
            is_connected,
            last_seen: cached.last_seen,
            is_simulator: cached.is_simulator,
            simulator_status,
            connection_logs: self.get_connection_logs(device_id).await?,
        }))
    }

    async fn send_haptic(&self, device_id: &str, pattern: HapticRequest) -> Result<(), AppError> {
        self.write_haptic_request(device_id, pattern).await
    }

    async fn send_test_haptic(
        &self,
        device_id: &str,
        test_pattern: TestHapticPattern,
    ) -> Result<(), AppError> {
        self.write_haptic_request(device_id, test_pattern_to_request(test_pattern))
            .await
    }

    async fn start_ota_update(
        &self,
        device_id: &str,
        firmware_data: Vec<u8>,
    ) -> Result<(), AppError> {
        let cached = self.ensure_connected(device_id).await?;
        let characteristic =
            find_characteristic(&cached.peripheral, ring_constants::OTA_UPDATE_UUID)
                .ok_or_else(|| AppError::Ble("OTA characteristic not found".to_string()))?;
        cached
            .peripheral
            .write(&characteristic, &firmware_data, WriteType::WithResponse)
            .await
            .map_err(map_ble_error)?;
        self.record_log(
            device_id,
            format!("Sent OTA payload ({} bytes)", firmware_data.len()),
        )
        .await;
        Ok(())
    }

    async fn start_gesture_monitoring(
        &self,
        device_id: &str,
        event_tx: tokio::sync::broadcast::Sender<BleEvent>,
    ) -> Result<(), AppError> {
        let cached = self.ensure_connected(device_id).await?;
        let peripheral = cached.peripheral.clone();

        {
            let tasks = self.gesture_tasks.read().await;
            if tasks.get(device_id).is_some_and(|task| !task.is_finished()) {
                return Ok(());
            }
        }

        let gesture_characteristic =
            find_characteristic(&peripheral, ring_constants::GESTURE_EVENT_UUID).ok_or_else(
                || AppError::Ble("Gesture event characteristic not found".to_string()),
            )?;
        let battery_characteristic =
            find_characteristic(&peripheral, ring_constants::BATTERY_LEVEL_UUID);
        let state_characteristic =
            find_characteristic(&peripheral, ring_constants::STATE_SNAPSHOT_UUID);

        let mut notifications = peripheral.notifications().await.map_err(map_ble_error)?;
        peripheral
            .subscribe(&gesture_characteristic)
            .await
            .map_err(map_ble_error)?;
        if let Some(characteristic) = &battery_characteristic {
            let _ = peripheral.subscribe(characteristic).await;
        }
        if let Some(characteristic) = &state_characteristic {
            let _ = peripheral.subscribe(characteristic).await;
        }

        let device_id_owned = device_id.to_string();
        let logs = self.connection_logs.clone();
        let handle = tokio::spawn(async move {
            while let Some(notification) = notifications.next().await {
                handle_notification(&device_id_owned, notification, &event_tx, &logs).await;
            }
        });

        self.gesture_tasks
            .write()
            .await
            .insert(device_id.to_string(), handle);
        self.record_log(device_id, "Started BLE notification monitoring".to_string())
            .await;
        Ok(())
    }

    async fn stop_gesture_monitoring(&self, device_id: &str) -> Result<(), AppError> {
        if let Some(handle) = self.gesture_tasks.write().await.remove(device_id) {
            handle.abort();
        }

        if let Some(cached) = self.get_cached_peripheral(device_id).await? {
            for uuid in [
                ring_constants::GESTURE_EVENT_UUID,
                ring_constants::BATTERY_LEVEL_UUID,
                ring_constants::STATE_SNAPSHOT_UUID,
            ] {
                if let Some(characteristic) = find_characteristic(&cached.peripheral, uuid) {
                    let _ = cached.peripheral.unsubscribe(&characteristic).await;
                }
            }
        }

        self.record_log(device_id, "Stopped BLE notification monitoring".to_string())
            .await;
        Ok(())
    }

    async fn reset_simulator(&self, device_id: &str) -> Result<(), AppError> {
        self.stop_gesture_monitoring(device_id).await?;
        if let Some(cached) = self.get_cached_peripheral(device_id).await?
            && cached
                .peripheral
                .is_connected()
                .await
                .map_err(map_ble_error)?
        {
            cached
                .peripheral
                .disconnect()
                .await
                .map_err(map_ble_error)?;
        }
        self.record_log(device_id, "Disconnected BLE simulator".to_string())
            .await;
        Ok(())
    }

    async fn get_simulator_health(&self, device_id: &str) -> Result<SimulatorStatus, AppError> {
        let cached = self
            .get_cached_peripheral(device_id)
            .await?
            .ok_or_else(|| AppError::Ble(format!("Unknown BLE simulator: {device_id}")))?;
        if !cached.is_simulator {
            return Err(AppError::Ble("Not a simulator device".to_string()));
        }

        if !cached
            .peripheral
            .is_connected()
            .await
            .map_err(map_ble_error)?
        {
            return Ok(SimulatorStatus::Offline);
        }

        let (_, _, simulator_status) = self
            .read_status_fields(device_id, &cached.peripheral)
            .await?;
        Ok(simulator_status.unwrap_or(SimulatorStatus::Healthy))
    }

    async fn get_connection_logs(&self, device_id: &str) -> Result<Vec<String>, AppError> {
        Ok(self
            .connection_logs
            .read()
            .await
            .get(device_id)
            .cloned()
            .unwrap_or_default())
    }
}

/// Hybrid ring manager that prefers real external BLE devices when available and
/// falls back to the built-in simulator runtime otherwise.
pub struct HybridRingManager {
    external: Option<Arc<ExternalBleRingManager>>,
    internal: Arc<dyn RingManager>,
}

impl HybridRingManager {
    /// Creates a hybrid manager from an optional external BLE manager and a
    /// required internal fallback manager.
    pub fn new(
        external: Option<Arc<ExternalBleRingManager>>,
        internal: Arc<dyn RingManager>,
    ) -> Self {
        Self { external, internal }
    }

    async fn should_use_external(&self, device_id: &str) -> bool {
        if is_internal_runtime_device(device_id) {
            return false;
        }

        let Some(external) = &self.external else {
            return false;
        };

        if external.can_handle_device(device_id).await {
            return true;
        }

        external.refresh_device(device_id).await.unwrap_or(false)
    }
}

#[async_trait::async_trait]
impl RingManager for HybridRingManager {
    async fn scan_for_rings(&self) -> Result<Vec<String>, AppError> {
        let mut devices = self.internal.scan_for_rings().await?;
        if let Some(external) = &self.external {
            match external.scan_for_rings().await {
                Ok(mut external_devices) => devices.append(&mut external_devices),
                Err(error) => {
                    tracing::warn!(%error, "External BLE scan failed; falling back to internal runtime")
                }
            }
        }
        Ok(dedup_strings(devices))
    }

    async fn scan_for_simulators(&self) -> Result<Vec<String>, AppError> {
        let mut devices = self.internal.scan_for_simulators().await?;
        if let Some(external) = &self.external {
            match external.scan_for_simulators().await {
                Ok(mut external_devices) => devices.append(&mut external_devices),
                Err(error) => {
                    tracing::warn!(%error, "External BLE simulator scan failed; using internal runtime only")
                }
            }
        }
        Ok(dedup_strings(devices))
    }

    async fn pair_ring(&self, device_id: &str) -> Result<(), AppError> {
        if self.should_use_external(device_id).await {
            return self.external.as_ref().unwrap().pair_ring(device_id).await;
        }
        self.internal.pair_ring(device_id).await
    }

    async fn get_ring_status(&self, device_id: &str) -> Result<Option<RingStatus>, AppError> {
        if self.should_use_external(device_id).await {
            return self
                .external
                .as_ref()
                .unwrap()
                .get_ring_status(device_id)
                .await;
        }
        self.internal.get_ring_status(device_id).await
    }

    async fn send_haptic(&self, device_id: &str, pattern: HapticRequest) -> Result<(), AppError> {
        if self.should_use_external(device_id).await {
            return self
                .external
                .as_ref()
                .unwrap()
                .send_haptic(device_id, pattern)
                .await;
        }
        self.internal.send_haptic(device_id, pattern).await
    }

    async fn send_test_haptic(
        &self,
        device_id: &str,
        test_pattern: TestHapticPattern,
    ) -> Result<(), AppError> {
        if self.should_use_external(device_id).await {
            return self
                .external
                .as_ref()
                .unwrap()
                .send_test_haptic(device_id, test_pattern)
                .await;
        }
        self.internal
            .send_test_haptic(device_id, test_pattern)
            .await
    }

    async fn start_ota_update(
        &self,
        device_id: &str,
        firmware_data: Vec<u8>,
    ) -> Result<(), AppError> {
        if self.should_use_external(device_id).await {
            return self
                .external
                .as_ref()
                .unwrap()
                .start_ota_update(device_id, firmware_data)
                .await;
        }
        self.internal
            .start_ota_update(device_id, firmware_data)
            .await
    }

    async fn start_gesture_monitoring(
        &self,
        device_id: &str,
        event_tx: tokio::sync::broadcast::Sender<BleEvent>,
    ) -> Result<(), AppError> {
        if self.should_use_external(device_id).await {
            return self
                .external
                .as_ref()
                .unwrap()
                .start_gesture_monitoring(device_id, event_tx)
                .await;
        }
        self.internal
            .start_gesture_monitoring(device_id, event_tx)
            .await
    }

    async fn stop_gesture_monitoring(&self, device_id: &str) -> Result<(), AppError> {
        if self.should_use_external(device_id).await {
            return self
                .external
                .as_ref()
                .unwrap()
                .stop_gesture_monitoring(device_id)
                .await;
        }
        self.internal.stop_gesture_monitoring(device_id).await
    }

    async fn reset_simulator(&self, device_id: &str) -> Result<(), AppError> {
        if self.should_use_external(device_id).await {
            return self
                .external
                .as_ref()
                .unwrap()
                .reset_simulator(device_id)
                .await;
        }
        self.internal.reset_simulator(device_id).await
    }

    async fn get_simulator_health(&self, device_id: &str) -> Result<SimulatorStatus, AppError> {
        if self.should_use_external(device_id).await {
            return self
                .external
                .as_ref()
                .unwrap()
                .get_simulator_health(device_id)
                .await;
        }
        self.internal.get_simulator_health(device_id).await
    }

    async fn get_connection_logs(&self, device_id: &str) -> Result<Vec<String>, AppError> {
        if self.should_use_external(device_id).await {
            return self
                .external
                .as_ref()
                .unwrap()
                .get_connection_logs(device_id)
                .await;
        }
        self.internal.get_connection_logs(device_id).await
    }
}

fn map_ble_error(error: impl std::fmt::Display) -> AppError {
    AppError::Ble(error.to_string())
}

fn haptic_service_uuid() -> Uuid {
    Uuid::parse_str(ring_constants::HAPTIC_SERVICE_UUID).expect("valid BLE service UUID")
}

fn find_characteristic(peripheral: &Peripheral, uuid: &str) -> Option<Characteristic> {
    let target_uuid = Uuid::parse_str(uuid).ok()?;
    peripheral
        .characteristics()
        .into_iter()
        .find(|characteristic| characteristic.uuid == target_uuid)
}

fn is_ring_name(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    lower.contains("gestura") || lower.contains("haptic harmony")
}

fn is_simulator_name(name: &str) -> bool {
    name.to_ascii_lowercase().contains("simulator")
}

fn display_name(name: Option<&str>, device_id: &str) -> String {
    name.map(ToString::to_string)
        .unwrap_or_else(|| device_id.to_string())
}

fn is_internal_runtime_device(device_id: &str) -> bool {
    device_id.starts_with("sim-ring-") || device_id.starts_with("mock-")
}

fn dedup_strings(items: Vec<String>) -> Vec<String> {
    let mut seen = HashSet::new();
    items
        .into_iter()
        .filter(|item| seen.insert(item.clone()))
        .collect()
}

fn parse_battery_level(bytes: &[u8]) -> Option<u8> {
    serde_json::from_slice::<BleBatteryFrame>(bytes)
        .map(|frame| frame.level)
        .ok()
        .or_else(|| bytes.first().copied())
}

fn parse_state_snapshot(bytes: &[u8]) -> Option<StateSnapshotFrame> {
    serde_json::from_slice(bytes).ok()
}

fn snapshot_to_status(snapshot: StateSnapshotFrame) -> SimulatorStatus {
    if let Some(reason) = snapshot.revocation_reason {
        return SimulatorStatus::Error(reason);
    }
    if !snapshot.degraded_modes.is_empty() || !snapshot.privileged_actions_enabled {
        return SimulatorStatus::Degraded;
    }
    SimulatorStatus::Healthy
}

fn parse_gesture_event(bytes: &[u8]) -> Option<GestureType> {
    let frame: BleGestureFrame = serde_json::from_slice(bytes).ok()?;
    if let Ok(envelope) =
        serde_json::from_slice::<ProtocolEnvelope<SimulatorEventFrame>>(&frame.data)
    {
        let SimulatorEventFrame::Gesture(gesture) = envelope.payload;
        return semantic_gesture_to_app(&gesture.gesture);
    }

    legacy_gesture_to_app(&frame.gesture_type)
}

fn semantic_gesture_to_app(gesture: &SemanticGestureFrame) -> Option<GestureType> {
    match gesture {
        SemanticGestureFrame::Tap => Some(GestureType::Tap),
        SemanticGestureFrame::DoubleTap => Some(GestureType::DoubleTap),
        SemanticGestureFrame::Slide { direction } => Some(match direction {
            SemanticSlideDirection::Up => GestureType::TiltUp,
            SemanticSlideDirection::Down => GestureType::TiltDown,
            SemanticSlideDirection::Left => GestureType::TiltLeft,
            SemanticSlideDirection::Right => GestureType::TiltRight,
        }),
        SemanticGestureFrame::Tilt { angle_degrees } => Some(if *angle_degrees >= 0.0 {
            GestureType::TiltRight
        } else {
            GestureType::TiltLeft
        }),
        SemanticGestureFrame::Hold { duration_ms } => {
            let _ = duration_ms;
            None
        }
    }
}

fn legacy_gesture_to_app(gesture_type: &str) -> Option<GestureType> {
    match gesture_type.to_ascii_lowercase().as_str() {
        "tap" => Some(GestureType::Tap),
        "double_tap" | "doubletap" => Some(GestureType::DoubleTap),
        "tilt_left" | "slide_left" => Some(GestureType::TiltLeft),
        "tilt_right" | "slide_right" => Some(GestureType::TiltRight),
        "tilt_up" | "slide_up" => Some(GestureType::TiltUp),
        "tilt_down" | "slide_down" => Some(GestureType::TiltDown),
        _ => None,
    }
}

fn encode_haptic_request(request: &HapticRequest) -> Result<Vec<u8>, AppError> {
    let payload = CommandEnvelope {
        protocol_version: PROTOCOL_VERSION,
        message_kind: "command",
        message_id: Uuid::new_v4().to_string(),
        sequence: 0,
        timestamp_ms: chrono::Utc::now().timestamp_millis() as u64,
        payload: SimulatorCommandPayload::Haptic(HapticCommandPayload {
            pattern: match request.pattern {
                HapticPattern::Notification => SemanticHapticPattern::Notify,
                HapticPattern::Alert => SemanticHapticPattern::Error,
                HapticPattern::Click => SemanticHapticPattern::Success,
                HapticPattern::Pulse
                | HapticPattern::Ramp
                | HapticPattern::Heartbeat
                | HapticPattern::Custom(_) => SemanticHapticPattern::Custom {
                    intensity: request.intensity,
                    duration_ms: request.duration_ms as u64,
                },
            },
        }),
    };
    serde_json::to_vec(&payload).map_err(Into::into)
}

fn test_pattern_to_request(test_pattern: TestHapticPattern) -> HapticRequest {
    match test_pattern {
        TestHapticPattern::ConnectivityTest => HapticRequest {
            pattern: HapticPattern::Click,
            intensity: 0.5,
            duration_ms: 100,
            repeat_count: 1,
            repeat_delay_ms: 0,
        },
        TestHapticPattern::LatencyTest => HapticRequest {
            pattern: HapticPattern::Pulse,
            intensity: 0.8,
            duration_ms: 50,
            repeat_count: 0,
            repeat_delay_ms: 0,
        },
        TestHapticPattern::IntensityTest { max, .. } => HapticRequest {
            pattern: HapticPattern::Pulse,
            intensity: max,
            duration_ms: 200,
            repeat_count: 0,
            repeat_delay_ms: 0,
        },
        TestHapticPattern::DurationTest { durations } => HapticRequest {
            pattern: HapticPattern::Pulse,
            intensity: 0.7,
            duration_ms: durations.into_iter().max().unwrap_or(100),
            repeat_count: 0,
            repeat_delay_ms: 0,
        },
        TestHapticPattern::ComplexPattern { pattern } => HapticRequest {
            pattern: HapticPattern::Heartbeat,
            intensity: pattern
                .first()
                .map(|(intensity, _)| *intensity)
                .unwrap_or(0.8),
            duration_ms: pattern.iter().map(|(_, duration)| duration).sum(),
            repeat_count: 0,
            repeat_delay_ms: 0,
        },
    }
}

async fn handle_notification(
    device_id: &str,
    notification: ValueNotification,
    event_tx: &tokio::sync::broadcast::Sender<BleEvent>,
    logs: &Arc<RwLock<HashMap<String, Vec<String>>>>,
) {
    let uuid = notification.uuid.to_string();
    append_log(logs, device_id, format!("Notification received on {uuid}")).await;

    match uuid.as_str() {
        ring_constants::GESTURE_EVENT_UUID => {
            if let Some(event) =
                parse_gesture_event(&notification.value).map(BleEvent::GestureDetected)
            {
                let _ = event_tx.send(event);
            }
        }
        ring_constants::BATTERY_LEVEL_UUID => {
            if let Some(level) = parse_battery_level(&notification.value) {
                let _ = event_tx.send(BleEvent::BatteryLevel(level));
            }
        }
        ring_constants::STATE_SNAPSHOT_UUID => {
            if let Some(snapshot) = parse_state_snapshot(&notification.value) {
                let _ = event_tx.send(BleEvent::FirmwareVersion(snapshot.firmware_version.clone()));
                let _ = event_tx.send(BleEvent::SimulatorStatus(
                    device_id.to_string(),
                    snapshot_to_status(snapshot),
                ));
            }
        }
        _ => {}
    }
}

async fn append_log(
    logs: &Arc<RwLock<HashMap<String, Vec<String>>>>,
    device_id: &str,
    message: String,
) {
    logs.write()
        .await
        .entry(device_id.to_string())
        .or_default()
        .push(format!("{} {message}", chrono::Utc::now().to_rfc3339()));
}

impl ExternalBleRingManager {
    async fn record_log(&self, device_id: &str, message: String) {
        append_log(&self.connection_logs, device_id, message).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encode_haptic_request_uses_shared_command_shape() {
        let payload = encode_haptic_request(&HapticRequest {
            pattern: HapticPattern::Notification,
            intensity: 1.0,
            duration_ms: 200,
            repeat_count: 0,
            repeat_delay_ms: 0,
        })
        .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&payload).unwrap();

        assert_eq!(json["protocol_version"], PROTOCOL_VERSION);
        assert_eq!(json["message_kind"], "command");
        assert_eq!(json["payload"]["command_kind"], "haptic");
        assert_eq!(
            json["payload"]["command"]["pattern"]["pattern_kind"],
            "notify"
        );
    }

    #[test]
    fn parse_gesture_event_maps_shared_slide_direction() {
        let bytes = serde_json::json!({
            "gesture_type": "slide",
            "data": serde_json::to_vec(&serde_json::json!({
                "payload": {
                    "event_kind": "gesture",
                    "event": {
                        "gesture": {
                            "gesture_kind": "slide",
                            "direction": "left"
                        }
                    }
                }
            }))
            .unwrap(),
        });

        assert_eq!(
            parse_gesture_event(&serde_json::to_vec(&bytes).unwrap()),
            Some(GestureType::TiltLeft)
        );
    }
}
