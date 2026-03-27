#!/usr/bin/env bash
# shellcheck shell=bash
#
# Applies a custom icon to a mounted macOS volume.

set -euo pipefail

usage() {
  cat <<'EOF'
Usage:
  scripts/packaging/set-macos-volume-icon.sh <icon.icns> <mounted-volume-path>

Copies the icon to .VolumeIcon.icns and marks the mounted volume as having a
custom icon.
EOF
}

if [ "$#" -ne 2 ]; then
  usage >&2
  exit 64
fi

ICON_SOURCE="$1"
VOLUME_PATH="$2"
VOLUME_ICON_PATH="${VOLUME_PATH}/.VolumeIcon.icns"

[ -f "$ICON_SOURCE" ] || {
  printf '[error] Volume icon source not found: %s\n' "$ICON_SOURCE" >&2
  exit 1
}

[ -d "$VOLUME_PATH" ] || {
  printf '[error] Mounted volume path not found: %s\n' "$VOLUME_PATH" >&2
  exit 1
}

for cmd in cp SetFile; do
  command -v "$cmd" >/dev/null 2>&1 || {
    printf '[error] Missing required command: %s\n' "$cmd" >&2
    exit 1
  }
done

cp "$ICON_SOURCE" "$VOLUME_ICON_PATH"
SetFile -a C "$VOLUME_PATH"
SetFile -a V "$VOLUME_ICON_PATH"

printf '[info] Applied branded volume icon to %s\n' "$VOLUME_PATH"