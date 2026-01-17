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

/// Context hints for provider selection (agent, tenant, etc.)
#[derive(Debug, Clone, Default)]
pub struct AgentContext {
    pub agent_id: String,
}

/// Unified LLM interface (async)
#[async_trait::async_trait]
pub trait LlmProvider: Send + Sync {
    /// Call the LLM with a prompt and return the generated text
    async fn call(&self, prompt: &str) -> Result<String, AppError>;
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

#[async_trait::async_trait]
impl LlmProvider for OpenAiProvider {
    async fn call(&self, prompt: &str) -> Result<String, AppError> {
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
        Ok(text)
    }
}

/// HTTP-based Anthropic Claude provider
pub struct AnthropicProvider {
    pub api_key: String,
    pub base_url: String,
    pub model: String,
}

#[async_trait::async_trait]
impl LlmProvider for AnthropicProvider {
    async fn call(&self, prompt: &str) -> Result<String, AppError> {
        let url = format!("{}/v1/messages", self.base_url);
        let body = serde_json::json!({
            "model": self.model,
            "max_tokens": 512,
            "messages": [{"role":"user","content": [{"type":"text","text": prompt}]}]
        });
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
        let text = v["content"][0]["text"].as_str().unwrap_or("").to_string();
        Ok(text)
    }
}

/// HTTP-based Grok (xAI) provider (OpenAI-compatible endpoint)
pub struct GrokProvider {
    pub api_key: String,
    pub base_url: String,
    pub model: String,
}

#[async_trait::async_trait]
impl LlmProvider for GrokProvider {
    async fn call(&self, prompt: &str) -> Result<String, AppError> {
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
        Ok(text)
    }
}

/// HTTP-based Ollama local provider
pub struct OllamaProvider {
    pub base_url: String,
    pub model: String,
}

#[async_trait::async_trait]
impl LlmProvider for OllamaProvider {
    async fn call(&self, prompt: &str) -> Result<String, AppError> {
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
        Ok(text)
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
        // Default config has "echo" as primary, so select_provider should return EchoProvider
        let _provider = select_provider(&config, &ctx);
        // Provider is created successfully (can't easily test async call here)
    }
}
