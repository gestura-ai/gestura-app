//! LLM provider unification for Gestura
//!
//! Provides a trait and implementations for various LLM providers:
//! - OpenAI (GPT models)
//! - Anthropic (Claude models)
//! - Grok (xAI)
//! - Ollama (local models)
//! - Echo (testing only - requires `dev` feature)

use crate::config::AppConfig;
use crate::error::AppError;
use serde::{Deserialize, Serialize};
use std::time::Duration;

/// Default timeout for LLM API calls (2 minutes for slow local models)
const LLM_TIMEOUT_SECS: u64 = 120;

/// Create a reqwest client with appropriate timeouts
fn create_http_client() -> reqwest::Client {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(LLM_TIMEOUT_SECS))
        .connect_timeout(Duration::from_secs(10))
        .build()
        .unwrap_or_else(|_| reqwest::Client::new())
}

/// Token usage information from an LLM API call
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TokenUsage {
    /// Number of tokens in the input/prompt
    pub input_tokens: u32,
    /// Number of tokens in the output/completion
    pub output_tokens: u32,
    /// Total tokens (input + output)
    pub total_tokens: u32,
    /// Estimated cost in USD (if available)
    pub estimated_cost_usd: Option<f64>,
    /// Model used for the request
    pub model: Option<String>,
    /// Provider name
    pub provider: Option<String>,
}

impl TokenUsage {
    /// Create a new TokenUsage with the given counts
    pub fn new(input_tokens: u32, output_tokens: u32) -> Self {
        Self {
            input_tokens,
            output_tokens,
            total_tokens: input_tokens + output_tokens,
            estimated_cost_usd: None,
            model: None,
            provider: None,
        }
    }

    /// Create an empty/unknown token usage (for providers that don't report usage)
    pub fn unknown() -> Self {
        Self::default()
    }

    /// Set the estimated cost based on provider pricing
    pub fn with_cost(mut self, cost_usd: f64) -> Self {
        self.estimated_cost_usd = Some(cost_usd);
        self
    }

    /// Set the model name
    pub fn with_model(mut self, model: impl Into<String>) -> Self {
        self.model = Some(model.into());
        self
    }

    /// Set the provider name
    pub fn with_provider(mut self, provider: impl Into<String>) -> Self {
        self.provider = Some(provider.into());
        self
    }

    /// Calculate cost based on standard pricing (per 1M tokens)
    pub fn calculate_cost(&mut self, input_price_per_million: f64, output_price_per_million: f64) {
        let input_cost = (self.input_tokens as f64 / 1_000_000.0) * input_price_per_million;
        let output_cost = (self.output_tokens as f64 / 1_000_000.0) * output_price_per_million;
        self.estimated_cost_usd = Some(input_cost + output_cost);
    }
}

/// Response from an LLM call including token usage
#[derive(Debug, Clone)]
pub struct LlmCallResponse {
    /// The generated text
    pub text: String,
    /// Token usage information
    pub usage: TokenUsage,
}

impl LlmCallResponse {
    /// Create a new LlmCallResponse
    pub fn new(text: String, usage: TokenUsage) -> Self {
        Self { text, usage }
    }

    /// Create a response with unknown token usage
    pub fn with_unknown_usage(text: String) -> Self {
        Self {
            text,
            usage: TokenUsage::unknown(),
        }
    }
}

/// Context hints for provider selection (agent, tenant, etc.)
#[derive(Debug, Clone, Default)]
pub struct AgentContext {
    pub agent_id: String,
}

/// Unified LLM interface (async)
#[async_trait::async_trait]
pub trait LlmProvider: Send + Sync {
    /// Call the LLM with a prompt and return the generated text
    /// For backward compatibility, this returns just the text
    async fn call(&self, prompt: &str) -> Result<String, AppError>;

    /// Call the LLM with a prompt and return the response with token usage
    /// Default implementation calls `call` and returns unknown usage
    async fn call_with_usage(&self, prompt: &str) -> Result<LlmCallResponse, AppError> {
        let text = self.call(prompt).await?;
        Ok(LlmCallResponse::with_unknown_usage(text))
    }
}

/// A provider that echoes the prompt for scaffolding and tests.
///
/// **WARNING**: This provider is only available when the `dev` feature is enabled.
/// It should NEVER be used in production releases.
///
/// # Feature Flag
/// Requires `dev` feature: `gestura-core = { features = ["dev"] }`
#[cfg(any(feature = "dev", test))]
pub struct EchoProvider;

#[cfg(any(feature = "dev", test))]
#[async_trait::async_trait]
impl LlmProvider for EchoProvider {
    async fn call(&self, prompt: &str) -> Result<String, AppError> {
        tracing::warn!("EchoProvider is for development/testing only - not for production use");
        Ok(format!("ECHO: {}", prompt))
    }
}

/// A provider that returns an error when no real provider is configured.
/// Used in production when config is missing.
pub struct UnconfiguredProvider {
    pub provider_name: String,
}

#[async_trait::async_trait]
impl LlmProvider for UnconfiguredProvider {
    async fn call(&self, _prompt: &str) -> Result<String, AppError> {
        Err(AppError::Llm(format!(
            "LLM provider '{}' is not configured. Please configure it in Settings or run 'gestura config edit'.",
            self.provider_name
        )))
    }
}

/// HTTP-based OpenAI chat completion provider
pub struct OpenAiProvider {
    pub api_key: String,
    pub base_url: String,
    pub model: String,
}

impl OpenAiProvider {
    /// Parse token usage from OpenAI API response
    fn parse_usage(&self, response: &serde_json::Value) -> TokenUsage {
        let usage = &response["usage"];
        let input_tokens = usage["prompt_tokens"].as_u64().unwrap_or(0) as u32;
        let output_tokens = usage["completion_tokens"].as_u64().unwrap_or(0) as u32;

        let mut token_usage = TokenUsage::new(input_tokens, output_tokens)
            .with_model(self.model.clone())
            .with_provider("openai");

        // OpenAI pricing (approximate, varies by model)
        // GPT-4o: $2.50/$10 per 1M tokens (input/output)
        // GPT-4: $30/$60 per 1M tokens
        // GPT-3.5-turbo: $0.50/$1.50 per 1M tokens
        let (input_price, output_price) = match self.model.as_str() {
            m if m.starts_with("gpt-4o") => (2.50, 10.0),
            m if m.starts_with("gpt-4") => (30.0, 60.0),
            m if m.starts_with("gpt-3.5") => (0.50, 1.50),
            _ => (2.50, 10.0), // Default to GPT-4o pricing
        };
        token_usage.calculate_cost(input_price, output_price);

        token_usage
    }
}

#[async_trait::async_trait]
impl LlmProvider for OpenAiProvider {
    async fn call(&self, prompt: &str) -> Result<String, AppError> {
        let response = self.call_with_usage(prompt).await?;
        Ok(response.text)
    }

    async fn call_with_usage(&self, prompt: &str) -> Result<LlmCallResponse, AppError> {
        let url = format!("{}/v1/chat/completions", self.base_url);
        let body = serde_json::json!({
            "model": self.model,
            "messages": [{"role":"user","content": prompt}],
            "temperature": 0.2
        });
        let client = create_http_client();
        let resp = client
            .post(&url)
            .bearer_auth(&self.api_key)
            .json(&body)
            .send()
            .await
            .map_err(|e| AppError::Llm(format!("openai request failed: {}", e)))?;
        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(AppError::Llm(format!("openai http {}: {}", status, body)));
        }
        let v: serde_json::Value = resp.json().await?;
        let text = v["choices"][0]["message"]["content"]
            .as_str()
            .unwrap_or("")
            .to_string();

        let usage = self.parse_usage(&v);
        tracing::debug!(
            "OpenAI token usage: {} input, {} output, ${:.6} estimated",
            usage.input_tokens,
            usage.output_tokens,
            usage.estimated_cost_usd.unwrap_or(0.0)
        );

        Ok(LlmCallResponse::new(text, usage))
    }
}

/// HTTP-based Anthropic Claude provider
pub struct AnthropicProvider {
    pub api_key: String,
    pub base_url: String,
    pub model: String,

    /// Optional: enable Anthropic "extended thinking" in non-streaming calls.
    /// When set, we inject the `thinking` field into the request body.
    pub thinking_budget_tokens: Option<u32>,
}

impl AnthropicProvider {
    /// Parse token usage from Anthropic API response
    fn parse_usage(&self, response: &serde_json::Value) -> TokenUsage {
        let usage = &response["usage"];
        let input_tokens = usage["input_tokens"].as_u64().unwrap_or(0) as u32;
        let output_tokens = usage["output_tokens"].as_u64().unwrap_or(0) as u32;

        let mut token_usage = TokenUsage::new(input_tokens, output_tokens)
            .with_model(self.model.clone())
            .with_provider("anthropic");

        // Anthropic pricing (per 1M tokens)
        // Claude 3.5 Sonnet: $3/$15
        // Claude 3 Opus: $15/$75
        // Claude 3 Haiku: $0.25/$1.25
        let (input_price, output_price) = match self.model.as_str() {
            m if m.contains("opus") => (15.0, 75.0),
            m if m.contains("sonnet") => (3.0, 15.0),
            m if m.contains("haiku") => (0.25, 1.25),
            _ => (3.0, 15.0), // Default to Sonnet pricing
        };
        token_usage.calculate_cost(input_price, output_price);

        token_usage
    }
}

/// Extracts text and thinking content from an Anthropic `messages` response.
///
/// Anthropic returns `content` as an array of blocks (e.g. `text`, `tool_use`, and optionally
/// `thinking`). We conservatively collect known textual fields into two strings.
fn anthropic_extract_text_and_thinking(response_json: &serde_json::Value) -> (String, String) {
    let mut text = String::new();
    let mut thinking = String::new();

    let Some(blocks) = response_json["content"].as_array() else {
        return (text, thinking);
    };

    for block in blocks {
        let block_type = block["type"].as_str().unwrap_or("");
        match block_type {
            "text" => {
                if let Some(t) = block.get("text").and_then(|v| v.as_str()) {
                    text.push_str(t);
                }
            }
            "thinking" => {
                // Different schemas represent this payload with different keys.
                if let Some(t) = block
                    .get("thinking")
                    .and_then(|v| v.as_str())
                    .or_else(|| block.get("text").and_then(|v| v.as_str()))
                {
                    thinking.push_str(t);
                }
            }
            _ => {}
        }
    }

    (text, thinking)
}

#[async_trait::async_trait]
impl LlmProvider for AnthropicProvider {
    async fn call(&self, prompt: &str) -> Result<String, AppError> {
        let response = self.call_with_usage(prompt).await?;
        Ok(response.text)
    }

    async fn call_with_usage(&self, prompt: &str) -> Result<LlmCallResponse, AppError> {
        let url = format!("{}/v1/messages", self.base_url);
        let mut body = serde_json::json!({
            "model": self.model,
            "max_tokens": 512,
            "messages": [{"role":"user","content": [{"type":"text","text": prompt}]}]
        });

        if let Some(budget_tokens) = self.thinking_budget_tokens {
            // `body` is created from a JSON object literal above, so direct indexing is safe.
            body["thinking"] =
                serde_json::json!({ "type": "enabled", "budget_tokens": budget_tokens });
        }
        let client = create_http_client();
        let resp = client
            .post(&url)
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", "2023-06-01")
            .json(&body)
            .send()
            .await
            .map_err(|e| AppError::Llm(format!("anthropic request failed: {}", e)))?;
        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(AppError::Llm(format!(
                "anthropic http {}: {}",
                status, body
            )));
        }
        let v: serde_json::Value = resp.json().await?;
        let (text, thinking) = anthropic_extract_text_and_thinking(&v);
        let text = if thinking.trim().is_empty() {
            text
        } else {
            // Normalize provider-native thinking into our generic <think> format so the rest of the
            // pipeline can split it consistently.
            format!("<think>{}</think>{}", thinking, text)
        };

        let usage = self.parse_usage(&v);
        tracing::debug!(
            "Anthropic token usage: {} input, {} output, ${:.6} estimated",
            usage.input_tokens,
            usage.output_tokens,
            usage.estimated_cost_usd.unwrap_or(0.0)
        );

        Ok(LlmCallResponse::new(text, usage))
    }
}

/// HTTP-based Grok (xAI) provider (OpenAI-compatible endpoint)
pub struct GrokProvider {
    pub api_key: String,
    pub base_url: String,
    pub model: String,
}

impl GrokProvider {
    /// Parse token usage from Grok API response (OpenAI-compatible format)
    fn parse_usage(&self, response: &serde_json::Value) -> TokenUsage {
        let usage = &response["usage"];
        let input_tokens = usage["prompt_tokens"].as_u64().unwrap_or(0) as u32;
        let output_tokens = usage["completion_tokens"].as_u64().unwrap_or(0) as u32;

        let mut token_usage = TokenUsage::new(input_tokens, output_tokens)
            .with_model(self.model.clone())
            .with_provider("grok");

        // Grok pricing (per 1M tokens) - xAI pricing
        // Grok-2: $2/$10 (estimated)
        token_usage.calculate_cost(2.0, 10.0);

        token_usage
    }
}

#[async_trait::async_trait]
impl LlmProvider for GrokProvider {
    async fn call(&self, prompt: &str) -> Result<String, AppError> {
        let response = self.call_with_usage(prompt).await?;
        Ok(response.text)
    }

    async fn call_with_usage(&self, prompt: &str) -> Result<LlmCallResponse, AppError> {
        let url = format!("{}/v1/chat/completions", self.base_url);
        let body = serde_json::json!({
            "model": self.model,
            "messages": [{"role":"user","content": prompt}],
        });
        let client = create_http_client();
        let resp = client
            .post(&url)
            .bearer_auth(&self.api_key)
            .json(&body)
            .send()
            .await
            .map_err(|e| AppError::Llm(format!("grok request failed: {}", e)))?;
        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(AppError::Llm(format!("grok http {}: {}", status, body)));
        }
        let v: serde_json::Value = resp.json().await?;
        let text = v["choices"][0]["message"]["content"]
            .as_str()
            .unwrap_or("")
            .to_string();

        let usage = self.parse_usage(&v);
        tracing::debug!(
            "Grok token usage: {} input, {} output, ${:.6} estimated",
            usage.input_tokens,
            usage.output_tokens,
            usage.estimated_cost_usd.unwrap_or(0.0)
        );

        Ok(LlmCallResponse::new(text, usage))
    }
}

/// HTTP-based Ollama local provider
pub struct OllamaProvider {
    pub base_url: String,
    pub model: String,
}

impl OllamaProvider {
    /// Parse token usage from Ollama API response
    fn parse_usage(&self, response: &serde_json::Value) -> TokenUsage {
        // Ollama returns eval_count (output tokens) and prompt_eval_count (input tokens)
        let input_tokens = response["prompt_eval_count"].as_u64().unwrap_or(0) as u32;
        let output_tokens = response["eval_count"].as_u64().unwrap_or(0) as u32;

        // Ollama is local, so no cost
        TokenUsage::new(input_tokens, output_tokens)
            .with_model(self.model.clone())
            .with_provider("ollama")
            .with_cost(0.0)
    }
}

#[async_trait::async_trait]
impl LlmProvider for OllamaProvider {
    async fn call(&self, prompt: &str) -> Result<String, AppError> {
        let response = self.call_with_usage(prompt).await?;
        Ok(response.text)
    }

    async fn call_with_usage(&self, prompt: &str) -> Result<LlmCallResponse, AppError> {
        let url = format!("{}/api/chat", self.base_url);
        let body = serde_json::json!({
            "model": self.model,
            "messages": [{"role":"user","content": prompt}],
            "stream": false
        });
        let client = create_http_client();
        let resp = client
            .post(&url)
            .json(&body)
            .send()
            .await
            .map_err(|e| AppError::Llm(format!("ollama request failed: {}", e)))?;
        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(AppError::Llm(format!("ollama http {}: {}", status, body)));
        }
        let v: serde_json::Value = resp.json().await?;
        let text = v["message"]["content"].as_str().unwrap_or("").to_string();

        let usage = self.parse_usage(&v);
        tracing::debug!(
            "Ollama token usage: {} input, {} output (local, no cost)",
            usage.input_tokens,
            usage.output_tokens
        );

        Ok(LlmCallResponse::new(text, usage))
    }
}

/// Helper to create a fallback provider based on feature flags.
/// In dev/test mode, returns EchoProvider. In production, returns UnconfiguredProvider.
fn fallback_provider(provider_name: &str) -> Box<dyn LlmProvider> {
    #[cfg(any(feature = "dev", test))]
    {
        tracing::warn!(
            "Provider '{}' not configured, falling back to EchoProvider (dev mode)",
            provider_name
        );
        Box::new(EchoProvider)
    }
    #[cfg(not(any(feature = "dev", test)))]
    {
        Box::new(UnconfiguredProvider {
            provider_name: provider_name.to_string(),
        })
    }
}

/// Select a provider based on config and context.
///
/// # Production Behavior
/// If the selected provider is not configured, returns `UnconfiguredProvider` which
/// will return an error when called. This prevents silent failures in production.
///
/// # Development Behavior (with `dev` feature)
/// Falls back to `EchoProvider` for testing convenience.
pub fn select_provider(config: &AppConfig, _ctx: &AgentContext) -> Box<dyn LlmProvider> {
    match config.llm.primary.as_str() {
        #[cfg(any(feature = "dev", test))]
        "echo" => Box::new(EchoProvider),
        "openai" => {
            if let Some(c) = &config.llm.openai {
                Box::new(OpenAiProvider {
                    api_key: c.api_key.clone(),
                    base_url: c
                        .base_url
                        .clone()
                        .unwrap_or_else(|| "https://api.openai.com".into()),
                    model: c.model.clone(),
                })
            } else {
                fallback_provider("openai")
            }
        }
        "anthropic" => {
            if let Some(c) = &config.llm.anthropic {
                Box::new(AnthropicProvider {
                    api_key: c.api_key.clone(),
                    base_url: c
                        .base_url
                        .clone()
                        .unwrap_or_else(|| "https://api.anthropic.com".into()),
                    model: c.model.clone(),
                    thinking_budget_tokens: c.thinking_budget_tokens,
                })
            } else {
                fallback_provider("anthropic")
            }
        }
        "grok" => {
            if let Some(c) = &config.llm.grok {
                Box::new(GrokProvider {
                    api_key: c.api_key.clone(),
                    base_url: c
                        .base_url
                        .clone()
                        .unwrap_or_else(|| "https://api.x.ai".into()),
                    model: c.model.clone(),
                })
            } else {
                fallback_provider("grok")
            }
        }
        "ollama" => {
            if let Some(c) = &config.llm.ollama {
                Box::new(OllamaProvider {
                    base_url: c.base_url.clone(),
                    model: c.model.clone(),
                })
            } else {
                fallback_provider("ollama")
            }
        }
        other => fallback_provider(other),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[tokio::test]
    async fn test_echo_provider() {
        let provider = EchoProvider;
        let result = provider.call("Hello").await.unwrap();
        assert_eq!(result, "ECHO: Hello");
    }

    #[test]
    fn test_select_provider_default() {
        let config = AppConfig::default();
        let ctx = AgentContext::default();
        // Default config has "anthropic" as primary, but no API key configured
        // so select_provider should return fallback provider (EchoProvider in dev/test mode)
        let _provider = select_provider(&config, &ctx);
        // Provider is created successfully (can't easily test async call here)
    }

    #[test]
    fn test_anthropic_extract_text_and_thinking() {
        let v = json!({
            "content": [
                {"type": "thinking", "thinking": "plan\n"},
                {"type": "text", "text": "answer"}
            ]
        });
        let (text, thinking) = anthropic_extract_text_and_thinking(&v);
        assert_eq!(text, "answer");
        assert_eq!(thinking, "plan\n");
    }
}
