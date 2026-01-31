# shellcheck shell=bash
#
# Common helper functions shared by local packaging scripts.
#
# Note: This file is sourced. It intentionally does NOT change shell options
# (e.g., `set -euo pipefail`) so that callers remain in control.

# log_info prints an informational message.
log_info() {
  printf "[info] %s\n" "$*"
}

# log_warn prints a warning message.
log_warn() {
  printf "[warn] %s\n" "$*" >&2
}

# log_error prints an error message.
log_error() {
  printf "[error] %s\n" "$*" >&2
}

# die prints an error message and exits non-zero.
die() {
  log_error "$*"
  exit 1
}

# require_cmd verifies a command exists on PATH.
require_cmd() {
  local cmd="$1"
  command -v "$cmd" >/dev/null 2>&1 || die "Missing required command: ${cmd}"
}

# repo_root prints the git repository root.
repo_root() {
  git rev-parse --show-toplevel 2>/dev/null
}

# tauri_conf_version prints the version from a Tauri config JSON file.
tauri_conf_version() {
  local conf_path="$1"

  python3 - "$conf_path" <<'PY'
import json
import sys

path = sys.argv[1]
with open(path, 'r', encoding='utf-8') as f:
    data = json.load(f)

print(data.get('version', '0.0.0'))
PY
}

# resolve_tag determines the release tag string used for artifact naming.
#
# Precedence:
#  1) TAG environment variable (e.g., v0.2.0)
#  2) Most recent git tag (`git describe --tags --abbrev=0`)
#  3) crates/gestura-gui/tauri.conf.json "version" prefixed with "v"
#
# Sets global variables:
#  - TAG:         e.g. v0.2.0
#  - VERSION_NUM: e.g. 0.2.0
resolve_tag() {
  if [ -z "${TAG:-}" ]; then
    TAG="$(git describe --tags --abbrev=0 2>/dev/null || true)"
  fi

  if [ -z "${TAG:-}" ]; then
    require_cmd python3
    local root
    root="$(repo_root)"
    [ -n "$root" ] || die "Not in a git repository; set TAG=vX.Y.Z"
    TAG="v$(tauri_conf_version "$root/crates/gestura-gui/tauri.conf.json")"
  fi

  VERSION_NUM="${TAG#v}"
}

# ensure_fresh_dist_dir creates a dist directory.
#
# If the directory already exists, it returns a timestamp-suffixed directory to
# avoid permission issues from prior runs (e.g., if a previous run used sudo).
ensure_fresh_dist_dir() {
  local dist_dir="$1"
  if [ -d "$dist_dir" ]; then
    local ts
    ts="$(date +"%Y%m%d-%H%M%S")"
    log_warn "Existing dist directory (${dist_dir}) detected; using ${dist_dir}-${ts}"
    dist_dir="${dist_dir}-${ts}"
  fi
  mkdir -p "$dist_dir"
  printf "%s" "$dist_dir"
}

# write_sha256sums writes a SHA256SUMS-style manifest file for files in a directory.
#
# Arguments:
#  1) out_dir          Directory to scan.
#  2) manifest_name    Manifest filename to write (e.g., gestura-v0.2.0-SHA256SUMS.txt).
#
# The format matches GNU coreutils `sha256sum` output:
#   <sha256>  <filename>
write_sha256sums() {
  local out_dir="$1"
  local manifest_name="$2"

  (cd "$out_dir" || exit 1

    # Use nullglob so "*" expands to empty if there are no files.
    shopt -s nullglob

    local files=()
    local f
    for f in *; do
      [ -f "$f" ] || continue
      [ "$f" = "$manifest_name" ] && continue
      files+=("$f")
    done

    if [ ${#files[@]} -eq 0 ]; then
      log_warn "No files found in ${out_dir}; skipping checksums manifest"
      exit 0
    fi

    if command -v sha256sum >/dev/null 2>&1; then
      sha256sum "${files[@]}" > "$manifest_name"
    elif command -v shasum >/dev/null 2>&1; then
      shasum -a 256 "${files[@]}" > "$manifest_name"
    else
      log_warn "Neither sha256sum nor shasum is available; skipping checksums manifest"
      exit 0
    fi
  )
}
