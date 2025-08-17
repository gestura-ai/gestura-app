# Justfile for Gestura.app (compliments Makefile)

set shell := ["bash", "-cu"]

# Default target
default: help

help:
	@echo "just targets:"
	@echo "  validate             # run full production validation pipeline"
	@echo "  validate-quick       # run quick validation (format, clippy, test)"
	@echo "  build                # cargo build"
	@echo "  test                 # cargo test"
	@echo "  clean                # cargo clean"
	@echo "  build-voice-local    # build with whisper.cpp (voice-local)"
	@echo "  run-voice-local      # run with voice-local"
	@echo "  check-nats           # cargo check with nats feature"
	@echo "  dev                  # run frontend dev server"
	@echo "  package              # build production app"
	@echo "  doctor               # print environment info"

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

build:
	cargo build --manifest-path {{app_dir}}/Cargo.toml

test:
	cargo test --manifest-path {{app_dir}}/Cargo.toml -q

clean:
	cargo clean --manifest-path {{app_dir}}/Cargo.toml

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
	npm install
	npm run tauri:dev

package:
	npm install
	npm run tauri:build

doctor:
	@echo "Rustc: $(rustc --version)"
	@echo "Cargo: $(cargo --version)"
	@echo "cmake: $(cmake --version 2>/dev/null || echo 'not found')"
	@echo "OS: $(uname -a)"

# Packaging commands
package-mac: build
	@echo "📦 Creating macOS packages..."
	./scripts/package-mac.sh

package-windows: build
	@echo "📦 Creating Windows packages..."
	./scripts/package-windows.sh

package-linux: build
	@echo "📦 Creating Linux packages..."
	./scripts/package-linux.sh

package-all: build
	@echo "📦 Creating packages for all platforms..."
	just package-mac
	just package-windows
	just package-linux

# Code signing
sign-mac:
	@echo "🔐 Code signing macOS app..."
	./scripts/sign-mac.sh

notarize-mac: sign-mac
	@echo "🔐 Notarizing macOS app..."
	./scripts/notarize-mac.sh

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
	node scripts/test-tray.js

