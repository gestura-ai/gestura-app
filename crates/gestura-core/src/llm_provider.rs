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

/// A structured tool call returned by the LLM when using native function calling.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCallInfo {
    /// Provider-assigned call ID (e.g. `call_abc123` for OpenAI, `toolu_xxx` for Anthropic)
    pub id: String,
    /// Tool name
    pub name: String,
    /// JSON-encoded arguments string
    pub arguments: String,
}

/// Response from an LLM call including token usage
#[derive(Debug, Clone)]
pub struct LlmCallResponse {
    /// The generated text
    pub text: String,
    /// Token usage information
    pub usage: TokenUsage,
    /// Structured tool calls returned by the model (empty when the model responds with text only)
    pub tool_calls: Vec<ToolCallInfo>,
}

impl LlmCallResponse {
    /// Create a new LlmCallResponse (text-only, no tool calls)
    pub fn new(text: String, usage: TokenUsage) -> Self {
        Self {
            text,
            usage,
            tool_calls: Vec::new(),
        }
    }

    /// Create a response with unknown token usage
    pub fn with_unknown_usage(text: String) -> Self {
        Self {
            text,
            usage: TokenUsage::unknown(),
            tool_calls: Vec::new(),
        }
    }

    /// Create a new LlmCallResponse with tool calls
    pub fn with_tool_calls(
        text: String,
        usage: TokenUsage,
        tool_calls: Vec<ToolCallInfo>,
    ) -> Self {
        Self {
            text,
            usage,
            tool_calls,
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
    /// Call the LLM with a prompt and return the generated text.
    /// For backward compatibility, this returns just the text.
    async fn call(&self, prompt: &str) -> Result<String, AppError>;

    /// Call the LLM with a prompt and return the response with token usage.
    /// Default implementation calls `call` and returns unknown usage.
    async fn call_with_usage(&self, prompt: &str) -> Result<LlmCallResponse, AppError> {
        let text = self.call(prompt).await?;
        Ok(LlmCallResponse::with_unknown_usage(text))
    }

    /// Call the LLM with a prompt **and** optional tool schemas.
    ///
    /// When `tools` is `Some`, providers that support native tool/function calling
    /// will include the schemas in the API request body, enabling the model to
    /// return structured tool call responses.
    ///
    /// The default implementation ignores the tools parameter and delegates to
    /// [`call_with_usage`]. Providers should override this to pass tools natively.
    async fn call_with_tools(
        &self,
        prompt: &str,
        _tools: Option<&[serde_json::Value]>,
    ) -> Result<LlmCallResponse, AppError> {
        self.call_with_usage(prompt).await
    }
}

/// A provider that returns an error when no real provider is configured.
/// Used when config is missing or invalid.
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

/// A deterministic LLM provider for development/testing.
///
/// This provider simply returns the prompt verbatim ("echo"). It is only available
/// during tests or when the `dev` feature is enabled.
#[cfg(any(test, feature = "dev"))]
pub struct EchoProvider;

#[cfg(any(test, feature = "dev"))]
#[async_trait::async_trait]
impl LlmProvider for EchoProvider {
    /// Return the prompt as-is.
    async fn call(&self, prompt: &str) -> Result<String, AppError> {
        Ok(prompt.to_string())
    }

    /// Return the prompt as-is with a zero-cost, "echo" usage marker.
    async fn call_with_usage(&self, prompt: &str) -> Result<LlmCallResponse, AppError> {
        let usage = TokenUsage::unknown().with_provider("echo").with_cost(0.0);
        Ok(LlmCallResponse::new(prompt.to_string(), usage))
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
        self.call_with_tools(prompt, None).await
    }

    async fn call_with_tools(
        &self,
        prompt: &str,
        tools: Option<&[serde_json::Value]>,
    ) -> Result<LlmCallResponse, AppError> {
        let url = format!("{}/v1/chat/completions", self.base_url);
        // NOTE: We intentionally omit `temperature`.
        // Some OpenAI(-compatible) models only support the default value and will
        // return HTTP 400 if a non-default temperature is provided.
        let mut body = serde_json::json!({
            "model": self.model,
            "messages": [{"role":"user","content": prompt}]
        });

        if let Some(tools) = tools
            && !tools.is_empty()
        {
            body["tools"] = serde_json::Value::Array(tools.to_vec());
            body["tool_choice"] = serde_json::json!("auto");
        }

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

        // Extract structured tool calls from the response.
        let tool_calls = extract_openai_tool_calls(&v["choices"][0]["message"]);

        let usage = self.parse_usage(&v);
        tracing::debug!(
            "OpenAI token usage: {} input, {} output, ${:.6} estimated, {} tool calls",
            usage.input_tokens,
            usage.output_tokens,
            usage.estimated_cost_usd.unwrap_or(0.0),
            tool_calls.len()
        );

        Ok(LlmCallResponse::with_tool_calls(text, usage, tool_calls))
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

/// Extract structured tool calls from an OpenAI-compatible `message` object.
///
/// Works for OpenAI, Grok, and Ollama — all three use the same
/// `message.tool_calls[].{id, function.name, function.arguments}` format.
fn extract_openai_tool_calls(message: &serde_json::Value) -> Vec<ToolCallInfo> {
    let Some(tool_calls) = message["tool_calls"].as_array() else {
        return Vec::new();
    };

    tool_calls
        .iter()
        .filter_map(|call| {
            let name = call["function"]["name"].as_str()?;
            let id = call["id"]
                .as_str()
                .unwrap_or_default()
                .to_string();
            let arguments = call["function"]["arguments"]
                .as_str()
                .unwrap_or("{}")
                .to_string();
            Some(ToolCallInfo {
                id,
                name: name.to_string(),
                arguments,
            })
        })
        .collect()
}

/// Parsed content from an Anthropic `messages` response.
struct AnthropicContent {
    text: String,
    thinking: String,
    tool_calls: Vec<ToolCallInfo>,
}

/// Extracts text, thinking, and tool_use content from an Anthropic `messages` response.
///
/// Anthropic returns `content` as an array of blocks (e.g. `text`, `tool_use`, and optionally
/// `thinking`). We extract all three block types.
fn anthropic_extract_content(response_json: &serde_json::Value) -> AnthropicContent {
    let mut result = AnthropicContent {
        text: String::new(),
        thinking: String::new(),
        tool_calls: Vec::new(),
    };

    let Some(blocks) = response_json["content"].as_array() else {
        return result;
    };

    for block in blocks {
        let block_type = block["type"].as_str().unwrap_or("");
        match block_type {
            "text" => {
                if let Some(t) = block.get("text").and_then(|v| v.as_str()) {
                    result.text.push_str(t);
                }
            }
            "thinking" => {
                // Different schemas represent this payload with different keys.
                if let Some(t) = block
                    .get("thinking")
                    .and_then(|v| v.as_str())
                    .or_else(|| block.get("text").and_then(|v| v.as_str()))
                {
                    result.thinking.push_str(t);
                }
            }
            "tool_use" => {
                let id = block["id"].as_str().unwrap_or_default().to_string();
                let name = block["name"].as_str().unwrap_or_default().to_string();
                // Anthropic returns `input` as a JSON object; serialize it to a string.
                let arguments = if let Some(input) = block.get("input") {
                    serde_json::to_string(input).unwrap_or_default()
                } else {
                    "{}".to_string()
                };
                if !name.is_empty() {
                    result.tool_calls.push(ToolCallInfo {
                        id,
                        name,
                        arguments,
                    });
                }
            }
            _ => {}
        }
    }

    result
}

/// Backwards-compatible wrapper that extracts only text and thinking.
///
/// Used by test code to validate extraction without needing the full `AnthropicContent` struct.
#[cfg(test)]
fn anthropic_extract_text_and_thinking(response_json: &serde_json::Value) -> (String, String) {
    let content = anthropic_extract_content(response_json);
    (content.text, content.thinking)
}

#[async_trait::async_trait]
impl LlmProvider for AnthropicProvider {
    async fn call(&self, prompt: &str) -> Result<String, AppError> {
        let response = self.call_with_usage(prompt).await?;
        Ok(response.text)
    }

    async fn call_with_usage(&self, prompt: &str) -> Result<LlmCallResponse, AppError> {
        self.call_with_tools(prompt, None).await
    }

    async fn call_with_tools(
        &self,
        prompt: &str,
        tools: Option<&[serde_json::Value]>,
    ) -> Result<LlmCallResponse, AppError> {
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

        // Anthropic uses its own tool schema format: {name, description, input_schema}.
        if let Some(tools) = tools
            && !tools.is_empty()
        {
            body["tools"] = serde_json::Value::Array(tools.to_vec());
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
        let content = anthropic_extract_content(&v);
        let text = if content.thinking.trim().is_empty() {
            content.text
        } else {
            // Normalize provider-native thinking into our generic <think> format so the rest of the
            // pipeline can split it consistently.
            format!("<think>{}</think>{}", content.thinking, content.text)
        };

        let usage = self.parse_usage(&v);
        tracing::debug!(
            "Anthropic token usage: {} input, {} output, ${:.6} estimated, {} tool calls",
            usage.input_tokens,
            usage.output_tokens,
            usage.estimated_cost_usd.unwrap_or(0.0),
            content.tool_calls.len()
        );

        Ok(LlmCallResponse::with_tool_calls(
            text,
            usage,
            content.tool_calls,
        ))
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
        self.call_with_tools(prompt, None).await
    }

    async fn call_with_tools(
        &self,
        prompt: &str,
        tools: Option<&[serde_json::Value]>,
    ) -> Result<LlmCallResponse, AppError> {
        let url = format!("{}/v1/chat/completions", self.base_url);
        // Grok is OpenAI-compatible, so uses the same tool schema format.
        let mut body = serde_json::json!({
            "model": self.model,
            "messages": [{"role":"user","content": prompt}],
        });

        if let Some(tools) = tools
            && !tools.is_empty()
        {
            body["tools"] = serde_json::Value::Array(tools.to_vec());
            body["tool_choice"] = serde_json::json!("auto");
        }

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

        // Extract structured tool calls (Grok uses OpenAI-compatible format).
        let tool_calls = extract_openai_tool_calls(&v["choices"][0]["message"]);

        let usage = self.parse_usage(&v);
        tracing::debug!(
            "Grok token usage: {} input, {} output, ${:.6} estimated, {} tool calls",
            usage.input_tokens,
            usage.output_tokens,
            usage.estimated_cost_usd.unwrap_or(0.0),
            tool_calls.len()
        );

        Ok(LlmCallResponse::with_tool_calls(text, usage, tool_calls))
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
        self.call_with_tools(prompt, None).await
    }

    async fn call_with_tools(
        &self,
        prompt: &str,
        tools: Option<&[serde_json::Value]>,
    ) -> Result<LlmCallResponse, AppError> {
        let url = format!("{}/api/chat", self.base_url);
        // Ollama uses OpenAI-compatible tool schema format.
        let mut body = serde_json::json!({
            "model": self.model,
            "messages": [{"role":"user","content": prompt}],
            "stream": false
        });

        if let Some(tools) = tools
            && !tools.is_empty()
        {
            body["tools"] = serde_json::Value::Array(tools.to_vec());
        }

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

        // Extract structured tool calls (Ollama uses OpenAI-compatible format).
        let tool_calls = extract_openai_tool_calls(&v["message"]);

        let usage = self.parse_usage(&v);
        tracing::debug!(
            "Ollama token usage: {} input, {} output (local, no cost), {} tool calls",
            usage.input_tokens,
            usage.output_tokens,
            tool_calls.len()
        );

        Ok(LlmCallResponse::with_tool_calls(text, usage, tool_calls))
    }
}

/// Create an unconfigured provider that returns an error when called.
/// Used when a provider is not properly configured.
fn unconfigured_provider(provider_name: &str) -> Box<dyn LlmProvider> {
    Box::new(UnconfiguredProvider {
        provider_name: provider_name.to_string(),
    })
}

/// Select a provider based on config and context.
///
/// If the selected provider is not configured, returns `UnconfiguredProvider` which
/// will return an error when called. This prevents silent failures.
pub fn select_provider(config: &AppConfig, _ctx: &AgentContext) -> Box<dyn LlmProvider> {
    match config.llm.primary.as_str() {
        #[cfg(any(test, feature = "dev"))]
        "echo" => Box::new(EchoProvider),
        #[cfg(not(any(test, feature = "dev")))]
        "echo" => unconfigured_provider("echo"),
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
                unconfigured_provider("openai")
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
                unconfigured_provider("anthropic")
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
                unconfigured_provider("grok")
            }
        }
        "ollama" => {
            if let Some(c) = &config.llm.ollama {
                Box::new(OllamaProvider {
                    base_url: c.base_url.clone(),
                    model: c.model.clone(),
                })
            } else {
                unconfigured_provider("ollama")
            }
        }
        other => unconfigured_provider(other),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[tokio::test]
    async fn test_unconfigured_provider_returns_error() {
        let provider = UnconfiguredProvider {
            provider_name: "test".to_string(),
        };
        let result = provider.call("Hello").await;
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.to_string().contains("not configured"));
    }

    #[test]
    fn test_select_provider_unconfigured() {
        let config = AppConfig::default();
        let ctx = AgentContext::default();
        // Default config has "anthropic" as primary, but no API key configured
        // so select_provider should return UnconfiguredProvider
        let _provider = select_provider(&config, &ctx);
        // Provider is created successfully (returns error when called)
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
