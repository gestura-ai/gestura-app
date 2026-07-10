# Haptica Harmony B1 — Shared Semantic Protocol

**Canonical definition: `src/protocol.rs` in this crate** (user decision
2026-07-02: the contract's home is the gestura-app SDK). The simulator
(`haptic-harmony-simulator/src/protocol.rs`) and the ring firmware mirror it.
Changes are shared-contract governed: propose → cross-check with the firmware
lane → user confirms → land everywhere.

Current version: **0.3.0** (see the version changelog in `src/protocol.rs`).

## Transport

JSON over BLE GATT (socket and MCP projections exist in the simulator).
Everything rides `ProtocolEnvelope{protocol_version, message_kind, message_id,
sequence, timestamp_ms, payload}`. Commands are sequenced (≥1; 0 =
unsequenced/legacy) and acknowledged via the `ack` event
(`{sequence, status: ok|denied|error, reason}`), which rides the state-snapshot
characteristic as a full envelope.

**`timestamp_ms` is DEVICE UPTIME milliseconds, not epoch time** — the ring
has no wall clock (boots near 0, monotonic). Hosts must not interpret it as
Unix time; correlate to wall-clock host-side at receipt if needed. The
simulator's device-core path uses the same uptime semantics.

**Transport realities** (mirrored by the simulator's strict-transport mode):
notifications larger than the negotiated ATT payload (MTU − 3) fail silently
on hardware — state snapshots (~400 B) need MTU ≥ ~460, so hosts must
negotiate MTU (firmware supports up to 498). Writes beyond the ATT payload
(e.g. waveform commands) travel as GATT long writes (prepared writes
assembled by offset). The device is single-connection (`BT_MAX_CONN=1`),
re-advertises on disconnect, and **resets trust to untrusted on every
disconnect** (bonds persist across reboots; trust state does not survive a
link drop).

## GATT allocation (FINAL, firmware-minted base, user-ratified 2026-07-02)

Base `E3B742D4-51C9-4F0E-9D26-7A48C1F0B9xx`, last byte = ordinal:

| xx | Role | Props | Notes |
|----|------|-------|-------|
| BC | Ring service | — | |
| BD | Haptic command | write | trust-gated (Enrolled+) |
| BE | Gesture event | notify | `BleGestureData` wrapper w/ embedded envelope |
| BF | Battery level | read+notify | raw byte or `BleBatteryData` JSON |
| C0 | OTA update | write+indicate | MCUmgr/SMP (firmware) |
| C1 | State snapshot | read+notify | `DeviceStateSnapshot`; also carries `ack` envelopes |
| C2 | Config | write | trust-gated + encrypted link; layout below |
| C3 | Raw sensor stream | notify | opt-in via config; subscription trust-gated (Bonded+); frame schema ratified 2026-07-09 (below) |

The v0.2-era service UUID `12345678-…9abc` remains only as a discovery
fallback (`LEGACY_SERVICE_UUID`); remove after all sides ship.

## Vocabulary (ratified)

**Gestures** (`gesture_kind`): `tap`, `double_tap`, `hold{duration_ms}`,
`swipe{left|right}`, `rotate{cw|ccw}` are device-truth kinds;
`slide{up|down|left|right}` and `tilt{angle_degrees}` are simulator-only.
`triple_tap` was dropped (no device source); `shake` is deferred to a later
firmware rev. Confidence: firmware sends 0–255, SDK normalizes to f32 0–1.

**Haptics** (`pattern_kind`): `confirm`, `error`, `tick`, `double_tick`,
`waveform{data: base64, sample_rate_hz, intensity}`, `custom{intensity,
duration_ms}`. Read-aliases for v0.1.0 peers: `success`→confirm,
`notify`→tick. Waveform limits (from the BOS1921 datasheet pass, 2026-07-07):
samples are **12-bit two's-complement sent as int16** (not 8-bit PCM),
device FIFO is 1024 samples, so firmware rejects >1024 samples (2 KiB)
until streaming refill lands; the protocol-level cap stays 4 KiB.

## Config characteristic layout (C2)

| byte | field | default |
|------|-------|---------|
| 0 | gesture sensitivity 0–255 | 0x80 |
| 1 | raw sensor stream opt-in | 0 |
| 2 | enabled-gesture bitmask (below) | 0xFF |
| 3 | HID projection enable | 1 (ON) |

Gesture bitmask bit assignment (**RATIFIED 2026-07-09**, "ratify as is"):
bit0 tap · bit1 double_tap · bit2 swipe_left · bit3 swipe_right ·
bit4 rotate_cw · bit5 rotate_ccw · bit6 hold · bit7 reserved.

Byte 3 is optional (shorter writes are accepted; approved 2026-07-07).
`protocol::RingConfig` is the typed writer.

**Readable C2 (RATIFIED 2026-07-08):** C2 supports READ; hosts
read-modify-write instead of clobbering config with defaults. The SDK does RMW
with graceful fallback against pre-read firmware (`RingConfig::from_bytes` +
`hid_set`); firmware implements the read callback (commit `cf945c6`).

Trust gating on C2 (read and write): **the wire-contract device guarantee is
Bonded** — the strictest link-layer-verifiable state; firmware gates both at
bonded (read gate hardened in `cf945c6`). On-device "Enrolled+" enforcement
awaits a future trust-attestation proposal. The simulator gates at Enrolled as
deliberately reference-stricter behavior — hosts must not assume more than the
bonded guarantee from real devices.

Denied writes are rejected *before* persisting — a denied write that sticks
becomes readable state. Verified on both the simulator (gate-before-store fix,
regression-tested) and firmware (cross-check answer #3, verified correct).

## HID coexistence (approved 2026-07-07)

Firmware ships a standard BLE HID (HOGP) consumer-control service **ON by
default** — the ring works as a standalone smart remote with no SDK
(tap→Play/Pause, double_tap→Next, swipe L/R→Prev/Next, rotate→Vol±,
hold→Voice). When Gestura.app takes over a connection, the SDK writes config
**byte 3 = 0** to suppress HID so the OS doesn't double-act on gestures, and
restores **byte 3 = 1** on release. Implemented in `SimulatorBackend`
(`connect`/`disconnect`) and gestura-gui's `ExternalBleRingManager`
(`ensure_connected`/`reset_simulator`). The write is trust-gated: on an
unenrolled link the suppression is refused and HID stays on — by design.

## Industry-standard alignment (approved 2026-07-07)

### Matter Generic Switch cluster — 1:1 semantic mapping

The protocol's press vocabulary maps losslessly onto Matter's Generic Switch
(`MSM`/`MSL` features) event model. A future Matter bridge in Gestura.app is
a **rename, not a redesign**:

| Protocol gesture | Matter Generic Switch event(s) |
|---|---|
| `tap` | `InitialPress` → `ShortRelease` (single press; `MultiPressComplete{count:1}` on MSM switches) |
| `double_tap` | `MultiPressOngoing` → `MultiPressComplete{count:2}` (double press) |
| `hold{duration_ms}` | `InitialPress` → `LongPress` (at threshold) → `LongRelease` (at release; `duration_ms` spans press→release) |
| `swipe`/`rotate` | no Generic Switch equivalent — bridge as separate endpoints/clusters (e.g. LevelControl for rotate) or omit |

Rules for the bridge: one `tap` = one complete press sequence (the ring does
its own debouncing/multi-press detection on-device, so the bridge emits
Matter's *composed* events, never synthesized raw press/release pairs except
for `hold`, where `LongPress`/`LongRelease` map to the hold's start/end).

### W3C-style naming — TypeScript SDK event surface

The TS SDK exposes gestures using W3C UI-Events conventions (lowercase,
no separators, noun-verb composition — the `pointerdown`/`pointerup`
family), while wire names stay snake_case:

| Wire (`gesture_kind`) | TS SDK event type |
|---|---|
| `tap` | `"tap"` |
| `double_tap` | `"doubletap"` |
| `hold` | `"holdstart"` / `"holdend"` (detail: `durationMs`) |
| `swipe{left/right}` | `"swipeleft"` / `"swiperight"` |
| `rotate{cw/ccw}` | `"rotatecw"` / `"rotateccw"` |

Listener registration follows the `EventTarget` idiom
(`ring.addEventListener("doubletap", …)`), event payloads use camelCase
(`confidence`, `durationMs`, `timestampMs`).

## Raw sensor stream (C3) — decisions RATIFIED 2026-07-09

Per the firmware proposal (`haptic-basic-firmware/proposals/
2026-07-07-sensor-frame-schema.md`), user-ratified with platform cross-check:

1. **Binary frames on C3** (no JSON envelope — 100 Hz × 6-axis JSON would be
   ~20 KB/s of text), leading `frame_version` byte (0x01), batched samples.
2. **i16 units** — accel in mg (sat ±32 g), gyro in deci-dps (sat ±3276 dps);
   **20-sample batching** (~2.3 kB/s at 100 Hz; firmware reduces N at small
   MTUs). SDK converts deci-dps→mdps (×100) for the tuning-CSV format.
3. **Continuous while ACTIVE, suspended in IDLE** (not gesture-gated) —
   tuning captures need continuity through gesture pauses.

**⚠️ Exact byte map pending from firmware:** the proposal's arithmetic
doesn't reconcile (states 22 B/sample + 6 B header, but the listed fields —
6×i16 + u16 slider + u8 flags + u8 pad — sum to 16 B, and the header layout
shown is 8 B). Firmware publishes the definitive byte map plus a golden
vector in `conformance/`; the SDK's `SensorFrame` decoder is written against
those, not against the proposal prose. C3 carries no bits until then.

## Trust model

Deny-by-default everywhere (user decision 2026-07-02). States:
`discovered < bonded < enrolled < attested`, plus `revoked` (fails closed).
Haptic/protocol commands require Enrolled+; config read/write carries a
**Bonded device guarantee** (see the config section — on-device Enrolled+
awaits a trust-attestation proposal; the simulator gates config at Enrolled as
reference-stricter behavior); raw-stream subscription and sensitive
diagnostics require Bonded+. **Device-reported trust is a subset of the
ladder: the ring only ever reports `discovered`, `bonded`, or `revoked`**
(bonded is the strongest state it can verify) — hosts must never require
`enrolled`/`attested` from a device snapshot; those are host/policy-side
states. Degraded modes (low battery, sensor fault,
firmware mismatch, operator block) gate privileged actions independently.
Policy denials surface as `ack{status: denied}`.
