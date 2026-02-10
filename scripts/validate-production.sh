#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
FRONTEND_DIR="$ROOT_DIR/crates/gestura-gui/frontend"

RUN_RUST=1
RUN_FRONTEND=1
CI_MODE=0
PLAYWRIGHT_INSTALL=1

usage() {
  cat <<'EOF'
Usage: scripts/validate-production.sh [options]

Runs a production-oriented validation pass that mirrors Gestura's CI quality gates.

Options:
  --ci                    Use CI-like behavior (e.g. npm ci; Playwright install).
  --skip-rust              Skip Rust workspace checks.
  --skip-frontend          Skip frontend checks (lint/build/e2e).
  --no-playwright-install  Skip Playwright browser installation.
  -h, --help               Show this help.
EOF
}

log() { printf "\n==> %s\n" "$*"; }

while [[ $# -gt 0 ]]; do
  case "$1" in
    --ci) CI_MODE=1; shift ;;
    --skip-rust) RUN_RUST=0; shift ;;
    --skip-frontend) RUN_FRONTEND=0; shift ;;
    --no-playwright-install) PLAYWRIGHT_INSTALL=0; shift ;;
    -h|--help) usage; exit 0 ;;
    *) echo "Unknown option: $1"; usage; exit 2 ;;
  esac
done

cd "$ROOT_DIR"

if [[ "$RUN_RUST" -eq 1 ]]; then
  log "Rust: format (check)"
  cargo fmt --all -- --check

  log "Rust: clippy (workspace, all targets, all features; deny warnings)"
  cargo clippy --workspace --all-targets --all-features -- -D warnings

  log "Rust: tests (workspace, all features)"
  cargo test --workspace --all-features
fi

if [[ "$RUN_FRONTEND" -eq 1 ]]; then
  if [[ ! -d "$FRONTEND_DIR" ]]; then
    echo "Frontend directory not found: $FRONTEND_DIR" >&2
    exit 1
  fi

  log "Frontend: lint/build/e2e (cwd: crates/gestura-gui/frontend)"
  cd "$FRONTEND_DIR"

  if [[ "$CI_MODE" -eq 1 ]]; then
    log "Frontend: install dependencies (npm ci)"
    npm ci
  fi

  log "Frontend: lint"
  npm run lint

  log "Frontend: build (tsc + Vite)"
  npm run build

  log "Frontend: unit tests (Vitest)"
  npm run test:unit

  if [[ "$PLAYWRIGHT_INSTALL" -eq 1 ]]; then
    if [[ "$(uname -s)" == "Linux" ]]; then
      log "Frontend: install Playwright Chromium (Linux)"
      if [[ "$CI_MODE" -eq 1 ]]; then
        npx playwright install --with-deps chromium
      else
        npx playwright install chromium
      fi
    else
      log "Frontend: install Playwright Chromium"
      npx playwright install chromium
    fi
  fi

  log "Frontend: e2e smoke (Playwright)"
  npm run test:e2e:smoke
fi

log "✅ Production validation complete"
