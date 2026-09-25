/**
 * @gestura/ring-sdk — ergonomic TypeScript API for the Haptica Harmony B1
 * ring and its simulator.
 *
 * Design: the wire codec is the Rust `gestura-protocol` crate compiled to
 * WebAssembly — this module never re-implements it. It binds that core to a
 * pluggable {@link RingTransport} and exposes a W3C UI-Events-style surface
 * (`ring.addEventListener("doubletap", …)`), matching PROTOCOL.md's TS naming.
 *
 * @example
 * ```ts
 * const ring = await GesturaRing.open({ transport });
 * ring.addEventListener("doubletap", () => console.log("execute!"));
 * ring.addEventListener("gesture", (e) => console.log(e.detail.label, "→", e.detail.action));
 * ring.addEventListener("statesnapshot", (e) => console.log("trust:", e.detail.trust_state));
 * ring.addEventListener("sensorframe", (e) => render(e.detail.samples));
 * await ring.sendHaptic("tick");
 * await ring.enableSensorStream(true);
 * ```
 */

import { loadWasm, type GesturaWasm, type RingUuids } from "./wasm.js";
import type { RingTransport } from "./transport.js";

export type { RingTransport } from "./transport.js";
export { MockTransport } from "./transport.js";
export { loadWasm } from "./wasm.js";
export type { GesturaWasm, RingUuids } from "./wasm.js";

/** Semantic haptic patterns (ratified v0.3 vocabulary). */
export type HapticPattern = "confirm" | "error" | "tick" | "doubleTick";

/** Decoded C3 raw sensor sample (accel mg, gyro deci-dps). */
export interface SensorSample {
  ax_mg: number;
  ay_mg: number;
  az_mg: number;
  gx_ddps: number;
  gy_ddps: number;
  gz_ddps: number;
  slider_pos: number;
  touched: boolean;
}

export interface SensorFrame {
  frame_version: number;
  touch_valid: boolean;
  t0_ms: number;
  period_ms: number;
  samples: SensorSample[];
}

/**
 * Trust ladder (PROTOCOL.md). A REAL ring only ever reports `discovered`,
 * `bonded` or `revoked`; `enrolled`/`attested` are host/policy-side states.
 * `revoked` is not "above" `attested` — it fails closed. Compare with
 * {@link trustPermits}, never lexically or by position.
 */
export type TrustState = "discovered" | "bonded" | "enrolled" | "attested" | "revoked";

export type DegradedMode = "low_battery" | "sensor_fault" | "firmware_mismatch" | "operator_blocked";

export interface BatterySnapshot {
  level_percent: number;
  is_charging: boolean;
  voltage: number;
  temperature_celsius: number;
  health: string;
  time_remaining_minutes: number | null;
}

/** Device state snapshot (C1), the input to any host-side safety gate. */
export interface DeviceStateSnapshot {
  battery: BatterySnapshot;
  trust_state: TrustState;
  degraded_modes: DegradedMode[];
  firmware_version: string;
  protocol_version: string;
  revocation_reason: string | null;
  privileged_actions_enabled: boolean;
}

const TRUST_RANK: Record<Exclude<TrustState, "revoked">, number> = {
  discovered: 0,
  bonded: 1,
  enrolled: 2,
  attested: 3,
};

/**
 * True when `state` satisfies `required` on the trust ladder. `revoked`
 * never permits anything, whatever is required.
 */
export function trustPermits(state: TrustState | undefined, required: Exclude<TrustState, "revoked">): boolean {
  if (state === undefined || state === "revoked") return false;
  return TRUST_RANK[state] >= TRUST_RANK[required];
}

/**
 * Detail of the catch-all `gesture` event. Fires for every device-truth
 * gesture (the kinds a real ring emits); the simulator-only `slide`/`tilt`
 * kinds are not surfaced as events.
 */
export interface GestureEventDetail {
  /** Wire kind: `tap`, `double_tap`, `hold`, `swipe`, `rotate`. */
  type: string;
  /** Canonical label, e.g. `swipe_left`, `rotate_cw` (direction included). */
  label: string;
  /** Present for `swipe`/`rotate`. */
  direction?: string;
  /** Device classifier confidence, 0–1. */
  confidence: number;
  /** Default action from the SDK's gesture→action table (see PROTOCOL.md). */
  action: string;
  /** Mapping confidence of `action`, 0–1. */
  actionConfidence: number;
  /** Device uptime ms of the gesture (never epoch time). */
  timestampMs: number;
}

/** Event map for `GesturaRing` (W3C-style lowercase names). */
export interface GesturaRingEventMap {
  tap: CustomEvent<{ confidence: number }>;
  doubletap: CustomEvent<{ confidence: number }>;
  holdstart: CustomEvent<{ confidence: number }>;
  holdend: CustomEvent<{ confidence: number; durationMs: number }>;
  swipeleft: CustomEvent<{ confidence: number }>;
  swiperight: CustomEvent<{ confidence: number }>;
  rotatecw: CustomEvent<{ confidence: number }>;
  rotateccw: CustomEvent<{ confidence: number }>;
  /** Any device-truth gesture, with its label and mapped default action. */
  gesture: CustomEvent<GestureEventDetail>;
  /** C3 raw sensor stream frame (~5/s at 100 Hz, 20-sample batches). */
  sensorframe: CustomEvent<SensorFrame>;
  battery: CustomEvent<{ levelPercent: number }>;
  /** Full device state (trust, degraded modes, battery) — C1 notifications. */
  statesnapshot: CustomEvent<DeviceStateSnapshot>;
  ack: CustomEvent<{ sequence: number; status: string; reason: string | null }>;
}

interface SemanticGesture {
  gesture_kind: string;
  direction?: string;
  duration_ms?: number;
  angle_degrees?: number;
}

/** What `decodeGestureEvent` returns (see `gestura-protocol/src/wasm.rs`). */
interface DecodedGesture {
  gesture: SemanticGesture;
  label: string;
  action: string;
  actionConfidence: number;
  confidence: number;
  timestampMs: number;
}

/** Maps a decoded semantic gesture to its W3C event name + detail. */
function gestureToEvent(
  g: SemanticGesture,
  confidence: number,
): { name: keyof GesturaRingEventMap; detail: Record<string, unknown> } | undefined {
  switch (g.gesture_kind) {
    case "tap":
      return { name: "tap", detail: { confidence } };
    case "double_tap":
      return { name: "doubletap", detail: { confidence } };
    case "hold":
      // Firmware emits a single hold event at release with duration.
      return { name: "holdend", detail: { confidence, durationMs: g.duration_ms ?? 0 } };
    // Unknown/missing directions are dropped, not defaulted — a malformed or
    // future-extended payload must not masquerade as a real user gesture.
    case "swipe":
      return g.direction === "left"
        ? { name: "swipeleft", detail: { confidence } }
        : g.direction === "right"
          ? { name: "swiperight", detail: { confidence } }
          : undefined;
    case "rotate":
      return g.direction === "cw"
        ? { name: "rotatecw", detail: { confidence } }
        : g.direction === "ccw"
          ? { name: "rotateccw", detail: { confidence } }
          : undefined;
    default:
      return undefined;
  }
}

export interface OpenOptions {
  transport: RingTransport;
  /** Optional explicit WASM core (defaults to the inlined module). */
  wasm?: GesturaWasm;
}

/**
 * A connected ring (or simulator). Extends `EventTarget`, so the standard
 * `addEventListener`/`removeEventListener` idiom applies with the typed event
 * map above.
 */
export class GesturaRing extends EventTarget {
  private seq = 1n;
  private unsubscribers: Array<() => void> = [];
  private hidSuppressed = false;
  private snapshot: DeviceStateSnapshot | undefined;

  private constructor(
    private readonly transport: RingTransport,
    private readonly wasm: GesturaWasm,
    private readonly uuids: RingUuids,
  ) {
    super();
  }

  /** Opens a ring over a transport, loading the WASM core if needed. */
  static async open(opts: OpenOptions): Promise<GesturaRing> {
    const wasm = opts.wasm ?? (await loadWasm());
    const uuids = JSON.parse(wasm.ringUuids()) as RingUuids;
    const ring = new GesturaRing(opts.transport, wasm, uuids);
    ring.wire();
    return ring;
  }

  /** The ratified protocol version the WASM core implements. */
  get protocolVersion(): string {
    return this.wasm.protocolVersion();
  }

  /** Ring GATT UUIDs (from the canonical Rust allocation). */
  get ringUuids(): RingUuids {
    return this.uuids;
  }

  /** Last device state snapshot received, if any. */
  get lastSnapshot(): DeviceStateSnapshot | undefined {
    return this.snapshot;
  }

  /** Trust state from the last snapshot; `undefined` until one arrives. */
  get trustState(): TrustState | undefined {
    return this.snapshot?.trust_state;
  }

  private wire(): void {
    // Gesture characteristic → gesture events. The core accepts both wire
    // shapes (bare envelope from firmware, legacy simulator wrapper).
    this.unsubscribers.push(
      this.transport.onNotify(this.uuids.gestureEvent, (bytes) => {
        const json = this.wasm.decodeGestureEvent(bytes);
        if (!json) return;
        const parsed = JSON.parse(json) as DecodedGesture;
        const mapped = gestureToEvent(parsed.gesture, parsed.confidence);
        if (!mapped) return;
        this.dispatchEvent(new CustomEvent(mapped.name, { detail: mapped.detail }));
        const detail: GestureEventDetail = {
          type: parsed.gesture.gesture_kind,
          label: parsed.label,
          ...(parsed.gesture.direction !== undefined ? { direction: parsed.gesture.direction } : {}),
          confidence: parsed.confidence,
          action: parsed.action,
          actionConfidence: parsed.actionConfidence,
          timestampMs: parsed.timestampMs,
        };
        this.dispatchEvent(new CustomEvent("gesture", { detail }));
      }),
    );

    // Battery characteristic → raw byte (hardware) or JSON (legacy sim).
    this.unsubscribers.push(
      this.transport.onNotify(this.uuids.batteryLevel, (bytes) => {
        const level = bytes.length === 1 ? bytes[0]! : this.parseBatteryJson(bytes);
        if (level !== undefined) {
          this.dispatchEvent(new CustomEvent("battery", { detail: { levelPercent: level } }));
        }
      }),
    );

    // State-snapshot characteristic carries snapshots AND acks (v0.3 projection).
    this.unsubscribers.push(
      this.transport.onNotify(this.uuids.stateSnapshot, (bytes) => {
        const json = this.wasm.decodeEvent(bytes);
        if (!json) return;
        const { kind, event } = JSON.parse(json) as { kind: string; event: Record<string, unknown> };
        if (kind === "ack") {
          this.dispatchEvent(new CustomEvent("ack", { detail: event }));
        } else if (kind === "stateSnapshot") {
          // The snapshot carries battery too, but the device also notifies
          // the battery characteristic for every change — no synthetic
          // `battery` event here, or consumers would see each change twice.
          const snapshot = event as unknown as DeviceStateSnapshot;
          this.snapshot = snapshot;
          this.dispatchEvent(new CustomEvent("statesnapshot", { detail: snapshot }));
        } else if (kind === "battery") {
          this.dispatchEvent(
            new CustomEvent("battery", { detail: { levelPercent: event.level_percent } }),
          );
        }
      }),
    );

    // C3 raw sensor stream → decoded frames.
    this.unsubscribers.push(
      this.transport.onNotify(this.uuids.rawSensorStream, (bytes) => {
        try {
          const frame = JSON.parse(this.wasm.decodeSensorFrame(bytes)) as SensorFrame;
          this.dispatchEvent(new CustomEvent("sensorframe", { detail: frame }));
        } catch {
          /* malformed frame — drop, like hardware */
        }
      }),
    );
  }

  private parseBatteryJson(bytes: Uint8Array): number | undefined {
    try {
      const obj = JSON.parse(new TextDecoder().decode(bytes)) as { level?: number };
      return obj.level;
    } catch {
      return undefined;
    }
  }

  /** Sends a named haptic pattern. Returns the sequence for ack correlation. */
  async sendHaptic(pattern: HapticPattern): Promise<bigint> {
    const kind = pattern === "doubleTick" ? "double_tick" : pattern;
    return this.writeHaptic(JSON.stringify({ pattern_kind: kind }));
  }

  /**
   * Streams a custom waveform (int16 samples, ≤1024 — the device FIFO cap).
   * `samples` are little-endian int16; the SDK base64-encodes them for the
   * `waveform` payload the firmware decodes.
   */
  async sendWaveform(samples: Int16Array, sampleRateHz = 8000, intensity = 1.0): Promise<bigint> {
    if (samples.length > 1024) {
      throw new RangeError(`waveform too large: ${samples.length} > 1024-sample device FIFO`);
    }
    const bytes = new Uint8Array(samples.buffer, samples.byteOffset, samples.byteLength);
    let bin = "";
    for (let i = 0; i < bytes.length; i++) bin += String.fromCharCode(bytes[i]!);
    const data = btoa(bin);
    return this.writeHaptic(
      JSON.stringify({ pattern_kind: "waveform", data, sample_rate_hz: sampleRateHz, intensity }),
    );
  }

  private async writeHaptic(patternJson: string): Promise<bigint> {
    const seq = this.seq++;
    const bytes = this.wasm.encodeHapticCommand(seq, patternJson);
    await this.transport.write(this.uuids.hapticCommand, bytes);
    return seq;
  }

  /**
   * Updates config with clobber-free read-modify-write (readable-C2): reads
   * current bytes, applies the patch, writes back.
   */
  async setConfig(patch: Partial<{
    sensitivity: number;
    rawStreamOptIn: boolean;
    gestureMask: number;
    hidEnabled: boolean;
  }>): Promise<void> {
    let current = { sensitivity: 0x80, rawStreamOptIn: false, gestureMask: 0xff, hidEnabled: true };
    try {
      const read = await this.transport.read(this.uuids.config);
      if (read.length > 0) {
        current = JSON.parse(this.wasm.decodeConfig(read)) as typeof current;
      }
    } catch {
      /* pre-read firmware: fall back to defaults */
    }
    const next = { ...current, ...patch };
    const bytes = this.wasm.encodeConfig(
      next.sensitivity,
      next.rawStreamOptIn,
      next.gestureMask,
      next.hidEnabled,
    );
    await this.transport.write(this.uuids.config, bytes);
    if (patch.hidEnabled !== undefined) this.hidSuppressed = !patch.hidEnabled;
  }

  /** Opt in/out of the C3 raw sensor stream (config byte 1). */
  async enableSensorStream(enabled: boolean): Promise<void> {
    await this.setConfig({ rawStreamOptIn: enabled });
  }

  /**
   * Suppress the ring's standalone HID projection while the app owns it
   * (PROTOCOL.md "HID coexistence"). Restored automatically by {@link close}.
   */
  async takeOverHid(): Promise<void> {
    await this.setConfig({ hidEnabled: false });
  }

  /** Restore the ring's standalone HID projection (the release half of takeover). */
  async releaseHid(): Promise<void> {
    await this.setConfig({ hidEnabled: true });
  }

  /**
   * Releases the ring: restores HID if this session suppressed it (so the
   * ring keeps working as a standalone remote after the app goes away),
   * detaches listeners and disconnects the transport.
   */
  async close(): Promise<void> {
    if (this.hidSuppressed) {
      try {
        await this.releaseHid();
      } catch {
        /* best-effort: the link may already be gone */
      }
    }
    this.unsubscribers.forEach((u) => u());
    this.unsubscribers = [];
    await this.transport.disconnect();
  }

  // Typed addEventListener overloads.
  addEventListener<K extends keyof GesturaRingEventMap>(
    type: K,
    listener: (ev: GesturaRingEventMap[K]) => void,
    options?: boolean | AddEventListenerOptions,
  ): void;
  addEventListener(
    type: string,
    listener: EventListenerOrEventListenerObject,
    options?: boolean | AddEventListenerOptions,
  ): void;
  addEventListener(
    type: string,
    listener: EventListenerOrEventListenerObject | ((ev: Event) => void),
    options?: boolean | AddEventListenerOptions,
  ): void {
    super.addEventListener(type, listener as EventListenerOrEventListenerObject, options);
  }
}
