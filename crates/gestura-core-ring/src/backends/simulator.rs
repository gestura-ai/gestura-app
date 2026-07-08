use crate::protocol::{
    self, BleBatteryData, BleGestureData, HapticCommandPayload, ProtocolEnvelope, RingConfig,
    SemanticGesture, SemanticHapticPattern, SemanticRotateDirection, SemanticSlideDirection,
    SemanticSwipeDirection, SimulatorCommand, SimulatorEvent, ring_uuids,
};
use base64::Engine as _;
use crate::{DeviceStatus, RingBackend};
use async_trait::async_trait;
use btleplug::api::{CharPropFlags, Characteristic, Peripheral as _};
use btleplug::platform::{Adapter, Peripheral};
use futures::stream::StreamExt;
use gestura_core_gestures::Gesture;
use gestura_core_haptics::HapticPattern;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use tokio::sync::{Mutex, broadcast};

/// Legacy raw gestures (pre-Shared-Semantic-Protocol simulators). Kept only as
/// a parse fallback so old simulator builds keep working; the current wire
/// format is `BleGestureData` / `ProtocolEnvelope` (see `crate::protocol`).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum SimulatorRawGesture {
    Tap {
        intensity: f32,
    },
    DoubleTap,
    Hold {
        start_time: u64,
    },
    Slide {
        direction: SlideDirection,
        distance: u32,
    },
    Tilt {
        angle: f32,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum SlideDirection {
    Up,
    Down,
    Left,
    Right,
}

impl SimulatorRawGesture {
    /// Normalize the legacy BLE enum into the generic `Gesture` struct.
    ///
    /// `gesture_type` is always one of the bounded set recognised by
    /// `gestura-core-intent::gesture_to_action`: `tap`, `double_tap`, `hold`,
    /// `tilt_up`, `tilt_down`, `tilt_left`, `tilt_right`.
    pub fn into_gesture(self) -> Gesture {
        match self {
            Self::Tap { intensity } => Gesture {
                gesture_type: "tap".to_string(),
                confidence: intensity.clamp(0.0, 1.0),
                acceleration: None,
                gyroscope: None,
            },
            Self::DoubleTap => Gesture {
                gesture_type: "double_tap".to_string(),
                confidence: 1.0,
                acceleration: None,
                gyroscope: None,
            },
            Self::Hold { .. } => Gesture {
                gesture_type: "hold".to_string(),
                confidence: 1.0,
                acceleration: None,
                gyroscope: None,
            },
            Self::Slide {
                direction,
                distance,
            } => {
                let tilt_type = match direction {
                    SlideDirection::Up => "tilt_up",
                    SlideDirection::Down => "tilt_down",
                    SlideDirection::Left => "tilt_left",
                    SlideDirection::Right => "tilt_right",
                };
                Gesture {
                    gesture_type: tilt_type.to_string(),
                    confidence: 1.0,
                    acceleration: Some([distance as f32, 0.0, 0.0]),
                    gyroscope: None,
                }
            }
            Self::Tilt { angle } => Gesture {
                gesture_type: if angle >= 0.0 { "tilt_right" } else { "tilt_left" }.to_string(),
                confidence: 1.0,
                acceleration: None,
                gyroscope: Some([angle, 0.0, 0.0]),
            },
        }
    }
}

/// Converts a protocol `SemanticGesture` event into the app-level `Gesture`,
/// preserving the `gesture_to_action` vocabulary (`tap`, `double_tap`, `hold`,
/// `tilt_*`). `Slide` maps onto `tilt_*` exactly like the legacy path did, so
/// downstream intent mapping is unchanged.
fn semantic_to_gesture(gesture: SemanticGesture, confidence: f32) -> Gesture {
    match gesture {
        SemanticGesture::Tap => Gesture {
            gesture_type: "tap".to_string(),
            confidence: confidence.clamp(0.0, 1.0),
            acceleration: None,
            gyroscope: None,
        },
        SemanticGesture::DoubleTap => Gesture {
            gesture_type: "double_tap".to_string(),
            confidence: confidence.clamp(0.0, 1.0),
            acceleration: None,
            gyroscope: None,
        },
        SemanticGesture::Hold { duration_ms } => Gesture {
            gesture_type: "hold".to_string(),
            confidence: confidence.clamp(0.0, 1.0),
            // Carry hold duration in the x-axis acceleration slot so
            // downstream never parses the type string for it.
            acceleration: Some([duration_ms as f32, 0.0, 0.0]),
            gyroscope: None,
        },
        // Device-truth kinds (v0.3.0). Emitted gesture_type strings stay in
        // the vocabulary gestura-core-intent::gesture_to_action recognizes:
        // swipes ride as tilt_* (→ previous/next), rotates as twist_* (→
        // increase/decrease).
        SemanticGesture::Swipe { direction } => Gesture {
            gesture_type: match direction {
                SemanticSwipeDirection::Left => "tilt_left",
                SemanticSwipeDirection::Right => "tilt_right",
            }
            .to_string(),
            confidence: confidence.clamp(0.0, 1.0),
            acceleration: None,
            gyroscope: None,
        },
        SemanticGesture::Rotate { direction } => Gesture {
            gesture_type: match direction {
                SemanticRotateDirection::Cw => "twist_cw",
                SemanticRotateDirection::Ccw => "twist_ccw",
            }
            .to_string(),
            confidence: confidence.clamp(0.0, 1.0),
            acceleration: None,
            gyroscope: None,
        },
        SemanticGesture::Slide { direction } => {
            let tilt_type = match direction {
                SemanticSlideDirection::Up => "tilt_up",
                SemanticSlideDirection::Down => "tilt_down",
                SemanticSlideDirection::Left => "tilt_left",
                SemanticSlideDirection::Right => "tilt_right",
            };
            Gesture {
                gesture_type: tilt_type.to_string(),
                confidence: confidence.clamp(0.0, 1.0),
                acceleration: None,
                gyroscope: None,
            }
        }
        SemanticGesture::Tilt { angle_degrees } => Gesture {
            gesture_type: if angle_degrees >= 0.0 {
                "tilt_right"
            } else {
                "tilt_left"
            }
            .to_string(),
            confidence: confidence.clamp(0.0, 1.0),
            acceleration: None,
            gyroscope: Some([angle_degrees, 0.0, 0.0]),
        },
    }
}

/// Default waveform sample rate when the app-level API doesn't specify one.
const DEFAULT_WAVEFORM_SAMPLE_RATE_HZ: u32 = 8_000;

/// Maps the app-level `HapticPattern` onto the protocol vocabulary.
///
/// v0.3.0: full one-to-one mapping. `Waveform` bytes travel first-class as
/// base64 samples (Proposal B, approved 2026-07-02); sample rate defaults to
/// 8 kHz since the app-level `send_haptic` API doesn't carry one yet.
fn haptic_to_semantic(
    pattern: &HapticPattern,
    intensity: f32,
    duration_ms: u32,
) -> SemanticHapticPattern {
    let _ = duration_ms;
    match pattern {
        HapticPattern::Confirm => SemanticHapticPattern::Confirm,
        HapticPattern::Error => SemanticHapticPattern::Error,
        HapticPattern::Tick => SemanticHapticPattern::Tick,
        HapticPattern::DoubleTick => SemanticHapticPattern::DoubleTick,
        HapticPattern::Waveform(samples) => SemanticHapticPattern::Waveform {
            data: base64::engine::general_purpose::STANDARD.encode(samples),
            sample_rate_hz: DEFAULT_WAVEFORM_SAMPLE_RATE_HZ,
            intensity,
        },
    }
}

/// Parses one gesture-characteristic notification payload into a `Gesture`.
///
/// Accepts, in order: the current `BleGestureData` wrapper with an embedded
/// `ProtocolEnvelope<SimulatorEvent>`, a bare envelope, and the legacy
/// `SimulatorRawGesture` shape. Returns `None` (with a warning) when nothing
/// parses — the previous implementation dropped these silently, which is how
/// the app↔simulator format drift went unnoticed.
fn parse_gesture_notification(value: &[u8]) -> Option<Gesture> {
    if let Ok(wrapper) = serde_json::from_slice::<BleGestureData>(value) {
        if let Ok(envelope) =
            serde_json::from_slice::<ProtocolEnvelope<SimulatorEvent>>(&wrapper.data)
            && let SimulatorEvent::Gesture(event) = envelope.payload
        {
            return Some(semantic_to_gesture(event.gesture, event.confidence));
        }
        tracing::warn!(
            gesture_type = %wrapper.gesture_type,
            "BleGestureData wrapper parsed but embedded protocol envelope did not; \
             dropping gesture"
        );
        return None;
    }

    if let Ok(envelope) = serde_json::from_slice::<ProtocolEnvelope<SimulatorEvent>>(value)
        && let SimulatorEvent::Gesture(event) = envelope.payload
    {
        return Some(semantic_to_gesture(event.gesture, event.confidence));
    }

    if let Ok(raw) = serde_json::from_slice::<SimulatorRawGesture>(value) {
        return Some(raw.into_gesture());
    }

    tracing::warn!(
        payload_len = value.len(),
        "Unrecognized gesture notification payload; dropping. \
         Payload prefix: {:?}",
        String::from_utf8_lossy(&value[..value.len().min(64)])
    );
    None
}

#[derive(Debug, Clone, Default)]
struct LatestDeviceState {
    battery_level: Option<u8>,
    is_charging: Option<bool>,
    trust_state: Option<String>,
}

/// Applies a battery-characteristic payload (raw single byte or JSON
/// `BleBatteryData`) to the cached device state.
fn apply_battery_payload(state: &mut LatestDeviceState, value: &[u8]) {
    if value.len() == 1 {
        // Initial raw single-byte battery value.
        state.battery_level = Some(value[0]);
    } else if let Ok(battery) = serde_json::from_slice::<BleBatteryData>(value) {
        state.battery_level = Some(battery.level);
        state.is_charging = Some(battery.is_charging);
    }
}

/// Applies a state-snapshot payload (`DeviceStateSnapshot` JSON, consumed
/// loosely) to the cached device state.
fn apply_state_snapshot_payload(state: &mut LatestDeviceState, value: &[u8]) {
    if let Ok(snapshot) = serde_json::from_slice::<serde_json::Value>(value) {
        if let Some(trust) = snapshot.get("trust_state").and_then(|v| v.as_str()) {
            state.trust_state = Some(trust.to_string());
        }
        // Battery may also arrive embedded in a full snapshot.
        if let Some(battery) = snapshot.get("battery") {
            if let Some(level) = battery.get("level_percent").and_then(|v| v.as_u64()) {
                state.battery_level = Some(level.min(100) as u8);
            }
            if let Some(charging) = battery.get("is_charging").and_then(|v| v.as_bool()) {
                state.is_charging = Some(charging);
            }
        }
    }
}

pub struct SimulatorBackend {
    tx: broadcast::Sender<Gesture>,
    peripheral: Arc<Mutex<Option<Peripheral>>>,
    haptic_char: Arc<Mutex<Option<Characteristic>>>,
    config_char: Arc<Mutex<Option<Characteristic>>>,
    adapter: Arc<Mutex<Option<Adapter>>>,
    sequence: Arc<AtomicU64>,
    device_state: Arc<Mutex<LatestDeviceState>>,
}

impl Default for SimulatorBackend {
    fn default() -> Self {
        Self::new()
    }
}

impl SimulatorBackend {
    pub fn new() -> Self {
        let (tx, _) = broadcast::channel(100);
        Self {
            tx,
            peripheral: Arc::new(Mutex::new(None)),
            haptic_char: Arc::new(Mutex::new(None)),
            config_char: Arc::new(Mutex::new(None)),
            adapter: Arc::new(Mutex::new(None)),
            sequence: Arc::new(AtomicU64::new(1)),
            device_state: Arc::new(Mutex::new(LatestDeviceState::default())),
        }
    }

    /// Sets the HID projection flag via clobber-free read-modify-write when
    /// the Config characteristic is readable (readable-C2, ratified
    /// 2026-07-08), falling back to a defaults-based write when it isn't
    /// (pre-read firmware).
    async fn write_hid_config(&self, hid_enabled: bool) {
        let p_lock = self.peripheral.lock().await;
        let c_lock = self.config_char.lock().await;
        if let (Some(peripheral), Some(char)) = (&*p_lock, &*c_lock) {
            let base = if char.properties.contains(CharPropFlags::READ) {
                match peripheral.read(char).await {
                    Ok(bytes) => RingConfig::from_bytes(&bytes),
                    Err(e) => {
                        tracing::debug!(
                            "Config read failed ({}); writing defaults-based config",
                            e
                        );
                        RingConfig::default()
                    }
                }
            } else {
                RingConfig::default()
            };
            let config = base.hid_set(hid_enabled);

            let write_type = if char
                .properties
                .contains(CharPropFlags::WRITE_WITHOUT_RESPONSE)
            {
                btleplug::api::WriteType::WithoutResponse
            } else {
                btleplug::api::WriteType::WithResponse
            };
            if let Err(e) = peripheral
                .write(char, &config.to_bytes(), write_type)
                .await
            {
                tracing::warn!("Failed to write ring config: {}", e);
            }
        }
    }

    async fn find_simulator(&self) -> Result<Peripheral, String> {
        let result = gestura_core_ble::scanner::find_device_by_service_uuid(
            ring_uuids::HAPTIC_SERVICE_UUID,
            10,
            std::time::Duration::from_millis(500),
        )
        .await;

        // Transition fallback: peers that haven't shipped the v0.3.0 UUID
        // base still advertise the legacy service. Connecting to one will
        // fail cleanly at characteristic lookup with an "incompatible
        // protocol version" error rather than silently misbehaving.
        let (adapter, peripheral) = match result {
            Ok(found) => found,
            Err(primary_err) => {
                tracing::warn!(
                    "No device advertising the v0.3.0 service UUID ({primary_err}); \
                     retrying with the legacy v0.2 service UUID"
                );
                gestura_core_ble::scanner::find_device_by_service_uuid(
                    ring_uuids::LEGACY_SERVICE_UUID,
                    4,
                    std::time::Duration::from_millis(500),
                )
                .await?
            }
        };

        *self.adapter.lock().await = Some(adapter);
        Ok(peripheral)
    }

    /// Spawns a background task that routes notifications by characteristic
    /// UUID: gesture events to the broadcast channel, battery and state
    /// snapshots into `device_state`.
    fn spawn_event_listener(
        &self,
        peripheral: Peripheral,
        subscribed: Vec<Characteristic>,
    ) {
        let tx = self.tx.clone();
        let device_state = self.device_state.clone();
        tokio::spawn(async move {
            for characteristic in &subscribed {
                if let Err(e) = peripheral.subscribe(characteristic).await {
                    tracing::error!(
                        uuid = %characteristic.uuid,
                        "Failed to subscribe to characteristic: {}",
                        e
                    );
                }
            }

            let mut notification_stream = match peripheral.notifications().await {
                Ok(stream) => stream,
                Err(e) => {
                    tracing::error!("Failed to get notification stream: {}", e);
                    return;
                }
            };

            tracing::info!("Listening for simulator protocol notifications");

            while let Some(data) = notification_stream.next().await {
                if data.uuid == ring_uuids::GESTURE_EVENT_UUID {
                    if let Some(gesture) = parse_gesture_notification(&data.value) {
                        let _ = tx.send(gesture);
                    }
                } else if data.uuid == ring_uuids::BATTERY_LEVEL_UUID {
                    let mut state = device_state.lock().await;
                    apply_battery_payload(&mut state, &data.value);
                } else if data.uuid == ring_uuids::STATE_SNAPSHOT_UUID {
                    let mut state = device_state.lock().await;
                    apply_state_snapshot_payload(&mut state, &data.value);
                }
            }
        });
    }

    /// Helper for testing environment simulating Tauri invoke fallback.
    pub async fn _untested_tauri_fallback_trigger(&self, raw: SimulatorRawGesture) {
        let _ = self.tx.send(raw.into_gesture());
    }
}

#[async_trait]
impl RingBackend for SimulatorBackend {
    async fn connect(&self) -> Result<(), String> {
        tracing::info!("SimulatorBackend initializing connection sequence");
        let peripheral = self.find_simulator().await?;

        peripheral
            .connect()
            .await
            .map_err(|e| format!("Connection failed: {}", e))?;
        peripheral
            .discover_services()
            .await
            .map_err(|e| format!("Service discovery failed: {}", e))?;

        // Target characteristics by their shared protocol UUIDs — never by
        // "first NOTIFY/WRITE found". The simulator exposes three NOTIFY and
        // two WRITE characteristics; picking by flags nondeterministically
        // subscribed to battery instead of gestures and wrote haptic JSON to
        // the OTA characteristic.
        let chars = peripheral.characteristics();
        let by_uuid = |uuid: uuid::Uuid| chars.iter().find(|c| c.uuid == uuid).cloned();

        let gesture_char = by_uuid(ring_uuids::GESTURE_EVENT_UUID);
        let haptic_char = by_uuid(ring_uuids::HAPTIC_COMMAND_UUID);
        let battery_char = by_uuid(ring_uuids::BATTERY_LEVEL_UUID);
        let snapshot_char = by_uuid(ring_uuids::STATE_SNAPSHOT_UUID);
        let config_char = by_uuid(ring_uuids::CONFIG_UUID);

        let Some(gesture_char) = gesture_char else {
            return Err(format!(
                "Connected, but gesture characteristic {} is missing — wrong device \
                 or incompatible protocol version",
                ring_uuids::GESTURE_EVENT_UUID
            ));
        };
        if !gesture_char.properties.contains(CharPropFlags::NOTIFY) {
            return Err("Gesture characteristic does not support NOTIFY".to_string());
        }
        if haptic_char.is_none() {
            tracing::warn!(
                "Haptic command characteristic {} not found; send_haptic will be a no-op",
                ring_uuids::HAPTIC_COMMAND_UUID
            );
        }

        *self.peripheral.lock().await = Some(peripheral.clone());
        *self.haptic_char.lock().await = haptic_char;
        *self.config_char.lock().await = config_char;

        // Takeover: suppress the device's HID projection while we own the
        // connection so the OS doesn't double-act on gestures (approved
        // 2026-07-07; firmware ships HID ON by default). Restored in
        // disconnect(). Note: the write is trust-gated device-side, so on an
        // unenrolled link it is refused and HID stays on — correct behavior.
        self.write_hid_config(false).await;

        // Prime device state from the READable characteristics so get_status()
        // is meaningful before the first notification arrives.
        if let Some(ref battery) = battery_char
            && battery.properties.contains(CharPropFlags::READ)
            && let Ok(value) = peripheral.read(battery).await
        {
            let mut state = self.device_state.lock().await;
            apply_battery_payload(&mut state, &value);
        }
        if let Some(ref snapshot) = snapshot_char
            && snapshot.properties.contains(CharPropFlags::READ)
            && let Ok(value) = peripheral.read(snapshot).await
        {
            let mut state = self.device_state.lock().await;
            apply_state_snapshot_payload(&mut state, &value);
        }

        let mut subscribed = vec![gesture_char];
        subscribed.extend(battery_char.into_iter());
        subscribed.extend(snapshot_char.into_iter());
        self.spawn_event_listener(peripheral, subscribed);

        tracing::info!("SimulatorBackend bound to protocol characteristics");
        Ok(())
    }

    async fn subscribe_to_gestures(&self) -> tokio::sync::broadcast::Receiver<Gesture> {
        self.tx.subscribe()
    }

    async fn send_haptic(&self, pattern: HapticPattern, intensity: f32, duration_ms: u32) {
        tracing::debug!(
            "SimulatorBackend sending haptic: {:?} (int: {}, dur: {}ms)",
            pattern,
            intensity,
            duration_ms
        );

        let p_lock = self.peripheral.lock().await;
        let c_lock = self.haptic_char.lock().await;

        if let (Some(peripheral), Some(char)) = (&*p_lock, &*c_lock) {
            let sequence = self.sequence.fetch_add(1, Ordering::Relaxed);
            let envelope = protocol::command_envelope(
                sequence,
                SimulatorCommand::Haptic(HapticCommandPayload {
                    pattern: haptic_to_semantic(&pattern, intensity, duration_ms),
                }),
            );
            let payload = match serde_json::to_vec(&envelope) {
                Ok(payload) => payload,
                Err(e) => {
                    tracing::error!("Failed to serialize haptic command envelope: {}", e);
                    return;
                }
            };

            // Choose the write type the characteristic actually supports.
            let write_type = if char
                .properties
                .contains(CharPropFlags::WRITE_WITHOUT_RESPONSE)
            {
                btleplug::api::WriteType::WithoutResponse
            } else {
                btleplug::api::WriteType::WithResponse
            };

            if let Err(e) = peripheral.write(char, &payload, write_type).await {
                tracing::warn!(
                    pattern = ?pattern,
                    "Failed to send haptic command envelope via BLE write: {}",
                    e
                );
            }
        }
    }

    async fn disconnect(&self) {
        // Release: restore the device's HID projection before dropping the
        // link, so the ring keeps working as a standalone HID remote.
        self.write_hid_config(true).await;

        let peripheral = self.peripheral.lock().await.take();
        *self.haptic_char.lock().await = None;
        *self.config_char.lock().await = None;
        if let Some(peripheral) = peripheral
            && let Err(e) = peripheral.disconnect().await
        {
            tracing::warn!("BLE disconnect failed: {}", e);
        }
        *self.device_state.lock().await = LatestDeviceState::default();
    }

    async fn get_status(&self) -> DeviceStatus {
        let is_connected = self.peripheral.lock().await.is_some();
        let state = self.device_state.lock().await.clone();

        let connection_state = if is_connected {
            match state.trust_state {
                Some(trust) => format!("simulator_ble_connected(trust:{trust})"),
                None => "simulator_ble_connected".to_string(),
            }
        } else {
            "simulator_disconnected".to_string()
        };

        DeviceStatus {
            battery: state.battery_level.unwrap_or(0),
            is_charging: state.is_charging.unwrap_or(false),
            connection_state,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn simulator_gesture_bytes(event_json: serde_json::Value) -> Vec<u8> {
        // Mirror of the simulator's BleProtocolAdapter::project_event output.
        let envelope = serde_json::json!({
            "protocol_version": "0.1.0",
            "message_kind": "event",
            "message_id": "test",
            "sequence": 0,
            "timestamp_ms": 10,
            "payload": { "event_kind": "gesture", "event": event_json }
        });
        let wrapper = serde_json::json!({
            "gesture_type": "irrelevant-for-parse",
            "timestamp": 10,
            "confidence": 0.9,
            "data": serde_json::to_vec(&envelope).unwrap()
        });
        serde_json::to_vec(&wrapper).unwrap()
    }

    #[test]
    fn parses_wrapped_protocol_tap() {
        let bytes = simulator_gesture_bytes(serde_json::json!({
            "gesture": { "gesture_kind": "tap" },
            "confidence": 0.98,
            "timestamp_ms": 10
        }));
        let gesture = parse_gesture_notification(&bytes).expect("must parse");
        assert_eq!(gesture.gesture_type, "tap");
        assert!((gesture.confidence - 0.98).abs() < f32::EPSILON);
    }

    #[test]
    fn parses_wrapped_protocol_slide_to_tilt_vocab() {
        let bytes = simulator_gesture_bytes(serde_json::json!({
            "gesture": { "gesture_kind": "slide", "direction": "down" },
            "confidence": 0.93,
            "timestamp_ms": 10
        }));
        let gesture = parse_gesture_notification(&bytes).expect("must parse");
        // Slide maps into the tilt_* vocabulary recognised by
        // gestura-core-intent::gesture_to_action.
        assert_eq!(gesture.gesture_type, "tilt_down");
    }

    #[test]
    fn parses_bare_envelope() {
        let envelope = serde_json::json!({
            "protocol_version": "0.1.0",
            "message_kind": "event",
            "message_id": "test",
            "sequence": 2,
            "timestamp_ms": 20,
            "payload": {
                "event_kind": "gesture",
                "event": {
                    "gesture": { "gesture_kind": "double_tap" },
                    "confidence": 1.0,
                    "timestamp_ms": 20
                }
            }
        });
        let bytes = serde_json::to_vec(&envelope).unwrap();
        let gesture = parse_gesture_notification(&bytes).expect("must parse");
        assert_eq!(gesture.gesture_type, "double_tap");
    }

    #[test]
    fn parses_legacy_raw_gesture_fallback() {
        let bytes = br#"{"type":"Tilt","angle":-30.0}"#;
        let gesture = parse_gesture_notification(bytes).expect("must parse");
        assert_eq!(gesture.gesture_type, "tilt_left");
        assert_eq!(gesture.gyroscope, Some([-30.0, 0.0, 0.0]));
    }

    #[test]
    fn rejects_garbage_without_panic() {
        assert!(parse_gesture_notification(b"not json").is_none());
        assert!(parse_gesture_notification(b"{}").is_none());
    }

    #[test]
    fn battery_payload_raw_and_json() {
        let mut state = LatestDeviceState::default();
        apply_battery_payload(&mut state, &[85]);
        assert_eq!(state.battery_level, Some(85));

        let json = serde_json::json!({
            "level": 42, "is_charging": true, "voltage": 3.9,
            "temperature": 25.0, "health": "Good", "time_remaining": 90
        });
        apply_battery_payload(&mut state, &serde_json::to_vec(&json).unwrap());
        assert_eq!(state.battery_level, Some(42));
        assert_eq!(state.is_charging, Some(true));
    }

    #[test]
    fn state_snapshot_payload_extracts_trust_and_battery() {
        // Shape mirrors the simulator's DeviceStateSnapshot serialization.
        let json = serde_json::json!({
            "battery": {
                "level_percent": 61, "is_charging": false, "voltage": 3.8,
                "temperature_celsius": 25.0, "health": "Good",
                "time_remaining_minutes": 120
            },
            "trust_state": "enrolled",
            "degraded_modes": [],
            "firmware_version": "1.0.0-sim",
            "protocol_version": "0.1.0",
            "revocation_reason": null,
            "privileged_actions_enabled": true
        });
        let mut state = LatestDeviceState::default();
        apply_state_snapshot_payload(&mut state, &serde_json::to_vec(&json).unwrap());
        assert_eq!(state.trust_state.as_deref(), Some("enrolled"));
        assert_eq!(state.battery_level, Some(61));
        assert_eq!(state.is_charging, Some(false));
    }

    #[test]
    fn ratified_haptic_vocabulary_maps_one_to_one() {
        assert_eq!(
            haptic_to_semantic(&HapticPattern::Confirm, 1.0, 200),
            SemanticHapticPattern::Confirm
        );
        assert_eq!(
            haptic_to_semantic(&HapticPattern::Error, 1.0, 300),
            SemanticHapticPattern::Error
        );
        assert_eq!(
            haptic_to_semantic(&HapticPattern::Tick, 0.5, 50),
            SemanticHapticPattern::Tick
        );
        assert_eq!(
            haptic_to_semantic(&HapticPattern::DoubleTick, 0.5, 120),
            SemanticHapticPattern::DoubleTick
        );
        // Waveform: first-class base64 payload as of v0.3.0 (approved).
        assert_eq!(
            haptic_to_semantic(&HapticPattern::Waveform(vec![0, 1, 2]), 0.7, 90),
            SemanticHapticPattern::Waveform {
                data: "AAEC".to_string(),
                sample_rate_hz: 8000,
                intensity: 0.7
            }
        );
    }

    #[test]
    fn v0_3_device_gestures_map_to_intent_vocabulary() {
        let bytes = simulator_gesture_bytes(serde_json::json!({
            "gesture": { "gesture_kind": "swipe", "direction": "left" },
            "confidence": 0.9,
            "timestamp_ms": 5
        }));
        assert_eq!(
            parse_gesture_notification(&bytes).expect("must parse").gesture_type,
            "tilt_left"
        );

        let bytes = simulator_gesture_bytes(serde_json::json!({
            "gesture": { "gesture_kind": "rotate", "direction": "ccw" },
            "confidence": 0.9,
            "timestamp_ms": 6
        }));
        assert_eq!(
            parse_gesture_notification(&bytes).expect("must parse").gesture_type,
            "twist_ccw"
        );
    }
}
