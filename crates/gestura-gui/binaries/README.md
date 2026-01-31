# Bundled external binaries (Gestura CLI)

This directory is used as a **build-time staging location** for the Gestura CLI so the GUI bundle can ship a **Full install** (GUI + CLI).

The Tauri bundler is configured in `crates/gestura-gui/tauri.conf.json` to include the CLI via:

- `bundle.externalBin: ["binaries/gestura"]`

On Linux, the DEB/RPM packages additionally install the CLI onto PATH by mapping the staged file into `usr/bin/gestura`.

## Naming convention (required by Tauri)

Tauri expects platform-specific binaries using the `externalBin` base name plus a target triple suffix.

Examples of the **expected staged filenames**:

- Linux: `binaries/gestura-x86_64-unknown-linux-gnu`
- Windows: `binaries/gestura-x86_64-pc-windows-msvc.exe`
- macOS universal: `binaries/gestura-universal-apple-darwin`

## How this directory is populated

CI (and optional local packaging scripts) should:

1. Build the CLI (`cargo build --release -p gestura-cli ...`).
2. Copy the resulting binary into this folder under the expected name.
3. Ensure Linux/macOS binaries are executable (`chmod +x`).

Do **not** commit compiled binaries to git; only commit documentation and ignore rules.
