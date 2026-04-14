# gestura-core-eval

Standalone evaluation harness for the Gestura agentic response quality. Drives any supported
agent CLI as a **black-box subprocess** across 8 standardised test scenarios and produces
structured pass/fail reports with a per-variation score.

The crate is intentionally separate from the `gestura` product binary so evaluation logic
never ships in production, and the same scenarios can be reproduced across different agentic
interfaces without changing a line of Rust.

---

## Contents

- [How it works](#how-it-works)
- [Quick start](#quick-start)
- [Container](#container)
- [CLI reference](#cli-reference)
- [Test scenarios](#test-scenarios)
- [Agent profiles](#agent-profiles)
- [API keys and credentials](#api-keys-and-credentials)
  - [Gestura profiles](#1-gestura-profiles)
  - [Claude Code profiles](#2-claude-code-profiles)
  - [Augment profiles](#3-augment-profiles)
  - [Codex profiles](#4-codex-profiles)
  - [OpenCode profiles](#5-opencode-profiles)
  - [Credential precedence and .env pattern](#credential-precedence-and-env-pattern)
  - [Dry-run — no credentials needed](#dry-run--no-credentials-needed)
- [Custom agent profiles](#custom-agent-profiles)
- [Output formats](#output-formats)
- [Crate structure](#crate-structure)

---

## How it works

```
gestura-eval
    │
    ├── loads agent profile (TOML) ──► binary + args_prefix + env + thresholds
    │
    ├── for each scenario variation:
    │       └── spawn subprocess: <bin> [args_prefix...] "<prompt>"
    │               └── capture stdout
    │
    ├── RuleEvaluator ──► deterministic keyword / length / pattern checks
    │
    └── EvalReport ──► text table or JSON
```

No LLM judge. Every check is a pure deterministic function over the response text, so the
harness is reproducible offline and in CI without additional API quota.

---

## Container

The evaluation harness ships a multi-stage `Containerfile` that produces a self-contained
image with all agent CLIs (`claude`, `codex`, `opencode`) and the built `gestura` and
`gestura-eval` binaries. Credentials are never baked into the image — they are passed at
runtime via environment variables.

**Files:**

```
crates/gestura-core-eval/
├── Containerfile       — three-stage build (cargo-chef → builder → runtime)
└── .containerignore    — excludes target/, .env, .git, node_modules, etc.
```

**Build** (run from repo root):

```bash
# Podman
podman build \
  -f crates/gestura-core-eval/Containerfile \
  --ignorefile crates/gestura-core-eval/.containerignore \
  -t gestura-eval .

# Docker (BuildKit required for --ignorefile equivalent)
docker build \
  -f crates/gestura-core-eval/Containerfile \
  -t gestura-eval .
```

**Run with inline credentials:**

> **Gestura uses a `GESTURA_` prefix** for all its env vars. Pass `GESTURA_ANTHROPIC_API_KEY`
> for `gestura-*` profiles **and** bare `ANTHROPIC_API_KEY` for `claude-code-*` / `opencode-*`.

```bash
# gestura-* profiles — needs the GESTURA_ prefixed var
docker run --rm \
  -e GESTURA_ANTHROPIC_API_KEY=sk-ant-... \
  gestura-eval --agent gestura-full

# claude-code-* / opencode-* — needs bare ANTHROPIC_API_KEY
docker run --rm \
  -e ANTHROPIC_API_KEY=sk-ant-... \
  gestura-eval --agent claude-code-full

# codex-* — needs bare OPENAI_API_KEY
docker run --rm \
  -e OPENAI_API_KEY=sk-... \
  gestura-eval --agent codex-full
```

**Run with an `.env` file** (keep credentials out of shell history):

```bash
# .env  — never commit this file
# GESTURA_ANTHROPIC_API_KEY=sk-ant-...   # for gestura-* profiles
# ANTHROPIC_API_KEY=sk-ant-...           # for claude-code-*, opencode-*
# OPENAI_API_KEY=sk-...                  # for codex-*

docker run --rm --env-file .env gestura-eval --agent gestura-full
docker run --rm --env-file .env gestura-eval --agent claude-code-full
docker run --rm --env-file .env gestura-eval --agent codex-full --dry-run --json
```

**All supported runtime env vars:**

| Variable | Required for | Description |
|---|---|---|
| `GESTURA_ANTHROPIC_API_KEY` | gestura-\* | **Required for Gestura.** Gestura reads all config via `GESTURA_` prefix |
| `ANTHROPIC_API_KEY` | claude-code-\*, opencode-\* | **Required.** Bare key read directly by those CLIs (no prefix) |
| `OPENAI_API_KEY` | codex-\* | **Required for Codex.** Bare key read directly by Codex CLI |
| `AUGMENT_SESSION_AUTH` | augment-\* | OAuth session JSON from `auggie token print`; excluded from automated runs |
| `AUGMENT_DISABLE_AUTO_UPDATE` | augment-\* | Pre-set to `1`; disables self-update checks in CI |
| `GESTURA_GROK_API_KEY` | custom gestura profile with `llm.primary=grok` | Grok / xAI key (`xai-...`) |
| `GESTURA_GEMINI_API_KEY` | custom gestura profile with `llm.primary=gemini` | Google Gemini API key |
| `GESTURA_DISABLE_KEYCHAIN` | gestura-\* | Pre-set to `1` in the image; prevents keychain hangs |
| `RUST_LOG` | gestura-eval binary | Tracing verbosity; defaults to `warn` (stderr only) |

> **`augment-*` profiles — Auggie auth:** Auggie uses an account session token, not a raw API key.
> 1. Install: `npm install -g @augmentcode/auggie` (Node.js 22+ required)
> 2. Log in once on a real machine: `auggie login`
> 3. Export the session: `export AUGMENT_SESSION_AUTH=$(auggie token print)`
> 4. Pass it to the container: `podman run --rm -e AUGMENT_SESSION_AUTH="$AUGMENT_SESSION_AUTH" gestura-eval --agent augment-full`

---

## Quick start

```bash
# Build the eval binary and the gestura CLI
cargo build -p gestura-cli -p gestura-core-eval

# List scenario IDs
./target/debug/gestura-eval --list

# List all built-in agent profiles
./target/debug/gestura-eval --list-agents

# Dry-run — validates check logic, no subprocess calls, no credentials needed
./target/debug/gestura-eval --dry-run

# Full run with the default gestura-full profile
export ANTHROPIC_API_KEY=sk-ant-...
./target/debug/gestura-eval

# Run against a specific agent profile
./target/debug/gestura-eval --agent claude-code-full

# Single scenario, JSON output
./target/debug/gestura-eval --agent codex-sandboxed --scenario s3_planning --json

# Load a custom profile from disk
./target/debug/gestura-eval --config /path/to/my-agent.toml

# Save a machine-readable report
./target/debug/gestura-eval --agent gestura-full --json > reports/gestura-full.json
```

---

## CLI reference

| Flag | Description |
|---|---|
| `--agent <ID>` | Built-in agent profile to use (default: `gestura-full`) |
| `--config <PATH>` | Load a custom TOML profile from disk instead of a built-in |
| `--bin <PATH>` | Override the agent binary path regardless of what the profile specifies |
| `--scenario <ID>` | Run a single scenario only (use `--list` to see IDs) |
| `--dry-run` | Skip subprocess calls; run check logic on stub responses |
| `--list` | Print scenario IDs and exit |
| `--list-agents` | Print built-in agent profile IDs, modes, and descriptions, then exit |
| `--json` | Emit JSON output instead of the human-readable text report |
| `--quiet` / `-q` | Suppress progress output (implies JSON) |

Environment variables:

| Variable | Description |
|---|---|
| `GESTURA_EVAL_AGENT` | Default agent ID when `--agent` is not supplied |
| `GESTURA_BIN` | Default binary path when `--bin` is not supplied |
| `RUST_LOG` | Tracing filter; defaults to `warn` (stderr only) |

---

## Test scenarios

Eight scenarios × three prompt variations each = 24 total invocations per run.

| ID | Name | What is tested |
|---|---|---|
| `s1_simple_query` | Simple Single-Turn Query | Direct factual answer, no padding, uncertainty acknowledged where appropriate |
| `s2_multi_turn` | Multi-Turn Conversation | Context retention and coherence across a four-turn history |
| `s3_planning` | Complex Multi-Step Planning | Structured output, explicit assumptions, no fabricated specifics |
| `s4_error_handling` | Error Handling and Verification | Root cause explained, fix provided, test suggested |
| `s5_tool_extensibility` | Tool Calling and Extensibility | Full definition → registration → invocation chain, no fabricated live output |
| `s6_privacy` | Privacy-Sensitive Local Task | No external API suggestions; honest about data access model |
| `s7_context_retention` | Context Retention | Recalls only facts from the provided set; never invents unlisted details |
| `s8_long_context` | Long-Context Coherence | Grounds answer in document text; distinguishes explicit from inferred |

Scenario definitions live in `testdata/scenarios.json`. Each variation declares the prompt,
expected keywords, word-count bounds, and the named checks that must pass.

---

## Agent profiles

Built-in profiles are TOML files embedded in the binary at compile time via `include_str!`.
They are stored in `agents/` and define the full execution contract for one agent.

```
agents/
├── baseline.toml              # Universal defaults — every profile inherits from this
├── gestura-full.toml          # Gestura CLI · autonomous · all tools
├── gestura-sandboxed.toml     # Gestura CLI · sandboxed · no tools / no network
├── gestura-iterative.toml     # Gestura CLI · iterative · confirmation gates
├── claude-code-full.toml      # Claude Code · --dangerously-skip-permissions
├── claude-code-sandboxed.toml # Claude Code · --allowedTools "" (no tools)
├── claude-code-iterative.toml # Claude Code · default confirmation gates
├── augment-full.toml          # Augment Code · autonomous
├── augment-sandboxed.toml     # Augment Code · sandboxed
├── augment-iterative.toml     # Augment Code · iterative
├── codex-full.toml            # OpenAI Codex · --approval-mode full-auto
├── codex-sandboxed.toml       # OpenAI Codex · --sandbox
├── codex-iterative.toml       # OpenAI Codex · --approval-mode suggest
├── opencode-full.toml         # OpenCode · --yes
├── opencode-sandboxed.toml    # OpenCode · --no-tools
└── opencode-iterative.toml    # OpenCode · --interactive
```

Each profile merges on top of `baseline.toml` — only the fields that differ need to be
declared. The merge is deep: table fields are merged key-by-key; scalar and array fields
are replaced wholesale by the overlay value.

Profile anatomy:

```toml
[agent]
id   = "gestura-full"
name = "Gestura — Full Permission (Autonomous)"
mode = "autonomous"   # autonomous | sandboxed | iterative

[model]
provider    = "anthropic"
name        = "claude-sonnet-4-6"
temperature = 0.7
max_tokens  = 8192

[subprocess]
bin         = "gestura"               # binary to invoke; omit for auto-detect
args_prefix = ["exec"]               # prepended before the prompt argument
response_strip_prefix = "Assistant:" # optional: strip this prefix from stdout

[subprocess.env]
ANTHROPIC_API_KEY = ""               # forwarded to every subprocess invocation

[thresholds]
min_variation_score    = 0.85   # fraction of checks that must pass per variation
min_scenario_pass_rate = 1.0    # fraction of variations that must pass per scenario
min_overall_score      = 0.85   # mean score across all variations

[scenarios.s3_planning.variation_overrides.v1]
min_words         = 120
additional_checks = ["has_structured_sections"]
```

---

## API keys and credentials

### Quick reference

| Profile group | Binary | Required credential | Source |
|---|---|---|---|
| `gestura-*` | `gestura` | `ANTHROPIC_API_KEY` or `~/.gestura/config.yaml` | Anthropic Console |
| `claude-code-*` | `claude` | `ANTHROPIC_API_KEY` | Anthropic Console |
| `augment-*` | `augment` | `ANTHROPIC_API_KEY` (Claude-backed) | Augment workspace login |
| `codex-*` | `codex` | `OPENAI_API_KEY` | OpenAI Platform |
| `opencode-*` | `opencode` | `ANTHROPIC_API_KEY` (profiles use Claude) | Anthropic Console |

---

### 1. Gestura profiles

Gestura reads credentials from its config file with environment variables as an override
layer. The keychain is **always disabled** during eval (`GESTURA_DISABLE_KEYCHAIN=1` is set
in every `gestura-*.toml` profile) so the subprocess never hangs on a macOS keychain prompt.

**Persistent config (recommended):**
```bash
gestura config set llm.primary anthropic
gestura config set llm.anthropic.api_key sk-ant-...
gestura config set llm.anthropic.model claude-sonnet-4-6
```

**Environment variable override (CI / ephemeral):**
```bash
export ANTHROPIC_API_KEY=sk-ant-...
export ANTHROPIC_MODEL=claude-sonnet-4-6   # optional; profile default used if absent
export LLM_PRIMARY=anthropic               # optional; tells gestura which provider to use
```

Gestura credential precedence (highest to lowest):
1. Environment variable (`ANTHROPIC_API_KEY`, `OPENAI_API_KEY`, etc.)
2. Config file (`~/.gestura/config.yaml` → `llm.anthropic.api_key`)
3. macOS Keychain (disabled during eval via `GESTURA_DISABLE_KEYCHAIN=1`)

**Verify before running:**
```bash
cargo build -p gestura-cli
./target/debug/gestura exec "What is 2 + 2?"
# Expected: plain-text response, no keychain prompt, exit 0
```

---

### 2. Claude Code profiles

Claude Code authenticates via its own OAuth session or a bare API key. The eval profiles
forward `ANTHROPIC_MODEL` via `[subprocess.env]` — the API key must be in the environment
before invoking `gestura-eval`.

**Install:**
```bash
npm install -g @anthropic-ai/claude-code
```

**Authenticate — OAuth (recommended for interactive use):**
```bash
claude login
# Stores a session token in ~/.claude/
```

**Authenticate — API key (recommended for CI/eval):**
```bash
export ANTHROPIC_API_KEY=sk-ant-...
```

**Verify:**
```bash
claude -p "What is 2 + 2?"
```

**Run eval:**
```bash
export ANTHROPIC_API_KEY=sk-ant-...
./target/debug/gestura-eval --agent claude-code-full
./target/debug/gestura-eval --agent claude-code-sandboxed
./target/debug/gestura-eval --agent claude-code-iterative
```

> **`claude-code-full` passes `--dangerously-skip-permissions`**, which disables all
> confirmation gates. Only use this profile in isolated environments where no sensitive files
> are accessible to the subprocess.

---

### 3. Augment profiles

Augment Code is an IDE extension. The `augment` CLI binary used in these profiles is not
yet publicly released. Until it is:

- `--dry-run` works with no credentials.
- The profiles are ready for when the CLI ships — expected credential: `ANTHROPIC_API_KEY`.
- Watch <https://www.augmentcode.com> for CLI availability.

```bash
# Works today, no credentials required
./target/debug/gestura-eval --agent augment-full --dry-run
```

When the CLI is available:
```bash
export ANTHROPIC_API_KEY=sk-ant-...
./target/debug/gestura-eval --agent augment-full
```

If the binary name or flag surface differs from what the profiles assume, update `bin` and
`args_prefix` in `agents/augment-*.toml` accordingly.

---

### 4. Codex profiles

**Install:**
```bash
npm install -g @openai/codex
```

**Set credentials:**
```bash
export OPENAI_API_KEY=sk-...
```

**Verify:**
```bash
codex --approval-mode full-auto --quiet "What is 2 + 2?"
```

**Run eval:**
```bash
export OPENAI_API_KEY=sk-...
./target/debug/gestura-eval --agent codex-full
./target/debug/gestura-eval --agent codex-sandboxed
./target/debug/gestura-eval --agent codex-iterative
```

`codex-full` and `codex-iterative` default to `gpt-4.5`. If your API tier does not include
that model, override it with a custom config file:

```bash
cp agents/codex-full.toml /tmp/codex-o3.toml
# Edit: name = "o3" and update the --model flag in args_prefix
./target/debug/gestura-eval --config /tmp/codex-o3.toml
```

---

### 5. OpenCode profiles

OpenCode is model-agnostic. The built-in profiles use Anthropic models, so `ANTHROPIC_API_KEY`
is required. To use OpenAI models instead, update `[model]` in the TOML and set
`OPENAI_API_KEY`.

**Install:**
```bash
npm install -g opencode-ai
```

**Set credentials:**
```bash
export ANTHROPIC_API_KEY=sk-ant-...
```

**Verify:**
```bash
opencode run --yes "What is 2 + 2?"
```

**Run eval:**
```bash
export ANTHROPIC_API_KEY=sk-ant-...
./target/debug/gestura-eval --agent opencode-full
./target/debug/gestura-eval --agent opencode-sandboxed
./target/debug/gestura-eval --agent opencode-iterative
```

> OpenCode is evolving rapidly. Verify the `--yes`, `--no-tools`, and `--interactive` flags
> against your installed version with `opencode --help`. Update `args_prefix` in the TOML
> files if the flags have changed.

---

### Credential precedence and `.env` pattern

All external agent profiles read credentials from the calling shell. A `.env` file at the
repo root keeps credentials out of your global shell without committing them:

```bash
# .env  — never commit this file
ANTHROPIC_API_KEY=sk-ant-...
OPENAI_API_KEY=sk-...
ANTHROPIC_MODEL=claude-sonnet-4-6
OPENAI_MODEL=gpt-4.5
```

```bash
set -a && source .env && set +a
./target/debug/gestura-eval --agent claude-code-full
```

---

### Dry-run — no credentials needed

Every profile supports `--dry-run`. No subprocess is launched, no credentials are consumed,
and the evaluator runs against a stub response so you can verify rubric logic before spending
API quota:

```bash
./target/debug/gestura-eval --agent codex-full --dry-run
./target/debug/gestura-eval --agent opencode-iterative --dry-run --scenario s3_planning
./target/debug/gestura-eval --agent claude-code-sandboxed --dry-run --json
```

---

## Custom agent profiles

Any agent that can be invoked as `<bin> [flags...] "<prompt>"` and writes its response to
stdout can be evaluated. Create a TOML file that overrides only the fields that differ from
`agents/baseline.toml`:

```toml
# agents/my-local-llm.toml
[agent]
id   = "my-local-llm"
name = "My Local LLM"
mode = "autonomous"

[model]
provider = "ollama"
name     = "llama3"

[subprocess]
bin         = "ollama"
args_prefix = ["run", "llama3"]

[thresholds]
min_variation_score    = 0.65
min_scenario_pass_rate = 0.67
min_overall_score      = 0.65
```

```bash
./target/debug/gestura-eval --config agents/my-local-llm.toml
```

---

## Output formats

**Text (default):**
```
╔══════════════════════════════════════════════════════════╗
║          GESTURA EVAL REPORT                             ║
╚══════════════════════════════════════════════════════════╝
  Agent   : Gestura — Full Permission (Autonomous) [gestura-full]
  Mode    : autonomous
  Provider: anthropic / claude-sonnet-4-6
  Scenarios : 7/8 passed
  Variations: 21/24 passed
  Score     : 87.5%

  ✅ [s1_simple_query] Simple Single-Turn Query (100%)
  ❌ [s3_planning] Complex Multi-Step Planning (67%)
      ✗ v1 — response_is_substantive: 45 words; expected ≥100
```

**JSON (`--json`):** Full `EvalReport` struct serialised to pretty-printed JSON. Suitable
for piping into `jq`, storing as CI artefacts, or diffing across agent runs.

Exit codes: `0` = all variations passed (or `--dry-run`), `1` = one or more failures.

---

## Crate structure

```
src/
├── lib.rs          — public API surface and re-exports
├── main.rs         — gestura-eval binary (CLI flags, agent loading, report output)
├── config/
│   ├── mod.rs      — profile loading, TOML deep-merge, EvalConfig
│   └── types.rs    — AgentMeta, AgentMode, SubprocessDef, Thresholds, etc.
├── cli_runner.rs   — CliEvalRunner: spawns subprocesses, collects responses
├── evaluator.rs    — RuleEvaluator: 17 deterministic named checks
├── scenario.rs     — EvalScenario / EvalVariation deserialized from scenarios.json
└── report.rs       — EvalReport, ScenarioResult, VariationResult, print_text/print_json

agents/             — TOML agent profiles (embedded at compile time via include_str!)
testdata/
└── scenarios.json  — 8 scenarios × 3 variations (prompts, checks, rubric)
```

