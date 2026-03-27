#!/usr/bin/env bash
# shellcheck shell=bash
#
# Applies a branded Finder custom icon to a file using a raster image source.
#
# This is used for macOS distribution artifacts such as flat .pkg installers.

set -euo pipefail

usage() {
  cat <<'EOF'
Usage:
  scripts/packaging/set-macos-custom-icon.sh <icon.png> <target-file>

Applies a macOS Finder custom icon to the target file.
EOF
}

if [ "$#" -ne 2 ]; then
  usage >&2
  exit 64
fi

ICON_SOURCE="$1"
TARGET_PATH="$2"

[ -f "$ICON_SOURCE" ] || {
  printf '[error] Icon source not found: %s\n' "$ICON_SOURCE" >&2
  exit 1
}

[ -f "$TARGET_PATH" ] || {
  printf '[error] Target file not found: %s\n' "$TARGET_PATH" >&2
  exit 1
}

for cmd in cp sips DeRez Rez SetFile; do
  command -v "$cmd" >/dev/null 2>&1 || {
    printf '[error] Missing required command: %s\n' "$cmd" >&2
    exit 1
  }
done

tmp_dir="$(mktemp -d "${TMPDIR:-/tmp}/gestura-custom-icon.XXXXXX")"
cleanup() {
  rm -rf "$tmp_dir"
}
trap cleanup EXIT

icon_copy="${tmp_dir}/icon.png"
icon_resource="${tmp_dir}/icon.rsrc"

cp "$ICON_SOURCE" "$icon_copy"
sips -i "$icon_copy" >/dev/null
DeRez -only icns "$icon_copy" > "$icon_resource"

[ -s "$icon_resource" ] || {
  printf '[error] Failed to extract icon resource from: %s\n' "$ICON_SOURCE" >&2
  exit 1
}

Rez -append "$icon_resource" -o "$TARGET_PATH"
SetFile -a C "$TARGET_PATH"

printf '[info] Applied branded Finder icon to %s\n' "$TARGET_PATH"