/**
 * Typed view of the WebAssembly core generated from the `gestura-protocol`
 * Rust crate (`wasm-pack build --features wasm`). The codec lives in Rust —
 * this module only declares the shapes the wrapper consumes, so there is one
 * source of truth for the wire format and zero re-implementation in TS.
 *
 * The generated package is imported lazily so the SDK can be loaded in
 * environments that initialize WASM differently (bundler vs web target).
 */

// Shape of the wasm-pack `bundler`-target module. `@gestura/protocol-wasm` is
// the generated package name (see the SDK build script); until it's built,
// this import is declared, not resolved.
export interface GesturaWasm {
  protocolVersion(): string;
  ringUuids(): string;
  decodeGestureEvent(bytes: Uint8Array): string | undefined;
  decodeEvent(bytes: Uint8Array): string | undefined;
  decodeSensorFrame(bytes: Uint8Array): string;
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

let cached: GesturaWasm | undefined;

/**
 * Loads the WASM core once. `loader` lets the host app control how the
 * generated module is imported (bundler resolves `@gestura/protocol-wasm`
 * directly; a web target may need `init()` first).
 */
export async function loadWasm(loader?: () => Promise<GesturaWasm>): Promise<GesturaWasm> {
  if (cached) return cached;
  cached = loader
    ? await loader()
    : ((await import(/* @vite-ignore */ "@gestura/protocol-wasm")) as unknown as GesturaWasm);
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
