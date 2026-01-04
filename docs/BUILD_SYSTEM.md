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

# Install ImageMagick (for icon generation)
# macOS: brew install imagemagick
# Ubuntu: sudo apt install imagemagick
```

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
