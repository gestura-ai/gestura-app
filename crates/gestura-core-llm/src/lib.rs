//! Feature-gated LLM provider implementations and shared provider abstractions.
//!
//! `gestura-core-llm` is the domain crate behind Gestura's provider layer. It
//! defines the common `LlmProvider` trait, shared response/token models, model
//! listing helpers, default model catalogs, and the concrete provider
//! implementations used by the runtime.
//!
//! ## Supported providers
//!
//! Provider implementations are enabled with Cargo features and currently cover:
//!
//! - OpenAI
//! - Anthropic
//! - Grok (xAI)
//! - Gemini
//! - Ollama (local)
//!
//! ## Design role
//!
//! This crate owns provider-specific HTTP behavior and response normalization.
//! Higher-level concerns such as configuration-driven provider selection,
//! runtime overrides, and pipeline orchestration remain in `gestura-core`.
//!
//! The stable public import path for most consumers remains
//! `gestura_core::llm_provider::*`.
//!
//! ## Shared abstractions
//!
//! - `LlmProvider`: async provider interface used by the runtime
//! - `LlmCallResponse`: normalized response with text, usage, and tool calls
//! - `TokenUsage`: provider-agnostic token accounting and estimated cost data
//! - `ToolCallInfo`: normalized native function/tool call representation
//! - `default_models`, `model_listing`, `token_tracker`: support modules for
//!   model defaults, discovery, and token accounting
//!
//! ## Native tool calling
//!
//! Where providers support it, Gestura normalizes native function/tool calling
//! into a common `ToolCallInfo` representation so the pipeline can process tool
//! calls consistently across providers.
//!
//! ## Feature-gated workspace design
//!
//! This crate is intentionally feature-gated so applications can compile only
//! the providers they need. That keeps optional integrations isolated and makes
//! the workspace easier to reason about in `cargo doc` and CI.

pub mod default_models;
pub mod model_listing;
pub mod token_tracker;

use gestura_core_foundation::AppError;
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
    pub fn with_tool_calls(text: String, usage: TokenUsage, tool_calls: Vec<ToolCallInfo>) -> Self {
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
    /// [`Self::call_with_usage`]. Providers should override this to pass tools
    /// natively.
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

#[cfg(feature = "openai")]
/// HTTP-based OpenAI completion provider
pub struct OpenAiProvider {
    pub api_key: String,
    pub base_url: String,
    pub model: String,
}

#[cfg(feature = "openai")]
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

#[cfg(feature = "openai")]
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
        let url = format!(
            "{}/v1/chat/completions",
            self.base_url.trim_end_matches('/')
        );
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

#[cfg(feature = "anthropic")]
/// HTTP-based Anthropic Claude provider
pub struct AnthropicProvider {
    pub api_key: String,
    pub base_url: String,
    pub model: String,

    /// Optional: enable Anthropic "extended thinking" in non-streaming calls.
    /// When set, we inject the `thinking` field into the request body.
    pub thinking_budget_tokens: Option<u32>,
}

#[cfg(feature = "anthropic")]
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

#[cfg(any(feature = "openai", feature = "grok", feature = "ollama"))]
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
            let id = call["id"].as_str().unwrap_or_default().to_string();
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

#[cfg(feature = "anthropic")]
/// Parsed content from an Anthropic `messages` response.
struct AnthropicContent {
    text: String,
    thinking: String,
    tool_calls: Vec<ToolCallInfo>,
}

#[cfg(feature = "anthropic")]
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
#[cfg(all(test, feature = "anthropic"))]
fn anthropic_extract_text_and_thinking(response_json: &serde_json::Value) -> (String, String) {
    let content = anthropic_extract_content(response_json);
    (content.text, content.thinking)
}

#[cfg(feature = "anthropic")]
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

#[cfg(feature = "grok")]
/// HTTP-based Grok (xAI) provider (OpenAI-compatible endpoint)
pub struct GrokProvider {
    pub api_key: String,
    pub base_url: String,
    pub model: String,
}

#[cfg(feature = "grok")]
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

#[cfg(feature = "grok")]
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
        let url = format!(
            "{}/v1/chat/completions",
            self.base_url.trim_end_matches('/')
        );
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

#[cfg(feature = "gemini")]
/// HTTP-based Google Gemini provider (Generative Language API).
///
/// Gemini uses a distinct authentication scheme (API key as a query parameter)
/// and a unique response format where text and tool calls are returned as
/// `parts` inside `candidates[0].content`.
pub struct GeminiProvider {
    /// API key for the Generative Language API.
    pub api_key: String,
    /// Base URL (default: `https://generativelanguage.googleapis.com`).
    pub base_url: String,
    /// Model identifier (e.g. `gemini-2.0-flash`).
    pub model: String,
}

#[cfg(feature = "gemini")]
impl GeminiProvider {
    /// Parse token usage from a Gemini `generateContent` response.
    ///
    /// Gemini reports usage in `usageMetadata.{promptTokenCount, candidatesTokenCount}`.
    fn parse_usage(&self, response: &serde_json::Value) -> TokenUsage {
        let usage = &response["usageMetadata"];
        let input_tokens = usage["promptTokenCount"].as_u64().unwrap_or(0) as u32;
        let output_tokens = usage["candidatesTokenCount"].as_u64().unwrap_or(0) as u32;

        let mut token_usage = TokenUsage::new(input_tokens, output_tokens)
            .with_model(self.model.clone())
            .with_provider("gemini");

        // Gemini pricing (per 1M tokens, as of 2026-02)
        // Gemini 2.0 Flash:      $0.10 / $0.40  (input / output)
        // Gemini 2.0 Flash-Lite: $0.075 / $0.30
        // Gemini 1.5 Pro:        $1.25 / $5.00
        // Gemini 1.5 Flash:      $0.075 / $0.30
        let (input_price, output_price) = match self.model.as_str() {
            m if m.contains("1.5-pro") => (1.25, 5.00),
            m if m.contains("flash-lite") => (0.075, 0.30),
            m if m.contains("1.5-flash") => (0.075, 0.30),
            m if m.contains("flash") => (0.10, 0.40), // 2.0 Flash default
            _ => (0.10, 0.40),
        };
        token_usage.calculate_cost(input_price, output_price);

        token_usage
    }
}

/// Parsed content from a Gemini `generateContent` response.
#[cfg(feature = "gemini")]
struct GeminiContent {
    text: String,
    tool_calls: Vec<ToolCallInfo>,
}

/// Extract text and `functionCall` parts from a Gemini response.
///
/// Gemini returns `candidates[0].content.parts[]` where each part is either
/// `{"text": "..."}` or `{"functionCall": {"name": "...", "args": {...}}}`.
/// Gemini does not assign call-specific IDs, so we synthesize one per call.
#[cfg(feature = "gemini")]
fn gemini_extract_content(response: &serde_json::Value) -> GeminiContent {
    let mut result = GeminiContent {
        text: String::new(),
        tool_calls: Vec::new(),
    };

    let Some(parts) = response["candidates"][0]["content"]["parts"].as_array() else {
        return result;
    };

    for (idx, part) in parts.iter().enumerate() {
        if let Some(text) = part["text"].as_str() {
            if !result.text.is_empty() {
                result.text.push('\n');
            }
            result.text.push_str(text);
        }
        if let Some(fc) = part.get("functionCall") {
            let name = fc["name"].as_str().unwrap_or_default().to_string();
            let args = if let Some(a) = fc.get("args") {
                serde_json::to_string(a).unwrap_or_default()
            } else {
                "{}".to_string()
            };
            if !name.is_empty() {
                result.tool_calls.push(ToolCallInfo {
                    id: format!("gemini-call-{idx}"),
                    name,
                    arguments: args,
                });
            }
        }
    }

    result
}

#[cfg(feature = "gemini")]
#[async_trait::async_trait]
impl LlmProvider for GeminiProvider {
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
        // Gemini authenticates via query parameter, not Bearer token.
        let url = format!(
            "{}/v1beta/models/{}:generateContent?key={}",
            self.base_url, self.model, self.api_key
        );

        let mut body = serde_json::json!({
            "contents": [{"role": "user", "parts": [{"text": prompt}]}]
        });

        // Gemini wraps tool schemas inside `functionDeclarations`.
        if let Some(tools) = tools
            && !tools.is_empty()
        {
            body["tools"] = serde_json::json!([{"functionDeclarations": tools}]);
            body["toolConfig"] = serde_json::json!({"functionCallingConfig": {"mode": "AUTO"}});
        }

        let client = create_http_client();
        let resp = client
            .post(&url)
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await
            .map_err(|e| AppError::Llm(format!("gemini request failed: {e}")))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(AppError::Llm(format!("gemini http {status}: {body}")));
        }

        let v: serde_json::Value = resp.json().await?;
        let content = gemini_extract_content(&v);

        let usage = self.parse_usage(&v);
        tracing::debug!(
            "Gemini token usage: {} input, {} output, ${:.6} estimated, {} tool calls",
            usage.input_tokens,
            usage.output_tokens,
            usage.estimated_cost_usd.unwrap_or(0.0),
            content.tool_calls.len()
        );

        Ok(LlmCallResponse::with_tool_calls(
            content.text,
            usage,
            content.tool_calls,
        ))
    }
}

#[cfg(feature = "ollama")]
/// HTTP-based Ollama local provider
pub struct OllamaProvider {
    pub base_url: String,
    pub model: String,
}

#[cfg(feature = "ollama")]
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

#[cfg(feature = "ollama")]
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
        let url = format!("{}/api/chat", self.base_url.trim_end_matches('/'));
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
pub fn unconfigured_provider(provider_name: &str) -> Box<dyn LlmProvider> {
    Box::new(UnconfiguredProvider {
        provider_name: provider_name.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(any(feature = "anthropic", feature = "gemini"))]
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
    #[cfg(feature = "anthropic")]
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

    #[test]
    #[cfg(feature = "gemini")]
    fn test_gemini_extract_content_text_only() {
        let v = json!({
            "candidates": [{
                "content": {
                    "parts": [{"text": "Hello, world!"}],
                    "role": "model"
                }
            }],
            "usageMetadata": {
                "promptTokenCount": 5,
                "candidatesTokenCount": 3,
                "totalTokenCount": 8
            }
        });
        let content = gemini_extract_content(&v);
        assert_eq!(content.text, "Hello, world!");
        assert!(content.tool_calls.is_empty());
    }

    #[test]
    #[cfg(feature = "gemini")]
    fn test_gemini_extract_content_with_tool_calls() {
        let v = json!({
            "candidates": [{
                "content": {
                    "parts": [
                        {"text": "Let me check that file."},
                        {"functionCall": {
                            "name": "file_read",
                            "args": {"path": "/tmp/test.txt"}
                        }}
                    ],
                    "role": "model"
                }
            }]
        });
        let content = gemini_extract_content(&v);
        assert_eq!(content.text, "Let me check that file.");
        assert_eq!(content.tool_calls.len(), 1);
        assert_eq!(content.tool_calls[0].name, "file_read");
        assert_eq!(content.tool_calls[0].id, "gemini-call-1");
        let args: serde_json::Value =
            serde_json::from_str(&content.tool_calls[0].arguments).unwrap();
        assert_eq!(args["path"], "/tmp/test.txt");
    }

    #[test]
    #[cfg(feature = "gemini")]
    fn test_gemini_extract_content_multiple_tool_calls() {
        let v = json!({
            "candidates": [{
                "content": {
                    "parts": [
                        {"functionCall": {
                            "name": "file_read",
                            "args": {"path": "a.txt"}
                        }},
                        {"functionCall": {
                            "name": "shell_exec",
                            "args": {"command": "ls"}
                        }}
                    ],
                    "role": "model"
                }
            }]
        });
        let content = gemini_extract_content(&v);
        assert!(content.text.is_empty());
        assert_eq!(content.tool_calls.len(), 2);
        assert_eq!(content.tool_calls[0].name, "file_read");
        assert_eq!(content.tool_calls[0].id, "gemini-call-0");
        assert_eq!(content.tool_calls[1].name, "shell_exec");
        assert_eq!(content.tool_calls[1].id, "gemini-call-1");
    }

    #[test]
    #[cfg(feature = "gemini")]
    fn test_gemini_extract_content_empty_response() {
        let v = json!({"candidates": [{"content": {"parts": []}}]});
        let content = gemini_extract_content(&v);
        assert!(content.text.is_empty());
        assert!(content.tool_calls.is_empty());
    }

    #[test]
    #[cfg(feature = "gemini")]
    fn test_gemini_parse_usage() {
        let provider = GeminiProvider {
            api_key: "test".to_string(),
            base_url: "https://example.com".to_string(),
            model: "gemini-2.0-flash".to_string(),
        };
        let v = json!({
            "usageMetadata": {
                "promptTokenCount": 100,
                "candidatesTokenCount": 50,
                "totalTokenCount": 150
            }
        });
        let usage = provider.parse_usage(&v);
        assert_eq!(usage.input_tokens, 100);
        assert_eq!(usage.output_tokens, 50);
        assert_eq!(usage.total_tokens, 150);
        assert_eq!(usage.provider.as_deref(), Some("gemini"));
        assert_eq!(usage.model.as_deref(), Some("gemini-2.0-flash"));
        // 2.0 Flash: $0.10/1M input, $0.40/1M output
        // 100 input → 0.00001, 50 output → 0.00002 → total 0.00003
        let cost = usage.estimated_cost_usd.unwrap();
        assert!((cost - 0.00003).abs() < 1e-9);
    }

    #[test]
    #[cfg(feature = "gemini")]
    fn test_gemini_parse_usage_pro_pricing() {
        let provider = GeminiProvider {
            api_key: "test".to_string(),
            base_url: "https://example.com".to_string(),
            model: "gemini-1.5-pro".to_string(),
        };
        let v = json!({
            "usageMetadata": {
                "promptTokenCount": 1_000_000,
                "candidatesTokenCount": 1_000_000,
                "totalTokenCount": 2_000_000
            }
        });
        let usage = provider.parse_usage(&v);
        // 1.5 Pro: $1.25/1M input, $5.00/1M output → total $6.25
        let cost = usage.estimated_cost_usd.unwrap();
        assert!((cost - 6.25).abs() < 1e-6);
    }
}
