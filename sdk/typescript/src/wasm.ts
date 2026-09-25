/**
 * Typed view of the WebAssembly core generated from the `gestura-protocol`
 * Rust crate (`wasm-pack build --target web --features wasm`, then
 * `scripts/inline-wasm.mjs`). The codec lives in Rust — this module only
 * declares the shapes the wrapper consumes, so there is one source of truth
 * for the wire format and zero re-implementation in TS.
 *
 * Two generated modules ship in `../wasm`:
 * - `gestura_protocol_inline.js` — the .wasm embedded as base64, initialized
 *   synchronously on import. Zero-config: works in Node, Vitest, Vite,
 *   webpack and Tauri WebViews with no WASM plugin. This is the default.
 * - `gestura_protocol.js` — the plain wasm-bindgen `web` module whose default
 *   export `init()` fetches `gestura_protocol_bg.wasm`; ~25% smaller on the
 *   wire for hosts that can serve the .wasm and prefer to stream it.
 */

/** Shape of the wasm-bindgen module the wrapper consumes. */
export interface GesturaWasm {
  protocolVersion(): string;
  ringUuids(): string;
  /** Gesture notification (bare envelope or legacy wrapper) → JSON or undefined. */
  decodeGestureEvent(bytes: Uint8Array): string | undefined;
  /** Any event notification → `{kind, event, sequence, timestampMs}` JSON or undefined. */
  decodeEvent(bytes: Uint8Array): string | undefined;
  decodeSensorFrame(bytes: Uint8Array): string;
  /** Typed gesture object (JSON) → `{label, action, confidence}` JSON. */
  gestureAction(gestureJson: string): string;
  /** Gesture LABEL → `{action, confidence}` JSON (legacy; cannot see a direction). */
  gestureToAction(gestureType: string): string;
  encodeHapticCommand(sequence: bigint, patternJson: string): Uint8Array;
  encodeConfig(
    sensitivity: number,
    rawStreamOptIn: boolean,
    gestureMask: number,
    hidEnabled: boolean,
  ): Uint8Array;
  decodeConfig(bytes: Uint8Array): string;
}

let cached: Promise<GesturaWasm> | undefined;

/**
 * Loads the WASM core once. With no `loader`, the inlined module is used
 * (no configuration needed anywhere). Pass a `loader` to use the streaming
 * module instead, e.g.
 * ```ts
 * import init, * as core from "@gestura/ring-sdk/wasm";
 * const ring = await GesturaRing.open({ transport, wasm: await loadWasm(async () => { await init(); return core; }) });
 * ```
 */
export async function loadWasm(loader?: () => Promise<GesturaWasm>): Promise<GesturaWasm> {
  if (!cached) {
    cached = loader
      ? loader()
      : (import("../wasm/gestura_protocol_inline.js") as unknown as Promise<GesturaWasm>);
  }
  return cached;
}

/** Ring GATT characteristic UUIDs (from the Rust `ring_uuids`). */
export interface RingUuids {
  service: string;
  hapticCommand: string;
  gestureEvent: string;
  batteryLevel: string;
  otaUpdate: string;
  stateSnapshot: string;
  config: string;
  rawSensorStream: string;
}
