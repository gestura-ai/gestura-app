/**
 * SDK tests against MockTransport + a fake WASM core. These verify the
 * wrapper's wiring (transport bytes → typed events, high-level calls →
 * transport writes). The REAL codec is covered by the Rust golden-vector
 * tests in `gestura-protocol`; here we stub WASM so the suite runs without a
 * built wasm package.
 */

import { describe, it, expect, vi } from "vitest";
import { GesturaRing, MockTransport } from "./index.js";
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

// A fake WASM core: just enough behavior to drive the wrapper. Real decoding
// is the Rust crate's job (golden-vector tested there).
function fakeWasm(): GesturaWasm {
  return {
    protocolVersion: () => "0.3.0",
    ringUuids: () => JSON.stringify(UUIDS),
    decodeGestureEvent: (bytes) => new TextDecoder().decode(bytes),
    decodeEvent: (bytes) => new TextDecoder().decode(bytes),
    decodeSensorFrame: (bytes) => new TextDecoder().decode(bytes),
    gestureToAction: (t) =>
      JSON.stringify({ action: t === "double_tap" ? "execute" : "confirm", confidence: 0.9 }),
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

describe("GesturaRing", () => {
  it("maps a double_tap gesture notification to a typed doubletap event + action", async () => {
    const transport = new MockTransport();
    const ring = await GesturaRing.open({ transport, wasm: fakeWasm() });

    const onDouble = vi.fn();
    const onGesture = vi.fn();
    ring.addEventListener("doubletap", onDouble);
    ring.addEventListener("gesture", onGesture);

    transport.emit(
      UUIDS.gestureEvent,
      new TextEncoder().encode(JSON.stringify({ gesture: { gesture_kind: "double_tap" }, confidence: 0.92 })),
    );

    expect(onDouble).toHaveBeenCalledOnce();
    expect(onGesture.mock.calls[0]![0].detail).toMatchObject({ type: "double_tap", action: "execute" });
  });

  it("maps swipe direction to swipeleft/swiperight", async () => {
    const transport = new MockTransport();
    const ring = await GesturaRing.open({ transport, wasm: fakeWasm() });
    const onLeft = vi.fn();
    ring.addEventListener("swipeleft", onLeft);
    transport.emit(
      UUIDS.gestureEvent,
      new TextEncoder().encode(
        JSON.stringify({ gesture: { gesture_kind: "swipe", direction: "left" }, confidence: 0.85 }),
      ),
    );
    expect(onLeft).toHaveBeenCalledOnce();
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

  it("rejects oversized waveforms client-side", async () => {
    const transport = new MockTransport();
    const ring = await GesturaRing.open({ transport, wasm: fakeWasm() });
    await expect(ring.sendWaveform(new Int16Array(2000))).rejects.toThrow(/1024/);
  });
});
