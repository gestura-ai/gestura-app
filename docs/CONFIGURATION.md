# Gestura Configuration

This document describes the configuration file location, structure, and first-run behavior for the Gestura desktop application.

## Configuration File Location

Gestura stores its configuration in a unified location across all platforms:

```
~/.gestura/config.json
```

This path expands to:
- **macOS**: `/Users/<username>/.gestura/config.json`
- **Linux**: `/home/<username>/.gestura/config.json`
- **Windows**: `C:\Users\<username>\.gestura\config.json`

## Directory Structure

The `~/.gestura/` directory contains all Gestura user data:

```
~/.gestura/
├── config.json          # Main configuration file
├── models/
│   └── whisper/         # Downloaded Whisper STT models
│       └── ggml-base.en.bin
├── logs/                # Application logs (if enabled)
└── cache/               # Temporary cache data
```

## First-Run Detection

Gestura detects first-run by checking if `~/.gestura/config.json` exists:

- **First run**: No config file exists → Show onboarding/setup wizard
- **Subsequent runs**: Config file exists → Load saved configuration

### API Commands

```javascript
// Check if this is the first run
const isFirstRun = await window.__TAURI__.core.invoke('is_first_run');

// Get the config file path
const configPath = await window.__TAURI__.core.invoke('get_config_path');

// Load configuration
const config = await window.__TAURI__.core.invoke('get_config');

// Save configuration
await window.__TAURI__.core.invoke('save_config', { cfg: config });
```

## Configuration Structure

The configuration file is a JSON object with the following structure:

```json
{
  "hotkey_listen": "Cmd+Shift+G",
  "voice": {
    "provider": "local",
    "local_model_path": "~/.gestura/models/whisper/ggml-base.en.bin",
    "openai_api_key": ""
  },
  "llm": {
    "primary": "echo",
    "openai": {
      "api_key": "",
      "model": "gpt-4o-mini"
    },
    "anthropic": {
      "api_key": "",
      "model": "claude-3-5-sonnet-20241022"
    },
    "grok": {
      "api_key": "",
      "model": "grok-beta"
    },
    "ollama": {
      "endpoint": "http://localhost:11434",
      "model": "",
      "temperature": 0.7,
      "context_length": 4096
    }
  },
  "mcp_tools": [],
  "mdh_pointers": {},
  "ui_prefs": {
    "theme": "system",
    "show_notifications": true
  }
}
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
| `primary` | string | Active LLM provider: `"echo"`, `"openai"`, `"anthropic"`, `"grok"`, `"ollama"` |
| `openai` | object | OpenAI configuration (api_key, model) |
| `anthropic` | object | Anthropic configuration (api_key, model) |
| `grok` | object | Grok configuration (api_key, model) |
| `ollama` | object | Ollama configuration (endpoint, model, temperature, context_length) |

### UI Preferences (`ui_prefs`)

| Field | Type | Description |
|-------|------|-------------|
| `theme` | string | UI theme: `"light"`, `"dark"`, or `"system"` |
| `show_notifications` | boolean | Whether to show system notifications |

## Backup and Migration

To backup your configuration:
```bash
cp ~/.gestura/config.json ~/.gestura/config.json.backup
```

To reset to defaults, delete the config file:
```bash
rm ~/.gestura/config.json
```

The next app launch will create a fresh configuration with default values.

## Troubleshooting

### Config file not loading
1. Check file permissions: `ls -la ~/.gestura/config.json`
2. Validate JSON syntax: `cat ~/.gestura/config.json | python -m json.tool`
3. Check for backup: `ls ~/.gestura/*.backup`

### First-run wizard keeps appearing
1. Ensure config file was saved: `ls -la ~/.gestura/config.json`
2. Check write permissions on `~/.gestura/` directory
3. Verify the config file contains valid JSON

