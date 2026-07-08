//! Real BLE central integration for Haptic Harmony rings and simulators.

use crate::AppError;
use crate::ble::{
    BleEvent, GestureType, RingManager, RingStatus, SimulatorStatus, TestHapticPattern,
};
use crate::haptics::{HapticPattern, HapticRequest};
use btleplug::api::{
    Central, CharPropFlags, Characteristic, Manager as _, Peripheral as _, ScanFilter,
    ValueNotification, WriteType,
};
use btleplug::platform::{Adapter, Manager, Peripheral};
use futures::StreamExt;
// Wire types come from the canonical contract (gestura-core-ring::protocol,
// v0.3.0) — this file no longer defines any protocol shapes of its own
// (dedup approved by user 2026-07-02).
use gestura_core_ring::protocol::{
    self as ring_protocol, BleBatteryData, BleGestureData, DeviceStateSnapshot,
    HapticCommandPayload, ProtocolEnvelope, RingConfig, SemanticGesture, SemanticHapticPattern,
    SemanticRotateDirection, SemanticSlideDirection, SemanticSwipeDirection, SimulatorCommand,
    SimulatorEvent, ring_uuids,
};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;
use tokio::sync::RwLock;
use tokio::task::JoinHandle;
use uuid::Uuid;

const EXTERNAL_SCAN_WINDOW: Duration = Duration::from_secs(2);

/// Monotonic command sequence (v0.3.0). Starts at 1 — sequence 0 means
/// "unsequenced" in the protocol. Correlates with `BleEvent::CommandAck`.
static NEXT_COMMAND_SEQUENCE: AtomicU64 = AtomicU64::new(1);

#[derive(Clone)]
struct CachedPeripheral {
    peripheral: Peripheral,
    name: Option<String>,
    is_simulator: bool,
    last_seen: chrono::DateTime<chrono::Utc>,
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
        let mut freshly_connected = false;
        if !peripheral.is_connected().await.map_err(map_ble_error)? {
            self.record_log(device_id, "Connecting to BLE peripheral".to_string())
                .await;
            peripheral.connect().await.map_err(map_ble_error)?;
            freshly_connected = true;
        }

        peripheral
            .discover_services()
            .await
            .map_err(map_ble_error)?;
        self.record_log(device_id, "Discovered BLE services".to_string())
            .await;

        if freshly_connected {
            // Takeover: suppress the ring's HID projection while Gestura.app
            // owns the connection (approved 2026-07-07; HID ships ON by
            // default). Restored on release in reset_simulator. Trust-gated
            // device-side, so an unenrolled link leaves HID untouched.
            self.write_hid_enabled(device_id, &peripheral, false).await;
        }

        Ok(cached)
    }

    async fn read_status_fields(
        &self,
        device_id: &str,
        peripheral: &Peripheral,
    ) -> Result<(Option<u8>, Option<String>, Option<SimulatorStatus>), AppError> {
        let battery_level = if let Some(characteristic) =
            find_characteristic(peripheral, ring_uuids::BATTERY_LEVEL_UUID)
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
            find_characteristic(peripheral, ring_uuids::STATE_SNAPSHOT_UUID)
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

    /// Writes the Config characteristic (C2) with the HID projection flag,
    /// when the characteristic exists. Uses clobber-free read-modify-write
    /// when C2 is readable (readable-C2, ratified 2026-07-08); falls back to
    /// defaults against pre-read firmware. Best-effort: failures are logged,
    /// not fatal — a trust-gate refusal is expected on unbonded links.
    async fn write_hid_enabled(&self, device_id: &str, peripheral: &Peripheral, enabled: bool) {
        let Some(characteristic) = find_characteristic(peripheral, ring_uuids::CONFIG_UUID) else {
            return;
        };
        let base = if characteristic.properties.contains(CharPropFlags::READ) {
            match peripheral.read(&characteristic).await {
                Ok(bytes) => RingConfig::from_bytes(&bytes),
                Err(_) => RingConfig::default(),
            }
        } else {
            RingConfig::default()
        };
        let config = base.hid_set(enabled).to_bytes();
        match peripheral
            .write(&characteristic, &config, WriteType::WithResponse)
            .await
        {
            Ok(()) => {
                self.record_log(
                    device_id,
                    format!(
                        "HID projection {}",
                        if enabled { "restored" } else { "suppressed (app takeover)" }
                    ),
                )
                .await;
            }
            Err(error) => {
                self.record_log(device_id, format!("HID config write refused: {error}"))
                    .await;
            }
        }
    }

    async fn write_haptic_request(
        &self,
        device_id: &str,
        request: HapticRequest,
    ) -> Result<(), AppError> {
        let cached = self.ensure_connected(device_id).await?;
        let characteristic =
            find_characteristic(&cached.peripheral, ring_uuids::HAPTIC_COMMAND_UUID)
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
            find_characteristic(&cached.peripheral, ring_uuids::OTA_UPDATE_UUID)
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
            find_characteristic(&peripheral, ring_uuids::GESTURE_EVENT_UUID).ok_or_else(
                || AppError::Ble("Gesture event characteristic not found".to_string()),
            )?;
        let battery_characteristic =
            find_characteristic(&peripheral, ring_uuids::BATTERY_LEVEL_UUID);
        let state_characteristic =
            find_characteristic(&peripheral, ring_uuids::STATE_SNAPSHOT_UUID);

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
                ring_uuids::GESTURE_EVENT_UUID,
                ring_uuids::BATTERY_LEVEL_UUID,
                ring_uuids::STATE_SNAPSHOT_UUID,
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
            // Release: restore the ring's HID projection before dropping the
            // link so it keeps working as a standalone HID remote.
            self.write_hid_enabled(device_id, &cached.peripheral, true)
                .await;
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
    ring_uuids::HAPTIC_SERVICE_UUID
}

fn find_characteristic(peripheral: &Peripheral, uuid: Uuid) -> Option<Characteristic> {
    peripheral
        .characteristics()
        .into_iter()
        .find(|characteristic| characteristic.uuid == uuid)
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
    serde_json::from_slice::<BleBatteryData>(bytes)
        .map(|frame| frame.level)
        .ok()
        .or_else(|| (bytes.len() == 1).then(|| bytes[0]))
}

fn parse_state_snapshot(bytes: &[u8]) -> Option<DeviceStateSnapshot> {
    serde_json::from_slice(bytes).ok()
}

/// Acks ride the state-snapshot characteristic as full envelopes (v0.3.0
/// projection decision); try this when a payload isn't a snapshot.
fn parse_ack_envelope(bytes: &[u8]) -> Option<ring_protocol::AckPayload> {
    match serde_json::from_slice::<ProtocolEnvelope<SimulatorEvent>>(bytes) {
        Ok(envelope) => match envelope.payload {
            SimulatorEvent::Ack(ack) => Some(ack),
            _ => None,
        },
        Err(_) => None,
    }
}

fn snapshot_to_status(snapshot: DeviceStateSnapshot) -> SimulatorStatus {
    if let Some(reason) = snapshot.revocation_reason {
        return SimulatorStatus::Error(reason);
    }
    if !snapshot.degraded_modes.is_empty() || !snapshot.privileged_actions_enabled {
        return SimulatorStatus::Degraded;
    }
    SimulatorStatus::Healthy
}

fn parse_gesture_event(bytes: &[u8]) -> Option<GestureType> {
    let frame: BleGestureData = serde_json::from_slice(bytes).ok()?;

    // Prefer the embedded canonical envelope; parse leniently (payload only)
    // so partial/older peers don't get dropped for missing envelope metadata.
    if let Ok(value) = serde_json::from_slice::<serde_json::Value>(&frame.data)
        && let Some(payload) = value.get("payload")
        && let Ok(SimulatorEvent::Gesture(gesture)) =
            serde_json::from_value::<SimulatorEvent>(payload.clone())
    {
        return semantic_gesture_to_app(&gesture.gesture);
    }

    legacy_gesture_to_app(&frame.gesture_type)
}

fn semantic_gesture_to_app(gesture: &SemanticGesture) -> Option<GestureType> {
    match gesture {
        SemanticGesture::Tap => Some(GestureType::Tap),
        SemanticGesture::DoubleTap => Some(GestureType::DoubleTap),
        // Device-truth kinds (v0.3.0): swipes map to the GUI's tilt events,
        // rotations to the rotate events.
        SemanticGesture::Swipe { direction } => Some(match direction {
            SemanticSwipeDirection::Left => GestureType::TiltLeft,
            SemanticSwipeDirection::Right => GestureType::TiltRight,
        }),
        SemanticGesture::Rotate { direction } => Some(match direction {
            SemanticRotateDirection::Cw => GestureType::RotateCw,
            SemanticRotateDirection::Ccw => GestureType::RotateCcw,
        }),
        SemanticGesture::Slide { direction } => Some(match direction {
            SemanticSlideDirection::Up => GestureType::TiltUp,
            SemanticSlideDirection::Down => GestureType::TiltDown,
            SemanticSlideDirection::Left => GestureType::TiltLeft,
            SemanticSlideDirection::Right => GestureType::TiltRight,
        }),
        SemanticGesture::Tilt { angle_degrees } => Some(if *angle_degrees >= 0.0 {
            GestureType::TiltRight
        } else {
            GestureType::TiltLeft
        }),
        SemanticGesture::Hold { duration_ms } => {
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
        "rotate_cw" | "twist_cw" => Some(GestureType::RotateCw),
        "rotate_ccw" | "twist_ccw" => Some(GestureType::RotateCcw),
        _ => None,
    }
}

fn encode_haptic_request(request: &HapticRequest) -> Result<Vec<u8>, AppError> {
    // Canonical envelope from the SDK contract, with a monotonic sequence so
    // acks can be correlated back to the command that triggered them.
    let payload = ring_protocol::command_envelope(
        NEXT_COMMAND_SEQUENCE.fetch_add(1, Ordering::Relaxed),
        SimulatorCommand::Haptic(HapticCommandPayload {
            // GUI-request → ratified-vocabulary mapping. Feel judgment calls
            // (flagged as tunable in the 2026-07-02 platform handoff):
            // a Click is a single Tick; a Notification is a DoubleTick;
            // an Alert is the Error pattern.
            pattern: match request.pattern {
                HapticPattern::Notification => SemanticHapticPattern::DoubleTick,
                HapticPattern::Alert => SemanticHapticPattern::Error,
                HapticPattern::Click => SemanticHapticPattern::Tick,
                HapticPattern::Pulse
                | HapticPattern::Ramp
                | HapticPattern::Heartbeat
                | HapticPattern::Custom(_) => SemanticHapticPattern::Custom {
                    intensity: request.intensity,
                    duration_ms: request.duration_ms as u64,
                },
            },
        }),
    );
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
    append_log(
        logs,
        device_id,
        format!("Notification received on {}", notification.uuid),
    )
    .await;

    if notification.uuid == ring_uuids::GESTURE_EVENT_UUID {
        if let Some(event) = parse_gesture_event(&notification.value).map(BleEvent::GestureDetected)
        {
            let _ = event_tx.send(event);
        }
    } else if notification.uuid == ring_uuids::BATTERY_LEVEL_UUID {
        if let Some(level) = parse_battery_level(&notification.value) {
            let _ = event_tx.send(BleEvent::BatteryLevel(level));
        }
    } else if notification.uuid == ring_uuids::STATE_SNAPSHOT_UUID {
        if let Some(snapshot) = parse_state_snapshot(&notification.value) {
            let _ = event_tx.send(BleEvent::FirmwareVersion(snapshot.firmware_version.clone()));
            let _ = event_tx.send(BleEvent::SimulatorStatus(
                device_id.to_string(),
                snapshot_to_status(snapshot),
            ));
        } else if let Some(ack) = parse_ack_envelope(&notification.value) {
            // Acks ride the snapshot characteristic (v0.3.0).
            append_log(
                logs,
                device_id,
                format!(
                    "Command ack: seq={} status={:?} reason={}",
                    ack.sequence,
                    ack.status,
                    ack.reason.as_deref().unwrap_or("-")
                ),
            )
            .await;
            let _ = event_tx.send(BleEvent::CommandAck {
                device_id: device_id.to_string(),
                sequence: ack.sequence,
                ok: ack.status == ring_protocol::AckStatus::Ok,
                reason: ack.reason,
            });
        }
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

        assert_eq!(
            json["protocol_version"],
            ring_protocol::SHARED_PROTOCOL_VERSION
        );
        assert_eq!(json["message_kind"], "command");
        // Commands are sequenced (v0.3.0): 0 means unsequenced and is never emitted.
        assert!(json["sequence"].as_u64().unwrap() >= 1);
        assert_eq!(json["payload"]["command_kind"], "haptic");
        // Ratified vocabulary: a Notification request rides as double_tick.
        assert_eq!(
            json["payload"]["command"]["pattern"]["pattern_kind"],
            "double_tick"
        );
    }

    #[test]
    fn parse_gesture_event_maps_shared_slide_direction() {
        // Full BleGestureData + SemanticGestureEvent wire shapes, as the
        // simulator's BleProtocolAdapter actually emits them.
        let bytes = serde_json::json!({
            "gesture_type": "slide",
            "timestamp": 10,
            "confidence": 0.93,
            "data": serde_json::to_vec(&serde_json::json!({
                "payload": {
                    "event_kind": "gesture",
                    "event": {
                        "gesture": {
                            "gesture_kind": "slide",
                            "direction": "left"
                        },
                        "confidence": 0.93,
                        "timestamp_ms": 10
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

    #[test]
    fn parse_gesture_event_maps_device_truth_swipe() {
        let bytes = serde_json::json!({
            "gesture_type": "swipe",
            "timestamp": 11,
            "confidence": 0.9,
            "data": serde_json::to_vec(&serde_json::json!({
                "payload": {
                    "event_kind": "gesture",
                    "event": {
                        "gesture": {
                            "gesture_kind": "swipe",
                            "direction": "right"
                        },
                        "confidence": 0.9,
                        "timestamp_ms": 11
                    }
                }
            }))
            .unwrap(),
        });

        assert_eq!(
            parse_gesture_event(&serde_json::to_vec(&bytes).unwrap()),
            Some(GestureType::TiltRight)
        );
    }

    #[test]
    fn parse_ack_envelope_extracts_denial() {
        let envelope = ProtocolEnvelope {
            protocol_version: ring_protocol::SHARED_PROTOCOL_VERSION.to_string(),
            message_kind: ring_protocol::ProtocolMessageKind::Event,
            message_id: "test".to_string(),
            sequence: 9,
            timestamp_ms: 1,
            payload: SimulatorEvent::Ack(ring_protocol::AckPayload {
                sequence: 9,
                status: ring_protocol::AckStatus::Denied,
                reason: Some("device is not enrolled".to_string()),
            }),
        };
        let bytes = serde_json::to_vec(&envelope).unwrap();
        let ack = parse_ack_envelope(&bytes).expect("ack must parse");
        assert_eq!(ack.status, ring_protocol::AckStatus::Denied);
        assert_eq!(ack.sequence, 9);
    }
}
