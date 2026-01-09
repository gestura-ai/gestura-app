# macOS Packaging Guide

This document describes how to build, sign, notarize, and package Gestura for macOS distribution.

## Overview

Gestura produces two distribution formats for macOS:
- **DMG**: Disk image with drag-to-install experience
- **PKG**: Installer package for automated deployment

Both formats support Universal Binary (Intel + Apple Silicon).

---

## Quick Start

### Unsigned Development Build

```bash
just build-macos
just test-macos-app
```

### Signed Release Build

```bash
# Set environment variables (add to ~/.zshrc for persistence)
export APPLE_SIGNING_IDENTITY="Developer ID Application: Your Name (TEAMID)"
export APPLE_TEAM_ID="XXXXXXXXXX"
export APPLE_ID="your@email.com"
export APPLE_PASSWORD="@keychain:notarytool-password"

# Build, sign, and notarize
just build-macos-signed

# Create DMG and PKG
just package-macos-signed
```

---

## Prerequisites

### Required Tools

- **Xcode Command Line Tools**: `xcode-select --install`
- **create-dmg**: `brew install create-dmg`
- **Rust targets**: `rustup target add x86_64-apple-darwin aarch64-apple-darwin`

### For Signed Builds

- Apple Developer Program membership ($99/year)
- Developer ID Application certificate
- Developer ID Installer certificate (for PKG signing)
- App-specific password for notarization

---

## Local Development Signing Setup (Step-by-Step)

Follow these steps to configure your local machine for signed builds with `just build-macos-signed`:

### Step 1: Verify Your Certificate

Check that your Developer ID Application certificate is installed in Keychain:

```bash
just check-macos-signing
```

Look for output like:
```
  2) XXXXXXX "Developer ID Application: Your Name (TEAMID)"
```

If no certificate is found, download it from the [Apple Developer Portal](https://developer.apple.com/account/resources/certificates/list) and import it to your Keychain.

### Step 2: Create an App-Specific Password

1. Go to [appleid.apple.com](https://appleid.apple.com/account/manage)
2. Sign in with your Apple ID
3. Navigate to **Security** → **App-Specific Passwords**
4. Click **Generate Password**
5. Name it "Gestura Notarization"
6. Copy the generated password (format: `xxxx-xxxx-xxxx-xxxx`)

### Step 3: Store Credentials in Keychain

Store your notarization credentials securely in macOS Keychain:

```bash
xcrun notarytool store-credentials "notarytool-password" \
  --apple-id "your-apple-id@email.com" \
  --team-id "63WY89YNKN" \
  --password "xxxx-xxxx-xxxx-xxxx"
```

Replace:
- `your-apple-id@email.com` with your Apple ID email
- `63WY89YNKN` with your Team ID (from the certificate name)
- `xxxx-xxxx-xxxx-xxxx` with your app-specific password

### Step 4: Configure Environment Variables

Add these to your shell profile (`~/.zshrc` or `~/.bashrc`):

```bash
# Gestura macOS Signing Configuration
export APPLE_SIGNING_IDENTITY="Developer ID Application: Gestura AI LLC (63WY89YNKN)"
export APPLE_TEAM_ID="63WY89YNKN"
export APPLE_ID="your-apple-id@email.com"
export APPLE_PASSWORD="@keychain:notarytool-password"
```

Then reload your shell:
```bash
source ~/.zshrc  # or source ~/.bashrc
```

### Step 5: Verify Configuration

Run the check command again to confirm everything is set:

```bash
just check-macos-signing
```

Expected output:
```
🔍 Checking macOS code signing setup...
Available signing identities:
  2) XXXXXXX "Developer ID Application: Your Name (TEAMID)"

Environment variables:
APPLE_SIGNING_IDENTITY: ✅ Developer ID Application: ...
APPLE_ID: ✅ your@email.com
APPLE_PASSWORD: ✅ Set
APPLE_TEAM_ID: ✅ XXXXXXXXXX
```

### Step 6: Build and Sign

Now you can build a signed, notarized app:

```bash
# Full signed build with notarization
just build-macos-signed

# After notarization, create distribution packages
just package-macos-signed
```

### Step 7: Verify the Build

```bash
# Verify signature and notarization
just verify-macos

# Test the app
just test-macos-app
```

---

## Environment Variables

| Variable | Description | Example |
|----------|-------------|---------|
| `APPLE_SIGNING_IDENTITY` | Developer ID Application certificate | `Developer ID Application: Gestura AI LLC (63WY89YNKN)` |
| `APPLE_TEAM_ID` | 10-character Team ID | `63WY89YNKN` |
| `APPLE_ID` | Apple ID email | `developer@gestura.ai` |
| `APPLE_PASSWORD` | App-specific password or keychain reference | `@keychain:notarytool-password` |
| `APPLE_INSTALLER_IDENTITY` | (Optional) Developer ID Installer certificate | `Developer ID Installer: Gestura AI LLC (63WY89YNKN)` |

### Storing Credentials in Keychain

```bash
# Store notarization credentials
xcrun notarytool store-credentials notarytool-password \
  --apple-id your@email.com \
  --team-id XXXXXXXXXX \
  --password <app-specific-password>

# Then use in APPLE_PASSWORD
export APPLE_PASSWORD="@keychain:notarytool-password"
```

---

## Build Commands

| Command | Description |
|---------|-------------|
| `just build-macos` | Build unsigned app (current architecture) |
| `just build-macos-universal` | Build unsigned universal binary |
| `just build-macos-signed` | Build, sign, and notarize |
| `just package-macos` | Create DMG and PKG from unsigned build |
| `just package-macos-signed` | Create DMG and PKG from signed build |
| `just verify-macos` | Verify code signature |
| `just test-macos-app` | Test and launch app bundle |
| `just create-dmg` | Create DMG only |
| `just release-macos` | Full release workflow |

---

## Output Locations

### Build Artifacts

| Build Type | Location |
|------------|----------|
| Universal binary | `src-tauri/target/universal-apple-darwin/release/bundle/macos/Gestura.app` |
| Regular binary | `src-tauri/target/release/bundle/macos/Gestura.app` |

### Distribution Packages

| File | Location |
|------|----------|
| DMG | `dist/macos/Gestura-{version}-universal.dmg` |
| PKG | `dist/macos/Gestura-{version}-universal.pkg` |
| Checksums | `dist/macos/*.sha256` |
| Release info | `dist/macos/RELEASE_INFO.txt` |

---

## PKG Installation Layout

The PKG installer places files in these locations:

| Component | Install Location |
|-----------|------------------|
| Gestura.app | `/Applications/Gestura.app` |
| CLI tools (if present) | `/usr/local/bin/gestura` |

### Verifying PKG Contents

```bash
# List files in PKG
pkgutil --payload-files dist/macos/Gestura-0.1.0-universal.pkg

# Expected output:
# ./Applications/Gestura.app
# ./Applications/Gestura.app/Contents/...
```

---

## Verification Commands

### Check Signature

```bash
# Verify signature
codesign --verify --deep --strict --verbose=2 \
  src-tauri/target/universal-apple-darwin/release/bundle/macos/Gestura.app

# Display signature details
codesign -dv --verbose=4 \
  src-tauri/target/universal-apple-darwin/release/bundle/macos/Gestura.app
```

### Check Notarization

```bash
# Verify Gatekeeper approval
spctl -a -t exec -vv \
  src-tauri/target/universal-apple-darwin/release/bundle/macos/Gestura.app

# Check stapled ticket
xcrun stapler validate \
  src-tauri/target/universal-apple-darwin/release/bundle/macos/Gestura.app
```

---

## Troubleshooting

### "App is damaged and can't be opened"

The app is not properly signed or notarized:
1. Run `just verify-macos` to check signature
2. Ensure notarization completed successfully
3. Re-run `just build-macos-signed`

### "Developer cannot be verified"

The app is signed but not notarized:
1. Check notarization status: `xcrun notarytool history --keychain-profile notarytool-password`
2. Re-submit for notarization: `./scripts/notarize-mac.sh`

### Certificate not found

1. Check available certificates: `security find-identity -v -p codesigning`
2. Ensure certificate is not expired
3. Verify certificate name matches `APPLE_SIGNING_IDENTITY`

### PKG installs extra files

1. Check `scripts/package-mac.sh` for correct `pkgroot` setup
2. Verify only `Applications/Gestura.app` is in the package root
3. Run `pkgutil --payload-files` to inspect contents

