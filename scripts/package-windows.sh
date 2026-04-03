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
#  - Package a standalone CLI zip alongside the GUI installer
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
TAURI_CONF_PATH="${GUI_DIR}/tauri.conf.json"
TAURI_CONF_BACKUP=""

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

Env:
  WINDOWS_CERT_THUMBPRINT  If set, patches tauri.conf.json so Tauri signs the
                           MSI during the build.
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

  if ! command -v pwsh >/dev/null 2>&1 && ! command -v powershell.exe >/dev/null 2>&1; then
    die "Missing required command: pwsh or powershell.exe"
  fi

  if [ -n "${WINDOWS_CERT_THUMBPRINT:-}" ]; then
    require_cmd node
  fi
}

# patch_tauri_windows_signing injects the certificate thumbprint into the Tauri
# config for the duration of the packaging run.
patch_tauri_windows_signing() {
  if [ -z "${WINDOWS_CERT_THUMBPRINT:-}" ]; then
    return 0
  fi

  TAURI_CONF_BACKUP="${TAURI_CONF_PATH}.release-backup"
  cp "$TAURI_CONF_PATH" "$TAURI_CONF_BACKUP"

  node - "$TAURI_CONF_PATH" <<'NODE'
const fs = require('fs');

const confPath = process.argv[2];
const thumbprint = process.env.WINDOWS_CERT_THUMBPRINT;
const config = JSON.parse(fs.readFileSync(confPath, 'utf8'));

config.bundle ??= {};
config.bundle.windows ??= {};
config.bundle.windows.certificateThumbprint = thumbprint;

fs.writeFileSync(confPath, `${JSON.stringify(config, null, 2)}\n`);
NODE

  log_info "Injected Windows certificate thumbprint into ${TAURI_CONF_PATH}"
}

# restore_tauri_windows_signing restores the original Tauri config after patching.
restore_tauri_windows_signing() {
  if [ -n "$TAURI_CONF_BACKUP" ] && [ -f "$TAURI_CONF_BACKUP" ]; then
    mv "$TAURI_CONF_BACKUP" "$TAURI_CONF_PATH"
    log_info "Restored original ${TAURI_CONF_PATH}"
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
  local cli_features
  cli_features="$(filter_feature_csv "$FEATURES" voice-local nats security)"

  if [ -n "$cli_features" ]; then
    cargo build --release -p "$CLI_PACKAGE" --features "$cli_features" --target "$TARGET_TRIPLE"
  else
    cargo build --release -p "$CLI_PACKAGE" --target "$TARGET_TRIPLE"
  fi

  local src="target/${TARGET_TRIPLE}/release/${APP_NAME}.exe"
  [ -f "$src" ] || die "CLI binary not found at ${src}"

  local dst_dir="${GUI_DIR}/binaries"
  mkdir -p "$dst_dir"
  local dst="${dst_dir}/${APP_NAME}-${TARGET_TRIPLE}.exe"
  cp "$src" "$dst"

  log_info "Staged CLI for bundling: ${dst}"
}

# sign_cli signs the standalone CLI executable before it is archived.
sign_cli() {
  if [ -z "${WINDOWS_CERT_THUMBPRINT:-}" ]; then
    return 0
  fi

  local cli_src="target/${TARGET_TRIPLE}/release/${APP_NAME}.exe"
  [ -f "$cli_src" ] || die "CLI binary not found at ${cli_src}"

  if command -v pwsh >/dev/null 2>&1; then
    CLI_SRC="$cli_src" WINDOWS_CERT_THUMBPRINT="$WINDOWS_CERT_THUMBPRINT" \
      pwsh -NoProfile -Command "\$signtool = (Get-Command signtool.exe -ErrorAction Stop).Source; & \$signtool sign /sha1 \$env:WINDOWS_CERT_THUMBPRINT /fd SHA256 /tr http://timestamp.digicert.com /td SHA256 \$env:CLI_SRC"
  else
    CLI_SRC="$cli_src" WINDOWS_CERT_THUMBPRINT="$WINDOWS_CERT_THUMBPRINT" \
      powershell.exe -NoProfile -Command "\$signtool = (Get-Command signtool.exe -ErrorAction Stop).Source; & \$signtool sign /sha1 \$env:WINDOWS_CERT_THUMBPRINT /fd SHA256 /tr http://timestamp.digicert.com /td SHA256 \$env:CLI_SRC"
  fi

  log_info "Signed ${cli_src}"
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

  # BtbN provides GPL static Windows builds at a stable "latest" URL.
  # Filename format (as of 2025): ffmpeg-master-latest-<arch>-gpl.zip
  # Previously was ffmpeg-latest-<arch>-static.zip (no longer exists).
  local FFMPEG_TAG="${GESTURA_FFMPEG_VERSION:-latest}"
  # Map Rust triples to BtbN arch labels.
  local btbn_arch
  case "$TARGET_TRIPLE" in
    x86_64-*)  btbn_arch="win64" ;;
    i686-*)    btbn_arch="win32" ;;
    aarch64-*) btbn_arch="arm64" ;;
    *) die "No BtbN ffmpeg build known for triple: ${TARGET_TRIPLE}" ;;
  esac

  local url="https://github.com/BtbN/FFmpeg-Builds/releases/download/${FFMPEG_TAG}/ffmpeg-master-${FFMPEG_TAG}-${btbn_arch}-gpl.zip"

  log_info "Downloading static ffmpeg (${btbn_arch}) from BtbN/FFmpeg-Builds …"

  local tmp
  tmp="$(mktemp -d)"
  trap 'if [ -n "${tmp:-}" ]; then rm -rf -- "$tmp"; fi' RETURN

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

# collect_artifacts copies the MSI output and standalone CLI archive into the dist
# dir with canonical names.
collect_artifacts() {
  local out_dir
  out_dir="$(ensure_fresh_dist_dir "$DIST_DIR")"

  local bundle_root1="target/${TARGET_TRIPLE}/release/bundle"
  local bundle_root2="target/release/bundle"
  local bundle_root3="${GUI_DIR}/target/${TARGET_TRIPLE}/release/bundle"
  local bundle_root4="${GUI_DIR}/target/release/bundle"

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
  if [ -z "$msi_src" ]; then
    msi_src="$(ls -1 "${bundle_root3}/msi/"*.msi 2>/dev/null | head -1 || true)"
  fi
  if [ -z "$msi_src" ]; then
    msi_src="$(ls -1 "${bundle_root3}/wix/"*.msi 2>/dev/null | head -1 || true)"
  fi
  if [ -z "$msi_src" ]; then
    msi_src="$(ls -1 "${bundle_root4}/msi/"*.msi 2>/dev/null | head -1 || true)"
  fi
  if [ -z "$msi_src" ]; then
    msi_src="$(ls -1 "${bundle_root4}/wix/"*.msi 2>/dev/null | head -1 || true)"
  fi

  if [ -n "$msi_src" ]; then
    cp "$msi_src" "${out_dir}/${APP_DISPLAY_NAME}-${TAG}-windows-${ARCH_LABEL}.msi"
    log_info "Wrote ${out_dir}/${APP_DISPLAY_NAME}-${TAG}-windows-${ARCH_LABEL}.msi"
  else
    log_warn "No .msi found under bundle output"
  fi

  local cli_src="target/${TARGET_TRIPLE}/release/${APP_NAME}.exe"
  local cli_archive="${out_dir}/${APP_NAME}-cli-${TAG}-windows-${ARCH_LABEL}.zip"
  [ -f "$cli_src" ] || die "CLI binary not found at ${cli_src}"

  if command -v pwsh >/dev/null 2>&1; then
    CLI_SRC="$cli_src" CLI_ARCHIVE="$cli_archive" \
      pwsh -NoProfile -Command "Compress-Archive -Path \$env:CLI_SRC -DestinationPath \$env:CLI_ARCHIVE -Force"
  elif command -v powershell.exe >/dev/null 2>&1; then
    CLI_SRC="$cli_src" CLI_ARCHIVE="$cli_archive" \
      powershell.exe -NoProfile -Command "Compress-Archive -Path \$env:CLI_SRC -DestinationPath \$env:CLI_ARCHIVE -Force"
  else
    die "Missing required command: pwsh or powershell.exe"
  fi

  log_info "Wrote ${cli_archive}"

  write_sha256sums "$out_dir" "${APP_NAME}-${TAG}-SHA256SUMS.txt"
  if [ -f "${out_dir}/${APP_NAME}-${TAG}-SHA256SUMS.txt" ]; then
    log_info "Wrote ${out_dir}/${APP_NAME}-${TAG}-SHA256SUMS.txt"
  fi
}

# main is the entrypoint.
main() {
  trap restore_tauri_windows_signing EXIT

  check_prerequisites
  build_frontend
  stage_cli
  sign_cli
  stage_ffmpeg
  patch_tauri_windows_signing
  build_gui
  collect_artifacts
}

main "$@"
