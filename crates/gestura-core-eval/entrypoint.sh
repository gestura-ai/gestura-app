#!/bin/bash
# ─────────────────────────────────────────────────────────────────────────────
# entrypoint.sh — configure agent CLIs from runtime env vars, then exec
#                 gestura-eval with any arguments passed to the container.
#
# Problem: Codex CLI v0.120.0 reads its OpenAI API key from
#   ~/.codex/config.toml, not from the OPENAI_API_KEY environment variable.
#   The env var controls provider selection, but the actual credential used
#   in the Responses API WebSocket Authorization header comes from the config
#   file.  In a fresh container that file does not exist, so every request
#   gets a 401 "Missing bearer or basic authentication" even when the env var
#   is correctly set.
#
# Fix: write a minimal config.toml from the env var before handing off to
#   gestura-eval.  Only creates/overwrites the file when the env var is
#   non-empty, so authenticated runs are not broken by empty env vars.
# ─────────────────────────────────────────────────────────────────────────────
set -euo pipefail

# ── Codex: write ~/.codex/config.toml from env vars ──────────────────────────
# Codex v0.120.0 defaults to ChatGPT OAuth auth and ignores OPENAI_API_KEY
# for the Responses API WebSocket handshake unless two config keys are set:
#
#   forced_login_method = "api"       → use API key, not OAuth/ChatGPT login
#   cli_auth_credentials_store = "file" → avoid keychain (not available in containers)
#   openai_api_key = "sk-..."          → the actual key (belt-and-suspenders;
#                                        OPENAI_API_KEY env var is the primary source
#                                        once forced_login_method="api" is active)
#
# The args_prefix in each codex-*.toml also passes the first two via -c flags,
# but writing them here ensures any Codex startup code that reads config before
# processing CLI args also sees the correct values.
if [[ -n "${OPENAI_API_KEY:-}" ]]; then
    mkdir -p "${HOME}/.codex"
    cat > "${HOME}/.codex/config.toml" << TOML
forced_login_method = "api"
cli_auth_credentials_store = "file"
openai_api_key = "${OPENAI_API_KEY}"
TOML
fi

exec gestura-eval "$@"

