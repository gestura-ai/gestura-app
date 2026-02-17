//! Centralized default AI model constants for all providers.
//!
//! This module provides a single source of truth for default model identifiers
//! across OpenAI, Anthropic, Grok, Gemini, and Ollama providers. These constants
//! are used as default values in `AppConfig` structs (e.g. serde defaults).
//!
//! ## Model Selection Precedence
//!
//! The system uses the following precedence for model selection:
//! 1. **User configuration** (from `~/.gestura/config.yaml`)
//! 2. **API discovery** (dynamic model lists from provider APIs)
//!
//! There are no static fallback lists — if the API is unreachable or no API key
//! is configured, the model list is empty and the UI communicates this to the user.

// ============================================================================
// OpenAI Models
// ============================================================================

/// Default OpenAI model for agent/completion tasks.
///
/// GPT-4o provides the best balance of capability, speed, and cost as of 2025.
pub const DEFAULT_OPENAI_MODEL: &str = "gpt-4o";

/// Default OpenAI model for speech-to-text transcription.
///
/// GPT-4o Transcribe offers superior accuracy compared to Whisper V2,
/// with lower Word Error Rate (WER) for voice input.
pub const DEFAULT_OPENAI_STT_MODEL: &str = "gpt-4o-transcribe";

// ============================================================================
// Anthropic Models
// ============================================================================

/// Default Anthropic model for agent/completion tasks.
///
/// Claude Sonnet 4 (2025-05-14) is the latest and most capable model.
pub const DEFAULT_ANTHROPIC_MODEL: &str = "claude-sonnet-4-20250514";

// ============================================================================
// Grok Models (xAI)
// ============================================================================

/// Default Grok model for agent/completion tasks.
///
/// Grok-3 is the latest stable model from xAI.
pub const DEFAULT_GROK_MODEL: &str = "grok-3";

// ============================================================================
// Google Gemini Models
// ============================================================================

/// Default Gemini model for agent/completion tasks.
///
/// Gemini 2.0 Flash provides the best balance of speed and capability.
pub const DEFAULT_GEMINI_MODEL: &str = "gemini-2.0-flash";

/// Default Gemini API base URL (Google AI Studio / Generative Language API).
pub const DEFAULT_GEMINI_BASE_URL: &str = "https://generativelanguage.googleapis.com";

// ============================================================================
// Ollama Models
// ============================================================================

/// Default Ollama model for local inference.
///
/// Llama 3.2 provides good performance for local use cases.
pub const DEFAULT_OLLAMA_MODEL: &str = "llama3.2";

/// Default Ollama base URL for local inference.
pub const DEFAULT_OLLAMA_BASE_URL: &str = "http://localhost:11434";
