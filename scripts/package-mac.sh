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
#  - Package a standalone CLI tarball for release/Homebrew consumption
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
CLI_DIR="crates/gestura-cli"
PKG_ICON_SOURCE="${GUI_DIR}/icons/icon.png"
PKG_ICON_HELPER="${SCRIPT_DIR}/packaging/set-macos-custom-icon.sh"

DIST_DIR_DEFAULT="dist/macos"
FEATURES_DEFAULT="voice-local"

TARGET_AARCH64="aarch64-apple-darwin"
TARGET_X86_64="x86_64-apple-darwin"
TARGET_UNIVERSAL="universal-apple-darwin"

INSTALLER_IDENTITY="${APPLE_INSTALLER_IDENTITY:-}"
SIGNING_IDENTITY="${APPLE_SIGNING_IDENTITY:-}"

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
  APPLE_SIGNING_IDENTITY    If set, the universal CLI is codesigned and the app
                            bundle is expected to be signed/notarized by Tauri.
  APPLE_INSTALLER_IDENTITY  If set, productsign will be used to sign the PKG.
  APPLE_ID / APPLE_TEAM_ID / APPLE_PASSWORD
                            If set (or APPLE_PASSWORD uses @keychain:profile),
                            the PKG will be notarized and stapled after signing.
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
  require_cmd tar
  require_cmd sips
  require_cmd DeRez
  require_cmd Rez
  require_cmd SetFile

  [ -f "$PKG_ICON_SOURCE" ] || die "PKG icon source not found at ${PKG_ICON_SOURCE}"
  [ -x "$PKG_ICON_HELPER" ] || die "PKG icon helper is missing or not executable: ${PKG_ICON_HELPER}"

  if [ -n "$SIGNING_IDENTITY" ]; then
    require_cmd codesign
    require_cmd spctl
    require_cmd xcrun
  fi

  if [ -n "$INSTALLER_IDENTITY" ]; then
    require_cmd productsign
    require_cmd xcrun
  fi
}

# build_frontend builds the GUI frontend.
build_frontend() {
  log_info "Building frontend"
  (cd "$FRONTEND_DIR" && npm ci && npm run build)
}

# stage_ffmpeg downloads static ffmpeg builds and stages them as Tauri
# externalBin sidecars under three names:
#
#   binaries/ffmpeg-aarch64-apple-darwin   — used by the arm64 cargo build slice
#   binaries/ffmpeg-x86_64-apple-darwin    — used by the x86_64 cargo build slice
#   binaries/ffmpeg-universal-apple-darwin — lipo'd universal (for completeness)
#
# Sources (both signed & statically linked):
#   arm64  — https://ffmpeg.martin-riedl.de (snapshot; Apple Silicon)
#   x86_64 — https://ffmpeg.martin-riedl.de (release;  Intel)
#
# When `cargo tauri build --target universal-apple-darwin` runs it internally
# compiles two separate cargo build invocations (aarch64 + x86_64).  Each build
# script checks for `binaries/ffmpeg-<that-arch>-apple-darwin`, so both
# per-arch names must be present — not just the universal one.
#
# Set GESTURA_FFMPEG_SKIP_DOWNLOAD=1 to skip when pre-staged binaries exist.
stage_ffmpeg() {
  local dst_dir="${GUI_DIR}/binaries"
  local dst_arm64="${dst_dir}/ffmpeg-${TARGET_AARCH64}"
  local dst_x86_64="${dst_dir}/ffmpeg-${TARGET_X86_64}"
  local dst_universal="${dst_dir}/ffmpeg-${TARGET_UNIVERSAL}"

  if [ "${GESTURA_FFMPEG_SKIP_DOWNLOAD:-0}" = "1" ] \
       && [ -f "$dst_arm64" ] && [ -f "$dst_x86_64" ]; then
    log_info "Skipping ffmpeg download — pre-staged binaries found"
    return 0
  fi

  require_cmd curl
  require_cmd unzip
  require_cmd lipo

  mkdir -p "$dst_dir"

  local tmp
  tmp="$(mktemp -d)"
  trap 'if [ -n "${tmp:-}" ]; then rm -rf -- "$tmp"; fi' RETURN

  # martin-riedl.de provides signed, statically-linked macOS ffmpeg for both
  # architectures via stable redirect URLs.
  local ARM64_URL="https://ffmpeg.martin-riedl.de/redirect/latest/macos/arm64/snapshot/ffmpeg.zip"
  local X86_64_URL="https://ffmpeg.martin-riedl.de/redirect/latest/macos/amd64/release/ffmpeg.zip"

  log_info "Downloading static ffmpeg for macOS arm64 (martin-riedl.de) …"
  curl -fsSL -L "$ARM64_URL" \
       -o "${tmp}/ffmpeg-arm64.zip" \
       --retry 3

  log_info "Downloading static ffmpeg for macOS x86_64 (martin-riedl.de) …"
  curl -fsSL -L "$X86_64_URL" \
       -o "${tmp}/ffmpeg-x86_64.zip" \
       --retry 3

  unzip -q "${tmp}/ffmpeg-arm64.zip"  -d "${tmp}/arm64"
  unzip -q "${tmp}/ffmpeg-x86_64.zip" -d "${tmp}/x86_64"

  # Stage per-arch sidecars (required by Tauri's universal build slices).
  cp "${tmp}/arm64/ffmpeg"  "$dst_arm64"
  cp "${tmp}/x86_64/ffmpeg" "$dst_x86_64"
  chmod +x "$dst_arm64" "$dst_x86_64"

  # Lipo a universal binary for completeness / future use.
  lipo -create "$dst_arm64" "$dst_x86_64" -output "$dst_universal"
  chmod +x "$dst_universal"

  log_info "Staged ffmpeg sidecars: aarch64, x86_64, universal"
}

# build_cli_universal builds the CLI for both macOS arch targets and produces a
# universal binary at target/universal-apple-darwin/release/gestura.
build_cli_universal() {
  local arm64_target_dir="target/build-arm64"
  local x86_64_target_dir="target/build-x86"
  local cli_features
  cli_features="$(filter_feature_csv "$FEATURES" voice-local nats security)"

  log_info "Building CLI (${CLI_PACKAGE}) for ${TARGET_AARCH64}"
  if [ -n "$cli_features" ]; then
    CARGO_TARGET_DIR="$arm64_target_dir" \
      cargo build --release -p "$CLI_PACKAGE" --features "$cli_features" --target "$TARGET_AARCH64"
  else
    CARGO_TARGET_DIR="$arm64_target_dir" \
      cargo build --release -p "$CLI_PACKAGE" --target "$TARGET_AARCH64"
  fi

  log_info "Building CLI (${CLI_PACKAGE}) for ${TARGET_X86_64}"
  if [ -n "$cli_features" ]; then
    CARGO_TARGET_DIR="$x86_64_target_dir" \
      cargo build --release -p "$CLI_PACKAGE" --features "$cli_features" --target "$TARGET_X86_64"
  else
    CARGO_TARGET_DIR="$x86_64_target_dir" \
      cargo build --release -p "$CLI_PACKAGE" --target "$TARGET_X86_64"
  fi

  local out_dir="target/${TARGET_UNIVERSAL}/release"
  mkdir -p "$out_dir"

  lipo -create \
    "${arm64_target_dir}/${TARGET_AARCH64}/release/${APP_NAME}" \
    "${x86_64_target_dir}/${TARGET_X86_64}/release/${APP_NAME}" \
    -output "${out_dir}/${APP_NAME}"
  chmod +x "${out_dir}/${APP_NAME}"

  log_info "Wrote universal CLI: ${out_dir}/${APP_NAME}"
}

# sign_cli_universal codesigns the universal CLI binary when an Apple signing
# identity is configured.
sign_cli_universal() {
  if [ -z "$SIGNING_IDENTITY" ]; then
    log_info "APPLE_SIGNING_IDENTITY not set; skipping CLI codesign"
    return 0
  fi

  local cli_path="target/${TARGET_UNIVERSAL}/release/${APP_NAME}"
  [ -f "$cli_path" ] || die "CLI binary not found at ${cli_path}"

  codesign --force --options runtime --timestamp \
    --entitlements "${CLI_DIR}/entitlements.plist" \
    --sign "$SIGNING_IDENTITY" \
    "$cli_path"

  codesign --verify --deep --strict --verbose=2 "$cli_path"
  log_info "Signed universal CLI: ${cli_path}"
}

# stage_cli copies the per-arch and universal CLI binaries into the Tauri
# externalBin staging dir. Universal Tauri builds validate the per-arch names.
stage_cli() {
  local dst_dir="${GUI_DIR}/binaries"
  mkdir -p "$dst_dir"
  local arm64_src="target/build-arm64/${TARGET_AARCH64}/release/${APP_NAME}"
  local x86_64_src="target/build-x86/${TARGET_X86_64}/release/${APP_NAME}"
  local universal_src="target/${TARGET_UNIVERSAL}/release/${APP_NAME}"

  [ -f "$arm64_src" ] || die "CLI binary not found at ${arm64_src}"
  [ -f "$x86_64_src" ] || die "CLI binary not found at ${x86_64_src}"
  [ -f "$universal_src" ] || die "CLI binary not found at ${universal_src}"

  cp "$arm64_src" "${dst_dir}/${APP_NAME}-${TARGET_AARCH64}"
  cp "$x86_64_src" "${dst_dir}/${APP_NAME}-${TARGET_X86_64}"
  cp "$universal_src" "${dst_dir}/${APP_NAME}-${TARGET_UNIVERSAL}"
  chmod +x \
    "${dst_dir}/${APP_NAME}-${TARGET_AARCH64}" \
    "${dst_dir}/${APP_NAME}-${TARGET_X86_64}" \
    "${dst_dir}/${APP_NAME}-${TARGET_UNIVERSAL}"

  log_info "Staged CLI binaries for bundling: ${dst_dir}"
}

# sign_sidecars explicitly signs all binaries staged for Tauri bundling.
# This prevents Apple notarization rejections due to missing timestamps on sidecars.
sign_sidecars() {
  if [ -z "$SIGNING_IDENTITY" ]; then
    log_info "APPLE_SIGNING_IDENTITY not set; skipping sidecar codesigning"
    return 0
  fi

  local bin_dir="${GUI_DIR}/binaries"
  if [ -d "$bin_dir" ]; then
    for bin in "$bin_dir"/*; do
      if [ -f "$bin" ]; then
        log_info "Codesigning sidecar: $bin"
        codesign --force --options runtime --timestamp \
          --entitlements "${CLI_DIR}/entitlements.plist" \
          --sign "$SIGNING_IDENTITY" \
          "$bin"
      fi
    done
  fi
}

# build_gui runs the Tauri bundler to produce the .app.
# APPLE_ID / APPLE_PASSWORD / APPLE_TEAM_ID are suppressed so Tauri skips its
# own (incomplete) notarization pass. notarize-mac.sh handles the full
# sign → notarize → staple cycle after the bundle is produced.
build_gui() {
  log_info "Building GUI bundle via cargo tauri (${TARGET_UNIVERSAL}) — notarization deferred to notarize-mac.sh"
  (cd "$GUI_DIR" && env -u APPLE_ID -u APPLE_PASSWORD -u APPLE_TEAM_ID \
    MACOSX_DEPLOYMENT_TARGET=10.15 \
    cargo tauri build --features "$FEATURES" --target "$TARGET_UNIVERSAL")
}

# sign_and_notarize delegates the full code-sign → notarize → staple pipeline
# to notarize-mac.sh — the same battle-tested script used by `just release-macos`
# locally. This ensures CI and local builds use identical signing logic.
sign_and_notarize() {
  if [ -z "${SIGNING_IDENTITY:-}" ]; then
    log_info "APPLE_SIGNING_IDENTITY not set; skipping signing and notarization"
    return 0
  fi

  log_info "Delegating sign + notarize + staple to notarize-mac.sh"
  "${SCRIPT_DIR}/notarize-mac.sh"
}

# notarization_credentials_configured returns success when either direct Apple
# credentials or a notarytool keychain profile is available.
notarization_credentials_configured() {
  if [ -z "${APPLE_PASSWORD:-}" ]; then
    return 1
  fi

  if [[ "${APPLE_PASSWORD}" == @keychain:* ]]; then
    return 0
  fi

  [ -n "${APPLE_ID:-}" ] && [ -n "${APPLE_TEAM_ID:-}" ]
}

# submit_for_notarization submits a signed artifact with notarytool and waits.
submit_for_notarization() {
  local artifact_path="$1"

  if ! notarization_credentials_configured; then
    log_warn "Apple notarization credentials are not configured; skipping notarization for ${artifact_path}"
    return 0
  fi

  if [[ "${APPLE_PASSWORD}" == @keychain:* ]]; then
    local profile_name="${APPLE_PASSWORD#@keychain:}"
    xcrun notarytool submit "$artifact_path" \
      --keychain-profile "$profile_name" \
      --wait
  else
    xcrun notarytool submit "$artifact_path" \
      --apple-id "$APPLE_ID" \
      --team-id "$APPLE_TEAM_ID" \
      --password "$APPLE_PASSWORD" \
      --wait
  fi
}

# verify_app_bundle checks that a signed macOS app bundle is accepted by the OS.
verify_app_bundle() {
  if [ -z "$SIGNING_IDENTITY" ]; then
    log_info "APPLE_SIGNING_IDENTITY not set; skipping app signature verification"
    return 0
  fi

  local app_path
  app_path="$(find_app_bundle)"

  codesign --verify --deep --strict --verbose=2 "$app_path"
  spctl --assess --type exec --verbose=2 "$app_path"
  xcrun stapler validate "$app_path"

  log_info "Verified signed and notarized app bundle: ${app_path}"
}

# find_app_bundle locates the built Gestura.app.
find_app_bundle() {
  local universal_path="target/${TARGET_UNIVERSAL}/release/bundle/macos/${APP_DISPLAY_NAME}.app"
  local regular_path="target/release/bundle/macos/${APP_DISPLAY_NAME}.app"
  local universal_path_legacy="${GUI_DIR}/target/${TARGET_UNIVERSAL}/release/bundle/macos/${APP_DISPLAY_NAME}.app"
  local regular_path_legacy="${GUI_DIR}/target/release/bundle/macos/${APP_DISPLAY_NAME}.app"

  if [ -d "$universal_path" ]; then
    printf "%s" "$universal_path"
    return 0
  fi
  if [ -d "$regular_path" ]; then
    printf "%s" "$regular_path"
    return 0
  fi
  if [ -d "$universal_path_legacy" ]; then
    printf "%s" "$universal_path_legacy"
    return 0
  fi
  if [ -d "$regular_path_legacy" ]; then
    printf "%s" "$regular_path_legacy"
    return 0
  fi

  die "App bundle not found (expected ${universal_path}, ${regular_path}, ${universal_path_legacy}, or ${regular_path_legacy})"
}

# apply_pkg_icon assigns the branded Finder icon to the generated flat PKG.
apply_pkg_icon() {
  local pkg_path="$1"

  [ -f "$pkg_path" ] || die "PKG not found at ${pkg_path}"
  "$PKG_ICON_HELPER" "$PKG_ICON_SOURCE" "$pkg_path"
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

  # pkgroot is no longer needed after pkgbuild; remove it so the dist dir
  # only contains releasable files and the GitHub upload step doesn't choke
  # on trying to upload a directory as a release asset.
  rm -rf "$pkgroot"

  local final_pkg="${out_dir}/${APP_DISPLAY_NAME}-${TAG}-universal.pkg"
  if [ -n "$INSTALLER_IDENTITY" ]; then
    productsign --sign "$INSTALLER_IDENTITY" "$unsigned_pkg" "$final_pkg"
    rm -f "$unsigned_pkg"

    submit_for_notarization "$final_pkg"
    xcrun stapler staple "$final_pkg"
    xcrun stapler validate "$final_pkg"
  else
    mv "$unsigned_pkg" "$final_pkg"
  fi

  apply_pkg_icon "$final_pkg"

  log_info "Wrote ${final_pkg}"
}

# package_cli_archive writes the canonical standalone macOS CLI archive.
package_cli_archive() {
  local out_dir="$1"
  local cli_src="target/${TARGET_UNIVERSAL}/release/${APP_NAME}"
  local archive_path="${out_dir}/${APP_NAME}-cli-${TAG}-macos-universal.tar.gz"

  [ -f "$cli_src" ] || die "CLI binary not found at ${cli_src}"

  tar -C "$(dirname "$cli_src")" -czf "$archive_path" "$(basename "$cli_src")"
  log_info "Wrote ${archive_path}"
}

# main is the entrypoint.
main() {
  check_prerequisites
  build_frontend
  build_cli_universal
  sign_cli_universal
  stage_cli
  stage_ffmpeg
  sign_sidecars
  build_gui
  sign_and_notarize

  local out_dir
  out_dir="$(ensure_fresh_dist_dir "$DIST_DIR")"
  create_pkg "$out_dir"
  package_cli_archive "$out_dir"

  write_sha256sums "$out_dir" "${APP_NAME}-${TAG}-SHA256SUMS.txt"
  if [ -f "${out_dir}/${APP_NAME}-${TAG}-SHA256SUMS.txt" ]; then
    log_info "Wrote ${out_dir}/${APP_NAME}-${TAG}-SHA256SUMS.txt"
  fi
}

main "$@"
