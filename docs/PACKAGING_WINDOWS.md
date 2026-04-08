# Windows Packaging Guide

This document describes how to build, sign, and package Gestura for Windows distribution.

## Overview

Gestura produces two distribution formats for Windows:
- **MSI**: Windows Installer package (via WiX Toolset)
- **NSIS**: Nullsoft Scriptable Install System installer

Both formats support x86_64 architecture. ARM64 support is planned for future releases.

---

## Quick Start

### Unsigned Development Build

```powershell
# Build CLI with all features
cargo build --release -p gestura-cli

# Build GUI (requires frontend build first)
cd crates/gestura-gui/frontend
npm ci && npm run build
cd ../../..
cargo build --release -p gestura-gui
```

### Signed Release Build

Signing is handled automatically in CI when the SSL.com eSigner secrets are configured.

---

## Prerequisites

### Build Dependencies

Install the required build tools via Chocolatey:

```powershell
# Install Chocolatey (if not already installed)
Set-ExecutionPolicy Bypass -Scope Process -Force
[System.Net.ServicePointManager]::SecurityProtocol = [System.Net.SecurityProtocolType]::Tls12
iex ((New-Object System.Net.WebClient).DownloadString('https://community.chocolatey.org/install.ps1'))

# Install build dependencies
choco install cmake llvm -y

# Set LIBCLANG_PATH for bindgen (required for whisper-rs)
[Environment]::SetEnvironmentVariable("LIBCLANG_PATH", "C:\Program Files\LLVM\bin", "User")
```

### Visual Studio Build Tools

Install Visual Studio Build Tools with the "Desktop development with C++" workload:

1. Download from [Visual Studio Downloads](https://visualstudio.microsoft.com/downloads/)
2. Run the installer and select **Build Tools for Visual Studio 2022**
3. Under "Workloads", select **Desktop development with C++**
4. Install

### WebView2 Runtime

Tauri v2 requires Microsoft Edge WebView2 Runtime:

- **Windows 10 1803+** and **Windows 11**: Pre-installed
- **Older Windows**: Download from [Microsoft WebView2](https://developer.microsoft.com/en-us/microsoft-edge/webview2/)
- **Installer behavior**: NSIS/WiX installers automatically install WebView2 if missing

### Rust Targets

```powershell
# Add Windows target (usually default on Windows)
rustup target add x86_64-pc-windows-msvc

# Future ARM64 support
rustup target add aarch64-pc-windows-msvc
```

### Build Configuration

| Setting | Value | Location |
|---------|-------|----------|
| Target Architecture | x86_64-pc-windows-msvc | CI matrix |
| Installer Format | MSI (WiX) | tauri.conf.json |
| Timestamp Server | ts.ssl.com (CI eSigner) | `scripts/sign-windows-esigner.py` |
| Digest Algorithm | SHA256 | tauri.conf.json |

---

## Code Signing

### Certificate Requirements

Windows code signing requires an Authenticode certificate from a trusted Certificate Authority:

- **EV Code Signing Certificate**: Recommended for immediate SmartScreen reputation
- **Standard Code Signing Certificate**: Works but requires reputation building
- **Self-signed**: Development only (causes SmartScreen warnings)

### Obtaining a Certificate

Purchase from a trusted CA:
- DigiCert
- Sectigo (Comodo)
- GlobalSign
- SSL.com

### Setting Up CI Signing

1. **Provision SSL.com eSigner access** for the Windows code signing certificate
2. **Add GitHub Secrets**:
   - `ESIGNER_USERNAME`
   - `ESIGNER_PASSWORD`
   - `ESIGNER_CREDENTIAL_ID`
   - `ESIGNER_TOTP_SECRET`

### Tauri Configuration

The static configuration in `crates/gestura-gui/tauri.conf.json` keeps the Windows signing fields neutral by default:

```json
"windows": {
  "certificateThumbprint": null,
  "digestAlgorithm": "sha256",
  "timestampUrl": "http://timestamp.digicert.com"
}
```

- `certificateThumbprint`: Left unset in the checked-in config; CI uses a temporary custom sign command instead
- `signCommand`: Injected automatically during CI for eSigner-backed release builds, then restored after the build
- `timestampUrl`: Default timestamp setting for local signing flows; CI eSigner signing uses the SSL.com RFC3161 timestamp service

---

## CI/CD Integration

### GitHub Actions Workflow

The release workflow (`.github/workflows/release.yml`) handles Windows builds:

1. **Runner**: Defaults to `windows-2022`, but can be overridden with the `RELEASE_WINDOWS_RUNNER` repo/org variable
2. **Install dependencies**: cmake and LLVM via Chocolatey
3. **Install signing dependencies**: Python, Java, and Jsign when signing secrets are present
4. **MSI signing**: Inject a Tauri `signCommand` that calls `scripts/sign-windows-esigner.py` during the packaging run
5. **CLI signing**: Sign the standalone `gestura.exe` with the same eSigner-backed script before zipping it
6. **Verification**: Hard-fails published releases if `Get-AuthenticodeSignature` is not `Valid`

### Secrets Required

| Secret | Description |
|--------|-------------|
| `ESIGNER_USERNAME` | SSL.com eSigner username |
| `ESIGNER_PASSWORD` | SSL.com eSigner password |
| `ESIGNER_CREDENTIAL_ID` | SSL.com signing credential ID |
| `ESIGNER_TOTP_SECRET` | SSL.com TOTP secret for unattended signing |

### Verification

After build, verify signature:

```powershell
Get-AuthenticodeSignature .\Gestura-v1.0.0-windows-x86_64.msi
```

Expected output for signed installer:
```
Status: Valid
SignerCertificate: [Subject] CN=Your Company Name
```

---

## Troubleshooting

### Build Failures

| Issue | Solution |
|-------|----------|
| `LIBCLANG_PATH not found` | Set `LIBCLANG_PATH` to LLVM bin directory |
| `cmake not found` | Install cmake via Chocolatey |
| `MSVC not found` | Install Visual Studio Build Tools |
| `WebView2 missing` | Will be installed by Tauri during build |

### Signing Issues

| Issue | Solution |
|-------|----------|
| `eSigner authentication failed` | Verify the four `ESIGNER_*` secrets and confirm the credential ID matches the certificate |
| `Timestamp failed` | Check network access to `http://ts.ssl.com` |
| `SmartScreen warning` | Use EV certificate or build reputation over time |

