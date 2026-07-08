# Justfile for Gestura.app (compliments Makefile)

set shell := ["bash", "-cu"]

# Default target
default: help

help:
	@echo "just targets:"
	@echo ""
	@echo "🔧 Development:"
	@echo "  dev                  # run frontend dev server with hot reload"
	@echo "  build                # cargo build (debug)"
	@echo "  build-release        # cargo build (release)"
	@echo "  test                 # cargo test"
	@echo "  clean                # cargo clean"
	@echo ""
	@echo "🏗️ Platform Builds:"
	@echo "  build-macos          # build macOS app bundle"
	@echo "  build-macos-signed   # build and sign macOS app bundle"
	@echo "  build-macos-universal # build universal macOS binary"
	@echo "  build-windows        # build Windows executable"
	@echo "  build-windows-signed # build and sign Windows executable"
	@echo "  build-linux          # build Linux binary"
	@echo "  build-linux-deb      # build Linux deb package"
	@echo "  build-linux-appimage # build Linux AppImage"
	@echo ""
	@echo "📦 Packaging:"
	@echo "  package              # build production app (current platform)"
	@echo "  package-macos        # create macOS DMG and PKG"
	@echo "  package-windows      # create Windows installer"
	@echo "  package-linux        # create Linux packages"
	@echo "  package-all          # create packages for all platforms"
	@echo "  create-dmg           # create macOS DMG"
	@echo "  create-windows-msi   # create Windows MSI"
	@echo "  create-linux-deb     # create Linux deb"
	@echo "  create-linux-appimage # create Linux AppImage"
	@echo ""
	@echo "🔐 Code Signing:"
	@echo "  check-macos-signing  # check macOS signing setup"
	@echo "  check-windows-signing # check Windows signing setup"
	@echo "  check-linux-signing # check Linux signing setup"
	@echo "  check-all-signing    # check all platform signing"
	@echo "  install-apple-certificates # install Apple certificate chain"
	@echo "  sign-macos           # sign macOS app bundle"
	@echo "  sign-windows         # sign Windows executable"
	@echo "  notarize-macos       # notarize macOS app"
	@echo "  verify-macos         # verify macOS app signature"
	@echo "  verify-windows       # verify Windows signature"
	@echo ""
	@echo "🧪 Testing:"
	@echo "  validate             # run full production validation pipeline"
	@echo "  validate-quick       # run quick validation (format, clippy, test)"
	@echo "  test-ui              # run UI tests"
	@echo "  test-tray            # test system tray functionality"
	@echo "  test-macos-app       # test macOS app bundle"
	@echo "  test-windows-app     # test Windows executable"
	@echo "  test-linux-app       # test Linux packages"
	@echo "  test-all-platforms   # test all platform builds"
	@echo ""
	@echo "🛠️ Utilities:"
	@echo "  doctor               # print environment info"
	@echo "  clean-all            # clean all build artifacts"
	@echo "  icons                # generate app icons"
	@echo "  quick-dev            # quick development build"
	@echo "  full-build           # full build and test"
	@echo "  release-macos        # complete macOS release"
	@echo "  release-windows      # complete Windows release"
	@echo "  release-linux        # complete Linux release"
	@echo "  release-all          # complete all platform releases"

# Production validation - run all checks that CI will run
validate:
	@echo "🔍 Running production validation..."
	./scripts/validate-production.sh

# Quick validation - essential checks only
validate-quick:
	@echo "⚡ Running quick validation..."
	@echo "Checking formatting..."
	cargo fmt --manifest-path {{app_dir}}/Cargo.toml -- --check
	@echo "Running clippy (CLI)..."
	cargo clippy --manifest-path {{app_dir}}/Cargo.toml --features cli-only -- -D warnings
	@echo "Running tests (CLI)..."
	cargo test --manifest-path {{app_dir}}/Cargo.toml --features cli-only
	@echo "✅ Quick validation complete!"

# Root paths
app_dir := "src-tauri"

# Development Commands
# ====================

build:
	cargo build --manifest-path {{app_dir}}/Cargo.toml

build-release:
	cargo build --release --manifest-path {{app_dir}}/Cargo.toml

test:
	cargo test --manifest-path {{app_dir}}/Cargo.toml -q

clean:
	cargo clean --manifest-path {{app_dir}}/Cargo.toml

clean-all:
	@echo "🧹 Cleaning all build artifacts..."
	cargo clean --manifest-path {{app_dir}}/Cargo.toml
	rm -rf dist/
	rm -rf src-tauri/target/
	rm -rf node_modules/
	rm -f *.dmg *.pkg *.exe *.deb *.AppImage
	@echo "✅ All build artifacts cleaned"

# UI Testing commands
test-ui:
	node scripts/test-ui.js

test-ui-component component:
	node scripts/test-ui.js {{component}}

test-html:
	node scripts/test-ui.js html

check-cmake:
	command -v cmake >/dev/null 2>&1 || { echo "Error: cmake is required for voice-local builds."; exit 1; }

build-voice-local: check-cmake
	cargo build --manifest-path {{app_dir}}/Cargo.toml --features voice-local

run-voice-local: check-cmake
	cargo run --manifest-path {{app_dir}}/Cargo.toml --features voice-local

check-nats:
	cargo check --manifest-path {{app_dir}}/Cargo.toml --features nats

dev:
	@echo "🚀 Starting development server with hot reload..."
	npm install
	npm run tauri:dev

# Platform-Specific Builds
# =========================

# Build macOS app bundle (unsigned)
build-macos:
	@echo "🍎 Building macOS app bundle (unsigned)..."
	npm install
	npm run build
	npm run tauri:build

# Build and sign macOS app bundle
build-macos-signed:
	@echo "🍎🔐 Building and signing macOS app bundle..."
	@if [ -z "$APPLE_SIGNING_IDENTITY" ]; then \
		echo "❌ APPLE_SIGNING_IDENTITY environment variable not set"; \
		echo "Set it to your Developer ID Application certificate name"; \
		exit 1; \
	fi
	@if [ -z "$APPLE_CERTIFICATE" ]; then \
		echo "❌ APPLE_CERTIFICATE environment variable not set"; \
		echo "This should contain the certificate data"; \
		exit 1; \
	fi
	@echo "Using signing identity: $APPLE_SIGNING_IDENTITY"
	@echo "Using team ID: $APPLE_TEAM_ID"
	@echo "Certificate data: $(echo "$APPLE_CERTIFICATE" | wc -c) bytes"
	#!/bin/bash
	set -e
	# Do not modify signing environment; rely on already-set variables
	npm install
	npm run build
	npm run tauri:build

# Build universal macOS binary (Intel + Apple Silicon)
build-macos-universal:
	@echo "🍎🔄 Building universal macOS binary..."
	npm install
	npm run build
	npm run tauri build -- --target universal-apple-darwin

# Build Windows executable
build-windows:
	@echo "🪟 Building Windows executable..."
	npm install
	npm run build
	npm run tauri build -- --target x86_64-pc-windows-msvc

# Build and sign Windows executable
build-windows-signed:
	@echo "🪟🔐 Building and signing Windows executable..."
	@if [ -z "$$WINDOWS_SIGNING_CERT" ]; then \
		echo "❌ WINDOWS_SIGNING_CERT environment variable not set"; \
		echo "Set it to your Windows code signing certificate path"; \
		exit 1; \
	fi
	@echo "Using signing certificate: $$WINDOWS_SIGNING_CERT"
	export WINDOWS_SIGNING_CERT="$$WINDOWS_SIGNING_CERT" && \
	export WINDOWS_SIGNING_PASSWORD="$$WINDOWS_SIGNING_PASSWORD" && \
	npm install && \
	npm run build && \
	npm run tauri build -- --target x86_64-pc-windows-msvc

# Build Linux binary
build-linux:
	@echo "🐧 Building Linux binary..."
	npm install
	npm run build
	npm run tauri build -- --target x86_64-unknown-linux-gnu

# Build Linux AppImage
build-linux-appimage:
	@echo "🐧📦 Building Linux AppImage..."
	npm install
	npm run build
	npm run tauri build -- --target x86_64-unknown-linux-gnu --bundles appimage

# Build Linux deb package
build-linux-deb:
	@echo "🐧📦 Building Linux deb package..."
	npm install
	npm run build
	npm run tauri build -- --target x86_64-unknown-linux-gnu --bundles deb

package:
	@echo "📦 Building production app for current platform..."
	npm install
	npm run tauri:build

doctor:
	@echo "🩺 Gestura.app Development Environment"
	@echo "====================================="
	@echo ""
	@echo "🦀 Rust Toolchain:"
	@echo "  Rustc: $(rustc --version)"
	@echo "  Cargo: $(cargo --version)"
	@echo ""
	@echo "🌐 Node.js Environment:"
	@echo "  Node: $(node --version 2>/dev/null || echo '❌ not found')"
	@echo "  npm: $(npm --version 2>/dev/null || echo '❌ not found')"
	@echo ""
	@echo "🛠️ Build Tools:"
	@echo "  cmake: $(cmake --version 2>/dev/null | head -1 || echo '❌ not found')"
	@echo "  just: $(just --version 2>/dev/null || echo '❌ not found')"
	@echo "  make: $(make --version 2>/dev/null | head -1 || echo '❌ not found')"
	@echo ""
	@echo "🍎 macOS Tools:"
	@echo "  Xcode: $(xcodebuild -version 2>/dev/null | head -1 || echo '❌ not found')"
	@echo "  codesign: $(codesign --version 2>/dev/null || echo '❌ not found')"
	@echo "  hdiutil: $(which hdiutil >/dev/null && echo '✅ available' || echo '❌ not found')"
	@echo ""
	@echo "🔐 Code Signing:"
	@if [ -n "$$APPLE_SIGNING_IDENTITY" ]; then echo "  APPLE_SIGNING_IDENTITY: ✅ $$APPLE_SIGNING_IDENTITY"; else echo "  APPLE_SIGNING_IDENTITY: ❌ not set"; fi
	@if [ -n "$$APPLE_TEAM_ID" ]; then echo "  APPLE_TEAM_ID: ✅ $$APPLE_TEAM_ID"; else echo "  APPLE_TEAM_ID: ❌ not set"; fi
	@echo "  Developer certificates: $(security find-identity -v -p codesigning | grep -c "Developer ID Application" || echo '0')"
	@echo ""
	@echo "💻 System:"
	@echo "  OS: $(uname -a)"
	@echo "  Architecture: $(uname -m)"
	@echo ""
	@echo "📁 Project Status:"
	@echo "  Frontend built: $([ -d 'dist' ] && echo '✅ yes' || echo '❌ no')"
	@echo "  Rust binary: $([ -f 'src-tauri/target/release/gestura' ] && echo '✅ yes' || echo '❌ no')"
	@echo "  macOS app: $([ -d 'src-tauri/target/release/bundle/macos/Gestura.app' ] && echo '✅ yes' || echo '❌ no')"

# Quick development workflow
quick-dev: clean build-release test
	@echo "🚀 Quick development build complete!"

# Full build and test workflow
full-build: clean build-macos test-macos-app
	@echo "🎉 Full build and test complete!"

# Release workflow for macOS
release-macos: clean build-macos-signed verify-macos create-dmg
	@echo "🎉 macOS release build complete!"
	@echo "📁 Check for Gestura-*-macos.dmg file"

# Release workflow for Windows
release-windows: clean build-windows-signed create-windows-msi
	@echo "🎉 Windows release build complete!"
	@echo "📁 Check src-tauri/target/x86_64-pc-windows-msvc/release/bundle/msi/ for installer"

# Release workflow for Linux
release-linux: clean build-linux-deb build-linux-appimage
	@echo "🎉 Linux release build complete!"
	@echo "📁 Check src-tauri/target/x86_64-unknown-linux-gnu/release/bundle/ for packages"

# Release workflow for all platforms
release-all: clean
	@echo "🌍 Building releases for all platforms..."
	@echo "This will take a while..."
	just release-macos
	just release-windows
	just release-linux
	@echo "🎉 All platform releases complete!"

# Test all platforms
test-all-platforms:
	@echo "🧪🌍 Testing all platform builds..."
	@if [ -d "src-tauri/target/release/bundle/macos/Gestura.app" ]; then \
		echo "Testing macOS..."; \
		just test-macos-app; \
	else \
		echo "⚠️ macOS build not found"; \
	fi
	@if [ -f "src-tauri/target/x86_64-pc-windows-msvc/release/gestura.exe" ]; then \
		echo "Testing Windows..."; \
		just test-windows-app; \
	else \
		echo "⚠️ Windows build not found"; \
	fi
	@if [ -f "src-tauri/target/x86_64-unknown-linux-gnu/release/gestura" ]; then \
		echo "Testing Linux..."; \
		just test-linux-app; \
	else \
		echo "⚠️ Linux build not found"; \
	fi

# Check signing setup for all platforms
check-all-signing:
	@echo "🔍🌍 Checking code signing setup for all platforms..."
	@echo ""
	just check-macos-signing
	@echo ""
	just check-windows-signing
	@echo ""
	just check-linux-signing

# Packaging Commands
# ==================

# Create macOS DMG and PKG
package-macos: build-macos
	@echo "📦🍎 Creating macOS packages (DMG + PKG)..."
	./scripts/package-mac.sh

# Create Windows installer
package-windows: build-windows
	@echo "📦🪟 Creating Windows installer..."
	@if [ ! -f "scripts/package-windows.sh" ]; then \
		echo "⚠️ Windows packaging script not found, using basic build"; \
		just build-windows; \
	else \
		./scripts/package-windows.sh; \
	fi

# Create Windows MSI installer
create-windows-msi:
	@echo "📦🪟 Creating Windows MSI installer..."
	@if [ ! -d "src-tauri/target/x86_64-pc-windows-msvc/release/bundle/msi" ]; then \
		echo "❌ Windows MSI not found. Run 'just build-windows' first"; \
		exit 1; \
	fi
	@echo "✅ Windows MSI available in src-tauri/target/x86_64-pc-windows-msvc/release/bundle/msi/"
	@ls -la "src-tauri/target/x86_64-pc-windows-msvc/release/bundle/msi/"

# Create Linux packages (deb, rpm, AppImage)
package-linux: build-linux
	@echo "📦🐧 Creating Linux packages..."
	@if [ ! -f "scripts/package-linux.sh" ]; then \
		echo "⚠️ Linux packaging script not found, using basic build"; \
		just build-linux-deb && just build-linux-appimage; \
	else \
		./scripts/package-linux.sh; \
	fi

# Create Linux AppImage
create-linux-appimage:
	@echo "📦🐧 Creating Linux AppImage..."
	@if [ ! -f "src-tauri/target/x86_64-unknown-linux-gnu/release/bundle/appimage/gestura_0.1.0_amd64.AppImage" ]; then \
		echo "❌ Linux AppImage not found. Run 'just build-linux-appimage' first"; \
		exit 1; \
	fi
	@echo "✅ Linux AppImage available:"
	@ls -la "src-tauri/target/x86_64-unknown-linux-gnu/release/bundle/appimage/"

# Create Linux deb package
create-linux-deb:
	@echo "📦🐧 Creating Linux deb package..."
	@if [ ! -f "src-tauri/target/x86_64-unknown-linux-gnu/release/bundle/deb/gestura_0.1.0_amd64.deb" ]; then \
		echo "❌ Linux deb not found. Run 'just build-linux-deb' first"; \
		exit 1; \
	fi
	@echo "✅ Linux deb package available:"
	@ls -la "src-tauri/target/x86_64-unknown-linux-gnu/release/bundle/deb/"

# Create packages for all platforms
package-all:
	@echo "📦🌍 Creating packages for all platforms..."
	just package-macos
	just package-windows
	just package-linux

# Code Signing Commands
# =====================

# Check macOS signing setup
check-macos-signing:
	@echo "🔍 Checking macOS code signing setup..."
	@echo "Available signing identities:"
	security find-identity -v -p codesigning | grep "Developer ID Application" || echo "❌ No Developer ID Application certificates found"
	@echo ""
	@echo "Environment variables:"
	@if [ -n "$APPLE_CERTIFICATE" ]; then echo "APPLE_CERTIFICATE: ✅ $(echo "$APPLE_CERTIFICATE" | wc -c) bytes"; else echo "APPLE_CERTIFICATE: ❌ Not set"; fi
	@if [ -n "$APPLE_CERTIFICATE_PASSWORD" ]; then echo "APPLE_CERTIFICATE_PASSWORD: ✅ Set"; else echo "APPLE_CERTIFICATE_PASSWORD: ❌ Not set"; fi
	@if [ -n "$APPLE_SIGNING_IDENTITY" ]; then echo "APPLE_SIGNING_IDENTITY: ✅ $APPLE_SIGNING_IDENTITY"; else echo "APPLE_SIGNING_IDENTITY: ❌ Not set"; fi
	@if [ -n "$APPLE_ID" ]; then echo "APPLE_ID: ✅ $APPLE_ID"; else echo "APPLE_ID: ❌ Not set"; fi
	@if [ -n "$APPLE_PASSWORD" ]; then echo "APPLE_PASSWORD: ✅ Set"; else echo "APPLE_PASSWORD: ❌ Not set"; fi
	@if [ -n "$APPLE_TEAM_ID" ]; then echo "APPLE_TEAM_ID: ✅ $APPLE_TEAM_ID"; else echo "APPLE_TEAM_ID: ❌ Not set"; fi
	@echo ""
	@echo "📋 Required for signing (matching haptic-harmony-simulation):"
	@echo "  ✅ APPLE_CERTIFICATE - Certificate data (base64 encoded .p12)"
	@echo "  ✅ APPLE_CERTIFICATE_PASSWORD - Certificate password"
	@echo "  ✅ APPLE_SIGNING_IDENTITY - Identity name (e.g., 'Developer ID Application: ...')"
	@echo "  ✅ APPLE_ID - Apple ID for notarization"
	@echo "  ✅ APPLE_PASSWORD - App-specific password for notarization"
	@echo "  ✅ APPLE_TEAM_ID - Developer team ID"



# Sign macOS app bundle manually (fallback method)
sign-macos:
	@echo "🔐🍎 Manual code signing macOS app..."
	@if [ ! -d "src-tauri/target/release/bundle/macos/Gestura.app" ]; then \
		echo "❌ App bundle not found. Run 'just build-macos' first"; \
		exit 1; \
	fi
	@if [ -z "$APPLE_SIGNING_IDENTITY" ]; then \
		echo "❌ APPLE_SIGNING_IDENTITY environment variable not set"; \
		exit 1; \
	fi
	@echo "Attempting manual signing with identity: $APPLE_SIGNING_IDENTITY"
	@# Try signing without hardened runtime first
	codesign --force --sign "$APPLE_SIGNING_IDENTITY" \
		"src-tauri/target/release/bundle/macos/Gestura.app" || \
	echo "⚠️ Basic signing failed, trying with entitlements..." && \
	codesign --force --options runtime \
		--entitlements src-tauri/entitlements.plist \
		--sign "$APPLE_SIGNING_IDENTITY" \
		"src-tauri/target/release/bundle/macos/Gestura.app"
	@echo "✅ Manual code signing complete"

# Build unsigned then sign manually (often more reliable)
build-and-sign-macos: build-macos sign-macos
	@echo "🎉 Build and manual signing complete!"

# Notarize macOS app
notarize-macos: sign-macos
	@echo "🔐📋 Notarizing macOS app..."
	@echo "⚠️ Notarization requires Apple Developer account and app-specific password"
	@echo "See: https://developer.apple.com/documentation/security/notarizing_macos_software_before_distribution"
	./scripts/notarize-mac.sh

# Verify macOS app signature
verify-macos:
	@echo "🔍🍎 Verifying macOS app signature..."
	@if [ ! -d "src-tauri/target/release/bundle/macos/Gestura.app" ]; then \
		echo "❌ App bundle not found. Run 'just build-macos' first"; \
		exit 1; \
	fi
	codesign --verify --deep --strict --verbose=2 "src-tauri/target/release/bundle/macos/Gestura.app"
	codesign -dv --verbose=4 "src-tauri/target/release/bundle/macos/Gestura.app"
	spctl --assess --type open --context context:primary-signature --verbose=2 "src-tauri/target/release/bundle/macos/Gestura.app"

# Check Windows signing setup
check-windows-signing:
	@echo "🔍🪟 Checking Windows code signing setup..."
	@echo "Environment variables:"
	@if [ -n "$$WINDOWS_SIGNING_CERT" ]; then echo "WINDOWS_SIGNING_CERT: ✅ $$WINDOWS_SIGNING_CERT"; else echo "WINDOWS_SIGNING_CERT: ❌ Not set"; fi
	@if [ -n "$$WINDOWS_SIGNING_PASSWORD" ]; then echo "WINDOWS_SIGNING_PASSWORD: ✅ Set"; else echo "WINDOWS_SIGNING_PASSWORD: ❌ Not set"; fi
	@echo ""
	@echo "Note: Windows signing requires signtool.exe (Windows SDK)"

# Sign Windows executable
sign-windows:
	@echo "🔐🪟 Code signing Windows executable..."
	@if [ ! -f "src-tauri/target/x86_64-pc-windows-msvc/release/gestura.exe" ]; then \
		echo "❌ Windows executable not found. Run 'just build-windows' first"; \
		exit 1; \
	fi
	@if [ -z "$$WINDOWS_SIGNING_CERT" ]; then \
		echo "❌ WINDOWS_SIGNING_CERT environment variable not set"; \
		exit 1; \
	fi
	@echo "⚠️ Windows signing requires running on Windows with signtool.exe"
	@echo "Certificate: $$WINDOWS_SIGNING_CERT"

# Verify Windows executable signature
verify-windows:
	@echo "🔍🪟 Verifying Windows executable signature..."
	@if [ ! -f "src-tauri/target/x86_64-pc-windows-msvc/release/gestura.exe" ]; then \
		echo "❌ Windows executable not found. Run 'just build-windows' first"; \
		exit 1; \
	fi
	@echo "⚠️ Windows signature verification requires running on Windows"
	@echo "Use: Get-AuthenticodeSignature -FilePath gestura.exe"

# Check Linux signing setup (for AppImage)
check-linux-signing:
	@echo "🔍🐧 Checking Linux signing setup..."
	@echo "Linux packages typically don't require code signing"
	@echo "For AppImage signing, you would need:"
	@echo "  - GPG key for signing"
	@echo "  - appimagetool with --sign option"
	@if command -v gpg >/dev/null 2>&1; then \
		echo "GPG: ✅ available"; \
		gpg --list-secret-keys --keyid-format LONG | head -5; \
	else \
		echo "GPG: ❌ not found"; \
	fi

# Distribution
upload-releases:
	@echo "☁️ Uploading releases..."
	./scripts/upload-releases.sh

# Automated UI testing
test-ui-automated:
	node scripts/automated-ui-test.js

test-ui-full:
	node scripts/automated-ui-test.js

# Test tray functionality
test-tray:
	@echo "🧪📱 Testing system tray functionality..."
	node scripts/test-tray.js

# Test macOS app bundle
test-macos-app:
	@echo "🧪🍎 Testing macOS app bundle..."
	@if [ ! -d "src-tauri/target/release/bundle/macos/Gestura.app" ]; then \
		echo "❌ App bundle not found. Run 'just build-macos' first"; \
		exit 1; \
	fi
	@echo "📋 App bundle info:"
	@ls -la "src-tauri/target/release/bundle/macos/Gestura.app/Contents/"
	@echo ""
	@echo "📄 Info.plist:"
	@plutil -p "src-tauri/target/release/bundle/macos/Gestura.app/Contents/Info.plist"
	@echo ""
	@echo "🔐 Signature status:"
	@codesign -dv "src-tauri/target/release/bundle/macos/Gestura.app" 2>&1 || echo "❌ App is not signed"
	@echo ""
	@echo "🚀 Launching app for testing..."
	@open "src-tauri/target/release/bundle/macos/Gestura.app"
	@echo "✅ App launched. Check system tray for Gestura icon."

# Test Windows executable
test-windows-app:
	@echo "🧪🪟 Testing Windows executable..."
	@if [ ! -f "src-tauri/target/x86_64-pc-windows-msvc/release/gestura.exe" ]; then \
		echo "❌ Windows executable not found. Run 'just build-windows' first"; \
		exit 1; \
	fi
	@echo "📋 Windows executable info:"
	@ls -la "src-tauri/target/x86_64-pc-windows-msvc/release/gestura.exe"
	@echo ""
	@echo "📦 Available installers:"
	@if [ -d "src-tauri/target/x86_64-pc-windows-msvc/release/bundle/msi" ]; then \
		echo "✅ MSI installer:"; \
		ls -la "src-tauri/target/x86_64-pc-windows-msvc/release/bundle/msi/"; \
	else \
		echo "❌ No MSI installer found"; \
	fi
	@echo ""
	@echo "⚠️ To test on Windows, copy the executable to a Windows machine"

# Test Linux packages
test-linux-app:
	@echo "🧪🐧 Testing Linux packages..."
	@if [ ! -f "src-tauri/target/x86_64-unknown-linux-gnu/release/gestura" ]; then \
		echo "❌ Linux binary not found. Run 'just build-linux' first"; \
		exit 1; \
	fi
	@echo "📋 Linux binary info:"
	@ls -la "src-tauri/target/x86_64-unknown-linux-gnu/release/gestura"
	@file "src-tauri/target/x86_64-unknown-linux-gnu/release/gestura"
	@echo ""
	@echo "📦 Available packages:"
	@if [ -f "src-tauri/target/x86_64-unknown-linux-gnu/release/bundle/deb/gestura_0.1.0_amd64.deb" ]; then \
		echo "✅ DEB package:"; \
		ls -la "src-tauri/target/x86_64-unknown-linux-gnu/release/bundle/deb/"; \
	else \
		echo "❌ No DEB package found"; \
	fi
	@if [ -f "src-tauri/target/x86_64-unknown-linux-gnu/release/bundle/appimage/gestura_0.1.0_amd64.AppImage" ]; then \
		echo "✅ AppImage:"; \
		ls -la "src-tauri/target/x86_64-unknown-linux-gnu/release/bundle/appimage/"; \
	else \
		echo "❌ No AppImage found"; \
	fi
	@echo ""
	@echo "🚀 Testing binary (if on Linux)..."
	@if [ "$$(uname)" = "Linux" ]; then \
		echo "Running quick test..."; \
		"src-tauri/target/x86_64-unknown-linux-gnu/release/gestura" --version || echo "Binary test failed"; \
	else \
		echo "⚠️ Not on Linux - cannot test binary directly"; \
	fi

# Create DMG for distribution
create-dmg:
	@echo "💿 Creating DMG for distribution..."
	@if [ ! -d "src-tauri/target/release/bundle/macos/Gestura.app" ]; then \
		echo "❌ App bundle not found. Run 'just build-macos' first"; \
		exit 1; \
	fi
	@echo "Creating Gestura-0.1.0-macos.dmg..."
	@rm -f "Gestura-0.1.0-macos.dmg"
	@hdiutil create -volname "Gestura" \
		-srcfolder "src-tauri/target/release/bundle/macos/Gestura.app" \
		-ov -format UDZO "Gestura-0.1.0-macos.dmg"
	@echo "✅ DMG created: Gestura-0.1.0-macos.dmg"
	@ls -lh "Gestura-0.1.0-macos.dmg"

# Utility Commands
# ================

# Generate app icons from source
icons:
	@echo "🎨 Generating app icons..."
	@if [ ! -f "assets/icon.png" ]; then \
		echo "❌ Source icon not found at assets/icon.png"; \
		exit 1; \
	fi
	./scripts/generate-icons.sh
	@echo "✅ Icons generated"

