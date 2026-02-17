//! Streaming LLM provider support for Gestura.
//!
//! Domain types and provider configs are defined in `gestura-core-streaming` and
//! re-exported here.  This module adds the core-owned [`streaming_config_from`]
//! function which bridges [`crate::config::AppConfig`] to a
//! [`StreamingConfig`].

pub use gestura_core_streaming::streaming::*;
pub use gestura_core_streaming::{
    AnthropicProviderConfig, GeminiProviderConfig, GrokProviderConfig, OllamaProviderConfig,
    OpenAiProviderConfig, StreamingConfig,
};

use crate::config::AppConfig;

/// Convert an `AppConfig` reference to a `StreamingConfig`.
///
/// This is a free function because the orphan rule prevents `impl From<&AppConfig> for StreamingConfig`
/// when both types are defined in foreign crates.
pub fn streaming_config_from(app: &AppConfig) -> StreamingConfig {
    StreamingConfig {
        primary: app.llm.primary.clone(),
        fallback: app.llm.fallback.clone(),
        openai: app.llm.openai.as_ref().map(|c| OpenAiProviderConfig {
            api_key: c.api_key.clone(),
            base_url: c.base_url.clone(),
            model: c.model.clone(),
        }),
        anthropic: app.llm.anthropic.as_ref().map(|c| AnthropicProviderConfig {
            api_key: c.api_key.clone(),
            base_url: c.base_url.clone(),
            model: c.model.clone(),
            thinking_budget_tokens: c.thinking_budget_tokens,
        }),
        gemini: app.llm.gemini.as_ref().map(|c| GeminiProviderConfig {
            api_key: c.api_key.clone(),
            base_url: c.base_url.clone(),
            model: c.model.clone(),
        }),
        grok: app.llm.grok.as_ref().map(|c| GrokProviderConfig {
            api_key: c.api_key.clone(),
            base_url: c.base_url.clone(),
            model: c.model.clone(),
        }),
        ollama: app.llm.ollama.as_ref().map(|c| OllamaProviderConfig {
            base_url: c.base_url.clone(),
            model: c.model.clone(),
        }),
    }
}
