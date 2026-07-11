//! WebAssembly surface for the TypeScript SDK.
//!
//! Deliberately thin and dependency-light: every function takes/returns byte
//! arrays (Uint8Array) and JSON strings, so the only wasm dep is
//! `wasm-bindgen` (no `serde-wasm-bindgen`). The TS layer JSON.parses the
//! results. This crate is the ONE source of truth for the wire format — the
//! TS SDK never re-implements a codec.
//!
//! Build (from this crate dir):
//! ```sh
//! wasm-pack build --release --features wasm --target web
//! ```
//! or `--target bundler` for the Tauri/Vite frontend. The generated package
//! lands in `pkg/`; the TS SDK depends on it.

use wasm_bindgen::prelude::*;

use crate::{
    HapticCommandPayload, ProtocolEnvelope, RingConfig, SemanticHapticPattern, SensorFrame,
    SimulatorCommand, SimulatorEvent, command_envelope, gesture_to_action, ring_uuids,
};

/// The ratified Shared Semantic Protocol version this module implements.
#[wasm_bindgen(js_name = protocolVersion)]
pub fn protocol_version() -> String {
    crate::SHARED_PROTOCOL_VERSION.to_string()
}

/// Ring GATT UUIDs as a JSON object (lowercased hyphenated strings), so the
/// TS transport targets characteristics by the canonical allocation without
/// hardcoding them.
#[wasm_bindgen(js_name = ringUuids)]
pub fn ring_uuids_json() -> String {
    serde_json::json!({
        "service": ring_uuids::HAPTIC_SERVICE_UUID.to_string(),
        "hapticCommand": ring_uuids::HAPTIC_COMMAND_UUID.to_string(),
        "gestureEvent": ring_uuids::GESTURE_EVENT_UUID.to_string(),
        "batteryLevel": ring_uuids::BATTERY_LEVEL_UUID.to_string(),
        "otaUpdate": ring_uuids::OTA_UPDATE_UUID.to_string(),
        "stateSnapshot": ring_uuids::STATE_SNAPSHOT_UUID.to_string(),
        "config": ring_uuids::CONFIG_UUID.to_string(),
        "rawSensorStream": ring_uuids::RAW_SENSOR_STREAM_UUID.to_string(),
    })
    .to_string()
}

/// Decodes a notification from the gesture characteristic — the bare
/// `ProtocolEnvelope<SimulatorEvent>` the ring emits. Returns the gesture as
/// JSON `{gesture, confidence, timestampMs}` (with `gesture` = the semantic
/// gesture object), or `null` if the payload isn't a gesture event.
#[wasm_bindgen(js_name = decodeGestureEvent)]
pub fn decode_gesture_event(bytes: &[u8]) -> Option<String> {
    let envelope: ProtocolEnvelope<SimulatorEvent> = serde_json::from_slice(bytes).ok()?;
    match envelope.payload {
        SimulatorEvent::Gesture(ev) => Some(
            serde_json::json!({
                "gesture": ev.gesture,
                "confidence": ev.confidence,
                "timestampMs": ev.timestamp_ms,
            })
            .to_string(),
        ),
        _ => None,
    }
}

/// Decodes a full envelope notification (gesture / battery / state snapshot /
/// ack) into `{kind, event}` JSON, so the TS layer can route any state-
/// characteristic notification. `null` on a non-envelope payload.
#[wasm_bindgen(js_name = decodeEvent)]
pub fn decode_event(bytes: &[u8]) -> Option<String> {
    let envelope: ProtocolEnvelope<SimulatorEvent> = serde_json::from_slice(bytes).ok()?;
    let (kind, event) = match &envelope.payload {
        SimulatorEvent::Gesture(e) => ("gesture", serde_json::to_value(e).ok()?),
        SimulatorEvent::Battery(e) => ("battery", serde_json::to_value(e).ok()?),
        SimulatorEvent::StateSnapshot(e) => ("stateSnapshot", serde_json::to_value(e).ok()?),
        SimulatorEvent::Ack(e) => ("ack", serde_json::to_value(e).ok()?),
    };
    Some(serde_json::json!({ "kind": kind, "event": event }).to_string())
}

/// Decodes a C3 raw sensor frame (binary) into JSON. Throws on malformed
/// input so the TS side surfaces a real error.
#[wasm_bindgen(js_name = decodeSensorFrame)]
pub fn decode_sensor_frame(bytes: &[u8]) -> Result<String, JsError> {
    let frame = SensorFrame::decode(bytes).map_err(|e| JsError::new(&e.to_string()))?;
    serde_json::to_string(&frame).map_err(|e| JsError::new(&e.to_string()))
}

/// Maps a gesture type string to `{action, confidence}` JSON.
#[wasm_bindgen(js_name = gestureToAction)]
pub fn gesture_to_action_json(gesture_type: &str) -> String {
    let (action, confidence) = gesture_to_action(gesture_type);
    serde_json::json!({ "action": action, "confidence": confidence }).to_string()
}

/// Encodes a haptic command to the bytes written to the haptic characteristic.
/// `pattern_json` is a `SemanticHapticPattern` (e.g.
/// `{"pattern_kind":"tick"}` or a `waveform`/`custom` object). `sequence`
/// correlates the resulting `ack`.
#[wasm_bindgen(js_name = encodeHapticCommand)]
pub fn encode_haptic_command(sequence: u64, pattern_json: &str) -> Result<Vec<u8>, JsError> {
    let pattern: SemanticHapticPattern =
        serde_json::from_str(pattern_json).map_err(|e| JsError::new(&e.to_string()))?;
    let envelope = command_envelope(
        sequence,
        SimulatorCommand::Haptic(HapticCommandPayload { pattern }),
    );
    serde_json::to_vec(&envelope).map_err(|e| JsError::new(&e.to_string()))
}

/// Encodes the 4-byte config characteristic write.
#[wasm_bindgen(js_name = encodeConfig)]
pub fn encode_config(
    sensitivity: u8,
    raw_stream_opt_in: bool,
    gesture_mask: u8,
    hid_enabled: bool,
) -> Vec<u8> {
    RingConfig {
        sensitivity,
        raw_stream_opt_in,
        gesture_mask,
        hid_enabled,
    }
    .to_bytes()
    .to_vec()
}

/// Parses a config read-back (readable-C2) into `{sensitivity, rawStreamOptIn,
/// gestureMask, hidEnabled}` JSON — for clobber-free read-modify-write.
#[wasm_bindgen(js_name = decodeConfig)]
pub fn decode_config(bytes: &[u8]) -> String {
    let c = RingConfig::from_bytes(bytes);
    serde_json::json!({
        "sensitivity": c.sensitivity,
        "rawStreamOptIn": c.raw_stream_opt_in,
        "gestureMask": c.gesture_mask,
        "hidEnabled": c.hid_enabled,
    })
    .to_string()
}
