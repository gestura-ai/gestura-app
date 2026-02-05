//! Centralized default AI model constants for all providers.
//!
//! This module provides a single source of truth for default model identifiers
//! across OpenAI, Anthropic, Grok, and Ollama providers. These constants are used:
//!
//! 1. As default values in `AppConfig` structs
//! 2. As fallback lists when API model discovery fails
//! 3. In frontend static fallbacks when backend is unavailable
//!
//! ## Model Selection Precedence
//!
//! The system uses the following precedence for model selection:
//! 1. **User configuration** (from `~/.gestura/config.yaml`)
//! 2. **API discovery** (dynamic model lists from provider APIs)
//! 3. **Static defaults** (constants defined in this module)
//!
//! ## Updating Models
//!
//! When new models are released or deprecated:
//! 1. Update the constants in this file
//! 2. Run tests to ensure backward compatibility
//! 3. Update documentation if default recommendations change

// ============================================================================
// OpenAI Models
// ============================================================================

/// Default OpenAI model for chat/completion tasks.
///
/// GPT-4o provides the best balance of capability, speed, and cost as of 2025.
pub const DEFAULT_OPENAI_MODEL: &str = "gpt-4o";

/// Default OpenAI model for speech-to-text transcription.
///
/// GPT-4o Transcribe offers superior accuracy compared to Whisper V2,
/// with lower Word Error Rate (WER) for voice input.
pub const DEFAULT_OPENAI_STT_MODEL: &str = "gpt-4o-transcribe";

/// Static list of known OpenAI chat models.
///
/// Used as fallback when API model discovery fails or API key is unavailable.
/// Ordered by recommendation (best first).
pub const OPENAI_CHAT_MODELS: &[&str] = &["gpt-4o", "gpt-4o-mini", "gpt-4-turbo", "gpt-3.5-turbo"];

/// Static list of known OpenAI STT models.
///
/// Used as fallback when API model discovery fails.
/// Ordered by recommendation (best first).
pub const OPENAI_STT_MODELS: &[&str] =
    &["gpt-4o-transcribe", "gpt-4o-mini-transcribe", "whisper-1"];

// ============================================================================
// Anthropic Models
// ============================================================================

/// Default Anthropic model for chat/completion tasks.
///
/// Claude Sonnet 4 (2025-05-14) is the latest and most capable model.
pub const DEFAULT_ANTHROPIC_MODEL: &str = "claude-sonnet-4-20250514";

/// Static list of known Anthropic models.
///
/// Used as fallback when API model discovery fails or API key is unavailable.
/// Ordered by recommendation (best first).
pub const ANTHROPIC_MODELS: &[&str] = &[
    "claude-sonnet-4-20250514",
    "claude-3-5-sonnet-20241022",
    "claude-3-opus-20240229",
    "claude-3-sonnet-20240229",
    "claude-3-haiku-20240307",
];

// ============================================================================
// Grok Models (xAI)
// ============================================================================

/// Default Grok model for chat/completion tasks.
///
/// Grok-3 is the latest model from xAI.
pub const DEFAULT_GROK_MODEL: &str = "grok-3";

/// Static list of known Grok models.
///
/// Used as fallback when API model discovery fails or API key is unavailable.
/// Ordered by recommendation (best first).
pub const GROK_MODELS: &[&str] = &["grok-3", "grok-2-1212", "grok-2-vision-1212", "grok-beta"];

// ============================================================================
// Ollama Models
// ============================================================================

/// Default Ollama model for local inference.
///
/// Llama 3.2 provides good performance for local use cases.
pub const DEFAULT_OLLAMA_MODEL: &str = "llama3.2";

/// Default Ollama base URL for local inference.
pub const DEFAULT_OLLAMA_BASE_URL: &str = "http://localhost:11434";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_models_are_in_static_lists() {
        assert!(OPENAI_CHAT_MODELS.contains(&DEFAULT_OPENAI_MODEL));
        assert!(OPENAI_STT_MODELS.contains(&DEFAULT_OPENAI_STT_MODEL));
        assert!(ANTHROPIC_MODELS.contains(&DEFAULT_ANTHROPIC_MODEL));
        assert!(GROK_MODELS.contains(&DEFAULT_GROK_MODEL));
    }
}
