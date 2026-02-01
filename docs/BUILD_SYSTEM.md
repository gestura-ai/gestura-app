# Haptic Harmony Simulator - Build System Documentation

This document describes the comprehensive build system for multi-platform releases and package manager publishing.

## Overview

The build system supports:
- **Multi-platform builds**: macOS, Linux, Windows
- **Multi-architecture**: x64 (Intel) and ARM64 architectures
- **Package managers**: Homebrew, Chocolatey, Winget, Snap, Flatpak, AppImage
- **Automated CI/CD**: GitHub Actions workflows
- **Icon generation**: Automated from source PNG

## Quick Start

### Generate Icons
```bash
./scripts/generate-icons.sh
```

### Build All Platforms
```bash
./scripts/build-release.sh [version]
```

### Manual Build for Specific Target
```bash
# CLI version
cargo build --release --target x86_64-apple-darwin

# GUI version (build frontend first)
cd ui && npm run build && cd ..
cargo build --release --target x86_64-apple-darwin --features tauri-gui
```

## Supported Platforms and Architectures

### macOS
- **x64 (Intel)**: `x86_64-apple-darwin`
- **ARM64 (Apple Silicon)**: `aarch64-apple-darwin`

### Linux
- **x64 (Intel)**: `x86_64-unknown-linux-gnu`
- **ARM64**: `aarch64-unknown-linux-gnu`

### Windows
- **x64 (Intel)**: `x86_64-pc-windows-msvc`
- **ARM64**: `aarch64-pc-windows-msvc`

## Package Managers

### Homebrew (macOS/Linux)
- **Formula**: `packaging/homebrew/haptic-harmony-simulator.rb`
- **Installation**: `brew install gestura-ai/tap/haptic-harmony-simulator`
- **Auto-update**: Via GitHub Actions on release

### Chocolatey (Windows)
- **Package**: Auto-generated from release
- **Installation**: `choco install haptic-harmony-simulator`
- **Auto-publish**: Via GitHub Actions on release

### Winget (Windows)
- **Package**: `GesturaAI.HapticHarmonySimulator`
- **Installation**: `winget install GesturaAI.HapticHarmonySimulator`
- **Auto-update**: Via GitHub Actions on release

### Snap (Linux)
- **Package**: `haptic-harmony-simulator`
- **Installation**: `sudo snap install haptic-harmony-simulator`
- **Config**: `snap/snapcraft.yaml`

### Flatpak (Linux)
- **App ID**: `ai.gestura.HapticHarmonySimulator`
- **Installation**: `flatpak install ai.gestura.HapticHarmonySimulator`
- **Config**: `ai.gestura.HapticHarmonySimulator.yml`

### AppImage (Linux)
- **Portable**: Self-contained executable
- **Download**: From GitHub releases
- **Auto-generated**: Via GitHub Actions

## CI/CD Workflows

### Main Release Workflow (`.github/workflows/release.yml`)
Triggers on:
- Git tags (`v*`)
- Manual dispatch

Builds:
- All platform/architecture combinations
- Both CLI and GUI versions
- Uploads to GitHub releases

### Package Manager Publishing (`.github/workflows/package-managers.yml`)
Triggers on:
- Release published
- Manual dispatch

Publishes to:
- Homebrew tap
- Chocolatey repository
- Winget repository
- Snap store
- Creates AppImage

### Continuous Integration (`.github/workflows/ci.yml`)
Triggers on:
- Push to main/develop
- Pull requests

Tests:
- Code formatting
- Clippy lints
- Unit tests
- Security audit
- Code coverage

## Scripts

### Icon Generation (`scripts/generate-icons.sh`)
Generates all required icon formats from `icons/icon.png`:
- PNG icons: 16x16 to 1024x1024
- High-DPI @2x versions
- Windows ICO format
- macOS ICNS format
- SVG format (if potrace available)

**Requirements**:
- ImageMagick (`brew install imagemagick`)
- iconutil (macOS built-in)
- potrace (optional, for SVG)

### Release Build (`scripts/build-release.sh`)
Comprehensive build script that:
- Installs all Rust targets
- Builds CLI and GUI versions
- Creates distribution archives
- Generates checksums
- Supports version parameter

**Usage**:
```bash
./scripts/build-release.sh 1.0.0
```

## Development Setup

### Prerequisites
```bash
# Install Rust
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh

# Install Node.js (for GUI builds)
# macOS: brew install node
# Ubuntu: sudo apt install nodejs npm

# Install system dependencies (Linux)
sudo apt-get install libwebkit2gtk-4.0-dev libgtk-3-dev libayatana-appindicator3-dev librsvg2-dev libudev-dev libdbus-1-dev

# Install system dependencies (macOS)
# cmake: Required for whisper-rs (whisper.cpp) compilation
# pkg-config: Required for locating system libraries
# Note: Accelerate framework is built-in to macOS (used by whisper-rs for BLAS)
brew install cmake pkg-config

# Install ImageMagick (for icon generation)
# macOS: brew install imagemagick
# Ubuntu: sudo apt install imagemagick
```

## Feature Flags

Gestura uses Cargo feature flags to enable optional functionality. All features are defined in `crates/gestura-core/Cargo.toml` and re-exported by CLI and GUI crates.

### Cross-Platform Features

| Feature | Description | Dependencies | CI Tested |
|---------|-------------|--------------|-----------|
| `voice-local` | Local speech-to-text via whisper.cpp bindings | `whisper-rs`, cmake, LLVM/clang | ✅ All platforms |
| `voice-openai` | Cloud speech-to-text via OpenAI Whisper HTTP API | None (uses reqwest) | ✅ All platforms |
| `nats` | NATS messaging integration | `async-nats` | ✅ All platforms |
| `ble` | BLE integration for Haptic Harmony ring | `btleplug` | ✅ All platforms |
| `security` | Encryption and keychain access | `ring`, `keyring` | ✅ All platforms |
| `json-ld` | JSON-LD processing for MDH | `json-ld` | ✅ All platforms |
| `dev` | Development-only code paths (do NOT enable in production) | None | ✅ All platforms |

### Platform-Specific Features

| Feature | Platform | Description | Dependencies |
|---------|----------|-------------|--------------|
| `macos-permissions` | macOS only | Native TCC permission dialogs (microphone, accessibility, bluetooth, screen recording) | `objc`, `cocoa` |
| `linux-permissions` | Linux only | xdg-desktop-portal integration for Wayland screen recording, D-Bus checks | `ashpd`, `zbus` |
| `windows-permissions` | Windows only | WinRT APIs for microphone/camera permission status, Settings integration | `windows` crate |

#### Permission Behavior by Platform

**macOS (`macos-permissions`):**
- Uses Apple's TCC (Transparency, Consent, and Control) framework
- Direct permission requests via system dialogs (AVCaptureDevice, AXIsProcessTrusted, etc.)
- Can open System Preferences to specific privacy panes
- All 4 permissions have explicit check/request APIs

**Linux (`linux-permissions`):**
- **Microphone/Bluetooth/Accessibility**: No per-app permission dialogs (managed by PulseAudio/PipeWire and user groups)
- **Screen Recording (Wayland)**: Uses xdg-desktop-portal screencast interface
- **Screen Recording (X11)**: Generally unrestricted, no permission needed
- Can open GNOME/KDE settings via `gnome-control-center` or `xdg-open`

**Windows (`windows-permissions`):**
- Uses WinRT `Windows.Media.Capture` APIs for microphone/camera status
- No direct "request permission" dialog - system prompts automatically on first access
- If access is denied, opens Windows Settings to the appropriate privacy page (`ms-settings:` URIs)
- Bluetooth and screen recording don't have TCC-style permissions on Windows

### Default Features

By default, builds enable `voice-local` only:
```toml
[features]
default = ["voice-local"]
```

### Building with Feature Flags

```bash
# Build with specific features
cargo build --release --features "voice-local,nats,security"

# Build with all cross-platform features
cargo build --release --features "voice-local,nats,ble,security,json-ld"

# macOS with all features (including platform-specific permissions)
cargo build --release --features "voice-local,nats,ble,security,json-ld,macos-permissions"

# Linux with all features (including platform-specific permissions)
cargo build --release --features "voice-local,nats,ble,security,json-ld,linux-permissions"

# Windows with all features (including platform-specific permissions)
cargo build --release --features "voice-local,nats,ble,security,json-ld,windows-permissions"

# Build without default features
cargo build --release --no-default-features --features "nats,security"
```

### macOS-Specific Notes

**MACOSX_DEPLOYMENT_TARGET**: Set to `10.15` (Catalina) for maximum compatibility. This is configured in:
- `crates/gestura-gui/build.rs` (sets default if not specified)
- `.github/workflows/release.yml` (explicitly set for CI)
- `crates/gestura-gui/tauri.conf.json` (minimumSystemVersion)

**Universal Binary**: macOS releases include a universal binary supporting both Intel (x86_64) and Apple Silicon (aarch64). The `lipo` tool combines architecture-specific binaries.

**Feature Flags**:
- `voice-local`: Requires cmake and uses Apple's Accelerate framework for BLAS operations
- `macos-permissions`: Uses objc/cocoa bindings for macOS permission dialogs

### Windows-Specific Notes

**Build Dependencies**:
```powershell
# Install via Chocolatey (recommended)
choco install cmake llvm -y

# Set LIBCLANG_PATH for bindgen (required for whisper-rs)
$env:LIBCLANG_PATH = "C:\Program Files\LLVM\bin"
```

**Visual Studio Build Tools**: The MSVC toolchain requires Visual Studio Build Tools with the "Desktop development with C++" workload. Install from [Visual Studio Downloads](https://visualstudio.microsoft.com/downloads/).

**WebView2 Runtime**: Tauri v2 requires Microsoft Edge WebView2 Runtime for the application UI. It is:
- Pre-installed on Windows 10 version 1803+ and Windows 11
- Automatically installed by NSIS/WiX installers if missing
- Can be downloaded from [Microsoft WebView2](https://developer.microsoft.com/en-us/microsoft-edge/webview2/)

**Feature Flags**:
- `voice-local`: Requires cmake and LLVM (for bindgen). Uses CPU-based inference on Windows.
- `windows-permissions`: Uses WinRT APIs (`Windows.Media.Capture`) for microphone permission status. Opens Windows Settings (`ms-settings:`) URIs when access is denied.
- All other features work identically to macOS/Linux

**ARM64 Windows (aarch64-pc-windows-msvc)**:
- Target support is available but not yet tested in CI
- Requires ARM64-compatible LLVM and Visual Studio toolchain
- Consider adding to release matrix when Windows on ARM market share grows

**Code Signing**: Windows builds can be signed using Authenticode certificates:
- Certificate stored as base64-encoded PFX in `WINDOWS_CERTIFICATE` secret
- Timestamp server: `http://timestamp.digicert.com` (configured in `tauri.conf.json`)
- Signatures verified in CI using `Get-AuthenticodeSignature`

### Add Rust Targets
```bash
# macOS
rustup target add x86_64-apple-darwin aarch64-apple-darwin

# Linux
rustup target add x86_64-unknown-linux-gnu aarch64-unknown-linux-gnu

# Windows (if cross-compiling)
rustup target add x86_64-pc-windows-msvc aarch64-pc-windows-msvc
```

## Release Process

### 1. Prepare Release
```bash
# Update version in Cargo.toml
# Update CHANGELOG.md
# Generate icons if updated
./scripts/generate-icons.sh
```

### 2. Create Release
```bash
# Tag the release
git tag v1.0.0
git push origin v1.0.0

# Or use GitHub CLI
gh release create v1.0.0 --generate-notes
```

### 3. Automated Publishing
The CI/CD system will automatically:
- Build all platform binaries
- Create GitHub release
- Publish to package managers
- Update Homebrew formula
- Submit to Winget
- Publish Snap package

### 4. Manual Package Updates (if needed)
```bash
# Update Homebrew formula
cd packaging/homebrew
# Edit haptic-harmony-simulator.rb with new version/checksums

# Update Flatpak manifest
# Edit ai.gestura.HapticHarmonySimulator.yml

# Update Snap config
# Edit snap/snapcraft.yaml
```

## Troubleshooting

### Build Failures
- Ensure all dependencies are installed
- Check Rust target is added: `rustup target list --installed`
- For GUI builds, ensure frontend is built: `cd ui && npm run build`

### Cross-compilation Issues
- Linux ARM64: Install `gcc-aarch64-linux-gnu`
- Windows: Use Windows runner or cross-compilation tools

### Package Manager Issues
- Check API keys are set in GitHub secrets
- Verify package configurations are valid
- Test locally before pushing

## Security Considerations

### Secrets Required
- `HOMEBREW_TAP_TOKEN`: GitHub token for Homebrew tap
- `CHOCOLATEY_API_KEY`: Chocolatey API key
- `WINGET_TOKEN`: GitHub token for Winget submissions
- `SNAPCRAFT_TOKEN`: Snapcraft store credentials

### Code Signing (Future)
- macOS: Apple Developer certificate
- Windows: Code signing certificate
- Linux: GPG signing for packages

## Local CI Testing with Act

[Act](https://github.com/nektos/act) allows running GitHub Actions workflows locally. However, there are important limitations for this project.

### Configuration

The `.actrc` file maps platform runners to Ubuntu containers:
```
-P macos-latest=catthehacker/ubuntu:act-latest
-P macos-14=catthehacker/ubuntu:act-latest
-P windows-latest=catthehacker/ubuntu:act-latest
-P windows-2022=catthehacker/ubuntu:act-latest
```

**Note**: Windows and macOS workflows run in Ubuntu containers under Act, meaning platform-specific features (code signing, SDK-dependent code, native toolchains) cannot be tested locally. Use Act for syntax validation and Linux-compatible tests only.

### What CAN Be Tested Locally

| Feature | Testable with Act? |
|---------|-------------------|
| Code formatting (`cargo fmt`) | ✅ Yes |
| Clippy lints | ✅ Yes |
| Rust compilation (Linux targets) | ✅ Yes |
| Unit tests (cross-platform) | ✅ Yes |
| Frontend build (npm) | ✅ Yes |

### What CANNOT Be Tested Locally

| Feature | Reason |
|---------|--------|
| macOS code signing | Requires Apple certificates and Keychain |
| macOS notarization | Requires Apple Developer account |
| Universal binary (`lipo`) | macOS-only tool |
| `macos-permissions` feature | Requires objc/cocoa frameworks |
| Gatekeeper validation (`spctl`) | macOS-only |
| Windows GUI builds | Requires Windows SDK and MSVC toolchain |
| Windows code signing | Requires Authenticode certificate |
| `windows-permissions` feature | Requires WinRT APIs and Windows runtime |
| WebView2-dependent features | Windows-only runtime |
| NSIS/WiX installer generation | Windows-only build tools |
| Windows signature verification | Requires Windows `signtool` or PowerShell |
| `linux-permissions` feature (full) | Requires xdg-desktop-portal and D-Bus session |

### Running Act

```bash
# Run CI workflow locally
act push

# Run specific job
act push -j test

# Run with verbose output
act push -v
```

### Detecting Act Environment

Workflows can detect when running under Act using the `ACT` environment variable:
```yaml
- name: Skip on Act
  if: ${{ !env.ACT }}
  run: echo "This only runs in real GitHub Actions"
```

## Monitoring and Maintenance

### Regular Tasks
- Update dependencies monthly
- Monitor build failures
- Update package manager configurations
- Review security audits

### Version Management
- Follow semantic versioning
- Update all package configurations
- Test on multiple platforms
- Coordinate release announcements
