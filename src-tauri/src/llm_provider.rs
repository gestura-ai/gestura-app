//! LLM provider unification (Stage 4 scaffolding)
//! LLM providers with real HTTP calls for OpenAI, Anthropic, Grok (xAI), and Ollama
//! Providers are selected by AppConfig.llm.primary; Echo is used if misconfigured.

//! Provides a trait and a default echo provider that keeps builds green without
//! adding external crates. Real providers will be added behind features.

use crate::{AppConfig, AppError};

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

/// A provider that echoes the prompt for scaffolding and tests
pub struct EchoProvider;
#[async_trait::async_trait]
impl LlmProvider for EchoProvider {
    async fn call(&self, prompt: &str) -> Result<String, AppError> {
        Ok(format!("ECHO: {}", prompt))
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
        let client = reqwest::Client::new();
        let resp = client
            .post(&url)
            .bearer_auth(&self.api_key)
            .json(&body)
            .send()
            .await?;
        if !resp.status().is_success() {
            return Err(AppError::Llm(format!("openai http {}", resp.status())));
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
        let client = reqwest::Client::new();
        let resp = client
            .post(&url)
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", "2023-06-01")
            .json(&body)
            .send()
            .await?;
        if !resp.status().is_success() {
            return Err(AppError::Llm(format!("anthropic http {}", resp.status())));
        }
        let v: serde_json::Value = resp.json().await?;
        let text = v["content"][0]["text"].as_str().unwrap_or("").to_string();
        Ok(text)
    }
}

/// HTTP-based Grok (xAI) provider (OpenAI-compatible endpoint for chat/completions)
pub struct GrokProvider {
    pub api_key: String,
    pub base_url: String,
    pub model: String,
}
#[async_trait::async_trait]
impl LlmProvider for GrokProvider {
    async fn call(&self, prompt: &str) -> Result<String, AppError> {
        // Treat Grok as OpenAI-compatible if base_url provides OpenAI-style API
        let url = format!("{}/v1/chat/completions", self.base_url);
        let body = serde_json::json!({
            "model": self.model,
            "messages": [{"role":"user","content": prompt}],
        });
        let client = reqwest::Client::new();
        let resp = client
            .post(&url)
            .bearer_auth(&self.api_key)
            .json(&body)
            .send()
            .await?;
        if !resp.status().is_success() {
            return Err(AppError::Llm(format!("grok http {}", resp.status())));
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
        // Important: set stream: false to get a single JSON response instead of NDJSON stream
        let body = serde_json::json!({
            "model": self.model,
            "messages": [{"role":"user","content": prompt}],
            "stream": false
        });
        let client = reqwest::Client::new();
        let resp = client.post(&url).json(&body).send().await?;
        if !resp.status().is_success() {
            return Err(AppError::Llm(format!("ollama http {}", resp.status())));
        }
        let v: serde_json::Value = resp.json().await?;
        // With stream: false, Ollama returns a single JSON object with message.content
        let text = v["message"]["content"].as_str().unwrap_or("").to_string();
        Ok(text)
    }
}

/// Select a provider based on config and context
pub fn select_provider(config: &AppConfig, _ctx: &AgentContext) -> Box<dyn LlmProvider> {
    match config.llm.primary.as_str() {
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
                Box::new(EchoProvider)
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
                Box::new(EchoProvider)
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
                Box::new(EchoProvider)
            }
        }
        "ollama" => {
            if let Some(c) = &config.llm.ollama {
                Box::new(OllamaProvider {
                    base_url: c.base_url.clone(),
                    model: c.model.clone(),
                })
            } else {
                Box::new(EchoProvider)
            }
        }
        _ => Box::new(EchoProvider),
    }
}
