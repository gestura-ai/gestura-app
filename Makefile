# Makefile for Gestura.app (Tauri v2)
# Convenience targets for common dev flows. Does not install system deps.

CARGO = cargo
TAURI_DIR = src-tauri

.DEFAULT_GOAL := help

help:
	@echo "Gestura Makefile"
	@echo "Targets:"
	@echo "  build              - Build default workspace"
	@echo "  test               - Run all tests (default profile)"
	@echo "  clean              - Clean target dirs"
	@echo "  build-voice-local  - Build with local whisper (feature voice-local)"
	@echo "  run-voice-local    - Run app with local whisper (feature voice-local)"
	@echo "  test-nats          - Check build with NATS feature"
	@echo ""
	@echo "Packaging targets:"
	@echo "  package-mac        - Create macOS .app bundle and .dmg"
	@echo "  package-windows    - Create Windows .exe installer"
	@echo "  package-linux      - Create Linux .deb, .rpm, .AppImage"
	@echo "  package-all        - Create packages for all platforms"
	@echo "  sign-mac           - Code sign macOS app"
	@echo "  notarize-mac       - Notarize macOS app"

build:
	@$(CARGO) build --manifest-path $(TAURI_DIR)/Cargo.toml

clean:
	@$(CARGO) clean --manifest-path $(TAURI_DIR)/Cargo.toml

# Local whisper builds require cmake (whisper-rs build).
check-cmake:
	@command -v cmake >/dev/null 2>&1 || { echo "Error: cmake is required for voice-local builds."; exit 1; }

build-voice-local: check-cmake
	@$(CARGO) build --manifest-path $(TAURI_DIR)/Cargo.toml --features voice-local

run-voice-local: check-cmake
	@$(CARGO) run --manifest-path $(TAURI_DIR)/Cargo.toml --features voice-local

# Ensure NATS feature compiles
test-nats:
	@$(CARGO) check --manifest-path $(TAURI_DIR)/Cargo.toml --features nats

# Default tests
.test-internal:
	@$(CARGO) test --manifest-path $(TAURI_DIR)/Cargo.toml -q

test: .test-internal
	@echo "Tests complete."

# Packaging commands
package-mac: build
	@echo "📦 Creating macOS packages..."
	@./scripts/package-mac.sh

package-windows: build
	@echo "📦 Creating Windows packages..."
	@./scripts/package-windows.sh

package-linux: build
	@echo "📦 Creating Linux packages..."
	@./scripts/package-linux.sh

package-all: build
	@echo "📦 Creating packages for all platforms..."
	@$(MAKE) package-mac
	@$(MAKE) package-windows
	@$(MAKE) package-linux

# Code signing
sign-mac:
	@echo "🔐 Code signing macOS app..."
	@./scripts/sign-mac.sh

notarize-mac: sign-mac
	@echo "🔐 Notarizing macOS app..."
	@./scripts/notarize-mac.sh

# Distribution
upload-releases:
	@echo "☁️ Uploading releases..."
	@./scripts/upload-releases.sh

.PHONY: help build clean test package-mac package-windows package-linux package-all sign-mac notarize-mac upload-releases

