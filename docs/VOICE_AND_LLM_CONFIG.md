# Voice and LLM Configuration Guide

This document describes how to configure speech-to-text (STT) and large language model (LLM) providers in Gestura.

## Overview

Gestura uses a two-stage pipeline for voice commands:
1. **Speech-to-Text (STT)**: Converts spoken audio to text
2. **LLM Processing**: Interprets the text and generates responses

Both stages must be properly configured for the voice assistant to function.

---

## Speech-to-Text (STT) Providers

### Local Whisper (Recommended)

Uses [whisper.cpp](https://github.com/ggerganov/whisper.cpp) for on-device transcription.

**Advantages:**
- Privacy: Audio never leaves your device
- No API costs
- Works offline

**Configuration:**
1. Go to **Settings → Voice & Audio**
2. Select **Local Whisper** as the provider
3. Download a model (see [Available Models](#available-whisper-models))
4. Set the model path

**Requirements:**
- Sufficient disk space for the model (75MB - 3.1GB)
- CPU with AVX2 support (most modern CPUs)

### OpenAI Whisper API

Uses OpenAI's cloud-based Whisper API.

**Advantages:**
- No local model download required
- Consistent high accuracy
- Supports 99+ languages

**Configuration:**
1. Go to **Settings → Voice & Audio**
2. Select **OpenAI Whisper** as the provider
3. Provide an OpenAI API key (any of the following works):
   - **Settings → Voice & Audio** (voice/STT-specific key), or
   - **Settings → AI Providers → OpenAI** (shared OpenAI key)

**API key resolution (highest precedence first):**

1. `config.voice.openai_api_key`
2. secure storage secret `VoiceOpenAi`
3. secure storage secret `OpenAi` (shared OpenAI key)
4. legacy `config.llm.openai.api_key` (backwards compatibility)

> Note: When secure storage is available, Gestura stores secrets in the OS keychain. Prefer that over
> keeping API keys in plaintext config files.

### Session voice overrides (optional)

Gestura can apply a per-session voice/STT override (provider and/or model) when voice input is routed
to an “active” agent session.

**Precedence:**

1. Session override (non-empty)
2. Global config (`config.voice.*`)
3. Default values (e.g. `gpt-4o-transcribe` for OpenAI transcription)

**Requirements:**
- OpenAI API key with Whisper access
- Internet connection

---

## Available Whisper Models

| Model | Size | Speed | Accuracy | Languages |
|-------|------|-------|----------|-----------|
| Tiny (English) | 75 MB | Fastest | Basic | English |
| Base (English) ⭐ | 142 MB | Fast | Good | English |
| Small (English) | 466 MB | Moderate | Better | English |
| Medium (English) | 1.5 GB | Slow | High | English |
| Tiny (Multilingual) | 75 MB | Fastest | Basic | 99+ |
| Base (Multilingual) | 142 MB | Fast | Good | 99+ |
| Small (Multilingual) | 466 MB | Moderate | Better | 99+ |
| Medium (Multilingual) | 1.5 GB | Slow | High | 99+ |
| Large v3 | 3.1 GB | Slowest | Best | 99+ |
| Large v3 Turbo | 1.6 GB | Moderate | Near-best | 99+ |

**Quantized Models** (smaller file sizes with minimal accuracy loss):
- Small Q5: 190 MB
- Medium Q5: 540 MB
- Large v3 Turbo Q5: 580 MB

⭐ = Recommended for most users

---

## LLM Providers

### OpenAI

Uses OpenAI's GPT models (GPT-4, GPT-4o, GPT-3.5-turbo).

**Configuration:**
1. Go to **Settings → AI Providers → OpenAI**
2. Enter your API key
3. Select a model (default: gpt-4o-mini)
4. Optionally set a custom base URL

### Anthropic

Uses Anthropic's Claude models.

**Configuration:**
1. Go to **Settings → AI Providers → Anthropic**
2. Enter your API key
3. Select a model (default: claude-3-5-sonnet-20241022)

### Grok (xAI)

Uses xAI's Grok models via OpenAI-compatible API.

**Configuration:**
1. Go to **Settings → AI Providers → Grok**
2. Enter your xAI API key
3. Select a model (default: grok-beta)

### Ollama (Local)

Uses locally-running Ollama for on-device LLM inference.

**Advantages:**
- Privacy: All processing stays local
- No API costs
- Works offline

**Configuration:**
1. Install [Ollama](https://ollama.ai)
2. Pull a model: `ollama pull llama3.2`
3. Go to **Settings → AI Providers → Ollama**
4. Set the endpoint (default: http://localhost:11434)
5. Select your model

---

## Validation and Error Codes

Gestura validates configuration before starting voice listening. Common errors:

### STT Errors

| Error Code | Message | Solution |
|------------|---------|----------|
| `NO_PROVIDER_CONFIGURED` | No STT provider configured | Select a provider in Settings → Voice & Audio |
| `LOCAL_MODEL_NOT_FOUND` | Local Whisper model not found | Download a model or check the path |
| `OPENAI_API_KEY_MISSING` | OpenAI API key not configured | Add an OpenAI key in Settings → Voice & Audio **or** Settings → AI Providers → OpenAI |
| `UNKNOWN_PROVIDER` | Unknown STT provider | Select a valid provider |

### LLM Errors

| Error Code | Message | Solution |
|------------|---------|----------|
| `LLM_PROVIDER_MISSING` | No LLM provider configured | Select a provider in Settings → AI Providers |
| `LLM_OPENAI_API_KEY_MISSING` | OpenAI API key missing | Add key in Settings → AI Providers → OpenAI |
| `LLM_ANTHROPIC_API_KEY_MISSING` | Anthropic API key missing | Add key in Settings → AI Providers → Anthropic |
| `LLM_GROK_API_KEY_MISSING` | Grok API key missing | Add key in Settings → AI Providers → Grok |
| `LLM_OLLAMA_MODEL_MISSING` | Ollama model not configured | Set model in Settings → AI Providers → Ollama |
| `LLM_PROVIDER_UNKNOWN` | Unknown LLM provider | Select a valid provider |

---

## Troubleshooting

### Voice listening won't start

1. Check the system tray notification for specific error messages
2. Verify your STT provider is configured correctly
3. Verify your LLM provider is configured correctly
4. Test your microphone in System Preferences

### Local Whisper not working

1. Ensure the model file exists at the configured path
2. Check that the model file is not corrupted (re-download if needed)
3. Verify your CPU supports AVX2 instructions

### OpenAI API errors

1. Verify your API key is valid
2. Check your OpenAI account has sufficient credits
3. Ensure you have access to the Whisper API

### Ollama connection failed

1. Ensure Ollama is running: `ollama serve`
2. Check the endpoint URL (default: http://localhost:11434)
3. Verify you have pulled a model: `ollama list`
4. Test connection: `curl http://localhost:11434/api/tags`

