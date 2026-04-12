//! Model capabilities registry for dynamic context and feature awareness.
//!
//! This module provides a centralized registry mapping model IDs to their
//! capabilities (context length, max output tokens, feature support, etc.).
//!
//! ## Design Goals
//!
//! - Enable dynamic adjustment of pipeline config based on selected model
//! - Prevent context_length_exceeded errors by pre-flight validation
//! - Support fallback to conservative defaults for unknown models
//!
//! ## Usage
//!
//! ```rust,ignore
//! use gestura_core_llm::model_capabilities::{get_model_capabilities, ModelCapabilities};
//!
//! let caps = get_model_capabilities("openai", "gpt-4o");
//! assert_eq!(caps.context_length, 128_000);
//! ```

use serde::{Deserialize, Serialize};

/// Model capabilities describing limits and supported features.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ModelCapabilities {
    /// Maximum context window in tokens (input + output combined for most models)
    pub context_length: usize,
    /// Maximum output/completion tokens the model can generate
    pub max_output_tokens: usize,
    /// Whether the model supports native tool/function calling
    pub supports_tools: bool,
    /// Whether the model supports vision/image inputs
    pub supports_vision: bool,
    /// Whether the model supports streaming responses
    pub supports_streaming: bool,
    /// Provider name for reference
    pub provider: String,
    /// Model ID for reference
    pub model_id: String,
}

impl Default for ModelCapabilities {
    fn default() -> Self {
        Self {
            context_length: 16_384, // Conservative default
            max_output_tokens: 4_096,
            supports_tools: true,
            supports_vision: false,
            supports_streaming: true,
            provider: "unknown".to_string(),
            model_id: "unknown".to_string(),
        }
    }
}

impl ModelCapabilities {
    /// Create capabilities for a known model
    pub fn new(
        provider: &str,
        model_id: &str,
        context_length: usize,
        max_output_tokens: usize,
    ) -> Self {
        Self {
            context_length,
            max_output_tokens,
            supports_tools: true,
            supports_vision: false,
            supports_streaming: true,
            provider: provider.to_string(),
            model_id: model_id.to_string(),
        }
    }

    /// Set vision support
    pub fn with_vision(mut self, supports: bool) -> Self {
        self.supports_vision = supports;
        self
    }

    /// Set tool support
    pub fn with_tools(mut self, supports: bool) -> Self {
        self.supports_tools = supports;
        self
    }

    /// Calculate the effective max input tokens (context - reserved output)
    pub fn max_input_tokens(&self) -> usize {
        self.context_length.saturating_sub(self.max_output_tokens)
    }
}

/// Get capabilities for a specific provider and model.
///
/// Returns known capabilities for recognized models, or conservative defaults
/// for unknown models to prevent context overflow errors.
pub fn get_model_capabilities(provider: &str, model_id: &str) -> ModelCapabilities {
    let model_lower = model_id.to_lowercase();

    match provider.to_lowercase().as_str() {
        "openai" => get_openai_capabilities(&model_lower, model_id),
        "anthropic" => get_anthropic_capabilities(&model_lower, model_id),
        "gemini" => get_gemini_capabilities(&model_lower, model_id),
        "grok" => get_grok_capabilities(&model_lower, model_id),
        "ollama" => get_ollama_capabilities(&model_lower, model_id),
        _ => ModelCapabilities {
            provider: provider.to_string(),
            model_id: model_id.to_string(),
            ..Default::default()
        },
    }
}

fn get_openai_capabilities(model_lower: &str, model_id: &str) -> ModelCapabilities {
    // GPT-4o family (128K context)
    if model_lower.starts_with("gpt-4o") || model_lower.starts_with("chatgpt-4o") {
        return ModelCapabilities::new("openai", model_id, 128_000, 16_384).with_vision(true);
    }

    // GPT-4 Turbo (128K context)
    if model_lower.contains("gpt-4-turbo") || model_lower.contains("gpt-4-1106") {
        return ModelCapabilities::new("openai", model_id, 128_000, 4_096).with_vision(true);
    }

    // GPT-4 base (8K context)
    if model_lower.starts_with("gpt-4") && !model_lower.contains("turbo") {
        return ModelCapabilities::new("openai", model_id, 8_192, 4_096);
    }

    // GPT-3.5-turbo (16K context for newer versions)
    if model_lower.contains("gpt-3.5-turbo") {
        // The 16k variant
        if model_lower.contains("16k") {
            return ModelCapabilities::new("openai", model_id, 16_385, 4_096);
        }
        // Standard 3.5-turbo (older models had 4K, newer have 16K)
        return ModelCapabilities::new("openai", model_id, 16_385, 4_096);
    }

    // o1/o3/o4/o5 reasoning models (128K+ context)
    if model_lower.starts_with("o1")
        || model_lower.starts_with("o3")
        || model_lower.starts_with("o4")
        || model_lower.starts_with("o5")
    {
        return ModelCapabilities::new("openai", model_id, 128_000, 32_768);
    }

    // GPT-5.x and codex models (assume large context)
    if model_lower.starts_with("gpt-5") || model_lower.contains("codex") {
        return ModelCapabilities::new("openai", model_id, 128_000, 16_384);
    }

    // Unknown OpenAI model - use conservative defaults
    ModelCapabilities::new("openai", model_id, 16_384, 4_096)
}

