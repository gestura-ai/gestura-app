#!/usr/bin/env bash
# Linux packaging script for the Gestura GUI + CLI (Full installers).
#
# This script is intended for local builds on Linux or in CI.
#
# Responsibilities:
#  - Build GUI frontend (Vite)
#  - Build CLI and stage it under crates/gestura-gui/binaries/ so Tauri can
#    bundle it into DEB/RPM installers
#  - Run `cargo tauri build` to produce DEB/RPM
#  - Package a standalone CLI tarball alongside the GUI installers
#  - Copy outputs into dist/linux using canonical artifact names
#
# Notes:
#  - This script does NOT auto-install dependencies.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "${SCRIPT_DIR}/packaging/common.sh"

APP_NAME="gestura"
APP_DISPLAY_NAME="Gestura"
GUI_DIR="crates/gestura-gui"
FRONTEND_DIR="${GUI_DIR}/frontend"
CLI_PACKAGE="gestura-cli"

TARGET_TRIPLE_DEFAULT="x86_64-unknown-linux-gnu"
ARCH_LABEL_DEFAULT="x86_64"
DIST_DIR_DEFAULT="dist/linux"
FEATURES_DEFAULT="voice-local"

# usage prints CLI help.
usage() {
  cat <<'EOF'
Usage:
  scripts/package-linux.sh [--tag vX.Y.Z] [--dist DIR] [--target TRIPLE] [--arch-label LABEL] [--features FEATURES]

Build Full installers for Linux (DEB + RPM) and ensure the CLI is included.

Options:
  --tag vX.Y.Z         Tag used for output naming (default: git tag or v<tauri.conf.json version>)
  --dist DIR           Output directory (default: dist/linux)
  --target TRIPLE      Rust target triple for build (default: x86_64-unknown-linux-gnu)
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
log_info "Starting Linux packaging for ${APP_DISPLAY_NAME} (${TAG})"

# check_prerequisites verifies required tools are installed.
check_prerequisites() {
  require_cmd cargo
  require_cmd npm
  require_cmd tar

  if ! command -v dpkg-deb >/dev/null 2>&1; then
    log_warn "dpkg-deb not found; DEB bundling may fail"
  fi
  if ! command -v rpmbuild >/dev/null 2>&1; then
    log_warn "rpmbuild not found; RPM bundling may fail"
  fi
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

  local src="target/${TARGET_TRIPLE}/release/${APP_NAME}"
  [ -f "$src" ] || die "CLI binary not found at ${src}"

  local dst_dir="${GUI_DIR}/binaries"
  mkdir -p "$dst_dir"
  local dst="${dst_dir}/${APP_NAME}-${TARGET_TRIPLE}"
  cp "$src" "$dst"
  chmod +x "$dst"

  log_info "Staged CLI for bundling: ${dst}"
}

# stage_ffmpeg downloads a static Linux ffmpeg build from johnvansickle.com and
# stages it as the Tauri externalBin sidecar (binaries/ffmpeg-<triple>).
#
# Set GESTURA_FFMPEG_SKIP_DOWNLOAD=1 to skip when a pre-staged binary exists.
stage_ffmpeg() {
  local dst_dir="${GUI_DIR}/binaries"
  local dst="${dst_dir}/ffmpeg-${TARGET_TRIPLE}"

  if [ "${GESTURA_FFMPEG_SKIP_DOWNLOAD:-0}" = "1" ] && [ -f "$dst" ]; then
    log_info "Skipping ffmpeg download — pre-staged binary found at ${dst}"
    return 0
  fi

  require_cmd curl
  require_cmd tar
  require_cmd xz

  mkdir -p "$dst_dir"

  # Map Rust triples to johnvansickle.com arch labels.
  local jv_arch
  case "$TARGET_TRIPLE" in
    x86_64-*)  jv_arch="amd64" ;;
    aarch64-*) jv_arch="arm64" ;;
    armv7-*)   jv_arch="armhf" ;;
    *) die "No johnvansickle ffmpeg build known for triple: ${TARGET_TRIPLE}" ;;
  esac

  local FFMPEG_RELEASE="${GESTURA_FFMPEG_VERSION:-release}"
  local url="https://johnvansickle.com/ffmpeg/releases/ffmpeg-${FFMPEG_RELEASE}-${jv_arch}-static.tar.xz"

  log_info "Downloading static ffmpeg (${jv_arch}) from johnvansickle.com …"

  local tmp
  tmp="$(mktemp -d)"
  trap 'rm -rf "$tmp"' RETURN

  curl -fsSL "$url" -o "${tmp}/ffmpeg.tar.xz" --retry 3
  tar -xJf "${tmp}/ffmpeg.tar.xz" -C "$tmp" --wildcards "*/ffmpeg" --strip-components=1

  [ -f "${tmp}/ffmpeg" ] || die "ffmpeg binary not found after extraction"

  cp "${tmp}/ffmpeg" "$dst"
  chmod +x "$dst"

  log_info "Staged bundled ffmpeg sidecar: ${dst}"
}

# build_gui runs the Tauri bundler.
build_gui() {
  log_info "Building GUI bundles via cargo tauri (${TARGET_TRIPLE})"
  (cd "$GUI_DIR" && cargo tauri build --features "$FEATURES" --target "$TARGET_TRIPLE")
}

# collect_artifacts copies DEB/RPM outputs and the standalone CLI archive into the
# dist dir with canonical names.
collect_artifacts() {
  local out_dir
  out_dir="$(ensure_fresh_dist_dir "$DIST_DIR")"

  local bundle_root1="target/${TARGET_TRIPLE}/release/bundle"
  local bundle_root2="target/release/bundle"
  local bundle_root3="${GUI_DIR}/target/${TARGET_TRIPLE}/release/bundle"
  local bundle_root4="${GUI_DIR}/target/release/bundle"

  local deb_src
  deb_src="$(ls -1 "${bundle_root1}/deb/"*.deb 2>/dev/null | head -1 || true)"
  if [ -z "$deb_src" ]; then
    deb_src="$(ls -1 "${bundle_root2}/deb/"*.deb 2>/dev/null | head -1 || true)"
  fi
  if [ -z "$deb_src" ]; then
    deb_src="$(ls -1 "${bundle_root3}/deb/"*.deb 2>/dev/null | head -1 || true)"
  fi
  if [ -z "$deb_src" ]; then
    deb_src="$(ls -1 "${bundle_root4}/deb/"*.deb 2>/dev/null | head -1 || true)"
  fi

  local rpm_src
  rpm_src="$(ls -1 "${bundle_root1}/rpm/"*.rpm 2>/dev/null | head -1 || true)"
  if [ -z "$rpm_src" ]; then
    rpm_src="$(ls -1 "${bundle_root2}/rpm/"*.rpm 2>/dev/null | head -1 || true)"
  fi
  if [ -z "$rpm_src" ]; then
    rpm_src="$(ls -1 "${bundle_root3}/rpm/"*.rpm 2>/dev/null | head -1 || true)"
  fi
  if [ -z "$rpm_src" ]; then
    rpm_src="$(ls -1 "${bundle_root4}/rpm/"*.rpm 2>/dev/null | head -1 || true)"
  fi

  if [ -n "$deb_src" ]; then
    cp "$deb_src" "${out_dir}/${APP_NAME}-${TAG}-linux-${ARCH_LABEL}.deb"
    log_info "Wrote ${out_dir}/${APP_NAME}-${TAG}-linux-${ARCH_LABEL}.deb"
  else
    log_warn "No .deb found under bundle output"
  fi

  if [ -n "$rpm_src" ]; then
    cp "$rpm_src" "${out_dir}/${APP_NAME}-${TAG}-linux-${ARCH_LABEL}.rpm"
    log_info "Wrote ${out_dir}/${APP_NAME}-${TAG}-linux-${ARCH_LABEL}.rpm"
  else
    log_warn "No .rpm found under bundle output"
  fi

  local cli_src="target/${TARGET_TRIPLE}/release/${APP_NAME}"
  local cli_archive="${out_dir}/${APP_NAME}-cli-${TAG}-linux-${ARCH_LABEL}.tar.gz"
  [ -f "$cli_src" ] || die "CLI binary not found at ${cli_src}"

  tar -C "$(dirname "$cli_src")" -czf "$cli_archive" "$(basename "$cli_src")"
  log_info "Wrote ${cli_archive}"

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
