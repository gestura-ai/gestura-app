//! Dynamic model capabilities discovery and caching.
//!
//! This module provides runtime discovery of model capabilities (context length,
//! max output tokens, feature support) through multiple strategies:
//!
//! 1. **API Discovery** - Query provider model endpoints for metadata
//!    - Anthropic: `/v1/models/{id}` → `max_input_tokens`
//!    - Gemini: `/v1beta/models/{id}` → `inputTokenLimit`
//!    - Grok: `/v1/language-models` → context window per model
//!    - Ollama: `/api/show` → `model_info.{arch}.context_length`
//! 2. **Error-Driven Learning** - Parse limits from context_length_exceeded errors
//! 3. **Cached Knowledge** - Remember discovered limits across requests
//! 4. **Conservative Fallback** - Safe defaults for unknown models
//!
//! ## Design Goals
//!
//! - **Dynamic over static** - Learn limits at runtime, not hardcoded
//! - **Graceful degradation** - Work even when APIs are unavailable
//! - **Error recovery** - Extract actual limits from error messages
//!
//! ## Usage
//!
//! ```rust,ignore
//! use gestura_core_llm::model_capabilities::{ModelCapabilities, ModelCapabilitiesCache};
//!
//! let cache = ModelCapabilitiesCache::new();
//!
//! // Discover from API (async)
//! cache.discover_from_api("anthropic", "claude-sonnet-4-20250514", Some(api_key)).await;
//!
//! // Learn from an error (sync)
//! cache.learn_from_error("openai", "gpt-3.5-turbo",
//!     "maximum context length is 16385 tokens");
//!
//! // Get capabilities (uses discovered/learned value, falls back to heuristic)
//! let caps = cache.get("openai", "gpt-3.5-turbo");
//! ```

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::{Arc, RwLock};

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
    /// How this capability was discovered
    pub source: CapabilitySource,
}

/// How the capability information was obtained
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
pub enum CapabilitySource {
    /// Queried from provider API
    ApiDiscovery,
    /// Extracted from an error message
    ErrorLearned,
    /// User-configured override
    UserConfig,
    /// Static fallback (least reliable)
    #[default]
    StaticFallback,
}

impl Default for ModelCapabilities {
    fn default() -> Self {
        Self {
            context_length: 8_192, // Very conservative default
            max_output_tokens: 4_096,
            supports_tools: true,
            supports_vision: false,
            supports_streaming: true,
            provider: "unknown".to_string(),
            model_id: "unknown".to_string(),
            source: CapabilitySource::StaticFallback,
        }
    }
}

impl ModelCapabilities {
    /// Create capabilities with known values
    pub fn new(
        provider: &str,
        model_id: &str,
        context_length: usize,
        max_output_tokens: usize,
        source: CapabilitySource,
    ) -> Self {
        Self {
            context_length,
            max_output_tokens,
            supports_tools: true,
            supports_vision: false,
            supports_streaming: true,
            provider: provider.to_string(),
            model_id: model_id.to_string(),
            source,
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

    /// Check if this capability is from a reliable source
    pub fn is_reliable(&self) -> bool {
        matches!(
            self.source,
            CapabilitySource::ApiDiscovery | CapabilitySource::UserConfig
        )
    }
}

/// Thread-safe cache for learned model capabilities.
///
/// Capabilities are discovered dynamically and cached for future use.
/// The cache persists for the lifetime of the application.
#[derive(Debug, Clone, Default)]
pub struct ModelCapabilitiesCache {
    cache: Arc<RwLock<HashMap<String, ModelCapabilities>>>,
}

impl ModelCapabilitiesCache {
    /// Create a new empty cache
    pub fn new() -> Self {
        Self {
            cache: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Generate cache key from provider and model
    fn cache_key(provider: &str, model_id: &str) -> String {
        format!("{}:{}", provider.to_lowercase(), model_id.to_lowercase())
    }

    /// Get capabilities for a model, using cache or falling back to heuristics
    pub fn get(&self, provider: &str, model_id: &str) -> ModelCapabilities {
        let key = Self::cache_key(provider, model_id);

        // Check cache first
        if let Some(caps) = self.cache.read().ok().and_then(|c| c.get(&key).cloned()) {
            return caps;
        }

        // Fall back to heuristic-based capabilities
        get_model_capabilities_heuristic(provider, model_id)
    }

    /// Learn model capabilities from a context_length_exceeded error message.
    ///
    /// Parses error messages like:
    /// - "maximum context length is 16385 tokens"
    /// - "your messages resulted in 17063 tokens"
    ///
    /// Returns the learned capabilities if parsing succeeded.
    pub fn learn_from_error(
        &self,
        provider: &str,
        model_id: &str,
        error_message: &str,
    ) -> Option<ModelCapabilities> {
        let context_length = parse_context_length_from_error(error_message)?;

        let caps = ModelCapabilities::new(
            provider,
            model_id,
            context_length,
            estimate_max_output(context_length),
            CapabilitySource::ErrorLearned,
        );

        // Cache the learned capability
        let key = Self::cache_key(provider, model_id);
        if let Ok(mut cache) = self.cache.write() {
            cache.insert(key, caps.clone());
        }

        tracing::info!(
            provider = provider,
            model = model_id,
            context_length = context_length,
            "Learned model context limit from error"
        );

        Some(caps)
    }

    /// Store capabilities discovered from API
    pub fn store_from_api(&self, caps: ModelCapabilities) {
        let key = Self::cache_key(&caps.provider, &caps.model_id);
        if let Ok(mut cache) = self.cache.write() {
            cache.insert(key, caps);
        }
    }

    /// Store user-configured override
    pub fn store_user_override(&self, provider: &str, model_id: &str, context_length: usize) {
        let caps = ModelCapabilities::new(
            provider,
            model_id,
            context_length,
            estimate_max_output(context_length),
            CapabilitySource::UserConfig,
        );
        let key = Self::cache_key(provider, model_id);
        if let Ok(mut cache) = self.cache.write() {
            cache.insert(key, caps);
        }
    }

    /// Clear the cache (useful for testing)
    pub fn clear(&self) {
        if let Ok(mut cache) = self.cache.write() {
            cache.clear();
        }
    }
}

/// Parse context length from an error message.
///
/// Handles various error formats from different providers:
/// - OpenAI: "maximum context length is 16385 tokens"
/// - Anthropic: "prompt is too long: X tokens > Y maximum"
fn parse_context_length_from_error(error_message: &str) -> Option<usize> {
    let msg = error_message.to_lowercase();

    // OpenAI format: "maximum context length is 16385 tokens"
    if let Some(idx) = msg.find("maximum context length is ") {
        let start = idx + "maximum context length is ".len();
        return extract_number_at(&msg[start..]);
    }

    // Alternative: "context length is X tokens"
    if let Some(idx) = msg.find("context length is ") {
        let start = idx + "context length is ".len();
        return extract_number_at(&msg[start..]);
    }

    // Anthropic format: "X tokens > Y maximum"
    if let Some(idx) = msg.find(" maximum") {
        // Look backwards for the number
        let before_max = &msg[..idx];
        if let Some(gt_idx) = before_max.rfind("> ") {
            let start = gt_idx + 2;
            return extract_number_at(&before_max[start..]);
        }
    }

    // Generic: look for "limit of X tokens"
    if let Some(idx) = msg.find("limit of ") {
        let start = idx + "limit of ".len();
        return extract_number_at(&msg[start..]);
    }

    None
}

/// Extract a number from the start of a string
fn extract_number_at(s: &str) -> Option<usize> {
    let num_str: String = s.chars().take_while(|c| c.is_ascii_digit()).collect();
    num_str.parse().ok()
}

/// Estimate max output tokens based on context length
fn estimate_max_output(context_length: usize) -> usize {
    match context_length {
        0..=8_192 => 2_048,
        8_193..=32_000 => 4_096,
        32_001..=128_000 => 8_192,
        _ => 16_384,
    }
}

/// Get capabilities using heuristics (static fallback).
///
/// This is used when no cached/learned capabilities exist.
/// Prefer using `ModelCapabilitiesCache::get()` which checks the cache first.
pub fn get_model_capabilities_heuristic(provider: &str, model_id: &str) -> ModelCapabilities {
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

/// Convenience function - get capabilities without a cache (uses heuristics only)
pub fn get_model_capabilities(provider: &str, model_id: &str) -> ModelCapabilities {
    get_model_capabilities_heuristic(provider, model_id)
}

fn get_openai_capabilities(model_lower: &str, model_id: &str) -> ModelCapabilities {
    let src = CapabilitySource::StaticFallback;

    // GPT-4o family (128K context)
    if model_lower.starts_with("gpt-4o") || model_lower.starts_with("chatgpt-4o") {
        return ModelCapabilities::new("openai", model_id, 128_000, 16_384, src).with_vision(true);
    }

    // GPT-4 Turbo (128K context)
    if model_lower.contains("gpt-4-turbo") || model_lower.contains("gpt-4-1106") {
        return ModelCapabilities::new("openai", model_id, 128_000, 4_096, src).with_vision(true);
    }

    // GPT-4 base (8K context)
    if model_lower.starts_with("gpt-4") && !model_lower.contains("turbo") {
        return ModelCapabilities::new("openai", model_id, 8_192, 4_096, src);
    }

    // GPT-3.5-turbo - use CONSERVATIVE default since we don't know which version
    if model_lower.contains("gpt-3.5-turbo") {
        // Conservative: assume older 4K limit, will learn actual limit from errors
        return ModelCapabilities::new("openai", model_id, 4_096, 2_048, src);
    }

    // o1/o3/o4/o5 reasoning models (128K+ context)
    if model_lower.starts_with("o1")
        || model_lower.starts_with("o3")
        || model_lower.starts_with("o4")
        || model_lower.starts_with("o5")
    {
        return ModelCapabilities::new("openai", model_id, 128_000, 32_768, src);
    }

    // GPT-5.x and codex models (assume large context)
    if model_lower.starts_with("gpt-5") || model_lower.contains("codex") {
        return ModelCapabilities::new("openai", model_id, 128_000, 16_384, src);
    }

    // Unknown OpenAI model - use VERY conservative defaults
    // Better to compact too early than hit API errors
    ModelCapabilities::new("openai", model_id, 8_192, 4_096, src)
}

fn get_anthropic_capabilities(model_lower: &str, model_id: &str) -> ModelCapabilities {
    let src = CapabilitySource::StaticFallback;

    // Claude 3.5/4 Sonnet and Opus (200K context)
    if model_lower.contains("claude-3")
        || model_lower.contains("claude-sonnet-4")
        || model_lower.contains("claude-opus-4")
    {
        return ModelCapabilities::new("anthropic", model_id, 200_000, 8_192, src)
            .with_vision(true);
    }

    // Claude 2.x (100K context)
    if model_lower.contains("claude-2") {
        return ModelCapabilities::new("anthropic", model_id, 100_000, 4_096, src);
    }

    // Unknown Anthropic model - conservative
    ModelCapabilities::new("anthropic", model_id, 32_000, 4_096, src)
}

fn get_gemini_capabilities(model_lower: &str, model_id: &str) -> ModelCapabilities {
    let src = CapabilitySource::StaticFallback;

    // Gemini 2.0 (1M context)
    if model_lower.contains("gemini-2") {
        return ModelCapabilities::new("gemini", model_id, 1_000_000, 8_192, src).with_vision(true);
    }

    // Gemini 1.5 Pro (1M context)
    if model_lower.contains("1.5-pro") || model_lower.contains("1.5pro") {
        return ModelCapabilities::new("gemini", model_id, 1_000_000, 8_192, src).with_vision(true);
    }

    // Gemini 1.5 Flash (1M context)
    if model_lower.contains("1.5-flash") || model_lower.contains("flash") {
        return ModelCapabilities::new("gemini", model_id, 1_000_000, 8_192, src).with_vision(true);
    }

    // Unknown Gemini model - conservative
    ModelCapabilities::new("gemini", model_id, 32_000, 8_192, src)
}

fn get_grok_capabilities(model_lower: &str, model_id: &str) -> ModelCapabilities {
    let src = CapabilitySource::StaticFallback;

    // Grok-2 and Grok-3 (131K context)
    if model_lower.contains("grok-2") || model_lower.contains("grok-3") {
        return ModelCapabilities::new("grok", model_id, 131_072, 8_192, src).with_vision(true);
    }

    // Grok-1 (8K context)
    if model_lower.contains("grok-1") || model_lower.contains("grok-beta") {
        return ModelCapabilities::new("grok", model_id, 8_192, 4_096, src);
    }

    // Unknown Grok model - conservative
    ModelCapabilities::new("grok", model_id, 32_000, 4_096, src)
}

fn get_ollama_capabilities(model_lower: &str, model_id: &str) -> ModelCapabilities {
    let src = CapabilitySource::StaticFallback;

    // Llama 3.2 (128K context)
    if model_lower.contains("llama3.2") || model_lower.contains("llama-3.2") {
        return ModelCapabilities::new("ollama", model_id, 128_000, 4_096, src);
    }

    // Llama 3.1 (128K context)
    if model_lower.contains("llama3.1") || model_lower.contains("llama-3.1") {
        return ModelCapabilities::new("ollama", model_id, 128_000, 4_096, src);
    }

    // Llama 3 (8K context)
    if model_lower.contains("llama3") || model_lower.contains("llama-3") {
        return ModelCapabilities::new("ollama", model_id, 8_192, 4_096, src);
    }

    // Mistral models (32K context)
    if model_lower.contains("mistral") {
        return ModelCapabilities::new("ollama", model_id, 32_000, 4_096, src);
    }

    // Mixtral (32K context)
    if model_lower.contains("mixtral") {
        return ModelCapabilities::new("ollama", model_id, 32_000, 4_096, src);
    }

    // CodeLlama (16K context)
    if model_lower.contains("codellama") {
        return ModelCapabilities::new("ollama", model_id, 16_384, 4_096, src);
    }

    // Qwen models (32K context for most)
    if model_lower.contains("qwen") {
        return ModelCapabilities::new("ollama", model_id, 32_000, 4_096, src);
    }

    // DeepSeek models (64K context)
    if model_lower.contains("deepseek") {
        return ModelCapabilities::new("ollama", model_id, 64_000, 4_096, src);
    }

    // Unknown Ollama model - very conservative default
    ModelCapabilities::new("ollama", model_id, 4_096, 2_048, src)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_gpt4o_capabilities() {
        let caps = get_model_capabilities("openai", "gpt-4o");
        assert_eq!(caps.context_length, 128_000);
        assert_eq!(caps.max_output_tokens, 16_384);
        assert!(caps.supports_vision);
        assert!(caps.supports_tools);
    }

    #[test]
    fn test_gpt35_turbo_uses_conservative_default() {
        // gpt-3.5-turbo uses conservative default since we can't know which version
        let caps = get_model_capabilities("openai", "gpt-3.5-turbo");
        assert_eq!(caps.context_length, 4_096); // Conservative - will learn actual limit
    }

    #[test]
    fn test_claude_capabilities() {
        let caps = get_model_capabilities("anthropic", "claude-sonnet-4-20250514");
        assert_eq!(caps.context_length, 200_000);
        assert!(caps.supports_vision);
    }

    #[test]
    fn test_gemini_capabilities() {
        let caps = get_model_capabilities("gemini", "gemini-2.0-flash");
        assert_eq!(caps.context_length, 1_000_000);
    }

    #[test]
    fn test_unknown_model_conservative_defaults() {
        let caps = get_model_capabilities("openai", "unknown-model-xyz");
        assert_eq!(caps.context_length, 8_192); // Very conservative default
    }

    #[test]
    fn test_max_input_tokens() {
        let caps = get_model_capabilities("openai", "gpt-4o");
        // 128K - 16K = 112K
        assert_eq!(caps.max_input_tokens(), 128_000 - 16_384);
    }

    #[test]
    fn test_parse_openai_error() {
        let error = "This model's maximum context length is 16385 tokens. \
                     However, your messages resulted in 17063 tokens";
        let length = parse_context_length_from_error(error);
        assert_eq!(length, Some(16385));
    }

    #[test]
    fn test_parse_generic_error() {
        let error = "Request exceeds limit of 8192 tokens";
        let length = parse_context_length_from_error(error);
        assert_eq!(length, Some(8192));
    }

    #[test]
    fn test_cache_learns_from_error() {
        let cache = ModelCapabilitiesCache::new();

        // Initially uses heuristic (conservative)
        let caps_before = cache.get("openai", "gpt-3.5-turbo");
        assert_eq!(caps_before.context_length, 4_096);

        // Learn from error
        cache.learn_from_error(
            "openai",
            "gpt-3.5-turbo",
            "maximum context length is 16385 tokens",
        );

        // Now uses learned value
        let caps_after = cache.get("openai", "gpt-3.5-turbo");
        assert_eq!(caps_after.context_length, 16385);
        assert_eq!(caps_after.source, CapabilitySource::ErrorLearned);
    }

    #[test]
    fn test_cache_user_override() {
        let cache = ModelCapabilitiesCache::new();

        cache.store_user_override("openai", "custom-model", 32_000);

        let caps = cache.get("openai", "custom-model");
        assert_eq!(caps.context_length, 32_000);
        assert_eq!(caps.source, CapabilitySource::UserConfig);
    }

    #[test]
    fn test_capability_source_reliability() {
        let api_caps =
            ModelCapabilities::new("test", "test", 1000, 100, CapabilitySource::ApiDiscovery);
        let error_caps =
            ModelCapabilities::new("test", "test", 1000, 100, CapabilitySource::ErrorLearned);
        let static_caps =
            ModelCapabilities::new("test", "test", 1000, 100, CapabilitySource::StaticFallback);

        assert!(api_caps.is_reliable());
        assert!(!error_caps.is_reliable()); // Learned is useful but not "reliable"
        assert!(!static_caps.is_reliable());
    }
}
