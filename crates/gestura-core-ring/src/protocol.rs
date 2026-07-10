//! Canonical home of the Haptica Harmony B1 Shared Semantic Protocol.
//!
//! Per the user decision of 2026-07-02, the protocol contract is centralized
//! in the gestura-app SDK — this module. The simulator's `protocol.rs`
//! (v0.1.0) was the starting definition; the simulator and the ring firmware
//! now follow this file. It remains a shared contract across the firmware and
//! platform lanes: vocabulary/shape changes are proposed, cross-checked with
//! the firmware lane, and confirmed by the user before landing here.
//!
//! v0.2.0 (2026-07-02): haptic vocabulary ratified as
//! {Confirm, Error, Tick, DoubleTick, Waveform} (user decision; firmware
//! renamed to match in its commit `d4eb83c`). `success`/`notify` are accepted
//! as read-aliases for envelopes from v0.1.0 peers.
//!
//! v0.3.0 (2026-07-02, user approved "proceed with all proposals"):
//! - Production GATT UUID base minted (replaces the example-looking
//!   `12345678-…` base; firmware to adopt — see the platform proposals note).
//! - First-class `waveform` haptic payload (base64 samples + sample_rate_hz).
//! - Gesture kinds `swipe` and `rotate` added (device truth); `slide`/`tilt`
//!   remain simulator-only kinds.
//! - `ack` event type added (emission wiring is a follow-up).
//! - Typed `DeviceStateSnapshot`/`TrustState`/`DegradedMode` (previously
//!   consumed loosely).

use serde::{Deserialize, Serialize};

/// Shared protocol version.
pub const SHARED_PROTOCOL_VERSION: &str = "0.3.0";

/// BLE service/characteristic UUIDs — the joint allocation, FINAL per user
/// decision 2026-07-02: the firmware-minted base is canonical for all lanes.
/// This module is the single source of truth on the host side; the simulator
/// mirrors it; firmware's `gatt_gestura.c` carries the same values.
/// Last byte is the characteristic ordinal (BC..C2).
pub mod ring_uuids {
    use uuid::Uuid;

    /// Main ring service UUID.
    pub const HAPTIC_SERVICE_UUID: Uuid = Uuid::from_u128(0xE3B742D4_51C9_4F0E_9D26_7A48C1F0B9BC);
    /// Haptic command characteristic (host → ring, write, trust-gated).
    pub const HAPTIC_COMMAND_UUID: Uuid = Uuid::from_u128(0xE3B742D4_51C9_4F0E_9D26_7A48C1F0B9BD);
    /// Gesture event characteristic (ring → host, notify).
    pub const GESTURE_EVENT_UUID: Uuid = Uuid::from_u128(0xE3B742D4_51C9_4F0E_9D26_7A48C1F0B9BE);
    /// Battery level characteristic (ring → host, read + notify).
    pub const BATTERY_LEVEL_UUID: Uuid = Uuid::from_u128(0xE3B742D4_51C9_4F0E_9D26_7A48C1F0B9BF);
    /// OTA update characteristic (host → ring, write + indicate).
    pub const OTA_UPDATE_UUID: Uuid = Uuid::from_u128(0xE3B742D4_51C9_4F0E_9D26_7A48C1F0B9C0);
    /// Device state snapshot characteristic (ring → host, read + notify).
    pub const STATE_SNAPSHOT_UUID: Uuid = Uuid::from_u128(0xE3B742D4_51C9_4F0E_9D26_7A48C1F0B9C1);
    /// Config characteristic (host → ring, write, encrypted/trust-gated):
    /// sensitivity, raw-stream opt-in, enabled-gesture set.
    pub const CONFIG_UUID: Uuid = Uuid::from_u128(0xE3B742D4_51C9_4F0E_9D26_7A48C1F0B9C2);
    /// Opt-in raw sensor stream (ring → host, notify; approved 2026-07-02).
    /// Privacy-sensitive: subscription is trust-gated (Bonded or better) on
    /// both the ring and the simulator, in addition to the config opt-in.
    /// Frame payload shape is not yet defined — allocation only.
    pub const RAW_SENSOR_STREAM_UUID: Uuid =
        Uuid::from_u128(0xE3B742D4_51C9_4F0E_9D26_7A48C1F0B9C3);

    /// v0.2-era service UUID, still advertised by not-yet-updated peers.
    /// Kept for discovery fallback during the transition; remove after all
    /// sides ship the new base.
    pub const LEGACY_SERVICE_UUID: Uuid = Uuid::from_u128(0x12345678_1234_5678_9abc_123456789abc);
}

/// Whether an envelope carries an event or a command payload.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProtocolMessageKind {
    Event,
    Command,
}

/// Versioned envelope shared by all simulator/ring transports.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProtocolEnvelope<T> {
    pub protocol_version: String,
    pub message_kind: ProtocolMessageKind,
    pub message_id: String,
    /// Per-session sequence value. `0` means unsequenced.
    pub sequence: u64,
    pub timestamp_ms: u64,
    pub payload: T,
}

/// Semantic slide directions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SemanticSlideDirection {
    Up,
    Down,
    Left,
    Right,
}

/// Horizontal swipe directions (device touch strip is single-axis).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SemanticSwipeDirection {
    Left,
    Right,
}

/// Rotation directions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SemanticRotateDirection {
    Cw,
    Ccw,
}

/// Semantic gesture kind shared across adapters.
///
/// v0.3.0: `swipe` and `rotate` are the device-truth kinds (the ring emits
/// SWIPE_LEFT/RIGHT and ROTATE_CW/CCW); `slide` and `tilt` are simulator-only
/// kinds produced by the simulator's pad/tilt UI.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "gesture_kind", rename_all = "snake_case")]
pub enum SemanticGesture {
    Tap,
    DoubleTap,
    Hold { duration_ms: u64 },
    Swipe { direction: SemanticSwipeDirection },
    Rotate { direction: SemanticRotateDirection },
    Slide { direction: SemanticSlideDirection },
    Tilt { angle_degrees: f32 },
}

/// Semantic gesture event payload.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SemanticGestureEvent {
    pub gesture: SemanticGesture,
    pub confidence: f32,
    pub timestamp_ms: u64,
}

/// Battery snapshot payload (protocol shape).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BatterySnapshot {
    pub level_percent: u8,
    pub is_charging: bool,
    pub voltage: f32,
    pub temperature_celsius: f32,
    pub health: String,
    pub time_remaining_minutes: Option<u32>,
}

/// Trust state of the device (mirrors the simulator/firmware trust model;
/// deny-by-default posture per user decision 2026-07-02).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TrustState {
    Discovered,
    Bonded,
    Enrolled,
    Attested,
    Revoked,
}

/// Degraded conditions that can gate privileged behavior.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DegradedMode {
    LowBattery,
    SensorFault,
    FirmwareMismatch,
    OperatorBlocked,
}

/// Full device-state snapshot (v0.3.0: typed; previously consumed loosely).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DeviceStateSnapshot {
    pub battery: BatterySnapshot,
    pub trust_state: TrustState,
    pub degraded_modes: Vec<DegradedMode>,
    pub firmware_version: String,
    pub protocol_version: String,
    pub revocation_reason: Option<String>,
    pub privileged_actions_enabled: bool,
}

/// Ack status for command acknowledgements (v0.3.0).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AckStatus {
    Ok,
    Denied,
    Error,
}

/// Command acknowledgement payload (v0.3.0). `sequence` correlates to the
/// command envelope's sequence value, making trust denials visible to the
/// host instead of writes silently vanishing.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AckPayload {
    pub sequence: u64,
    pub status: AckStatus,
    pub reason: Option<String>,
}

/// Transport-agnostic simulator/ring events.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "event_kind", content = "event", rename_all = "snake_case")]
pub enum SimulatorEvent {
    Gesture(SemanticGestureEvent),
    Battery(BatterySnapshot),
    StateSnapshot(DeviceStateSnapshot),
    /// v0.3.0: acknowledgement of a host command. Emission wiring in the
    /// simulator/firmware is a follow-up; the type is part of the contract.
    Ack(AckPayload),
}

/// Semantic haptic pattern (protocol shape), ratified vocabulary.
///
/// `success` and `notify` are accepted when *reading* envelopes from v0.1.0
/// peers (mapped to `Confirm` and `Tick` respectively) but are never emitted.
///
/// v0.3.0 (Proposal B, approved): `Waveform` is first-class — `data` is
/// base64-encoded samples (NOT a JSON byte array; ~4x smaller on the wire),
/// `sample_rate_hz` explicit so feel survives resampling. Soft cap 4 KiB per
/// waveform via GATT long write; the device may reject larger (BOS1921
/// RAM/FIFO limits are a firmware cross-check item). Real-ring playback lands
/// only after the firmware's v0-byte-encoding → JSON/nanopb migration; the
/// simulator supports it immediately. `Custom` remains a generic pulse.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "pattern_kind", rename_all = "snake_case")]
pub enum SemanticHapticPattern {
    #[serde(alias = "success")]
    Confirm,
    Error,
    #[serde(alias = "notify")]
    Tick,
    DoubleTick,
    Waveform {
        /// Base64-encoded samples: 12-bit two's-complement sent as int16
        /// (BOS1921 datasheet pass, 2026-07-07 — see PROTOCOL.md). Firmware
        /// rejects >1024 samples (device FIFO) until streaming refill lands;
        /// the protocol-level cap stays 4 KiB.
        data: String,
        sample_rate_hz: u32,
        intensity: f32,
    },
    Custom {
        intensity: f32,
        duration_ms: u64,
    },
}

/// Semantic haptic command payload.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HapticCommandPayload {
    pub pattern: SemanticHapticPattern,
}

/// Transport-agnostic commands directed at the simulator/ring.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "command_kind", content = "command", rename_all = "snake_case")]
pub enum SimulatorCommand {
    Haptic(HapticCommandPayload),
}

/// BLE projection wrapper the simulator notifies on the gesture characteristic.
/// `data` carries the full serialized `ProtocolEnvelope<SimulatorEvent>` bytes.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BleGestureData {
    pub gesture_type: String,
    pub timestamp: u64,
    pub confidence: f32,
    pub data: Vec<u8>,
}

/// BLE projection shape the simulator notifies on the battery characteristic.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BleBatteryData {
    pub level: u8,
    pub is_charging: bool,
    pub voltage: f32,
    pub temperature: f32,
    pub health: String,
    pub time_remaining: Option<u32>,
}

/// Ring configuration, written to the Config characteristic (C2).
///
/// Wire layout (all bytes optional from byte 1 onward — firmware treats
/// shorter writes as leaving trailing fields unchanged, per the 2026-07-07
/// firmware note "byte 3 optional / backward-compatible").
///
/// **Config READ path (readable-C2, ratified 2026-07-08):** current firmware
/// exposes C2 as readable, so hosts preserve device state via clobber-free
/// read-modify-write (`from_bytes` → mutate → `to_bytes`). Only against
/// pre-read firmware (no READ property on C2) do writers fall back to this
/// struct's defaults, which mirror firmware defaults — in that mode a full
/// 4-byte write may still clobber out-of-band config changes.
///
/// | byte | field | default |
/// |---|---|---|
/// | 0 | gesture sensitivity (0–255) | 0x80 |
/// | 1 | raw sensor stream opt-in (0/1) | 0 |
/// | 2 | enabled-gesture bitmask (RATIFIED 2026-07-09: bit0 tap, bit1
///       double_tap, bit2 swipe_left, bit3 swipe_right, bit4 rotate_cw,
///       bit5 rotate_ccw, bit6 hold, bit7 reserved) | 0xFF |
/// | 3 | **HID projection enable (0/1)** — firmware ships a BLE HID
///       consumer-control service ON by default; the SDK writes 0 on
///       connection takeover and 1 on release so the OS doesn't double-act
///       on gestures (approved 2026-07-07) | 1 |
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RingConfig {
    pub sensitivity: u8,
    pub raw_stream_opt_in: bool,
    pub gesture_mask: u8,
    pub hid_enabled: bool,
}

impl Default for RingConfig {
    fn default() -> Self {
        Self {
            sensitivity: 0x80,
            raw_stream_opt_in: false,
            gesture_mask: 0xFF,
            hid_enabled: true,
        }
    }
}

impl RingConfig {
    /// Byte index of the HID-enable flag in the config write.
    pub const HID_ENABLE_BYTE: usize = 3;

    /// Serializes the full 4-byte config write.
    pub fn to_bytes(self) -> [u8; 4] {
        [
            self.sensitivity,
            self.raw_stream_opt_in as u8,
            self.gesture_mask,
            self.hid_enabled as u8,
        ]
    }

    /// Parses a config read (missing trailing bytes take defaults, mirroring
    /// the write-side "shorter writes leave trailing fields" semantics).
    /// Host side of readable-C2 (ratified 2026-07-08): lets takeover writes
    /// preserve device state instead of clobbering it with defaults.
    pub fn from_bytes(bytes: &[u8]) -> Self {
        let default = Self::default();
        Self {
            sensitivity: bytes.first().copied().unwrap_or(default.sensitivity),
            raw_stream_opt_in: bytes
                .get(1)
                .map(|b| *b != 0)
                .unwrap_or(default.raw_stream_opt_in),
            gesture_mask: bytes.get(2).copied().unwrap_or(default.gesture_mask),
            hid_enabled: bytes
                .get(Self::HID_ENABLE_BYTE)
                .map(|b| *b != 0)
                .unwrap_or(default.hid_enabled),
        }
    }

    /// Convenience: default config with the HID projection toggled.
    pub fn with_hid(hid_enabled: bool) -> Self {
        Self {
            hid_enabled,
            ..Self::default()
        }
    }

    /// Returns a copy with only the HID flag changed — pair with
    /// `from_bytes` on a fresh device read for clobber-free takeover writes.
    pub fn hid_set(mut self, hid_enabled: bool) -> Self {
        self.hid_enabled = hid_enabled;
        self
    }
}

/// Returns the current wall-clock timestamp in milliseconds.
pub fn current_timestamp_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// Builds a command envelope around a payload.
pub fn command_envelope<T>(sequence: u64, payload: T) -> ProtocolEnvelope<T> {
    ProtocolEnvelope {
        protocol_version: SHARED_PROTOCOL_VERSION.to_string(),
        message_kind: ProtocolMessageKind::Command,
        message_id: uuid::Uuid::new_v4().to_string(),
        sequence,
        timestamp_ms: current_timestamp_ms(),
        payload,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Byte-for-byte examples of what the simulator's `BleProtocolAdapter`
    /// produces (shapes verified against haptic-harmony-simulator
    /// `src/protocol.rs` / `src/transport_adapters.rs`, 2026-07-02).
    #[test]
    fn parses_simulator_gesture_envelope() {
        let envelope_json = serde_json::json!({
            "protocol_version": "0.1.0",
            "message_kind": "event",
            "message_id": "6a3b0d54-0000-0000-0000-000000000000",
            "sequence": 0,
            "timestamp_ms": 200,
            "payload": {
                "event_kind": "gesture",
                "event": {
                    "gesture": { "gesture_kind": "slide", "direction": "up" },
                    "confidence": 0.93,
                    "timestamp_ms": 200
                }
            }
        });

        let envelope: ProtocolEnvelope<SimulatorEvent> =
            serde_json::from_value(envelope_json).expect("envelope must parse");
        match envelope.payload {
            SimulatorEvent::Gesture(event) => {
                assert_eq!(
                    event.gesture,
                    SemanticGesture::Slide {
                        direction: SemanticSlideDirection::Up
                    }
                );
                assert!((event.confidence - 0.93).abs() < f32::EPSILON);
            }
            other => panic!("expected gesture event, got {other:?}"),
        }
    }

    #[test]
    fn parses_ble_gesture_wrapper_with_embedded_envelope() {
        let inner = serde_json::json!({
            "protocol_version": "0.1.0",
            "message_kind": "event",
            "message_id": "m",
            "sequence": 1,
            "timestamp_ms": 75,
            "payload": {
                "event_kind": "gesture",
                "event": {
                    "gesture": { "gesture_kind": "tilt", "angle_degrees": -12.5 },
                    "confidence": 0.98,
                    "timestamp_ms": 75
                }
            }
        });
        let wrapper = serde_json::json!({
            "gesture_type": "tilt",
            "timestamp": 75,
            "confidence": 0.98,
            "data": serde_json::to_vec(&inner).unwrap()
        });

        let parsed: BleGestureData = serde_json::from_value(wrapper).unwrap();
        let envelope: ProtocolEnvelope<SimulatorEvent> =
            serde_json::from_slice(&parsed.data).expect("embedded envelope must parse");
        match envelope.payload {
            SimulatorEvent::Gesture(event) => {
                assert_eq!(
                    event.gesture,
                    SemanticGesture::Tilt {
                        angle_degrees: -12.5
                    }
                );
            }
            other => panic!("expected gesture event, got {other:?}"),
        }
    }

    #[test]
    fn haptic_command_envelope_matches_simulator_expectation() {
        let envelope = command_envelope(
            7,
            SimulatorCommand::Haptic(HapticCommandPayload {
                pattern: SemanticHapticPattern::Confirm,
            }),
        );
        let value = serde_json::to_value(&envelope).unwrap();

        // The simulator parses this via
        // `serde_json::from_slice::<ProtocolEnvelope<SimulatorCommand>>`.
        assert_eq!(value["protocol_version"], "0.3.0");
        assert_eq!(value["message_kind"], "command");
        assert_eq!(value["payload"]["command_kind"], "haptic");
        assert_eq!(
            value["payload"]["command"]["pattern"]["pattern_kind"],
            "confirm"
        );

        // Round-trip.
        let back: ProtocolEnvelope<SimulatorCommand> = serde_json::from_value(value).unwrap();
        assert_eq!(back.payload, envelope.payload);
    }

    #[test]
    fn v0_1_0_aliases_still_parse() {
        // Envelopes from v0.1.0 peers used `success`/`notify`; they must map
        // onto the ratified vocabulary on read.
        let success: SemanticHapticPattern =
            serde_json::from_value(serde_json::json!({ "pattern_kind": "success" })).unwrap();
        assert_eq!(success, SemanticHapticPattern::Confirm);

        let notify: SemanticHapticPattern =
            serde_json::from_value(serde_json::json!({ "pattern_kind": "notify" })).unwrap();
        assert_eq!(notify, SemanticHapticPattern::Tick);

        // And the ratified names parse as themselves.
        let double_tick: SemanticHapticPattern =
            serde_json::from_value(serde_json::json!({ "pattern_kind": "double_tick" })).unwrap();
        assert_eq!(double_tick, SemanticHapticPattern::DoubleTick);
    }

    #[test]
    fn v0_3_gesture_kinds_parse() {
        let swipe: SemanticGesture = serde_json::from_value(
            serde_json::json!({ "gesture_kind": "swipe", "direction": "left" }),
        )
        .unwrap();
        assert_eq!(
            swipe,
            SemanticGesture::Swipe {
                direction: SemanticSwipeDirection::Left
            }
        );

        let rotate: SemanticGesture = serde_json::from_value(
            serde_json::json!({ "gesture_kind": "rotate", "direction": "cw" }),
        )
        .unwrap();
        assert_eq!(
            rotate,
            SemanticGesture::Rotate {
                direction: SemanticRotateDirection::Cw
            }
        );
    }

    #[test]
    fn waveform_pattern_round_trips() {
        let pattern = SemanticHapticPattern::Waveform {
            data: "AAEC".to_string(),
            sample_rate_hz: 8000,
            intensity: 0.8,
        };
        let value = serde_json::to_value(&pattern).unwrap();
        assert_eq!(value["pattern_kind"], "waveform");
        assert_eq!(value["sample_rate_hz"], 8000);

        let back: SemanticHapticPattern = serde_json::from_value(value).unwrap();
        assert_eq!(back, pattern);
    }

    #[test]
    fn ack_event_round_trips() {
        let event = SimulatorEvent::Ack(AckPayload {
            sequence: 42,
            status: AckStatus::Denied,
            reason: Some("device is not enrolled".to_string()),
        });
        let value = serde_json::to_value(&event).unwrap();
        assert_eq!(value["event_kind"], "ack");
        assert_eq!(value["event"]["status"], "denied");

        let back: SimulatorEvent = serde_json::from_value(value).unwrap();
        assert_eq!(back, event);
    }

    #[test]
    fn ring_config_wire_layout() {
        // Default: HID on (firmware default), byte 3 is the HID flag.
        let default = RingConfig::default().to_bytes();
        assert_eq!(default, [0x80, 0, 0xFF, 1]);
        assert_eq!(default[RingConfig::HID_ENABLE_BYTE], 1);

        // Takeover write: HID suppressed, everything else at defaults.
        let takeover = RingConfig::with_hid(false).to_bytes();
        assert_eq!(takeover, [0x80, 0, 0xFF, 0]);
    }

    #[test]
    fn ring_config_read_modify_write_preserves_device_state() {
        // Device has non-default config (user-tuned sensitivity, raw stream
        // on, restricted gesture mask). A takeover must only touch byte 3.
        let device_bytes = [0x2A, 1, 0x0F, 1];
        let takeover = RingConfig::from_bytes(&device_bytes)
            .hid_set(false)
            .to_bytes();
        assert_eq!(takeover, [0x2A, 1, 0x0F, 0]);

        // Short reads (older firmware without byte 3) parse with defaults.
        let short = RingConfig::from_bytes(&[0x2A, 1, 0x0F]);
        assert!(short.hid_enabled);
        assert_eq!(short.sensitivity, 0x2A);

        // Empty read falls back to full defaults.
        assert_eq!(RingConfig::from_bytes(&[]), RingConfig::default());
    }

    #[test]
    fn parses_battery_shapes() {
        // Raw single-byte value (initial characteristic state in the simulator).
        let raw = [85u8];
        assert_eq!(raw.len(), 1);

        // JSON BleBatteryData (what update_battery_characteristic notifies).
        let json = serde_json::json!({
            "level": 42,
            "is_charging": true,
            "voltage": 3.9,
            "temperature": 25.0,
            "health": "Good",
            "time_remaining": 90
        });
        let parsed: BleBatteryData = serde_json::from_value(json).unwrap();
        assert_eq!(parsed.level, 42);
        assert!(parsed.is_charging);
    }
}
