# Python SDK Plan — `gestura-ring-sdk`

Status: **PLAN** (no code yet) · Owner: Brandon · Drafted 2026-07-10

Python SDK for the Gestura Haptica Harmony B1 ring and its simulator,
structured as the sibling of `sdk/typescript` (`@gestura/ring-sdk`): a thin,
ergonomic layer over the canonical Rust protocol — never a re-implementation
of the wire contract.

## 1. Ground rules (inherited from the TS SDK + PROTOCOL.md)

- **The codec is Rust.** `crates/gestura-protocol` (introduced on
  `feature/ts-sdk-wasm`; factored out of `gestura-core-ring/src/protocol.rs`)
  is the single source of truth for envelopes, gestures, haptics, config, and
  the C3 sensor stream. TS consumes it via wasm-bindgen; Python consumes it
  via **PyO3/maturin native bindings** — same crate, second binding target.
- **Shared-contract governance applies** (PROTOCOL.md): protocol changes are
  proposed, cross-checked with the firmware lane, user-confirmed, and landed
  in crate + simulator + firmware together. The Python SDK never forks the
  contract.
- **Transport-agnostic core**, mirroring `RingTransport` in TS: the SDK core
  takes any object satisfying a small `RingTransport` protocol
  (`write(bytes)`, `read() -> bytes`, `subscribe(callback)`, `close()`).
- **License**: `LicenseRef-Gestura-Prosperity-1.1`, same as the protocol
  crate and TS SDK. (Note: public sites still say "MIT" — being corrected;
  see developer-platform audit 2026-07-10.)

## 2. Package layout

```
sdk/python/
  PLAN.md                     # this file
  pyproject.toml              # maturin build backend; abi3 wheels
  Cargo.toml                  # pyo3 bindings crate: gestura-protocol-py
  src/                        #   (rust) #[pymodule] over gestura-protocol
  python/gestura_ring/
    __init__.py               # GesturaRing, HapticPattern, SensorFrame, …
    transport.py              # RingTransport protocol + MockTransport
    transports/ble.py         # bleak-based GATT transport (ratified UUIDs)
    transports/simulator.py   # socket projection of haptic-harmony-simulator
    events.py                 # event dispatch (sync callbacks + async iterator)
  tests/                      # pytest; parity with sdk/typescript vitest suite
  examples/
    gesture_logger.py         # print gestures from ring or simulator
    haptic_patterns.py        # semantic haptics + custom waveform (≤1024 samples)
```

PyPI distribution name: **`gestura-ring-sdk`**, import name `gestura_ring`.
(The sites currently print `pip install gestura-sdk`; align them to the real
name at first publish — tracked in the developer-platform repo.)

## 3. API surface (parity table with `@gestura/ring-sdk` v0.3.0)

Pythonic naming, identical semantics:

| TS (`@gestura/ring-sdk`) | Python (`gestura_ring`) |
|---|---|
| `GesturaRing.open({transport})` | `await GesturaRing.open(transport)` / `async with GesturaRing(transport)` |
| `ring.addEventListener("doubletap", fn)` | `ring.on("doubletap", fn)` + `async for ev in ring.events("gesture", "battery")` |
| event names `tap, doubletap, holdstart, holdend, swipeleft, swiperight, rotatecw, rotateccw, gesture, sensorframe, battery, ack` | same lowercase names (contract-governed; from PROTOCOL.md) |
| `sendHaptic("tick")` | `await ring.send_haptic("tick")` — `confirm/error/tick/doubleTick` |
| `sendWaveform(int16[], rate)` | `await ring.send_waveform(samples, rate_hz)` (≤1024 samples, device FIFO) |
| `enableSensorStream(true)` | `await ring.enable_sensor_stream(True)` (C3, bonded-gated device-side) |
| `takeOverHid()` | `await ring.take_over_hid()` |
| config read-modify-write (readable-C2) | `await ring.update_config(lambda c: …)` — clobber-free RMW |
| `MockTransport` | `MockTransport` (drives the pytest suite) |

Faithfulness details carried over from the contract: `timestamp_ms` is
**device uptime**, never epoch (expose as `uptime_ms`; host wall-clock
correlation is the caller's job); commands are sequenced with `ack`
(`ok|denied|error`) surfaced both as awaitable command results and as `ack`
events.

## 4. Transports (phased)

| Transport | Phase | Notes |
|---|---|---|
| `MockTransport` | 0.1 | in-memory; ports the TS mock; CI runs entirely on it |
| `SimulatorTransport` | 0.2 | socket projection of `haptic-harmony-simulator` (public repo); strict-transport mode for realistic MTU/timing |
| `BleTransport` | 0.3 | [bleak](https://github.com/hbldh/bleak) GATT against the ratified UUIDs (exported by the protocol crate, same as TS `RingUuids`) |
| MCP | later | the simulator already projects MCP; a Python MCP client example belongs in `examples/`, not in the SDK core |

## 5. Build & CI

- **maturin** builds `gestura-protocol-py` (PyO3, `abi3-py310`) so one wheel
  per platform covers CPython 3.10+. Wheels: macOS arm64/x86_64, Windows
  x86_64, manylinux x86_64/aarch64.
- Python ≥ 3.10, `asyncio`-first. Runtime deps: none for core; `bleak` as
  the `[ble]` extra (`pip install gestura-ring-sdk[ble]`).
- CI jobs (extend existing gestura-app workflows): `maturin build` matrix,
  `pytest` on MockTransport, one integration job that boots
  `haptic-harmony-simulator` headless and runs the gesture/haptic round-trip.
- **Conformance vectors**: export golden envelope fixtures from
  `gestura-protocol` tests once, consume from both vitest and pytest so the
  two SDKs cannot drift.

## 6. Milestones

1. **0.1 — codec + mock** (~small): pyo3 bindings crate, `GesturaRing` core,
   events, haptics, `MockTransport`, pytest parity suite. No radio.
2. **0.2 — simulator**: `SimulatorTransport` against the public simulator,
   integration CI, `gesture_logger.py` example. First TestPyPI publish.
3. **0.3 — BLE + parity**: `bleak` transport, sensor stream, waveforms,
   config RMW, HID takeover. Publish `gestura-ring-sdk` to PyPI proper and
   flip gestura.dev copy from "planned" to real install instructions.

## 7. Open questions for Brandon

1. **PyPI name**: `gestura-ring-sdk` (proposed) vs the `gestura-sdk` the
   sites have been printing. Squatting both and aliasing is cheap insurance.
2. **Where the bindings crate lives**: `sdk/python` alongside a
   `crates/gestura-protocol-py`, or bindings inline under `sdk/python/src`
   (proposed above, keeps the workspace's crates/ purely internal).
3. **Sync facade**: ship a blocking wrapper (`ring.sync.GesturaRing`) for
   notebook/scripting users in 0.2, or stay async-only until asked?
4. Confirm Prosperity-1.1 for the Python artifacts (wheels embed the license
   text; PyPI classifier will read `License :: Other/Proprietary`).
