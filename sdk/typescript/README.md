# @gestura/ring-sdk

TypeScript SDK for the Gestura **Haptica Harmony B1** ring and its simulator.

The wire codec is **not** re-implemented in TypeScript. It's the Rust
`gestura-protocol` crate — the canonical Shared Semantic Protocol
contract — compiled to **WebAssembly**. This SDK is a thin, ergonomic layer
over that core: a pluggable transport plus a W3C UI-Events-style API.

```
gestura-protocol (Rust)  ──wasm-pack──▶  wasm/  ──┐
                                                  ├─▶  @gestura/ring-sdk (this)
RingTransport (Tauri / Mock / Web BT) ────────────┘
```

## Install / build

```sh
# from sdk/typescript
npm install
npm run build          # builds the WASM core (wasm-pack) then compiles TS
npm test               # vitest — wrapper wiring, against MockTransport
```

`build:wasm` runs `wasm-pack build --features wasm --target bundler` on the
`gestura-protocol` crate and drops the package in `./wasm`. Requires
[`wasm-pack`](https://rustwasm.github.io/wasm-pack/) and the
`wasm32-unknown-unknown` target (`rustup target add wasm32-unknown-unknown`).

## Usage

```ts
import { GesturaRing } from "@gestura/ring-sdk";
import { tauriTransport } from "@gestura/ring-sdk/tauri";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

const transport = tauriTransport(deviceId, { invoke, listen });
const ring = await GesturaRing.open({ transport });

ring.addEventListener("doubletap", () => runCommand());
ring.addEventListener("rotatecw", () => volumeUp());
ring.addEventListener("gesture", (e) =>
  console.log(e.detail.type, "→", e.detail.action)); // e.g. "double_tap → execute"

// C3 raw sensor stream (opt-in, bonded-gated device-side)
await ring.enableSensorStream(true);
ring.addEventListener("sensorframe", (e) => {
  for (const s of e.detail.samples) applyImu(s.ax_mg, s.ay_mg, s.az_mg, s.gx_ddps);
});

// Haptics
await ring.sendHaptic("tick");
await ring.sendWaveform(myInt16Samples, 8000); // ≤1024 samples (device FIFO)

// Config uses clobber-free read-modify-write (readable-C2)
await ring.takeOverHid(); // suppress the ring's standalone HID projection
```

## Transports

| Transport | Status | Notes |
|---|---|---|
| `MockTransport` | ✅ | in-memory; tests + offline example runs |
| `tauriTransport` | 🔌 needs backend glue | bridges to gestura-gui's Rust BLE over IPC. Requires the thin `ring_write`/`ring_read`/`ring_subscribe`/`ring-notify` passthrough commands (see `src/transport/tauri.ts` header) — one Rust step. |
| Web Bluetooth | ⏭ next | `navigator.bluetooth` against the ratified UUIDs |

## Event names (W3C-style)

`tap`, `doubletap`, `holdstart`/`holdend`, `swipeleft`/`swiperight`,
`rotatecw`/`rotateccw`, plus `gesture` (with mapped action), `sensorframe`,
`battery`, `ack`. See PROTOCOL.md in `gestura-core-ring/` for the full mapping
and the Matter Generic Switch alignment.
