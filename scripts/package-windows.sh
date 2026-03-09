#!/usr/bin/env bash
# Windows packaging script for the Gestura GUI + CLI (Full installer).
#
# This script is intended for local builds on Windows (Git Bash / MSYS2) or in
# CI runners.
#
# Responsibilities:
#  - Build GUI frontend (Vite)
#  - Build CLI and stage it under crates/gestura-gui/binaries/ so Tauri can
#    bundle it into the MSI
#  - Run `cargo tauri build` to produce an MSI
#  - Copy outputs into dist/windows using canonical artifact names
#
# Notes:
#  - This script does NOT auto-install dependencies.
#  - The MSI produced by Tauri/WiX typically installs the CLI alongside the app.
#    Adding the CLI to PATH is a separate enhancement.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "${SCRIPT_DIR}/packaging/common.sh"

APP_NAME="gestura"
APP_DISPLAY_NAME="Gestura"
GUI_DIR="crates/gestura-gui"
FRONTEND_DIR="${GUI_DIR}/frontend"
CLI_PACKAGE="gestura-cli"

TARGET_TRIPLE_DEFAULT="x86_64-pc-windows-msvc"
ARCH_LABEL_DEFAULT="x86_64"
DIST_DIR_DEFAULT="dist/windows"
FEATURES_DEFAULT="voice-local"

# usage prints CLI help.
usage() {
  cat <<'EOF'
Usage:
  scripts/package-windows.sh [--tag vX.Y.Z] [--dist DIR] [--target TRIPLE] [--arch-label LABEL] [--features FEATURES]

Build Full installers for Windows (MSI) and ensure the CLI is included.

Options:
  --tag vX.Y.Z         Tag used for output naming (default: git tag or v<tauri.conf.json version>)
  --dist DIR           Output directory (default: dist/windows)
  --target TRIPLE      Rust target triple for build (default: x86_64-pc-windows-msvc)
  --arch-label LABEL   Label used in artifact name (default: x86_64)
  --features FEATURES  Cargo feature set (default: voice-local)
  -h, --help           Show this help
EOF
}

DIST_DIR="${DIST_DIR_DEFAULT}"
TARGET_TRIPLE="${TARGET_TRIPLE_DEFAULT}"
ARCH_LABEL="${ARCH_LABEL_DEFAULT}"
FEATURES="${FEATURES_DEFAULT}"

while [ $# -gt 0 ]; do
  case "$1" in
    --tag) TAG="$2"; shift 2 ;;
    --dist) DIST_DIR="$2"; shift 2 ;;
    --target) TARGET_TRIPLE="$2"; shift 2 ;;
    --arch-label) ARCH_LABEL="$2"; shift 2 ;;
    --features) FEATURES="$2"; shift 2 ;;
    -h|--help) usage; exit 0 ;;
    *) die "Unknown argument: $1" ;;
  esac
done

resolve_tag
log_info "Starting Windows packaging for ${APP_DISPLAY_NAME} (${TAG})"

# check_prerequisites verifies required tools are installed.
check_prerequisites() {
  require_cmd cargo
  require_cmd npm

  case "${OSTYPE:-}" in
    msys*|cygwin*) ;;
    *) log_warn "OSTYPE=${OSTYPE:-} (expected msys/cygwin). Continuing, but build may fail if not on Windows." ;;
  esac
}

# build_frontend builds the GUI frontend.
build_frontend() {
  log_info "Building frontend"
  (cd "$FRONTEND_DIR" && npm ci && npm run build)
}

# stage_cli builds the CLI and stages it for Tauri `bundle.externalBin`.
stage_cli() {
  log_info "Building CLI (${CLI_PACKAGE}) for ${TARGET_TRIPLE}"
  cargo build --release -p "$CLI_PACKAGE" --features "$FEATURES" --target "$TARGET_TRIPLE"

  local src="target/${TARGET_TRIPLE}/release/${APP_NAME}.exe"
  [ -f "$src" ] || die "CLI binary not found at ${src}"

  local dst_dir="${GUI_DIR}/binaries"
  mkdir -p "$dst_dir"
  local dst="${dst_dir}/${APP_NAME}-${TARGET_TRIPLE}.exe"
  cp "$src" "$dst"

  log_info "Staged CLI for bundling: ${dst}"
}

# stage_ffmpeg downloads a static Windows ffmpeg build (BtbN/FFmpeg-Builds) and
# stages it as the Tauri externalBin sidecar (binaries/ffmpeg-<triple>.exe).
#
# Set GESTURA_FFMPEG_SKIP_DOWNLOAD=1 to skip when a pre-staged binary exists.
stage_ffmpeg() {
  local dst_dir="${GUI_DIR}/binaries"
  local dst="${dst_dir}/ffmpeg-${TARGET_TRIPLE}.exe"

  if [ "${GESTURA_FFMPEG_SKIP_DOWNLOAD:-0}" = "1" ] && [ -f "$dst" ]; then
    log_info "Skipping ffmpeg download — pre-staged binary found at ${dst}"
    return 0
  fi

  require_cmd curl
  require_cmd unzip

  mkdir -p "$dst_dir"

  # BtbN provides static "essentials" Windows builds (~30 MB) at a stable URL.
  local FFMPEG_TAG="${GESTURA_FFMPEG_VERSION:-latest}"
  # Map Rust triples to BtbN arch labels.
  local btbn_arch
  case "$TARGET_TRIPLE" in
    x86_64-*)  btbn_arch="win64" ;;
    i686-*)    btbn_arch="win32" ;;
    aarch64-*) btbn_arch="arm64" ;;
    *) die "No BtbN ffmpeg build known for triple: ${TARGET_TRIPLE}" ;;
  esac

  local url="https://github.com/BtbN/FFmpeg-Builds/releases/download/${FFMPEG_TAG}/ffmpeg-${FFMPEG_TAG}-${btbn_arch}-static.zip"

  log_info "Downloading static ffmpeg (${btbn_arch}) from BtbN/FFmpeg-Builds …"

  local tmp
  tmp="$(mktemp -d)"
  trap 'rm -rf "$tmp"' RETURN

  curl -fsSL "$url" -L -o "${tmp}/ffmpeg.zip" --retry 3
  unzip -q "${tmp}/ffmpeg.zip" -d "${tmp}/extracted"

  # The zip contains a top-level directory; ffmpeg.exe is in its bin/ sub-dir.
  local ffmpeg_bin
  ffmpeg_bin="$(find "${tmp}/extracted" -name 'ffmpeg.exe' | head -1)"
  [ -n "$ffmpeg_bin" ] || die "ffmpeg.exe not found after extraction"

  cp "$ffmpeg_bin" "$dst"

  log_info "Staged bundled ffmpeg sidecar: ${dst}"
}

# build_gui runs the Tauri bundler.
build_gui() {
  log_info "Building GUI bundles via cargo tauri"
  (cd "$GUI_DIR" && cargo tauri build --features "$FEATURES" --target "$TARGET_TRIPLE")
}

# collect_artifacts copies the MSI output into the dist dir with a canonical name.
collect_artifacts() {
  local out_dir
  out_dir="$(ensure_fresh_dist_dir "$DIST_DIR")"

  local bundle_root1="${GUI_DIR}/target/${TARGET_TRIPLE}/release/bundle"
  local bundle_root2="${GUI_DIR}/target/release/bundle"

  local msi_src
  msi_src="$(ls -1 "${bundle_root1}/msi/"*.msi 2>/dev/null | head -1 || true)"
  if [ -z "$msi_src" ]; then
    msi_src="$(ls -1 "${bundle_root1}/wix/"*.msi 2>/dev/null | head -1 || true)"
  fi
  if [ -z "$msi_src" ]; then
    msi_src="$(ls -1 "${bundle_root2}/msi/"*.msi 2>/dev/null | head -1 || true)"
  fi
  if [ -z "$msi_src" ]; then
    msi_src="$(ls -1 "${bundle_root2}/wix/"*.msi 2>/dev/null | head -1 || true)"
  fi

  if [ -n "$msi_src" ]; then
    cp "$msi_src" "${out_dir}/${APP_DISPLAY_NAME}-${TAG}-windows-${ARCH_LABEL}.msi"
    log_info "Wrote ${out_dir}/${APP_DISPLAY_NAME}-${TAG}-windows-${ARCH_LABEL}.msi"
  else
    log_warn "No .msi found under bundle output"
  fi

  write_sha256sums "$out_dir" "${APP_NAME}-${TAG}-SHA256SUMS.txt"
  if [ -f "${out_dir}/${APP_NAME}-${TAG}-SHA256SUMS.txt" ]; then
    log_info "Wrote ${out_dir}/${APP_NAME}-${TAG}-SHA256SUMS.txt"
  fi
}

# main is the entrypoint.
main() {
  check_prerequisites
  build_frontend
  stage_cli
  stage_ffmpeg
  build_gui
  collect_artifacts
}

main "$@"
