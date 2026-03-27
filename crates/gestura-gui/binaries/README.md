# Bundled external binaries (Gestura CLI + ffmpeg)

This directory is the **build-time staging location** for binaries that are
bundled into the Gestura installer via Tauri's `externalBin` mechanism.

Currently two sidecars are bundled:

| Sidecar | Purpose |
|---------|---------|
| `gestura` | Gestura CLI (full agent access from a terminal) |
| `ffmpeg`  | Screen-recorder backend (screen_record tool works out of the box) |

Both are declared in `crates/gestura-gui/tauri.conf.json`:

```json
"externalBin": [
  "binaries/gestura",
  "binaries/ffmpeg"
]
```

On Linux, the DEB/RPM packages additionally install the CLI onto PATH by
mapping the staged file into `usr/bin/gestura`.

## Naming convention (required by Tauri)

Tauri appends the target triple to the `externalBin` base name, so the staged
file must be named `<base>-<triple>[.exe]`.

### Gestura CLI

| Platform | Expected filename |
|----------|------------------|
| macOS universal | `gestura-universal-apple-darwin` |
| Linux x86\_64   | `gestura-x86_64-unknown-linux-gnu` |
| Windows x86\_64 | `gestura-x86_64-pc-windows-msvc.exe` |

### ffmpeg sidecar

| Platform | Expected filename |
|----------|------------------|
| macOS universal | `ffmpeg-universal-apple-darwin` |
| Linux x86\_64   | `ffmpeg-x86_64-unknown-linux-gnu` |
| Linux arm64     | `ffmpeg-aarch64-unknown-linux-gnu` |
| Windows x86\_64 | `ffmpeg-x86_64-pc-windows-msvc.exe` |

At runtime `gestura-core-tools` probes for these names next to the running
executable (Tauri places sidecars in the same directory as the main binary).
If none is found it falls back to the system `ffmpeg` on `PATH`.
You can also override via the `GESTURA_FFMPEG_PATH` environment variable.

For shell-launch resume flows, the GUI now also prefers a matching Gestura CLI
next to the running app (or in the local `target/{debug,release}` tree during
development) before falling back to a globally installed `gestura` on `PATH`.
Set `GESTURA_CLI_PATH` to force a specific CLI binary when needed.

## How this directory is populated

The packaging scripts handle both sidecars automatically:

```bash
# macOS  — downloads static universal build from evermeet.cx
scripts/package-mac.sh

# Linux  — downloads static build from johnvansickle.com
scripts/package-linux.sh

# Windows — downloads static build from BtbN/FFmpeg-Builds
scripts/package-windows.sh
```

Set `GESTURA_FFMPEG_SKIP_DOWNLOAD=1` to skip the download step when a
pre-staged binary already exists (useful for CI layer caching).

For the Gestura CLI sidecar:

1. Build the CLI (`cargo build --release -p gestura-cli ...`).
2. Copy into this folder under the expected name.
3. Ensure Linux/macOS binaries are executable (`chmod +x`).

Do **not** commit compiled binaries to git; only commit documentation and
ignore rules.
