#!/usr/bin/env bash
#
# Gestura installer (macOS/Linux)
#
# This script installs Gestura from GitHub Releases in one of two modes:
#   - full (default): installs GUI + CLI using native packages (PKG/DEB/RPM)
#   - cli: installs only the CLI binary
#
# Usage examples:
#   curl -fsSL https://raw.githubusercontent.com/gestura-ai/gestura-app/main/install/install.sh | bash
#   curl -fsSL https://raw.githubusercontent.com/gestura-ai/gestura-app/main/install/install.sh | bash -s -- --mode cli
#   ./install.sh --tag v0.2.0 --mode full
#
set -euo pipefail
IFS=$'\n\t'

MODE="full"
REPO="gestura-ai/gestura-app"
TAG=""
PKG_FORMAT="auto" # linux only: auto|deb|rpm
NO_VERIFY="0"
REQUIRE_VERIFY="0"
DRY_RUN="0"
INSTALL_DIR="" # cli-only

log() {
  printf '%s\n' "[gestura-install] $*" >&2
}

die() {
  log "ERROR: $*"
  exit 1
}

have() {
  command -v "$1" >/dev/null 2>&1
}

usage() {
  cat <<'EOF'
Gestura installer (macOS/Linux)

Options:
  --mode <full|cli>        Install mode. Default: full
  --tag <vX.Y.Z>           Install a specific release tag. Default: latest
  --repo <owner/repo>      GitHub repo to download from. Default: gestura-ai/gestura-app
  --pkg-format <auto|deb|rpm>  Linux full-install package preference. Default: auto
  --install-dir <dir>      CLI-only install directory. Default: /usr/local/bin or ~/.local/bin
  --no-verify              Skip SHA-256 verification.
  --require-verify         Fail if checksum file is missing/unavailable.
  --dry-run                Print what would happen without installing.
  -h, --help               Show help.

Notes:
  - Full installs require elevated privileges (sudo/root) to install system packages.
  - CLI-only installs default to a user-writable bin directory when possible.
EOF
}

parse_args() {
  while [[ $# -gt 0 ]]; do
    case "$1" in
      --mode)
        MODE="${2:-}"; shift 2 ;;
      --tag)
        TAG="${2:-}"; shift 2 ;;
      --repo)
        REPO="${2:-}"; shift 2 ;;
      --pkg-format)
        PKG_FORMAT="${2:-}"; shift 2 ;;
      --install-dir)
        INSTALL_DIR="${2:-}"; shift 2 ;;
      --no-verify)
        NO_VERIFY="1"; shift ;;
      --require-verify)
        REQUIRE_VERIFY="1"; shift ;;
      --dry-run)
        DRY_RUN="1"; shift ;;
      -h|--help)
        usage; exit 0 ;;
      *)
        die "Unknown argument: $1" ;;
    esac
  done

  case "$MODE" in
    full|cli) ;;
    *) die "--mode must be 'full' or 'cli' (got '$MODE')" ;;
  esac
  case "$PKG_FORMAT" in
    auto|deb|rpm) ;;
    *) die "--pkg-format must be auto|deb|rpm (got '$PKG_FORMAT')" ;;
  esac
}

detect_os() {
  local uname_s
  uname_s="$(uname -s)"
  case "$uname_s" in
    Darwin) printf '%s' "macos" ;;
    Linux) printf '%s' "linux" ;;
    *) die "Unsupported OS: $uname_s" ;;
  esac
}

detect_arch() {
  local uname_m
  uname_m="$(uname -m)"
  case "$uname_m" in
    x86_64|amd64) printf '%s' "x86_64" ;;
    arm64|aarch64) printf '%s' "arm64" ;;
    *) die "Unsupported architecture: $uname_m" ;;
  esac
}

get_latest_tag() {
  have curl || die "curl is required"
  local url
  url="https://api.github.com/repos/${REPO}/releases/latest"
  curl -fsSL "$url" | sed -n 's/.*"tag_name"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p' | head -n 1
}

download_to() {
  # download_to <url> <dest>
  local url dest
  url="$1"; dest="$2"
  if have curl; then
    curl -fL --retry 3 --retry-delay 1 -o "$dest" "$url"
  elif have wget; then
    wget -O "$dest" "$url"
  else
    die "Either curl or wget is required"
  fi
}

try_download_assets() {
  # try_download_assets <tag> <out_dir> <asset1> [asset2 ...]
  local tag out_dir
  tag="$1"; out_dir="$2"; shift 2
  local asset url dest
  for asset in "$@"; do
    url="https://github.com/${REPO}/releases/download/${tag}/${asset}"
    dest="${out_dir}/${asset}"
    log "Downloading ${url}"
    if download_to "$url" "$dest" 2>/dev/null; then
      printf '%s' "$dest"
      return 0
    fi
  done
  return 1
}

sha256_of_file() {
  # sha256_of_file <path>
  local path
  path="$1"
  if have sha256sum; then
    sha256sum "$path" | awk '{print $1}'
  elif have shasum; then
    shasum -a 256 "$path" | awk '{print $1}'
  else
    die "Need sha256sum (Linux) or shasum (macOS) for verification"
  fi
}

maybe_verify() {
  # maybe_verify <tag> <work_dir> <asset_path>
  local tag work_dir asset_path asset_name sums_path expected actual
  tag="$1"; work_dir="$2"; asset_path="$3"
  asset_name="$(basename "$asset_path")"

  if [[ "$NO_VERIFY" == "1" ]]; then
    log "Skipping verification (--no-verify)"
    return 0
  fi

  sums_path="${work_dir}/gestura-${tag}-SHA256SUMS.txt"
  if ! download_to "https://github.com/${REPO}/releases/download/${tag}/gestura-${tag}-SHA256SUMS.txt" "$sums_path" 2>/dev/null; then
    if [[ "$REQUIRE_VERIFY" == "1" ]]; then
      die "Checksum file missing and --require-verify set"
    fi
    log "WARN: checksum file not available for ${tag}; proceeding without verification"
    return 0
  fi

  expected="$(grep -E "[[:space:]]${asset_name}$" "$sums_path" | awk '{print $1}' | head -n 1 || true)"
  if [[ -z "$expected" ]]; then
    if [[ "$REQUIRE_VERIFY" == "1" ]]; then
      die "Checksum entry for ${asset_name} missing and --require-verify set"
    fi
    log "WARN: checksum entry for ${asset_name} missing; proceeding without verification"
    return 0
  fi

  actual="$(sha256_of_file "$asset_path")"
  if [[ "$actual" != "$expected" ]]; then
    die "Checksum mismatch for ${asset_name}: expected ${expected}, got ${actual}"
  fi
  log "Verified SHA-256 for ${asset_name}"
}

default_cli_install_dir() {
  if [[ -n "$INSTALL_DIR" ]]; then
    printf '%s' "$INSTALL_DIR"
    return 0
  fi

  if [[ -w "/usr/local/bin" ]]; then
    printf '%s' "/usr/local/bin"
  else
    printf '%s' "${HOME}/.local/bin"
  fi
}

install_cli_binary() {
  # install_cli_binary <src_exe> <dest_dir>
  local src_exe dest_dir
  src_exe="$1"; dest_dir="$2"
  mkdir -p "$dest_dir"
  if [[ "$DRY_RUN" == "1" ]]; then
    log "DRY RUN: would install ${src_exe} -> ${dest_dir}/gestura"
    return 0
  fi
  install -m 0755 "$src_exe" "${dest_dir}/gestura"
  log "Installed CLI to ${dest_dir}/gestura"
}

install_cli_from_archive() {
  # install_cli_from_archive <archive_path>
  local archive_path tmp_extract dest_dir
  archive_path="$1"
  dest_dir="$(default_cli_install_dir)"
  tmp_extract="$(mktemp -d)"
  tar -xzf "$archive_path" -C "$tmp_extract"
  [[ -f "${tmp_extract}/gestura" ]] || die "Archive did not contain 'gestura'"
  install_cli_binary "${tmp_extract}/gestura" "$dest_dir"

  if [[ "$dest_dir" == "${HOME}/.local/bin" ]]; then
    log "Note: ensure ${dest_dir} is on your PATH (e.g., add 'export PATH=\"${dest_dir}:$PATH\"')"
  fi
}

install_full_macos() {
  # install_full_macos <pkg_path>
  local pkg_path
  pkg_path="$1"
  if [[ "$DRY_RUN" == "1" ]]; then
    log "DRY RUN: would run: sudo installer -pkg ${pkg_path} -target /"
    return 0
  fi
  if [[ "$(id -u)" -ne 0 ]]; then
    have sudo || die "sudo required for full install"
    sudo installer -pkg "$pkg_path" -target /
  else
    installer -pkg "$pkg_path" -target /
  fi
}

install_full_linux() {
  # install_full_linux <pkg_path> <format>
  local pkg_path format
  pkg_path="$1"; format="$2"
  if [[ "$DRY_RUN" == "1" ]]; then
    log "DRY RUN: would install ${pkg_path} (${format})"
    return 0
  fi
  if [[ "$(id -u)" -ne 0 ]]; then
    have sudo || die "sudo required for full install"
  fi

  case "$format" in
    deb)
      have dpkg || die "dpkg not found (cannot install .deb)"
      if [[ "$(id -u)" -ne 0 ]]; then sudo dpkg -i "$pkg_path"; else dpkg -i "$pkg_path"; fi
      ;;
    rpm)
      have rpm || die "rpm not found (cannot install .rpm)"
      if [[ "$(id -u)" -ne 0 ]]; then sudo rpm -Uvh "$pkg_path"; else rpm -Uvh "$pkg_path"; fi
      ;;
    *)
      die "Unknown linux package format: $format"
      ;;
  esac
}

main() {
  parse_args "$@"

  local os arch tag work_dir asset_path
  os="$(detect_os)"
  arch="$(detect_arch)"

  # Current release contract only guarantees x86_64 assets for Linux/Windows.
  if [[ "$os" == "linux" && "$arch" != "x86_64" ]]; then
    die "Linux ${arch} is not supported by the current release artifact contract"
  fi

  if [[ -z "$TAG" ]]; then
    tag="$(get_latest_tag)"
  else
    tag="$TAG"
  fi
  [[ -n "$tag" ]] || die "Could not determine release tag"
  log "Using release tag: ${tag}"

  work_dir="$(mktemp -d)"

  if [[ "$MODE" == "cli" ]]; then
    case "$os" in
      macos)
        asset_path="$(try_download_assets "$tag" "$work_dir" \
          "gestura-cli-${tag}-macos-universal.tar.gz" \
          "gestura-cli-macos-universal")" || die "Unable to download CLI asset"
        maybe_verify "$tag" "$work_dir" "$asset_path"
        if [[ "$(basename "$asset_path")" == "gestura-cli-macos-universal" ]]; then
          install_cli_binary "$asset_path" "$(default_cli_install_dir)"
        else
          install_cli_from_archive "$asset_path"
        fi
        ;;
      linux)
        asset_path="$(try_download_assets "$tag" "$work_dir" \
          "gestura-cli-${tag}-linux-x86_64.tar.gz" \
          "gestura-cli-linux-x86_64")" || die "Unable to download CLI asset"
        maybe_verify "$tag" "$work_dir" "$asset_path"
        if [[ "$(basename "$asset_path")" == "gestura-cli-linux-x86_64" ]]; then
          install_cli_binary "$asset_path" "$(default_cli_install_dir)"
        else
          install_cli_from_archive "$asset_path"
        fi
        ;;
    esac
    log "Done. Try: gestura --help"
    return 0
  fi

  # full
  case "$os" in
    macos)
      asset_path="$(try_download_assets "$tag" "$work_dir" "Gestura-${tag}-universal.pkg")" \
        || die "Unable to download full installer (PKG)"
      maybe_verify "$tag" "$work_dir" "$asset_path"
      install_full_macos "$asset_path"
      ;;
    linux)
      local chosen_format
      chosen_format="$PKG_FORMAT"
      if [[ "$chosen_format" == "auto" ]]; then
        if have dpkg; then
          chosen_format="deb"
        elif have rpm; then
          chosen_format="rpm"
        else
          die "Neither dpkg nor rpm found; cannot choose Linux package type"
        fi
      fi
      if [[ "$chosen_format" == "deb" ]]; then
        asset_path="$(try_download_assets "$tag" "$work_dir" "gestura-${tag}-linux-x86_64.deb" "gestura-${tag}-linux-x86_64.rpm")" \
          || die "Unable to download full installer (.deb/.rpm)"
      else
        asset_path="$(try_download_assets "$tag" "$work_dir" "gestura-${tag}-linux-x86_64.rpm" "gestura-${tag}-linux-x86_64.deb")" \
          || die "Unable to download full installer (.rpm/.deb)"
      fi
      maybe_verify "$tag" "$work_dir" "$asset_path"
      if [[ "$asset_path" == *.deb ]]; then
        install_full_linux "$asset_path" "deb"
      else
        install_full_linux "$asset_path" "rpm"
      fi
      ;;
  esac

  log "Done. GUI should be installed; CLI should be available as 'gestura'."
}

main "$@"
