//! Dynamic model metadata discovery via provider APIs.
//!
//! This module queries provider APIs at runtime to discover actual model
//! capabilities (context length, output limits, features) rather than relying
//! on static mappings.
//!
//! ## Supported Providers
//!
//! | Provider | Endpoint | Context Length Field |
//! |----------|----------|---------------------|
//! | Gemini | `GET /v1beta/models` | `inputTokenLimit` |
//! | Anthropic | `GET /v1/models` | `max_input_tokens` |
//! | Grok (xAI) | `GET /v1/language-models` | `context` field |
//! | Ollama | `POST /api/show` | `model_info.*.context_length` |
//! | OpenAI | N/A | Uses error-driven learning |

use crate::model_capabilities::{CapabilitySource, ModelCapabilities, ModelCapabilitiesCache};
use gestura_core_foundation::AppError;
use std::time::Duration;

/// Timeout for metadata discovery API calls (shorter than inference)
const DISCOVERY_TIMEOUT_SECS: u64 = 10;

/// Create a lightweight HTTP client for discovery calls
fn discovery_client() -> reqwest::Client {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(DISCOVERY_TIMEOUT_SECS))
        .connect_timeout(Duration::from_secs(5))
        .build()
        .unwrap_or_else(|_| reqwest::Client::new())
}

/// Discover model capabilities from provider API and store in cache.
///
/// Returns the discovered capabilities, or None if discovery failed.
/// Failures are logged but don't prevent operation - we fall back to heuristics.
pub async fn discover_model_capabilities(
    cache: &ModelCapabilitiesCache,
    provider: &str,
    model_id: &str,
    api_key: Option<&str>,
    base_url: Option<&str>,
) -> Option<ModelCapabilities> {
    let result = match provider.to_lowercase().as_str() {
        "gemini" => discover_gemini(model_id, api_key, base_url).await,
        "anthropic" => discover_anthropic(model_id, api_key).await,
        "grok" => discover_grok(model_id, api_key).await,
        "ollama" => discover_ollama(model_id, base_url).await,
        // OpenAI doesn't expose context length in API - use error-driven learning
        "openai" => {
            tracing::debug!(
                "OpenAI doesn't expose context length in API - using heuristics"
            );
            return None;
        }
        _ => {
            tracing::debug!(provider = provider, "Unknown provider for discovery");
            return None;
        }
    };

    match result {
        Ok(caps) => {
            tracing::info!(
                provider = provider,
                model = model_id,
                context_length = caps.context_length,
                "Discovered model capabilities from API"
            );
            cache.store_from_api(caps.clone());
            Some(caps)
        }
        Err(e) => {
            tracing::debug!(
                provider = provider,
                model = model_id,
                error = %e,
                "Failed to discover model capabilities - using heuristics"
            );
            None
        }
    }
}

/// Discover capabilities for a Gemini model via Google's API.
///
/// Endpoint: `GET /v1beta/models/{model_id}`
/// Returns: `inputTokenLimit`, `outputTokenLimit`
async fn discover_gemini(
    model_id: &str,
    api_key: Option<&str>,
    base_url: Option<&str>,
) -> Result<ModelCapabilities, AppError> {
    let key = api_key
        .filter(|k| !k.is_empty())
        .ok_or_else(|| AppError::Config("Gemini API key required for discovery".into()))?;

    let base = base_url
        .filter(|u| !u.is_empty())
        .unwrap_or("https://generativelanguage.googleapis.com");

    // Gemini model IDs may or may not have "models/" prefix
    let model_path = if model_id.starts_with("models/") {
        model_id.to_string()
    } else {
        format!("models/{}", model_id)
    };

    let url = format!("{}/v1beta/{}?key={}", base.trim_end_matches('/'), model_path, key);

    let resp = discovery_client()
        .get(&url)
        .send()
        .await
        .map_err(|e| AppError::Llm(format!("Gemini discovery failed: {e}")))?;

    if !resp.status().is_success() {
        return Err(AppError::Llm(format!(
            "Gemini API returned {}",
            resp.status()
        )));
    }

    let data: serde_json::Value = resp.json().await?;

    let input_limit = data
        .get("inputTokenLimit")
        .and_then(|v| v.as_u64())
        .unwrap_or(32_000) as usize;

    let output_limit = data
        .get("outputTokenLimit")
        .and_then(|v| v.as_u64())
        .unwrap_or(8_192) as usize;

    Ok(ModelCapabilities::new(
        "gemini",
        model_id,
        input_limit,
        output_limit,
        CapabilitySource::ApiDiscovery,
    ).with_vision(true))
}

/// Discover capabilities for an Anthropic model via their API.
///
/// Endpoint: `GET /v1/models/{model_id}`
/// Returns: `max_input_tokens`, `max_output_tokens`
async fn discover_anthropic(
    model_id: &str,
    api_key: Option<&str>,
) -> Result<ModelCapabilities, AppError> {
    let key = api_key
        .filter(|k| !k.is_empty())
        .ok_or_else(|| AppError::Config("Anthropic API key required for discovery".into()))?;

    let url = format!("https://api.anthropic.com/v1/models/{}", model_id);

    let resp = discovery_client()
        .get(&url)
        .header("x-api-key", key)
        .header("anthropic-version", "2023-06-01")
        .send()
        .await
        .map_err(|e| AppError::Llm(format!("Anthropic discovery failed: {e}")))?;

    if !resp.status().is_success() {
        return Err(AppError::Llm(format!(
            "Anthropic API returned {}",
            resp.status()
        )));
    }

    let data: serde_json::Value = resp.json().await?;

    // Anthropic returns max_input_tokens directly
    let input_limit = data
        .get("max_input_tokens")
        .and_then(|v| v.as_u64())
        .unwrap_or(200_000) as usize;

    let output_limit = data
        .get("max_output_tokens")
        .and_then(|v| v.as_u64())
        .unwrap_or(8_192) as usize;

    Ok(ModelCapabilities::new(
        "anthropic",
        model_id,
        input_limit,
        output_limit,
        CapabilitySource::ApiDiscovery,
    ).with_vision(true))
}

/// Discover capabilities for a Grok (xAI) model via their API.
///
/// Endpoint: `GET /v1/language-models`
/// Returns: List of models with context info
async fn discover_grok(
    model_id: &str,
    api_key: Option<&str>,
) -> Result<ModelCapabilities, AppError> {
    let key = api_key
        .filter(|k| !k.is_empty())
        .ok_or_else(|| AppError::Config("Grok API key required for discovery".into()))?;

    let url = "https://api.x.ai/v1/language-models";

    let resp = discovery_client()
        .get(url)
        .header("Authorization", format!("Bearer {}", key))
        .send()
        .await
        .map_err(|e| AppError::Llm(format!("Grok discovery failed: {e}")))?;

    if !resp.status().is_success() {
        return Err(AppError::Llm(format!(
            "Grok API returned {}",
            resp.status()
        )));
    }

    let data: serde_json::Value = resp.json().await?;

    // Find the model in the list
    let models = data.get("models").and_then(|m| m.as_array());
    let model_data = models.and_then(|list| {
        list.iter().find(|m| {
            m.get("id")
                .and_then(|id| id.as_str())
                .map(|id| id == model_id)
                .unwrap_or(false)
        })
    });

    let (input_limit, output_limit) = if let Some(model) = model_data {
        let input = model
            .get("input_modalities")
            .and_then(|m| m.get("text"))
            .and_then(|t| t.get("token_limit"))
            .and_then(|v| v.as_u64())
            .unwrap_or(131_072) as usize;

        let output = model
            .get("output_modalities")
            .and_then(|m| m.get("text"))
            .and_then(|t| t.get("token_limit"))
            .and_then(|v| v.as_u64())
            .unwrap_or(8_192) as usize;

        (input, output)
    } else {
        // Model not in list, use conservative defaults
        (32_000, 4_096)
    };

    Ok(ModelCapabilities::new(
        "grok",
        model_id,
        input_limit,
        output_limit,
        CapabilitySource::ApiDiscovery,
    ))
}

/// Discover capabilities for an Ollama model via local API.
///
/// Endpoint: `POST /api/show`
/// Returns: `model_info.{architecture}.context_length`
async fn discover_ollama(
    model_id: &str,
    base_url: Option<&str>,
) -> Result<ModelCapabilities, AppError> {
    let base = base_url
        .filter(|u| !u.is_empty())
        .unwrap_or("http://localhost:11434");

    let url = format!("{}/api/show", base.trim_end_matches('/'));

    let resp = discovery_client()
        .post(&url)
        .json(&serde_json::json!({ "name": model_id }))
        .send()
        .await
        .map_err(|e| AppError::Llm(format!("Ollama discovery failed: {e}")))?;

    if !resp.status().is_success() {
        return Err(AppError::Llm(format!(
            "Ollama API returned {}",
            resp.status()
        )));
    }

    let data: serde_json::Value = resp.json().await?;

    // Ollama stores context_length in model_info under architecture-specific keys
    // e.g., "llama.context_length", "gemma.context_length"
    let model_info = data.get("model_info");

    let context_length = model_info
        .and_then(|info| {
            // Try to find any key ending with ".context_length"
            info.as_object().and_then(|obj| {
                obj.iter()
                    .find(|(k, _)| k.ends_with(".context_length"))
                    .and_then(|(_, v)| v.as_u64())
            })
        })
        .unwrap_or(4_096) as usize;

    // Also check num_ctx in parameters (user override)
    let num_ctx = data
        .get("parameters")
        .and_then(|p| p.as_str())
        .and_then(|params| {
            // Parse "num_ctx N" from parameters string
            params
                .split_whitespace()
                .collect::<Vec<_>>()
                .windows(2)
                .find(|w| w[0] == "num_ctx")
                .and_then(|w| w[1].parse::<usize>().ok())
        });

    let effective_context = num_ctx.unwrap_or(context_length);

    Ok(ModelCapabilities::new(
        "ollama",
        model_id,
        effective_context,
        effective_context / 4, // Rough estimate for output
        CapabilitySource::ApiDiscovery,
    ))
}

#[cfg(test)]
mod tests {
    #[tokio::test]
    async fn test_ollama_parsing() {
        // Test that we can parse context_length from model_info
        let json = serde_json::json!({
            "model_info": {
                "llama.context_length": 8192,
                "llama.embedding_length": 4096
            }
        });

        let context = json
            .get("model_info")
            .and_then(|info| {
                info.as_object().and_then(|obj| {
                    obj.iter()
                        .find(|(k, _)| k.ends_with(".context_length"))
                        .and_then(|(_, v)| v.as_u64())
                })
            });

        assert_eq!(context, Some(8192));
    }
}

