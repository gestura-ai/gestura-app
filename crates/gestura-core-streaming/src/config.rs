//! Streaming-specific configuration types.
//!
//! These mirror the subset of `AppConfig::llm` fields needed by the streaming module.
//! Core provides `From<&AppConfig> for StreamingConfig` to bridge the two.

use serde::{Deserialize, Serialize};

/// Configuration for streaming LLM requests.
///
/// This type captures the minimal subset of application configuration
/// required by the streaming module: which provider to use and their credentials.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct StreamingConfig {
    /// Primary provider id: "openai" | "anthropic" | "gemini" | "grok" | "ollama" | "echo"
    pub primary: String,
    /// Fallback provider id (optional): used when primary fails
    #[serde(default)]
    pub fallback: Option<String>,
    /// OpenAI provider configuration
    pub openai: Option<OpenAiProviderConfig>,
    /// Anthropic provider configuration
    pub anthropic: Option<AnthropicProviderConfig>,
    /// Gemini (Google Generative Language API) provider configuration
    pub gemini: Option<GeminiProviderConfig>,
    /// Grok provider configuration
    pub grok: Option<GrokProviderConfig>,
    /// Ollama provider configuration
    pub ollama: Option<OllamaProviderConfig>,
}

impl Default for StreamingConfig {
    fn default() -> Self {
        Self {
            primary: "echo".to_string(),
            fallback: None,
            openai: None,
            anthropic: None,
            gemini: None,
            grok: None,
            ollama: None,
        }
    }
}

/// OpenAI provider credentials and settings.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct OpenAiProviderConfig {
    /// API key for authentication
    #[serde(default)]
    pub api_key: String,
    /// Optional custom base URL (e.g. for Azure OpenAI)
    pub base_url: Option<String>,
    /// Model to use (e.g. "gpt-4o")
    #[serde(default)]
    pub model: String,
}

/// Anthropic provider credentials and settings.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct AnthropicProviderConfig {
    /// API key for authentication
    #[serde(default)]
    pub api_key: String,
    /// Optional custom base URL
    pub base_url: Option<String>,
    /// Model to use (e.g. "claude-sonnet-4-20250514")
    #[serde(default)]
    pub model: String,
    /// Optional: enable Anthropic "extended thinking" streaming.
    #[serde(default)]
    pub thinking_budget_tokens: Option<u32>,
}

/// Grok (xAI) provider credentials and settings.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct GrokProviderConfig {
    /// API key for authentication
    #[serde(default)]
    pub api_key: String,
    /// Optional custom base URL
    pub base_url: Option<String>,
    /// Model to use
    #[serde(default)]
    pub model: String,
}

/// Gemini (Google Generative Language API) provider credentials and settings.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct GeminiProviderConfig {
    /// API key for authentication (passed as query parameter)
    #[serde(default)]
    pub api_key: String,
    /// Optional custom base URL
    pub base_url: Option<String>,
    /// Model to use (e.g. "gemini-2.0-flash")
    #[serde(default)]
    pub model: String,
}

/// Ollama (local) provider settings.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct OllamaProviderConfig {
    /// Base URL for the Ollama server
    pub base_url: String,
    /// Model to use
    pub model: String,
}
