# Gestura Configuration

This document describes the configuration file location, structure, and first-run behavior for the Gestura desktop application.

## Configuration File Location

Gestura stores its configuration in a unified location across all platforms:

```
~/.gestura/config.yaml
```

This path expands to:
- **macOS**: `/Users/<username>/.gestura/config.yaml`
- **Linux**: `/home/<username>/.gestura/config.yaml`
- **Windows**: `C:\Users\<username>\.gestura\config.yaml`

## Directory Structure

The `~/.gestura/` directory contains all Gestura user data:

```
~/.gestura/
├── config.yaml          # Main configuration file (YAML)
├── config.json.backup   # Legacy config backup (created on auto-migration, if present)
├── models/
│   └── whisper/         # Downloaded Whisper STT models
│       └── ggml-base.en.bin
├── logs/                # Application logs (if enabled)
└── cache/               # Temporary cache data
```

## First-Run Detection

Gestura detects first-run by checking if a config file exists:

- **First run**: No config file exists → Show onboarding/setup wizard
- **Subsequent runs**: Config file exists → Load saved configuration

Notes:
- Current versions persist config as `~/.gestura/config.yaml`.
- If `config.yaml` is missing but a legacy `~/.gestura/config.json` exists, Gestura will load it and auto-migrate it to YAML.

### Canonical API/Type Reference

This document focuses on the configuration file layout and operational setup.

For exact Rust configuration types and the canonical library/API surface, use
generated Rustdoc:

```bash
cargo doc -p gestura-core-config --no-deps
```

For GUI IPC command details such as `get_config` and `save_config`, see:

- `docs/IPC_CONTRACTS_GESTURA_GUI.md`

## Configuration Structure

The configuration file is a YAML mapping with the following structure:

```yaml
hotkey_listen: Cmd+Shift+G
voice:
  provider: local
  local_model_path: ~/.gestura/models/whisper/ggml-base.en.bin
  openai_api_key: ""
llm:
  primary: echo
  openai:
    api_key: ""
    model: gpt-4o-mini
  anthropic:
    api_key: ""
    model: claude-3-5-sonnet-20241022
  grok:
    api_key: ""
    model: grok-beta
  ollama:
    endpoint: http://localhost:11434
    model: ""
    temperature: 0.7
    context_length: 4096
mcp_tools: []
mdh_pointers: {}
ui_prefs:
  theme: system
  show_notifications: true
pipeline:
  max_history_messages: 10
  auto_compact_threshold_percent: 80
  compaction_strategy: Summarize
  max_context_tokens: 0
  log_token_usage: false
  agent_telemetry:
    enabled: false
    trace_export:
      enabled: false
      protocol: grpc
      endpoint: http://127.0.0.1:4317
  reflection:
    enabled: false
    quality_threshold_percent: 60
    max_injected: 3
    max_retry_attempts: 1
    promotion_confidence_percent: 75
```

## Configuration Fields

### Voice Settings (`voice`)

| Field | Type | Description |
|-------|------|-------------|
| `provider` | string | STT provider: `"local"` or `"openai"` |
| `local_model_path` | string | Path to local Whisper model file |
| `openai_api_key` | string | OpenAI API key for Whisper API |

### LLM Settings (`llm`)

| Field | Type | Description |
|-------|------|-------------|
| `primary` | string | Active LLM provider: `"openai"`, `"anthropic"`, `"grok"`, `"ollama"` |
| `openai` | object | OpenAI configuration (api_key, model) |
| `anthropic` | object | Anthropic configuration (api_key, model) |
| `grok` | object | Grok configuration (api_key, model) |
| `ollama` | object | Ollama configuration (endpoint, model, temperature, context_length) |

### UI Preferences (`ui_prefs`)

| Field | Type | Description |
|-------|------|-------------|
| `theme` | string | UI theme: `"light"`, `"dark"`, or `"system"` |
| `show_notifications` | boolean | Whether to show system notifications |

### Pipeline Settings (`pipeline`)

| Field | Type | Description |
|-------|------|-------------|
| `max_history_messages` | integer | Maximum conversation history messages included in prompt context |
| `auto_compact_threshold_percent` | integer | Trigger compaction when context reaches this percentage of the limit |
| `compaction_strategy` | string | Overflow handling strategy such as `Summarize`, `Truncate`, `Clear`, `Prompt`, or `MemoryBank` |
| `max_context_tokens` | integer | Maximum context tokens (0 uses provider defaults) |
| `log_token_usage` | boolean | Enable token usage logging for debugging/monitoring |

#### Agent Telemetry Settings (`pipeline.agent_telemetry`)

| Field | Type | Description |
|-------|------|-------------|
| `enabled` | boolean | Emit request-level agent pipeline telemetry to the local in-memory metric store |
| `trace_export.enabled` | boolean | Attach OTLP trace export to `tracing` so agent requests can be inspected in a collector such as SigNoz |
| `trace_export.protocol` | string | OTLP transport to use: `grpc` or `http`; defaults to `grpc` |
| `trace_export.endpoint` | string | Collector endpoint for the selected transport; defaults are `http://127.0.0.1:4317` for gRPC and `http://127.0.0.1:4318/v1/traces` for HTTP |

Example:

```yaml
pipeline:
  agent_telemetry:
    enabled: true
    trace_export:
      enabled: true
      protocol: grpc
      endpoint: http://127.0.0.1:4317
```

When both toggles are enabled, Gestura emits request-correlated spans that include identifiers such as `request_id`, `session_id`, `task_id`, `directive_id`, and `agent_id`.

For HTTP collectors, switch to:

```yaml
pipeline:
  agent_telemetry:
    trace_export:
      protocol: http
      endpoint: http://127.0.0.1:4318/v1/traces
```

#### Reflection Settings (`pipeline.reflection`)

| Field | Type | Description |
|-------|------|-------------|
| `enabled` | boolean | Turn on ERL-inspired experiential reflection for low-quality turns |
| `quality_threshold_percent` | integer | Reflection triggers when quality falls below this percentage |
| `max_injected` | integer | Maximum number of past reflections injected into future prompts |
| `max_retry_attempts` | integer | Number of same-turn text-only revision retries after reflection (`0` disables, current runtime applies at most `1`) |
| `promotion_confidence_percent` | integer | Minimum confidence before a reflection is promoted to long-term memory |

Example:

```yaml
pipeline:
  reflection:
    enabled: true
    quality_threshold_percent: 55
    max_injected: 4
    max_retry_attempts: 1
    promotion_confidence_percent: 80
```

## Backup and Migration

To backup your configuration:
```bash
cp ~/.gestura/config.yaml ~/.gestura/config.yaml.backup
```

If you are upgrading from an older version, you may also see `~/.gestura/config.json.backup` after the first load.

To reset to defaults, delete the config file:
```bash
rm ~/.gestura/config.yaml
```

The next app launch will create a fresh configuration with default values.

## Troubleshooting

### Config file not loading
1. Check file permissions: `ls -la ~/.gestura/config.yaml`
2. Validate YAML syntax (e.g., with `yq` or another YAML validator)
3. Check for backup: `ls ~/.gestura/*.backup`

### First-run wizard keeps appearing
1. Ensure config file was saved: `ls -la ~/.gestura/config.yaml`
2. Check write permissions on `~/.gestura/` directory
3. Verify the config file contains valid YAML

