/**
 * SDK wiring tests against MockTransport + a FAKE WASM core: transport bytes
 * → typed events, high-level calls → transport writes. The real codec is
 * covered by the Rust golden-vector tests in `gestura-protocol` and by
 * `wasm.real.test.ts`, which drives the same wrapper through the actual
 * compiled core (the fake here once hid a crash in `encodeHapticCommand` and
 * an `unknown_gesture` mapping for every swipe/rotate).
 */

import { describe, it, expect, vi } from "vitest";
import { GesturaRing, MockTransport, trustPermits } from "./index.js";
import type { GesturaWasm } from "./wasm.js";

const UUIDS = {
  service: "e3b742d4-51c9-4f0e-9d26-7a48c1f0b9bc",
  hapticCommand: "e3b742d4-51c9-4f0e-9d26-7a48c1f0b9bd",
  gestureEvent: "e3b742d4-51c9-4f0e-9d26-7a48c1f0b9be",
  batteryLevel: "e3b742d4-51c9-4f0e-9d26-7a48c1f0b9bf",
  otaUpdate: "e3b742d4-51c9-4f0e-9d26-7a48c1f0b9c0",
  stateSnapshot: "e3b742d4-51c9-4f0e-9d26-7a48c1f0b9c1",
  config: "e3b742d4-51c9-4f0e-9d26-7a48c1f0b9c2",
  rawSensorStream: "e3b742d4-51c9-4f0e-9d26-7a48c1f0b9c3",
};

const ACTIONS: Record<string, string> = {
  tap: "confirm",
  double_tap: "execute",
  hold: "select",
  swipe_left: "previous",
  swipe_right: "next",
  rotate_cw: "increase",
  rotate_ccw: "decrease",
};

function label(g: { gesture_kind: string; direction?: string }): string {
  return g.direction ? `${g.gesture_kind}_${g.direction}` : g.gesture_kind;
}

// A fake WASM core: just enough behavior to drive the wrapper. Real decoding
// is the Rust crate's job (golden-vector tested there).
function fakeWasm(): GesturaWasm {
  return {
    protocolVersion: () => "0.3.0",
    ringUuids: () => JSON.stringify(UUIDS),
    decodeGestureEvent: (bytes) => {
      const raw = JSON.parse(new TextDecoder().decode(bytes)) as {
        gesture: { gesture_kind: string; direction?: string };
        confidence: number;
      };
      const l = label(raw.gesture);
      return JSON.stringify({
        gesture: raw.gesture,
        label: l,
        action: ACTIONS[l] ?? "unknown_gesture",
        actionConfidence: 0.9,
        confidence: raw.confidence,
        timestampMs: 42,
      });
    },
    decodeEvent: (bytes) => new TextDecoder().decode(bytes),
    decodeSensorFrame: (bytes) => new TextDecoder().decode(bytes),
    gestureAction: (json) => {
      const l = label(JSON.parse(json));
      return JSON.stringify({ label: l, action: ACTIONS[l] ?? "unknown_gesture", confidence: 0.9 });
    },
    gestureToAction: (t) => JSON.stringify({ action: ACTIONS[t] ?? "unknown_gesture", confidence: 0.9 }),
    encodeHapticCommand: (seq, patternJson) =>
      new TextEncoder().encode(JSON.stringify({ seq: Number(seq), patternJson })),
    encodeConfig: (s, r, m, h) => Uint8Array.from([s, r ? 1 : 0, m, h ? 1 : 0]),
    decodeConfig: (b) =>
      JSON.stringify({
        sensitivity: b[0] ?? 0x80,
        rawStreamOptIn: (b[1] ?? 0) !== 0,
        gestureMask: b[2] ?? 0xff,
        hidEnabled: (b[3] ?? 1) !== 0,
      }),
  };
}

const gestureBytes = (gesture: Record<string, unknown>, confidence = 0.9) =>
  new TextEncoder().encode(JSON.stringify({ gesture, confidence }));

describe("GesturaRing", () => {
  it("maps a double_tap gesture notification to a typed doubletap event + action", async () => {
    const transport = new MockTransport();
    const ring = await GesturaRing.open({ transport, wasm: fakeWasm() });

    const onDouble = vi.fn();
    const onGesture = vi.fn();
    ring.addEventListener("doubletap", onDouble);
    ring.addEventListener("gesture", onGesture);

    transport.emit(UUIDS.gestureEvent, gestureBytes({ gesture_kind: "double_tap" }, 0.92));

    expect(onDouble).toHaveBeenCalledOnce();
    expect(onGesture.mock.calls[0]![0].detail).toMatchObject({
      type: "double_tap",
      label: "double_tap",
      action: "execute",
      confidence: 0.92,
    });
  });

  it("maps swipe direction to swipeleft/swiperight and keeps the direction on the gesture event", async () => {
    const transport = new MockTransport();
    const ring = await GesturaRing.open({ transport, wasm: fakeWasm() });
    const onLeft = vi.fn();
    const onGesture = vi.fn();
    ring.addEventListener("swipeleft", onLeft);
    ring.addEventListener("gesture", onGesture);
    transport.emit(UUIDS.gestureEvent, gestureBytes({ gesture_kind: "swipe", direction: "left" }, 0.85));
    expect(onLeft).toHaveBeenCalledOnce();
    expect(onGesture.mock.calls[0]![0].detail).toMatchObject({
      type: "swipe",
      direction: "left",
      label: "swipe_left",
      action: "previous",
    });
  });

  it("drops a swipe with an unknown direction instead of guessing", async () => {
    const transport = new MockTransport();
    const ring = await GesturaRing.open({ transport, wasm: fakeWasm() });
    const onGesture = vi.fn();
    ring.addEventListener("gesture", onGesture);
    transport.emit(UUIDS.gestureEvent, gestureBytes({ gesture_kind: "swipe", direction: "up" }));
    expect(onGesture).not.toHaveBeenCalled();
  });

  it("emits sensorframe events from C3 notifications", async () => {
    const transport = new MockTransport();
    const ring = await GesturaRing.open({ transport, wasm: fakeWasm() });
    const onFrame = vi.fn();
    ring.addEventListener("sensorframe", onFrame);
    transport.emit(
      UUIDS.rawSensorStream,
      new TextEncoder().encode(JSON.stringify({ frame_version: 1, samples: [{ ax_mg: 100 }] })),
    );
    expect(onFrame).toHaveBeenCalledOnce();
    expect(onFrame.mock.calls[0]![0].detail.frame_version).toBe(1);
  });

  it("surfaces state snapshots (trust, degraded modes) and acks from the C1 characteristic", async () => {
    const transport = new MockTransport();
    const ring = await GesturaRing.open({ transport, wasm: fakeWasm() });
    const onSnapshot = vi.fn();
    const onBattery = vi.fn();
    const onAck = vi.fn();
    ring.addEventListener("statesnapshot", onSnapshot);
    ring.addEventListener("battery", onBattery);
    ring.addEventListener("ack", onAck);

    expect(ring.trustState).toBeUndefined();
    transport.emit(
      UUIDS.stateSnapshot,
      new TextEncoder().encode(
        JSON.stringify({
          kind: "stateSnapshot",
          event: {
            battery: { level_percent: 88, is_charging: true },
            trust_state: "bonded",
            degraded_modes: ["low_battery"],
            privileged_actions_enabled: false,
          },
        }),
      ),
    );
    expect(onSnapshot).toHaveBeenCalledOnce();
    expect(onSnapshot.mock.calls[0]![0].detail.trust_state).toBe("bonded");
    expect(onBattery.mock.calls[0]![0].detail).toEqual({ levelPercent: 88 });
    expect(ring.trustState).toBe("bonded");
    expect(ring.lastSnapshot?.degraded_modes).toEqual(["low_battery"]);

    transport.emit(
      UUIDS.stateSnapshot,
      new TextEncoder().encode(
        JSON.stringify({ kind: "ack", event: { sequence: 3, status: "denied", reason: "not enrolled" } }),
      ),
    );
    expect(onAck.mock.calls[0]![0].detail).toMatchObject({ sequence: 3, status: "denied" });
  });

  it("config write is read-modify-write (preserves untouched bytes)", async () => {
    const transport = new MockTransport();
    // Device has non-default config: sensitivity 0x2A, stream on, mask 0x0F.
    transport.seed(UUIDS.config, Uint8Array.from([0x2a, 1, 0x0f, 1]));
    const ring = await GesturaRing.open({ transport, wasm: fakeWasm() });

    await ring.takeOverHid(); // flips only hidEnabled → 0

    const write = transport.writes.at(-1)!;
    expect(write.uuid).toBe(UUIDS.config);
    expect(Array.from(write.bytes)).toEqual([0x2a, 1, 0x0f, 0]); // bytes 0-2 preserved
  });

  it("close() restores HID when this session suppressed it, before disconnecting", async () => {
    const transport = new MockTransport();
    transport.seed(UUIDS.config, Uint8Array.from([0x2a, 1, 0x0f, 1]));
    const ring = await GesturaRing.open({ transport, wasm: fakeWasm() });
    await ring.takeOverHid();
    await ring.close();
    const last = transport.writes.at(-1)!;
    expect(last.uuid).toBe(UUIDS.config);
    expect(Array.from(last.bytes)).toEqual([0x2a, 1, 0x0f, 1]);

    // …and does NOT touch config when it never took HID over.
    const t2 = new MockTransport();
    const r2 = await GesturaRing.open({ transport: t2, wasm: fakeWasm() });
    await r2.close();
    expect(t2.writes).toHaveLength(0);
  });

  it("rejects oversized waveforms client-side", async () => {
    const transport = new MockTransport();
    const ring = await GesturaRing.open({ transport, wasm: fakeWasm() });
    await expect(ring.sendWaveform(new Int16Array(2000))).rejects.toThrow(/1024/);
  });
});

describe("trustPermits", () => {
  it("follows the ladder and never lets revoked through", () => {
    expect(trustPermits("bonded", "bonded")).toBe(true);
    expect(trustPermits("attested", "bonded")).toBe(true);
    expect(trustPermits("discovered", "bonded")).toBe(false);
    expect(trustPermits("revoked", "discovered")).toBe(false);
    expect(trustPermits(undefined, "discovered")).toBe(false);
  });
});
