#!/usr/bin/env bash
# macOS packaging script for the Gestura GUI + CLI (Full installer).
#
# This script is intended for local builds on macOS.
#
# Responsibilities:
#  - Build GUI frontend (Vite)
#  - Build a universal CLI binary and stage it under crates/gestura-gui/binaries/
#    so Tauri can bundle it
#  - Run `cargo tauri build` to produce the app bundle
#  - Create a PKG that installs:
#      - Gestura.app to /Applications
#      - gestura CLI to /usr/local/bin/gestura
#  - Copy outputs into dist/macos using canonical artifact names
#
# Notes:
#  - This script does NOT auto-install dependencies (no brew install). It will
#    detect missing commands and fail with an actionable message.
#  - DMG output is optional and not part of the canonical "full installer" set.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "${SCRIPT_DIR}/packaging/common.sh"

APP_NAME="gestura"
APP_DISPLAY_NAME="Gestura"
GUI_DIR="crates/gestura-gui"
FRONTEND_DIR="${GUI_DIR}/frontend"
CLI_PACKAGE="gestura-cli"

DIST_DIR_DEFAULT="dist/macos"
FEATURES_DEFAULT="voice-local"

TARGET_AARCH64="aarch64-apple-darwin"
TARGET_X86_64="x86_64-apple-darwin"
TARGET_UNIVERSAL="universal-apple-darwin"

INSTALLER_IDENTITY="${APPLE_INSTALLER_IDENTITY:-}"

# usage prints CLI help.
usage() {
  cat <<'EOF'
Usage:
  scripts/package-mac.sh [--tag vX.Y.Z] [--dist DIR] [--features FEATURES]

Build a macOS Full installer (PKG) that includes both the GUI and the CLI.

Options:
  --tag vX.Y.Z        Tag used for output naming (default: git tag or v<tauri.conf.json version>)
  --dist DIR          Output directory (default: dist/macos)
  --features FEATURES Cargo feature set (default: voice-local)
  -h, --help          Show this help

Env:
  APPLE_INSTALLER_IDENTITY  If set, productsign will be used to sign the PKG.
EOF
}

DIST_DIR="${DIST_DIR_DEFAULT}"
FEATURES="${FEATURES_DEFAULT}"

while [ $# -gt 0 ]; do
  case "$1" in
    --tag) TAG="$2"; shift 2 ;;
    --dist) DIST_DIR="$2"; shift 2 ;;
    --features) FEATURES="$2"; shift 2 ;;
    -h|--help) usage; exit 0 ;;
    *) die "Unknown argument: $1" ;;
  esac
done

resolve_tag
log_info "Starting macOS packaging for ${APP_DISPLAY_NAME} (${TAG})"

# check_prerequisites verifies required tools are installed.
check_prerequisites() {
  require_cmd cargo
  require_cmd npm
  require_cmd lipo
  require_cmd pkgbuild

  if [ -n "$INSTALLER_IDENTITY" ]; then
    require_cmd productsign
  fi
}

# build_frontend builds the GUI frontend.
build_frontend() {
  log_info "Building frontend"
  (cd "$FRONTEND_DIR" && npm ci && npm run build)
}

# build_cli_universal builds the CLI for both macOS arch targets and produces a
# universal binary at target/universal-apple-darwin/release/gestura.
build_cli_universal() {
  log_info "Building CLI (${CLI_PACKAGE}) for ${TARGET_AARCH64}"
  cargo build --release -p "$CLI_PACKAGE" --features "$FEATURES" --target "$TARGET_AARCH64"

  log_info "Building CLI (${CLI_PACKAGE}) for ${TARGET_X86_64}"
  cargo build --release -p "$CLI_PACKAGE" --features "$FEATURES" --target "$TARGET_X86_64"

  local out_dir="target/${TARGET_UNIVERSAL}/release"
  mkdir -p "$out_dir"

  lipo -create \
    "target/${TARGET_AARCH64}/release/${APP_NAME}" \
    "target/${TARGET_X86_64}/release/${APP_NAME}" \
    -output "${out_dir}/${APP_NAME}"
  chmod +x "${out_dir}/${APP_NAME}"

  log_info "Wrote universal CLI: ${out_dir}/${APP_NAME}"
}

# stage_cli copies the universal CLI into the Tauri externalBin staging dir.
stage_cli() {
  local src="target/${TARGET_UNIVERSAL}/release/${APP_NAME}"
  [ -f "$src" ] || die "CLI binary not found at ${src}"

  local dst_dir="${GUI_DIR}/binaries"
  mkdir -p "$dst_dir"
  local dst="${dst_dir}/${APP_NAME}-${TARGET_UNIVERSAL}"

  cp "$src" "$dst"
  chmod +x "$dst"

  log_info "Staged CLI for bundling: ${dst}"
}

# build_gui runs the Tauri bundler to produce the .app.
build_gui() {
  log_info "Building GUI bundle via cargo tauri (${TARGET_UNIVERSAL})"
  (cd "$GUI_DIR" && cargo tauri build --features "$FEATURES" --target "$TARGET_UNIVERSAL")
}

# find_app_bundle locates the built Gestura.app.
find_app_bundle() {
  local universal_path="${GUI_DIR}/target/${TARGET_UNIVERSAL}/release/bundle/macos/${APP_DISPLAY_NAME}.app"
  local regular_path="${GUI_DIR}/target/release/bundle/macos/${APP_DISPLAY_NAME}.app"

  if [ -d "$universal_path" ]; then
    printf "%s" "$universal_path"
    return 0
  fi
  if [ -d "$regular_path" ]; then
    printf "%s" "$regular_path"
    return 0
  fi

  die "App bundle not found (expected ${universal_path} or ${regular_path})"
}

# create_pkg builds a PKG that installs the GUI + CLI.
create_pkg() {
  local out_dir="$1"
  local version_num="${TAG#v}"

  local pkgroot="${out_dir}/pkgroot"
  rm -rf "$pkgroot"
  mkdir -p "${pkgroot}/Applications" "${pkgroot}/usr/local/bin"

  local app_path
  app_path="$(find_app_bundle)"
  cp -R "$app_path" "${pkgroot}/Applications/"

  local cli_src="target/${TARGET_UNIVERSAL}/release/${APP_NAME}"
  cp "$cli_src" "${pkgroot}/usr/local/bin/${APP_NAME}"
  chmod +x "${pkgroot}/usr/local/bin/${APP_NAME}"

  local unsigned_pkg="${out_dir}/${APP_DISPLAY_NAME}-${TAG}-unsigned.pkg"
  pkgbuild \
    --root "$pkgroot" \
    --identifier "ai.gestura.desktop" \
    --version "$version_num" \
    --install-location "/" \
    "$unsigned_pkg"

  local final_pkg="${out_dir}/${APP_DISPLAY_NAME}-${TAG}-universal.pkg"
  if [ -n "$INSTALLER_IDENTITY" ]; then
    productsign --sign "$INSTALLER_IDENTITY" "$unsigned_pkg" "$final_pkg"
    rm -f "$unsigned_pkg"
  else
    mv "$unsigned_pkg" "$final_pkg"
  fi

  log_info "Wrote ${final_pkg}"
}

# main is the entrypoint.
main() {
  check_prerequisites
  build_frontend
  build_cli_universal
  stage_cli
  build_gui

  local out_dir
  out_dir="$(ensure_fresh_dist_dir "$DIST_DIR")"
  create_pkg "$out_dir"

  write_sha256sums "$out_dir" "${APP_NAME}-${TAG}-SHA256SUMS.txt"
  if [ -f "${out_dir}/${APP_NAME}-${TAG}-SHA256SUMS.txt" ]; then
    log_info "Wrote ${out_dir}/${APP_NAME}-${TAG}-SHA256SUMS.txt"
  fi
}

main "$@"
