/**
 * Transport abstraction. The SDK's protocol logic (via WASM) is transport-
 * agnostic; concrete transports move bytes to/from the ring's GATT
 * characteristics. Ships with a Tauri adapter (see `./transport/tauri`) and a
 * MockTransport for tests and offline examples. Web Bluetooth is the natural
 * next adapter.
 */

/** Moves raw bytes to/from ring GATT characteristics, keyed by UUID. */
export interface RingTransport {
  /** Subscribe to notifications on a characteristic. Returns an unsubscribe fn. */
  onNotify(uuid: string, cb: (bytes: Uint8Array) => void): () => void;
  /** Write bytes to a characteristic (long-write assembly is the transport's job). */
  write(uuid: string, bytes: Uint8Array): Promise<void>;
  /** Read a characteristic's current value. */
  read(uuid: string): Promise<Uint8Array>;
  /** Release the connection (restores device HID projection etc. host-side). */
  disconnect(): Promise<void>;
}

/**
 * In-memory transport for tests and offline example runs. Feed it canned
 * notification bytes via `emit(uuid, bytes)`; it records writes for assertions
 * and answers reads from a seeded store.
 */
export class MockTransport implements RingTransport {
  private listeners = new Map<string, Set<(b: Uint8Array) => void>>();
  private store = new Map<string, Uint8Array>();
  readonly writes: { uuid: string; bytes: Uint8Array }[] = [];

  /** Seed a readable characteristic's value (e.g. config default). */
  seed(uuid: string, bytes: Uint8Array): void {
    this.store.set(uuid, bytes);
  }

  /** Simulate a notification arriving on a characteristic. */
  emit(uuid: string, bytes: Uint8Array): void {
    this.listeners.get(uuid)?.forEach((cb) => cb(bytes));
  }

  onNotify(uuid: string, cb: (bytes: Uint8Array) => void): () => void {
    let set = this.listeners.get(uuid);
    if (!set) {
      set = new Set();
      this.listeners.set(uuid, set);
    }
    set.add(cb);
    return () => set!.delete(cb);
  }

  async write(uuid: string, bytes: Uint8Array): Promise<void> {
    this.writes.push({ uuid, bytes });
    // A config write is stored so a subsequent read returns it (readable-C2).
    this.store.set(uuid, bytes);
  }

  async read(uuid: string): Promise<Uint8Array> {
    return this.store.get(uuid) ?? new Uint8Array();
  }

  async disconnect(): Promise<void> {
    this.listeners.clear();
  }
}
