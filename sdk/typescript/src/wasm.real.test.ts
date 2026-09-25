/**
 * The wrapper driven through the REAL compiled protocol core (the inlined
 * WASM module `npm run build:wasm` produces). No fakes: these are the checks
 * that the stubbed suite structurally cannot make — encode paths that call
 * into Rust, the gesture→action table, both gesture wire shapes, and the
 * firmware golden vectors.
 */

import { describe, it, expect, vi } from "vitest";
import { GesturaRing, MockTransport, loadWasm } from "./index.js";

const enc = new TextEncoder();
const dec = new TextDecoder();

function envelope(payload: unknown, sequence = 1, timestampMs = 1000): Uint8Array {
  return enc.encode(
    JSON.stringify({
      protocol_version: "0.3.0",
      message_kind: "event",
      message_id: `fw-${sequence}`,
      sequence,
      timestamp_ms: timestampMs,
      payload,
    }),
  );
}

const gestureEnvelope = (gesture: Record<string, unknown>, seq = 1) =>
  envelope({ event_kind: "gesture", event: { gesture, confidence: 0.94, timestamp_ms: 1000 + seq } }, seq, 1000 + seq);

// Byte-exact state snapshot from the firmware conformance suite (@102f520).
const GOLDEN_SNAPSHOT =
  '{"protocol_version":"0.3.0","message_kind":"event","message_id":"fw-5","sequence":0,"timestamp_ms":0,"payload":{"event_kind":"state_snapshot","event":{"battery":{"level_percent":88,"is_charging":true,"voltage":3.987,"temperature_celsius":0.0,"health":"unknown","time_remaining_minutes":null},"trust_state":"bonded","degraded_modes":[],"firmware_version":"0.1.0-dev","protocol_version":"0.3.0","revocation_reason":null,"privileged_actions_enabled":true}}}';

// FRAME_HEX from conformance/vectors/sensor_frame.expected (device-core 1.1).
const GOLDEN_FRAME_HEX =
  "010104030201020a640038ff2c01c2012efb00000102010000000000e80300000000000001020000";

describe("real WASM core", () => {
  it("loads and reports the ratified version and UUID allocation", async () => {
    const wasm = await loadWasm();
    expect(wasm.protocolVersion()).toBe("0.3.0");
    const uuids = JSON.parse(wasm.ringUuids()) as Record<string, string>;
    expect(uuids.service).toBe("e3b742d4-51c9-4f0e-9d26-7a48c1f0b9bc");
    expect(uuids.rawSensorStream).toBe("e3b742d4-51c9-4f0e-9d26-7a48c1f0b9c3");
  });

  it("encodes haptic commands without trapping (SystemTime on wasm32 regression)", async () => {
    const transport = new MockTransport();
    const ring = await GesturaRing.open({ transport });
    const seq = await ring.sendHaptic("confirm");
    expect(seq).toBe(1n);
    const written = JSON.parse(dec.decode(transport.writes.at(-1)!.bytes)) as Record<string, unknown>;
    expect(written).toMatchObject({
      protocol_version: "0.3.0",
      message_kind: "command",
      sequence: 1,
      payload: { command_kind: "haptic", command: { pattern: { pattern_kind: "confirm" } } },
    });
    expect(written.timestamp_ms as number).toBeGreaterThan(1_577_836_800_000); // host wall clock

    const wseq = await ring.sendWaveform(Int16Array.from([0, 1000, -1000]), 8000, 0.8);
    expect(wseq).toBe(2n);
    const wave = JSON.parse(dec.decode(transport.writes.at(-1)!.bytes)) as {
      payload: { command: { pattern: { pattern_kind: string; data: string; sample_rate_hz: number } } };
    };
    expect(wave.payload.command.pattern.pattern_kind).toBe("waveform");
    expect(wave.payload.command.pattern.sample_rate_hz).toBe(8000);
    // int16 LE [0, 1000, -1000] = 00 00 E8 03 18 FC
    expect(wave.payload.command.pattern.data).toBe("AADoAxj8");
  });

  it("maps all seven device-truth gestures to typed events AND real actions", async () => {
    const transport = new MockTransport();
    const ring = await GesturaRing.open({ transport });
    const uuids = ring.ringUuids;
    const typed = vi.fn();
    const gestures = vi.fn();
    for (const n of ["tap", "doubletap", "holdend", "swipeleft", "swiperight", "rotatecw", "rotateccw"] as const) {
      ring.addEventListener(n, () => typed(n));
    }
    ring.addEventListener("gesture", (e) => gestures(e.detail));

    const cases: Array<[Record<string, unknown>, string, string]> = [
      [{ gesture_kind: "tap" }, "tap", "confirm"],
      [{ gesture_kind: "double_tap" }, "doubletap", "execute"],
      [{ gesture_kind: "hold", duration_ms: 820 }, "holdend", "select"],
      [{ gesture_kind: "swipe", direction: "left" }, "swipeleft", "previous"],
      [{ gesture_kind: "swipe", direction: "right" }, "swiperight", "next"],
      [{ gesture_kind: "rotate", direction: "cw" }, "rotatecw", "increase"],
      [{ gesture_kind: "rotate", direction: "ccw" }, "rotateccw", "decrease"],
    ];
    cases.forEach(([g], i) => transport.emit(uuids.gestureEvent, gestureEnvelope(g, i + 1)));

    expect(typed.mock.calls.map((c) => c[0])).toEqual(cases.map((c) => c[1]));
    expect(gestures.mock.calls.map((c) => c[0].action)).toEqual(cases.map((c) => c[2]));
    expect(gestures.mock.calls[3]![0]).toMatchObject({ label: "swipe_left", direction: "left", timestampMs: 1004 });
    expect(gestures.mock.calls.every((c) => c[0].actionConfidence > 0.5)).toBe(true);
  });

  it("accepts the legacy simulator BleGestureData wrapper as well as the bare envelope", async () => {
    const transport = new MockTransport();
    const ring = await GesturaRing.open({ transport });
    const onTap = vi.fn();
    ring.addEventListener("tap", onTap);
    const inner = Array.from(gestureEnvelope({ gesture_kind: "tap" }, 7));
    transport.emit(
      ring.ringUuids.gestureEvent,
      enc.encode(JSON.stringify({ gesture_type: "tap", timestamp: 1007, confidence: 0.94, data: inner })),
    );
    expect(onTap).toHaveBeenCalledOnce();
    // Garbage and non-gesture envelopes are dropped silently.
    transport.emit(ring.ringUuids.gestureEvent, enc.encode("not json"));
    transport.emit(ring.ringUuids.gestureEvent, envelope({ event_kind: "ack", event: { sequence: 1, status: "ok", reason: null } }));
    expect(onTap).toHaveBeenCalledOnce();
  });

  it("decodes the firmware golden state snapshot and ack envelopes on C1", async () => {
    const transport = new MockTransport();
    const ring = await GesturaRing.open({ transport });
    const onSnapshot = vi.fn();
    const onAck = vi.fn();
    ring.addEventListener("statesnapshot", onSnapshot);
    ring.addEventListener("ack", onAck);

    transport.emit(ring.ringUuids.stateSnapshot, enc.encode(GOLDEN_SNAPSHOT));
    expect(onSnapshot.mock.calls[0]![0].detail).toMatchObject({
      trust_state: "bonded",
      firmware_version: "0.1.0-dev",
      privileged_actions_enabled: true,
      battery: { level_percent: 88, is_charging: true },
    });
    expect(ring.trustState).toBe("bonded");

    transport.emit(
      ring.ringUuids.stateSnapshot,
      envelope({ event_kind: "ack", event: { sequence: 1, status: "denied", reason: "not bonded" } }),
    );
    expect(onAck.mock.calls[0]![0].detail).toEqual({ sequence: 1, status: "denied", reason: "not bonded" });
  });

  it("decodes the firmware golden C3 sensor frame byte-for-byte", async () => {
    const transport = new MockTransport();
    const ring = await GesturaRing.open({ transport });
    const onFrame = vi.fn();
    ring.addEventListener("sensorframe", onFrame);
    const bytes = Uint8Array.from(GOLDEN_FRAME_HEX.match(/../g)!.map((h) => parseInt(h, 16)));
    transport.emit(ring.ringUuids.rawSensorStream, bytes);
    const frame = onFrame.mock.calls[0]![0].detail;
    expect(frame.frame_version).toBe(1);
    expect(frame.touch_valid).toBe(true);
    expect(frame.t0_ms).toBe(0x01020304);
    expect(frame.period_ms).toBe(10);
    expect(frame.samples).toHaveLength(2);
    expect(frame.samples[0]).toMatchObject({ ax_mg: 100, ay_mg: -200, az_mg: 300, gx_ddps: 450, gy_ddps: -1234, slider_pos: 513, touched: true });
    expect(frame.samples[1]).toMatchObject({ az_mg: 1000, touched: false });
    // A truncated frame is dropped, not thrown.
    transport.emit(ring.ringUuids.rawSensorStream, bytes.subarray(0, 20));
    expect(onFrame).toHaveBeenCalledOnce();
  });

  it("config encode/decode round-trips through Rust", async () => {
    const transport = new MockTransport();
    transport.seed("e3b742d4-51c9-4f0e-9d26-7a48c1f0b9c2", Uint8Array.from([0x2a, 1, 0x0f, 1]));
    const ring = await GesturaRing.open({ transport });
    await ring.takeOverHid();
    expect(Array.from(transport.writes.at(-1)!.bytes)).toEqual([0x2a, 1, 0x0f, 0]);
    await ring.enableSensorStream(false);
    expect(Array.from(transport.writes.at(-1)!.bytes)).toEqual([0x2a, 0, 0x0f, 0]);
    await ring.close();
    expect(Array.from(transport.writes.at(-1)!.bytes)).toEqual([0x2a, 0, 0x0f, 1]);
  });

  it("exposes the typed action table directly", async () => {
    const wasm = await loadWasm();
    const rotate = JSON.parse(wasm.gestureAction('{"gesture_kind":"rotate","direction":"ccw"}')) as {
      label: string;
      action: string;
      confidence: number;
    };
    expect(rotate).toMatchObject({ label: "rotate_ccw", action: "decrease" });
    expect(rotate.confidence).toBeCloseTo(0.82, 5); // f32 on the wire
    expect(JSON.parse(wasm.gestureToAction("swipe")).action).toBe("unknown_gesture"); // no direction → no guess
    expect(JSON.parse(wasm.gestureToAction("swipe_right")).action).toBe("next");
  });
});
