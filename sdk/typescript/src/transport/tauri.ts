/**
 * Tauri transport: bridges the SDK to the Gestura app's Rust BLE backend
 * (gestura-gui) over Tauri IPC. The app owns real BLE (btleplug /
 * CoreBluetooth); the SDK owns the codec (WASM). Bytes cross the boundary raw.
 *
 * Backend contract (provided by gestura-gui, `src/commands/ring_raw.rs`;
 * documented in `docs/IPC_CONTRACTS_GESTURA_GUI.md`):
 *   invoke("ring_write",       { deviceId, uuid, bytes: number[] }) -> void
 *   invoke("ring_read",        { deviceId, uuid }) -> number[]
 *   invoke("ring_subscribe",   { deviceId, uuid }) -> void
 *   invoke("ring_unsubscribe", { deviceId, uuid }) -> void
 *   invoke("ring_active_device") -> string   (see `activeDeviceId`)
 *   event  "ring-notify" payload { deviceId, uuid, bytes: number[] }
 * These are byte passthroughs — no decoding backend-side (the SDK/WASM does
 * that). Only external BLE devices (a real ring, or the simulator advertising
 * over BLE) support them; the app's internal simulator runtime refuses.
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
 * The external ring/simulator the app currently has connected, or `undefined`
 * when there is none (the backend errors in that case; callers usually fall
 * back to an offline transport).
 */
export async function activeDeviceId(api: Pick<TauriApi, "invoke">): Promise<string | undefined> {
  try {
    return await api.invoke<string>("ring_active_device");
  } catch {
    return undefined;
  }
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
      // Tear down backend subscriptions too — clearing local listeners alone
      // would leave the backend forwarding notifications for every UUID that
      // was never individually unsubscribed.
      const subscribed = Array.from(listeners.keys());
      listeners.clear();
      await Promise.all(
        subscribed.map((uuid) =>
          api.invoke("ring_unsubscribe", { deviceId, uuid }).catch(() => {
            /* best-effort: the device session is ending anyway */
          }),
        ),
      );
    },
  };
}
