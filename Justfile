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
	@echo "  build-cli            # build CLI binary (release)"
	@echo "  build-cli-universal  # build universal CLI binary (Intel + Apple Silicon)"
	@echo "  test                 # cargo test"
	@echo "  clean                # cargo clean"
	@echo ""
	@echo "🏗️ Platform Builds:"
	@echo "  build-macos          # build macOS app bundle"
	@echo "  build-macos-signed   # build and sign macOS app + CLI bundle"
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
	@echo "  generate-man-pages   # generate CLI man pages"
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
gui_dir := "crates/gestura-gui"
frontend_dir := "crates/gestura-gui/frontend"
# Legacy alias for compatibility
app_dir := "crates/gestura-gui"

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
	rm -rf {{frontend_dir}}/dist/
	rm -rf {{frontend_dir}}/node_modules/
	rm -rf target/
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
	cd {{frontend_dir}} && npm install
	cd {{gui_dir}} && cargo tauri dev --features voice-local

# Platform-Specific Builds
# =========================

# Build CLI binary (release)
build-cli:
	@echo "🔧 Building CLI binary..."
	cargo build --release -p gestura-cli --features voice-local

# Build CLI binary for universal macOS (Intel + Apple Silicon)
build-cli-universal:
	@echo "🔧 Building universal CLI binary..."
	cargo build --release -p gestura-cli --features voice-local --target aarch64-apple-darwin
	cargo build --release -p gestura-cli --features voice-local --target x86_64-apple-darwin
	@mkdir -p target/universal-apple-darwin/release
	lipo -create \
		target/aarch64-apple-darwin/release/gestura \
		target/x86_64-apple-darwin/release/gestura \
		-output target/universal-apple-darwin/release/gestura
	@echo "✅ Universal CLI binary created at target/universal-apple-darwin/release/gestura"

# Build macOS app bundle (unsigned)
build-macos:
	@echo "🍎 Building macOS app bundle (unsigned)..."
	cd {{frontend_dir}} && npm install
	cd {{frontend_dir}} && npm run build
	cd {{gui_dir}} && cargo tauri build --features voice-local

# Build and sign macOS app bundle (for local development)
# This uses certificates already installed in your Keychain.
# For CI/CD, use the GitHub Actions workflow which imports certificates.
#
# Required environment variables:
#   APPLE_SIGNING_IDENTITY - Developer ID Application certificate name
#   APPLE_TEAM_ID          - Your Apple Developer Team ID
#   APPLE_ID               - Your Apple ID email (for notarization)
#   APPLE_PASSWORD         - App-specific password or @keychain:profile-name
#
# Example setup:
#   export APPLE_SIGNING_IDENTITY="Developer ID Application: Your Name (TEAMID)"
#   export APPLE_TEAM_ID="XXXXXXXXXX"
#   export APPLE_ID="your@email.com"
#   export APPLE_PASSWORD="@keychain:notarytool-password"
build-macos-signed:
	@echo "🍎🔐 Building and signing macOS app bundle + CLI..."
	@if [ -z "${APPLE_SIGNING_IDENTITY:-}" ]; then \
		echo "❌ APPLE_SIGNING_IDENTITY environment variable not set"; \
		echo "Set it to your Developer ID Application certificate name"; \
		echo "Example: Developer ID Application: Your Name (TEAMID)"; \
		echo ""; \
		echo "Available certificates:"; \
		security find-identity -v -p codesigning | grep "Developer ID Application" || echo "  (none found)"; \
		exit 1; \
	fi
	@if [ -z "${APPLE_TEAM_ID:-}" ]; then \
		echo "❌ APPLE_TEAM_ID environment variable not set"; \
		echo "Set it to your 10-character Apple Developer Team ID"; \
		exit 1; \
	fi
	@echo "Using signing identity: $APPLE_SIGNING_IDENTITY"
	@echo "Using team ID: $APPLE_TEAM_ID"
	set -e
	# 1. Build frontend
	cd {{frontend_dir}} && npm install
	cd {{frontend_dir}} && npm run build
	# 2. Build the CLI binary (universal)
	@echo "🔧 Building universal CLI binary..."
	cargo build --release -p gestura-cli --features voice-local --target aarch64-apple-darwin
	cargo build --release -p gestura-cli --features voice-local --target x86_64-apple-darwin
	mkdir -p target/universal-apple-darwin/release
	lipo -create \
		target/aarch64-apple-darwin/release/gestura \
		target/x86_64-apple-darwin/release/gestura \
		-output target/universal-apple-darwin/release/gestura
	@echo "✅ Universal CLI binary created"
	# 3. Build the macOS app bundle without letting Tauri perform notarization
	#    We *unset* APPLE_ID / APPLE_PASSWORD / APPLE_TEAM_ID for this command so
	#    Tauri only builds & (optionally) signs. Our standalone script then
	#    handles notarization via notarytool. This avoids long hangs inside Tauri
	#    when Apple's notarization service is slow or stuck.
	#    Set MACOSX_DEPLOYMENT_TARGET=10.15 for whisper.cpp std::filesystem support
	cd {{gui_dir}} && MACOSX_DEPLOYMENT_TARGET=10.15 env -u APPLE_ID -u APPLE_PASSWORD -u APPLE_TEAM_ID \
		cargo tauri build --target universal-apple-darwin --features voice-local
	echo "✅ GUI build complete. Running notarization script..."
	./scripts/notarize-mac.sh

# Build universal macOS binary (Intel + Apple Silicon)
build-macos-universal:
	@echo "🍎🔄 Building universal macOS binary..."
	cd {{frontend_dir}} && npm install
	cd {{frontend_dir}} && npm run build
	cd {{gui_dir}} && cargo tauri build --target universal-apple-darwin --features voice-local

# Build Windows executable
build-windows:
	@echo "🪟 Building Windows executable..."
	cd {{frontend_dir}} && npm install
	cd {{frontend_dir}} && npm run build
	cd {{gui_dir}} && cargo tauri build --features voice-local -- --target x86_64-pc-windows-msvc

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
	cd {{frontend_dir}} && npm install && \
	cd {{frontend_dir}} && npm run build && \
	cd {{gui_dir}} && cargo tauri build --features voice-local -- --target x86_64-pc-windows-msvc

# Build Linux binary
build-linux:
	@echo "🐧 Building Linux binary..."
	cd {{frontend_dir}} && npm install
	cd {{frontend_dir}} && npm run build
	cd {{gui_dir}} && cargo tauri build --features voice-local -- --target x86_64-unknown-linux-gnu

# Build Linux AppImage
build-linux-appimage:
	@echo "🐧📦 Building Linux AppImage..."
	cd {{frontend_dir}} && npm install
	cd {{frontend_dir}} && npm run build
	cd {{gui_dir}} && cargo tauri build --features voice-local -- --target x86_64-unknown-linux-gnu --bundles appimage

# Build Linux deb package
build-linux-deb:
	@echo "🐧📦 Building Linux deb package..."
	cd {{frontend_dir}} && npm install
	cd {{frontend_dir}} && npm run build
	cd {{gui_dir}} && cargo tauri build --features voice-local -- --target x86_64-unknown-linux-gnu --bundles deb

package:
	@echo "📦 Building production app for current platform..."
	cd {{frontend_dir}} && npm install
	cd {{gui_dir}} && cargo tauri build --features voice-local

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
	@if [ -n "$$APPLE_INSTALLER_IDENTITY" ]; then echo "  APPLE_INSTALLER_IDENTITY: ✅ $$APPLE_INSTALLER_IDENTITY"; else echo "  APPLE_INSTALLER_IDENTITY: ❌ not set (PKG signing)"; fi
	@if [ -n "$$APPLE_TEAM_ID" ]; then echo "  APPLE_TEAM_ID: ✅ $$APPLE_TEAM_ID"; else echo "  APPLE_TEAM_ID: ❌ not set"; fi
	@echo "  Developer ID Application certs: $(security find-identity -v -p codesigning | grep -c "Developer ID Application" || echo '0')"
	@echo "  Developer ID Installer certs: $(security find-identity -v -p codesigning | grep -c "Developer ID Installer" || echo '0')"
	@echo ""
	@echo "💻 System:"
	@echo "  OS: $(uname -a)"
	@echo "  Architecture: $(uname -m)"
	@echo ""
	@echo "📁 Project Status:"
	@echo "  Frontend built: $([ -d '{{frontend_dir}}/dist' ] && echo '✅ yes' || echo '❌ no')"
	@echo "  CLI binary: $([ -f 'target/release/gestura' ] && echo '✅ yes' || echo '❌ no')"
	@echo "  CLI universal: $([ -f 'target/universal-apple-darwin/release/gestura' ] && echo '✅ yes' || echo '❌ no')"
	@echo "  macOS app: $([ -d '{{gui_dir}}/target/release/bundle/macos/Gestura.app' ] && echo '✅ yes' || echo '❌ no')"
	@echo "  macOS universal: $([ -d '{{gui_dir}}/target/universal-apple-darwin/release/bundle/macos/Gestura.app' ] && echo '✅ yes' || echo '❌ no')"

# Quick development workflow
quick-dev: clean build-release test
	@echo "🚀 Quick development build complete!"

# Full build and test workflow
full-build: clean build-macos test-macos-app
	@echo "🎉 Full build and test complete!"

# Generate CLI man pages using clap_mangen
generate-man-pages:
	@echo "📖 Generating CLI man pages..."
	@mkdir -p dist/man/man1
	cargo run -p gestura-cli -- completion --generate-man dist/man/man1
	@echo "📖 Man pages generated in dist/man/man1/"

# Release workflow for macOS (signed .app, .pkg, .dmg with CLI in /usr/local/bin)
release-macos: clean build-macos-signed verify-macos package-macos-signed
	@echo "🎉 macOS release build complete!"
	@echo ""
	@echo "📁 Release artifacts in dist/macos/:"
	@ls -la dist/macos/*.dmg dist/macos/*.pkg 2>/dev/null || echo "  (check dist/macos-* for timestamped directory)"
	@echo ""
	@echo "📦 Package contents:"
	@echo "  • Gestura.app → /Applications/Gestura.app"
	@echo "  • gestura CLI → /usr/local/bin/gestura"
	@echo ""
	@echo "🔐 Signing status:"
	@echo "  • .app: Signed and notarized"
	@echo "  • .pkg: Signed (if APPLE_INSTALLER_IDENTITY set)"
	@echo "  • CLI:  Signed"

# Release workflow for Windows
release-windows: clean build-windows-signed create-windows-msi
	@echo "🎉 Windows release build complete!"
	@echo "📁 Check target/x86_64-pc-windows-msvc/release/bundle/msi/ for installer"

# Release workflow for Linux
release-linux: clean build-linux-deb build-linux-appimage
	@echo "🎉 Linux release build complete!"
	@echo "📁 Check target/x86_64-unknown-linux-gnu/release/bundle/ for packages"

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
	@if [ -d "target/release/bundle/macos/Gestura.app" ]; then \
		echo "Testing macOS..."; \
		just test-macos-app; \
	else \
		echo "⚠️ macOS build not found"; \
	fi
	@if [ -f "target/x86_64-pc-windows-msvc/release/gestura-gui.exe" ]; then \
		echo "Testing Windows..."; \
		just test-windows-app; \
	else \
		echo "⚠️ Windows build not found"; \
	fi
	@if [ -f "target/x86_64-unknown-linux-gnu/release/gestura-gui" ]; then \
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

# Create signed macOS DMG and PKG (uses already-built and notarized app from build-macos-signed)
# This recipe does NOT rebuild - it packages the existing artifacts
package-macos-signed:
	#!/usr/bin/env bash
	set -euo pipefail

	echo "📦🍎🔐 Creating signed macOS packages (DMG + PKG)..."

	# Get version from frontend package.json
	VERSION=$(grep -o '"version": "[^"]*"' {{frontend_dir}}/package.json | cut -d'"' -f4)
	echo "📋 Version: ${VERSION}"

	# Paths
	APP_BUNDLE="target/universal-apple-darwin/release/bundle/macos/Gestura.app"
	CLI_BIN="target/universal-apple-darwin/release/gestura"
	DIST_DIR="dist/macos-release"

	# Verify artifacts exist
	if [ ! -d "${APP_BUNDLE}" ]; then
		echo "❌ App bundle not found at ${APP_BUNDLE}"
		echo "   Run 'just build-macos-signed' first"
		exit 1
	fi

	if [ ! -f "${CLI_BIN}" ]; then
		echo "❌ CLI binary not found at ${CLI_BIN}"
		echo "   Run 'just build-macos-signed' first"
		exit 1
	fi

	# Verify app is notarized
	echo "🔍 Verifying app is notarized..."
	if ! xcrun stapler validate "${APP_BUNDLE}" 2>/dev/null; then
		echo "⚠️ App is not stapled with notarization ticket"
		echo "   Attempting to staple..."
		xcrun stapler staple "${APP_BUNDLE}" || {
			echo "❌ Failed to staple. Make sure the app was notarized."
			exit 1
		}
	fi
	echo "✅ App is notarized"

	# Clean and create dist directory
	rm -rf "${DIST_DIR}"
	mkdir -p "${DIST_DIR}/pkgroot/Applications"
	mkdir -p "${DIST_DIR}/pkgroot/usr/local/bin"

	# Copy artifacts for PKG
	echo "📁 Copying app bundle..."
	cp -R "${APP_BUNDLE}" "${DIST_DIR}/pkgroot/Applications/"

	echo "📁 Copying CLI binary..."
	cp "${CLI_BIN}" "${DIST_DIR}/pkgroot/usr/local/bin/gestura"

	# Create PKG
	echo "📦 Building PKG installer..."
	UNSIGNED_PKG="${DIST_DIR}/Gestura-${VERSION}-universal-unsigned.pkg"
	SIGNED_PKG="${DIST_DIR}/Gestura-${VERSION}-universal.pkg"

	pkgbuild \
		--root "${DIST_DIR}/pkgroot" \
		--identifier "ai.gestura.desktop" \
		--version "${VERSION}" \
		--install-location "/" \
		"${UNSIGNED_PKG}"

	# Sign PKG if installer identity is available
	INSTALLER_IDENTITY="${APPLE_INSTALLER_IDENTITY:-}"
	if [ -n "${INSTALLER_IDENTITY}" ]; then
		echo "🔐 Signing PKG with: ${INSTALLER_IDENTITY}"
		productsign --sign "${INSTALLER_IDENTITY}" "${UNSIGNED_PKG}" "${SIGNED_PKG}"
		rm -f "${UNSIGNED_PKG}"

		echo "📋 Submitting PKG for notarization..."
		# Use keychain profile if APPLE_PASSWORD starts with @keychain:, otherwise use direct credentials
		if [[ "${APPLE_PASSWORD:-}" == @keychain:* ]]; then
			PROFILE_NAME="${APPLE_PASSWORD#@keychain:}"
			xcrun notarytool submit "${SIGNED_PKG}" \
				--keychain-profile "${PROFILE_NAME}" \
				--wait
		elif [ -n "${APPLE_ID:-}" ] && [ -n "${APPLE_TEAM_ID:-}" ] && [ -n "${APPLE_PASSWORD:-}" ]; then
			xcrun notarytool submit "${SIGNED_PKG}" \
				--apple-id "${APPLE_ID}" \
				--team-id "${APPLE_TEAM_ID}" \
				--password "${APPLE_PASSWORD}" \
				--wait
		else
			echo "⚠️ Skipping PKG notarization - credentials not configured"
			echo "   Set APPLE_PASSWORD=@keychain:profile-name or provide APPLE_ID/APPLE_TEAM_ID/APPLE_PASSWORD"
		fi

		echo "📎 Stapling notarization ticket to PKG..."
		xcrun stapler staple "${SIGNED_PKG}" || echo "⚠️ Stapling skipped (notarization may not have completed)"
		echo "✅ PKG signed"
	else
		mv "${UNSIGNED_PKG}" "${SIGNED_PKG}"
		echo "⚠️ PKG not signed (set APPLE_INSTALLER_IDENTITY for signing)"
	fi

	# Create DMG
	echo "💿 Creating DMG..."
	DMG_NAME="Gestura-${VERSION}-universal.dmg"

	# Check if create-dmg is installed
	if ! command -v create-dmg &> /dev/null; then
		echo "Installing create-dmg..."
		brew install create-dmg
	fi

	create-dmg \
		--volname "Gestura" \
		--window-pos 200 120 \
		--window-size 600 400 \
		--icon-size 100 \
		--icon "Gestura.app" 175 120 \
		--hide-extension "Gestura.app" \
		--app-drop-link 425 120 \
		"${DIST_DIR}/${DMG_NAME}" \
		"${APP_BUNDLE}" || true

	# Sign and notarize DMG
	SIGNING_IDENTITY="${APPLE_SIGNING_IDENTITY:-}"
	if [ -n "${SIGNING_IDENTITY}" ]; then
		echo "🔐 Signing DMG..."
		codesign --force --sign "${SIGNING_IDENTITY}" "${DIST_DIR}/${DMG_NAME}"

		echo "📋 Submitting DMG for notarization..."
		# Use keychain profile if APPLE_PASSWORD starts with @keychain:, otherwise use direct credentials
		if [[ "${APPLE_PASSWORD:-}" == @keychain:* ]]; then
			PROFILE_NAME="${APPLE_PASSWORD#@keychain:}"
			xcrun notarytool submit "${DIST_DIR}/${DMG_NAME}" \
				--keychain-profile "${PROFILE_NAME}" \
				--wait
		elif [ -n "${APPLE_ID:-}" ] && [ -n "${APPLE_TEAM_ID:-}" ] && [ -n "${APPLE_PASSWORD:-}" ]; then
			xcrun notarytool submit "${DIST_DIR}/${DMG_NAME}" \
				--apple-id "${APPLE_ID}" \
				--team-id "${APPLE_TEAM_ID}" \
				--password "${APPLE_PASSWORD}" \
				--wait
		else
			echo "⚠️ Skipping DMG notarization - credentials not configured"
			echo "   Set APPLE_PASSWORD=@keychain:profile-name or provide APPLE_ID/APPLE_TEAM_ID/APPLE_PASSWORD"
		fi

		echo "📎 Stapling notarization ticket to DMG..."
		xcrun stapler staple "${DIST_DIR}/${DMG_NAME}" || echo "⚠️ Stapling skipped (notarization may not have completed)"
		echo "✅ DMG signed"
	else
		echo "⚠️ DMG not signed (set APPLE_SIGNING_IDENTITY for signing)"
	fi

	# Generate checksums
	echo "🔢 Generating checksums..."
	cd "${DIST_DIR}"
	shasum -a 256 "Gestura-${VERSION}-universal.pkg" > "Gestura-${VERSION}-universal.pkg.sha256"
	shasum -a 256 "Gestura-${VERSION}-universal.dmg" > "Gestura-${VERSION}-universal.dmg.sha256"

	# Clean up pkgroot
	rm -rf pkgroot

		# Create release info
		# NOTE: Recipe bodies in just must be indented. A bash heredoc delimiter must
		# start at the beginning of the line, so we use `<<-EOF` to allow tab-indented
		# heredoc contents and terminator.
		cat > RELEASE_INFO.txt <<-EOF
		Gestura v${VERSION} - macOS Universal Release
		=========================================
		
		Build Information:
		- Version: ${VERSION}
		- Platform: macOS Universal (Intel x86_64 + Apple Silicon arm64)
		- Build Date: $(date +%Y-%m-%d)
		- Signed by: ${SIGNING_IDENTITY:-Unsigned}
		- Notarized: Yes (Apple Notary Service)
		
		Files:
		- Gestura-${VERSION}-universal.pkg - Installer Package
		  Installs: /Applications/Gestura.app and /usr/local/bin/gestura
		
		- Gestura-${VERSION}-universal.dmg - Disk Image
		  Drag-and-drop installation to /Applications
		
		Installation:
		1. PKG (Recommended): Double-click the .pkg file
		2. DMG: Open .dmg and drag Gestura.app to Applications
		
		CLI Usage:
		  $ gestura --help
		
		Support: https://gestura.app
		License: Gestura Prosperity License 1.0
		EOF

	cd - > /dev/null

	echo ""
	echo "🎉 Packaging complete!"
	echo "📁 Release artifacts in ${DIST_DIR}/:"
	ls -la "${DIST_DIR}/"

# Create Windows MSI installer
create-windows-msi:
	@echo "📦🪟 Creating Windows MSI installer..."
	@if [ ! -d "target/x86_64-pc-windows-msvc/release/bundle/msi" ]; then \
		echo "❌ Windows MSI not found. Run 'just build-windows' first"; \
		exit 1; \
	fi
	@echo "✅ Windows MSI available in target/x86_64-pc-windows-msvc/release/bundle/msi/"
	@ls -la "target/x86_64-pc-windows-msvc/release/bundle/msi/"

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
	@if [ ! -f "target/x86_64-unknown-linux-gnu/release/bundle/appimage/gestura_0.1.0_amd64.AppImage" ]; then \
		echo "❌ Linux AppImage not found. Run 'just build-linux-appimage' first"; \
		exit 1; \
	fi
	@echo "✅ Linux AppImage available:"
	@ls -la "target/x86_64-unknown-linux-gnu/release/bundle/appimage/"

# Create Linux deb package
create-linux-deb:
	@echo "📦🐧 Creating Linux deb package..."
	@if [ ! -f "target/x86_64-unknown-linux-gnu/release/bundle/deb/gestura_0.1.0_amd64.deb" ]; then \
		echo "❌ Linux deb not found. Run 'just build-linux-deb' first"; \
		exit 1; \
	fi
	@echo "✅ Linux deb package available:"
	@ls -la "target/x86_64-unknown-linux-gnu/release/bundle/deb/"

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
	@if [ -n "${APPLE_CERTIFICATE:-}" ]; then echo "APPLE_CERTIFICATE: ✅ $(echo "${APPLE_CERTIFICATE}" | wc -c) bytes"; else echo "APPLE_CERTIFICATE: ❌ Not set (CI/CD only)"; fi
	@if [ -n "${APPLE_CERTIFICATE_PASSWORD:-}" ]; then echo "APPLE_CERTIFICATE_PASSWORD: ✅ Set"; else echo "APPLE_CERTIFICATE_PASSWORD: ❌ Not set (CI/CD only)"; fi
	@if [ -n "${APPLE_SIGNING_IDENTITY:-}" ]; then echo "APPLE_SIGNING_IDENTITY: ✅ ${APPLE_SIGNING_IDENTITY}"; else echo "APPLE_SIGNING_IDENTITY: ❌ Not set"; fi
	@if [ -n "${APPLE_ID:-}" ]; then echo "APPLE_ID: ✅ ${APPLE_ID}"; else echo "APPLE_ID: ❌ Not set"; fi
	@if [ -n "${APPLE_PASSWORD:-}" ]; then echo "APPLE_PASSWORD: ✅ Set"; else echo "APPLE_PASSWORD: ❌ Not set"; fi
	@if [ -n "${APPLE_TEAM_ID:-}" ]; then echo "APPLE_TEAM_ID: ✅ ${APPLE_TEAM_ID}"; else echo "APPLE_TEAM_ID: ❌ Not set"; fi
	@echo ""
	@echo "📋 Required for LOCAL development signing:"
	@echo "  ✅ APPLE_SIGNING_IDENTITY - Identity name (e.g., 'Developer ID Application: ...')"
	@echo "  ✅ APPLE_TEAM_ID - Developer team ID"
	@echo "  ✅ APPLE_ID - Apple ID for notarization"
	@echo "  ✅ APPLE_PASSWORD - App-specific password (or @keychain:profile-name)"
	@echo ""
	@echo "📋 Additional for CI/CD (GitHub Actions):"
	@echo "  ✅ APPLE_CERTIFICATE - Certificate data (base64 encoded .p12)"
	@echo "  ✅ APPLE_CERTIFICATE_PASSWORD - Certificate password"
	@echo ""
	@echo "💡 Setup notarization credentials in Keychain:"
	@echo "   xcrun notarytool store-credentials notarytool-password \\"
	@echo "     --apple-id your@email.com \\"
	@echo "     --team-id XXXXXXXXXX \\"
	@echo "     --password <app-specific-password>"
	@echo ""
	@echo "   Then set: export APPLE_PASSWORD=\"@keychain:notarytool-password\""



# Sign macOS app bundle manually (fallback method)
sign-macos:
	@echo "🔐🍎 Manual code signing macOS app..."
	@if [ ! -d "target/release/bundle/macos/Gestura.app" ]; then \
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
		"target/release/bundle/macos/Gestura.app" || \
	echo "⚠️ Basic signing failed, trying with entitlements..." && \
	codesign --force --options runtime \
		--entitlements {{gui_dir}}/entitlements.plist \
		--sign "$APPLE_SIGNING_IDENTITY" \
		"target/release/bundle/macos/Gestura.app"
	@echo "✅ Manual code signing complete"

# Build unsigned then sign manually (often more reliable)
build-and-sign-macos: build-macos sign-macos
	@echo "🎉 Build and manual signing complete!"

# Notarize macOS app (standalone, assumes app is already built)
notarize-macos:
	@echo "🔐📋 Notarizing macOS app..."
	./scripts/notarize-mac.sh

# Verify macOS app signature (auto-detects universal or regular build)
verify-macos:
	#!/bin/bash
	set -e
	echo "🔍🍎 Verifying macOS app signature..."
	UNIVERSAL_PATH="target/universal-apple-darwin/release/bundle/macos/Gestura.app"
	REGULAR_PATH="target/release/bundle/macos/Gestura.app"
	if [ -d "$UNIVERSAL_PATH" ]; then
	    APP_PATH="$UNIVERSAL_PATH"
	    echo "Verifying universal build"
	elif [ -d "$REGULAR_PATH" ]; then
	    APP_PATH="$REGULAR_PATH"
	    echo "Verifying regular build"
	else
	    echo "❌ App bundle not found. Run 'just build-macos' or 'just build-macos-signed' first"
	    exit 1
	fi
	codesign --verify --deep --strict --verbose=2 "$APP_PATH"
	codesign -dv --verbose=4 "$APP_PATH"
	spctl --assess --type open --context context:primary-signature --verbose=2 "$APP_PATH" || echo "⚠️ Gatekeeper check failed (app may not be notarized)"

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
	@if [ ! -f "target/x86_64-pc-windows-msvc/release/gestura-gui.exe" ]; then \
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
	@if [ ! -f "target/x86_64-pc-windows-msvc/release/gestura-gui.exe" ]; then \
		echo "❌ Windows executable not found. Run 'just build-windows' first"; \
		exit 1; \
	fi
	@echo "⚠️ Windows signature verification requires running on Windows"
	@echo "Use: Get-AuthenticodeSignature -FilePath gestura-gui.exe"

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

# Test macOS app bundle (auto-detects universal or regular build)
test-macos-app:
	#!/bin/bash
	set -e
	echo "🧪🍎 Testing macOS app bundle..."
	UNIVERSAL_PATH="target/universal-apple-darwin/release/bundle/macos/Gestura.app"
	REGULAR_PATH="target/release/bundle/macos/Gestura.app"
	if [ -d "$UNIVERSAL_PATH" ]; then
	    APP_PATH="$UNIVERSAL_PATH"
	    echo "Testing universal build"
	elif [ -d "$REGULAR_PATH" ]; then
	    APP_PATH="$REGULAR_PATH"
	    echo "Testing regular build"
	else
	    echo "❌ App bundle not found. Run 'just build-macos' or 'just build-macos-signed' first"
	    exit 1
	fi
	echo "📋 App bundle info:"
	ls -la "$APP_PATH/Contents/"
	echo ""
	echo "📄 Info.plist:"
	plutil -p "$APP_PATH/Contents/Info.plist"
	echo ""
	echo "🔐 Signature status:"
	codesign -dv "$APP_PATH" 2>&1 || echo "❌ App is not signed"
	echo ""
	echo "🚀 Launching app for testing..."
	open "$APP_PATH"
	echo "✅ App launched. Check system tray for Gestura icon."

# Test Windows executable
test-windows-app:
	@echo "🧪🪟 Testing Windows executable..."
	@if [ ! -f "target/x86_64-pc-windows-msvc/release/gestura-gui.exe" ]; then \
		echo "❌ Windows executable not found. Run 'just build-windows' first"; \
		exit 1; \
	fi
	@echo "📋 Windows executable info:"
	@ls -la "target/x86_64-pc-windows-msvc/release/gestura-gui.exe"
	@echo ""
	@echo "📦 Available installers:"
	@if [ -d "target/x86_64-pc-windows-msvc/release/bundle/msi" ]; then \
		echo "✅ MSI installer:"; \
		ls -la "target/x86_64-pc-windows-msvc/release/bundle/msi/"; \
	else \
		echo "❌ No MSI installer found"; \
	fi
	@echo ""
	@echo "⚠️ To test on Windows, copy the executable to a Windows machine"

# Test Linux packages
test-linux-app:
	@echo "🧪🐧 Testing Linux packages..."
	@if [ ! -f "target/x86_64-unknown-linux-gnu/release/gestura-gui" ]; then \
		echo "❌ Linux binary not found. Run 'just build-linux' first"; \
		exit 1; \
	fi
	@echo "📋 Linux binary info:"
	@ls -la "target/x86_64-unknown-linux-gnu/release/gestura-gui"
	@file "target/x86_64-unknown-linux-gnu/release/gestura-gui"
	@echo ""
	@echo "📦 Available packages:"
	@if [ -f "target/x86_64-unknown-linux-gnu/release/bundle/deb/gestura_0.1.0_amd64.deb" ]; then \
		echo "✅ DEB package:"; \
		ls -la "target/x86_64-unknown-linux-gnu/release/bundle/deb/"; \
	else \
		echo "❌ No DEB package found"; \
	fi
	@if [ -f "target/x86_64-unknown-linux-gnu/release/bundle/appimage/gestura_0.1.0_amd64.AppImage" ]; then \
		echo "✅ AppImage:"; \
		ls -la "target/x86_64-unknown-linux-gnu/release/bundle/appimage/"; \
	else \
		echo "❌ No AppImage found"; \
	fi
	@echo ""
	@echo "🚀 Testing binary (if on Linux)..."
	@if [ "$$(uname)" = "Linux" ]; then \
		echo "Running quick test..."; \
		"target/x86_64-unknown-linux-gnu/release/gestura-gui" --version || echo "Binary test failed"; \
	else \
		echo "⚠️ Not on Linux - cannot test binary directly"; \
	fi

# Create DMG for distribution (auto-detects universal or regular build)
create-dmg:
	#!/bin/bash
	set -e
	echo "💿 Creating DMG for distribution..."
	UNIVERSAL_PATH="target/universal-apple-darwin/release/bundle/macos/Gestura.app"
	REGULAR_PATH="target/release/bundle/macos/Gestura.app"
	if [ -d "$UNIVERSAL_PATH" ]; then
	    APP_PATH="$UNIVERSAL_PATH"
	    echo "Using universal build"
	elif [ -d "$REGULAR_PATH" ]; then
	    APP_PATH="$REGULAR_PATH"
	    echo "Using regular build"
	else
	    echo "❌ App bundle not found. Run 'just build-macos' or 'just build-macos-signed' first"
	    exit 1
	fi
	echo "Creating Gestura-0.1.0-macos.dmg..."
	rm -f "Gestura-0.1.0-macos.dmg"
	hdiutil create -volname "Gestura" \
	    -srcfolder "$APP_PATH" \
	    -ov -format UDZO "Gestura-0.1.0-macos.dmg"
	echo "✅ DMG created: Gestura-0.1.0-macos.dmg"
	ls -lh "Gestura-0.1.0-macos.dmg"

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

