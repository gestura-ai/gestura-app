/**
 * VR Hand — a reference app for the Gestura ring SDK.
 *
 * A 3D hand/object reacts to ring gestures and the C3 raw IMU stream:
 *   - IMU gyro tilts/rotates the hand in real time (sensorframe events)
 *   - tap        → haptic pulse + a "poke" bounce
 *   - double-tap → spin the current object
 *   - swipe      → cycle to the next object
 *   - rotate     → scale up/down
 *
 * Transport: prefers the Tauri bridge (real simulator/ring); falls back to a
 * MockTransport driven by a synthetic motion feed so the demo runs anywhere.
 * The SDK — and therefore the exact Rust codec, via WASM — is identical in
 * both cases; only the byte source changes.
 */

import * as THREE from "three";
import { GesturaRing, MockTransport, type SensorFrame } from "@gestura/ring-sdk";

// ---- SDK wiring -------------------------------------------------------------

const hud = {
  source: document.getElementById("source")!,
  gesture: document.getElementById("gesture")!,
};

async function connectRing(): Promise<GesturaRing> {
  // Try the Tauri transport if we're inside the Gestura app.
  const tauri = (globalThis as { __TAURI__?: unknown }).__TAURI__;
  if (tauri) {
    try {
      const { tauriTransport } = await import("@gestura/ring-sdk/tauri");
      const { invoke } = await import("@tauri-apps/api/core");
      const { listen } = await import("@tauri-apps/api/event");
      // The app resolves the connected simulator's device id (command TBD in
      // the Tauri glue step; see the SDK's tauri.ts header).
      const deviceId = await invoke<string>("ring_active_device");
      const transport = tauriTransport(deviceId, { invoke, listen });
      hud.source.textContent = `Live · simulator via Tauri (${deviceId})`;
      const ring = await GesturaRing.open({ transport });
      await ring.enableSensorStream(true);
      return ring;
    } catch (e) {
      console.warn("Tauri transport unavailable, using mock:", e);
    }
  }

  // Offline: MockTransport + a synthetic IMU/gesture feed.
  const mock = new MockTransport();
  const uuids = await mockUuids();
  const ring = await GesturaRing.open({ transport: mock });
  hud.source.textContent = "Offline demo · synthetic motion (no ring)";
  startSyntheticFeed(mock, uuids.rawSensorStream, uuids.gestureEvent);
  return ring;
}

// In offline mode we don't have the WASM core loaded to report UUIDs, so use
// the ratified constants directly for the mock feed.
async function mockUuids() {
  return {
    gestureEvent: "e3b742d4-51c9-4f0e-9d26-7a48c1f0b9be",
    rawSensorStream: "e3b742d4-51c9-4f0e-9d26-7a48c1f0b9c3",
  };
}

/**
 * Drives the MockTransport with plausible C3 frames + keyboard-triggered
 * gestures so the scene is alive without hardware. Frame bytes are built to
 * the ratified sensor_frame.h layout so the WASM decoder parses them for real.
 */
function startSyntheticFeed(mock: MockTransport, sensorUuid: string, gestureUuid: string): void {
  let t = 0;
  setInterval(() => {
    t += 0.05;
    const gx = Math.round(Math.sin(t) * 20000); // sweeping gyro, mdps
    const gy = Math.round(Math.cos(t * 0.7) * 15000);
    mock.emit(sensorUuid, buildSensorFrame(gx, gy, 0));
  }, 100);

  window.addEventListener("keydown", (e) => {
    const g =
      e.key === "t" ? { gesture_kind: "tap" } :
      e.key === "d" ? { gesture_kind: "double_tap" } :
      e.key === "ArrowLeft" ? { gesture_kind: "swipe", direction: "left" } :
      e.key === "ArrowRight" ? { gesture_kind: "swipe", direction: "right" } :
      undefined;
    if (g) mock.emit(gestureUuid, new TextEncoder().encode(JSON.stringify({ gesture: g, confidence: 0.9 })));
  });
}

/** Builds one ratified C3 frame (1 sample) — LE, gyro in deci-dps. */
function buildSensorFrame(gxMdps: number, gyMdps: number, gzMdps: number): Uint8Array {
  const buf = new Uint8Array(8 + 16);
  const dv = new DataView(buf.buffer);
  dv.setUint8(0, 0x01); // version
  dv.setUint8(1, 0x00); // flags: touch not valid
  dv.setUint32(2, 0, true); // t0_ms
  dv.setUint8(6, 1); // count
  dv.setUint8(7, 10); // period
  const s = 8;
  dv.setInt16(s + 0, 0, true); // ax
  dv.setInt16(s + 2, 0, true); // ay
  dv.setInt16(s + 4, 1000, true); // az (gravity)
  dv.setInt16(s + 6, Math.round(gxMdps / 100), true); // gx deci-dps
  dv.setInt16(s + 8, Math.round(gyMdps / 100), true);
  dv.setInt16(s + 10, Math.round(gzMdps / 100), true);
  dv.setUint16(s + 12, 0, true); // slider
  dv.setUint8(s + 14, 0); // touch flags
  dv.setUint8(s + 15, 0); // pad
  return buf;
}

// ---- 3D scene ---------------------------------------------------------------

const scene = new THREE.Scene();
scene.background = new THREE.Color(0x0b0d12);
const camera = new THREE.PerspectiveCamera(50, innerWidth / innerHeight, 0.1, 100);
camera.position.set(0, 0.4, 4);

const renderer = new THREE.WebGLRenderer({ antialias: true });
renderer.setSize(innerWidth, innerHeight);
renderer.setPixelRatio(devicePixelRatio);
document.body.appendChild(renderer.domElement);

scene.add(new THREE.AmbientLight(0x8899aa, 0.6));
const key = new THREE.DirectionalLight(0xffffff, 1.1);
key.position.set(3, 5, 4);
scene.add(key);

// A stylized "hand" (palm + five finger boxes) grouped so IMU rotates it.
const hand = new THREE.Group();
const mat = new THREE.MeshStandardMaterial({ color: 0x66ccddff & 0xffffff, roughness: 0.5, metalness: 0.1 });
const palm = new THREE.Mesh(new THREE.BoxGeometry(1.1, 1.3, 0.35), mat);
hand.add(palm);
for (let i = 0; i < 4; i++) {
  const finger = new THREE.Mesh(new THREE.BoxGeometry(0.2, 0.8, 0.25), mat);
  finger.position.set(-0.42 + i * 0.28, 1.0, 0);
  hand.add(finger);
}
const thumb = new THREE.Mesh(new THREE.BoxGeometry(0.22, 0.55, 0.25), mat);
thumb.position.set(-0.62, 0.2, 0);
thumb.rotation.z = 0.7;
hand.add(thumb);
scene.add(hand);

// A row of "objects" the user cycles through with swipe.
const objects: THREE.Mesh[] = [
  new THREE.Mesh(new THREE.IcosahedronGeometry(0.5), new THREE.MeshStandardMaterial({ color: 0xffaa44 })),
  new THREE.Mesh(new THREE.TorusKnotGeometry(0.35, 0.13, 80, 12), new THREE.MeshStandardMaterial({ color: 0xff5599 })),
  new THREE.Mesh(new THREE.BoxGeometry(0.7, 0.7, 0.7), new THREE.MeshStandardMaterial({ color: 0x55ddaa })),
];
objects.forEach((o) => { o.position.set(2.2, 0, 0); o.visible = false; scene.add(o); });
let active = 0;
objects[0]!.visible = true;

// ---- gesture reactions ------------------------------------------------------

let targetRot = new THREE.Euler();
let spinVel = 0;
let poke = 0;

function applyFrame(frame: SensorFrame): void {
  // Integrate gyro (deci-dps) into hand orientation; light-touch mapping.
  for (const s of frame.samples) {
    targetRot.x += (s.gx_ddps / 100) * 0.00005;
    targetRot.y += (s.gy_ddps / 100) * 0.00005;
  }
}

async function main() {
  const ring = await connectRing();

  ring.addEventListener("sensorframe", (e) => applyFrame(e.detail));

  ring.addEventListener("tap", () => {
    hud.gesture.textContent = "tap → poke";
    poke = 0.3;
    void ring.sendHaptic("tick");
  });
  ring.addEventListener("doubletap", () => {
    hud.gesture.textContent = "double-tap → spin";
    spinVel = 0.35;
    void ring.sendHaptic("doubleTick");
  });
  ring.addEventListener("swipeleft", () => cycle(-1, ring));
  ring.addEventListener("swiperight", () => cycle(1, ring));
  ring.addEventListener("rotatecw", () => scaleActive(1.15));
  ring.addEventListener("rotateccw", () => scaleActive(1 / 1.15));
  ring.addEventListener("ack", (e) => console.log("haptic ack", e.detail));
}

function cycle(dir: number, ring: GesturaRing): void {
  objects[active]!.visible = false;
  active = (active + dir + objects.length) % objects.length;
  objects[active]!.visible = true;
  hud.gesture.textContent = `swipe → object ${active + 1}`;
  void ring.sendHaptic("tick");
}

function scaleActive(f: number): void {
  objects[active]!.scale.multiplyScalar(f);
  hud.gesture.textContent = f > 1 ? "rotate cw → bigger" : "rotate ccw → smaller";
}

// ---- render loop ------------------------------------------------------------

function animate(): void {
  requestAnimationFrame(animate);
  hand.rotation.x += (targetRot.x - hand.rotation.x) * 0.15;
  hand.rotation.y += (targetRot.y - hand.rotation.y) * 0.15 + spinVel;
  spinVel *= 0.92;
  const s = 1 + poke;
  hand.scale.setScalar(1 + poke * 0.2);
  poke *= 0.85;
  const obj = objects[active]!;
  obj.rotation.y += 0.01;
  void s;
  renderer.render(scene, camera);
}

addEventListener("resize", () => {
  camera.aspect = innerWidth / innerHeight;
  camera.updateProjectionMatrix();
  renderer.setSize(innerWidth, innerHeight);
});

void main();
animate();
