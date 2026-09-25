#!/usr/bin/env node
/**
 * `pretest` / `pretypecheck` hook: builds the WASM core only when it is
 * missing, so a fresh clone works with `npm test` and repeat runs stay fast.
 */
import { existsSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { spawnSync } from "node:child_process";

const here = dirname(fileURLToPath(import.meta.url));
const inline = join(here, "..", "wasm", "gestura_protocol_inline.js");

if (existsSync(inline)) process.exit(0);

console.log("ensure-wasm: WASM core not built yet — running `npm run build:wasm`");
const r = spawnSync("npm", ["run", "build:wasm"], { stdio: "inherit", cwd: join(here, ".."), shell: process.platform === "win32" });
process.exit(r.status ?? 1);
