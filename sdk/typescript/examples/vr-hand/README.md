# VR Hand — Gestura Ring SDK example

A 3D hand and objects driven by ring gestures and the **C3 raw IMU stream**,
using `@gestura/ring-sdk`. It's the canonical "how do I build an app for the
ring?" reference: connect, subscribe to typed events, send haptics.

```
IMU (gyro)   → hand tilts/rotates in real time
tap          → haptic pulse + "poke" bounce
double-tap   → spin the object
swipe ← / →  → cycle objects
rotate cw/ccw→ scale the object
```

## Run it

**Offline (no hardware, works anywhere):**
```sh
npm install
npm run dev            # http://localhost:5173
```
It falls back to a `MockTransport` with a synthetic motion feed. The frames it
generates are built to the real `sensor_frame.h` byte layout, so the WASM
decoder parses them exactly as it would device frames. Keys <kbd>T</kbd>
<kbd>D</kbd> <kbd>←</kbd> <kbd>→</kbd> trigger gestures.

**Live against the simulator (inside the Gestura app):**
1. Build the SDK's WASM core: `cd ../.. && npm run build`.
2. Start the [`haptic-harmony-simulator`](../../../../../haptic-harmony-simulator)
   advertising over BLE (`--features native-ble`).
3. Run this example inside the Tauri app's webview; it auto-selects the
   `tauriTransport` and calls `ring.enableSensorStream(true)`.

> The live path needs the thin `ring_write` / `ring_read` / `ring_subscribe` /
> `ring-notify` passthrough commands in gestura-gui (the one Rust glue step —
> see `sdk/typescript/src/transport/tauri.ts`). Until those land, the offline
> path is the runnable demo.

## What to copy

- `src/main.ts` — the entire integration: `GesturaRing.open`, `addEventListener`
  for each gesture, `sendHaptic`, `enableSensorStream`, and decoding
  `sensorframe` samples. Everything an app needs is in this one file.
