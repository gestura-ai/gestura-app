//! Compatibility facade for LLM providers.
//!
//! The implementation lives in `gestura-core-llm`; this module preserves the
//! stable public path `gestura_core::llm_provider::*`.

pub use gestura_core_llm::*;

use crate::config::AppConfig;

/// Select a provider based on config and context.
///
/// If the selected provider is not configured, returns `UnconfiguredProvider` which
/// will return an error when called. This prevents silent failures.
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
        "gemini" => {
            if let Some(c) = &config.llm.gemini {
                Box::new(GeminiProvider {
                    api_key: c.api_key.clone(),
                    base_url: c
                        .base_url
                        .clone()
                        .unwrap_or_else(|| "https://generativelanguage.googleapis.com".into()),
                    model: c.model.clone(),
                })
            } else {
                unconfigured_provider("gemini")
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
