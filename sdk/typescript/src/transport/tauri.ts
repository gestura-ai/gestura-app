/**
 * Tauri transport: bridges the SDK to the Gestura app's Rust BLE backend
 * (gestura-gui) over Tauri IPC. The app owns real BLE (btleplug /
 * CoreBluetooth); the SDK owns the codec (WASM). Bytes cross the boundary raw.
 *
 * REQUIRED BACKEND COMMANDS (thin passthroughs to add in gestura-gui — the
 * existing RingManager exposes high-level ops, not raw char I/O):
 *   invoke("ring_write",  { deviceId, uuid, bytes: number[] }) -> void
 *   invoke("ring_read",   { deviceId, uuid }) -> number[]
 *   invoke("ring_subscribe",   { deviceId, uuid }) -> void
 *   invoke("ring_unsubscribe", { deviceId, uuid }) -> void
 *   event  "ring-notify" payload { deviceId, uuid, bytes: number[] }
 * These are byte passthroughs — no decoding backend-side (the SDK/WASM does
 * that). Tracked as the one Rust glue step in the 2026-07-10 handoff.
 */

import type { RingTransport } from "../transport.js";

// Minimal structural types so this file doesn't hard-depend on @tauri-apps/api
// at type-check time (it's an optional peer dep).
interface TauriApi {
  invoke<T>(cmd: string, args?: Record<string, unknown>): Promise<T>;
  listen<T>(
    event: string,
    handler: (e: { payload: T }) => void,
  ): Promise<() => void>;
}

interface NotifyPayload {
  deviceId: string;
  uuid: string;
  bytes: number[];
}

/**
 * Builds a Tauri-backed transport for a connected device. Pass the resolved
 * `@tauri-apps/api/core` `invoke` and `@tauri-apps/api/event` `listen`.
 */
export function tauriTransport(deviceId: string, api: TauriApi): RingTransport {
  const listeners = new Map<string, Set<(b: Uint8Array) => void>>();
  let unlistenPromise: Promise<() => void> | undefined;

  const ensureGlobalListener = (): void => {
    if (unlistenPromise) return;
    unlistenPromise = api.listen<NotifyPayload>("ring-notify", ({ payload }) => {
      if (payload.deviceId !== deviceId) return;
      const set = listeners.get(payload.uuid);
      if (set) {
        const bytes = Uint8Array.from(payload.bytes);
        set.forEach((cb) => cb(bytes));
      }
    });
  };

  return {
    onNotify(uuid, cb) {
      ensureGlobalListener();
      let set = listeners.get(uuid);
      if (!set) {
        set = new Set();
        listeners.set(uuid, set);
        void api.invoke("ring_subscribe", { deviceId, uuid });
      }
      set.add(cb);
      return () => {
        set!.delete(cb);
        if (set!.size === 0) {
          listeners.delete(uuid);
          void api.invoke("ring_unsubscribe", { deviceId, uuid });
        }
      };
    },
    async write(uuid, bytes) {
      await api.invoke("ring_write", { deviceId, uuid, bytes: Array.from(bytes) });
    },
    async read(uuid) {
      const bytes = await api.invoke<number[]>("ring_read", { deviceId, uuid });
      return Uint8Array.from(bytes);
    },
    async disconnect() {
      const unlisten = await unlistenPromise;
      unlisten?.();
      listeners.clear();
    },
  };
}
