//! Dynamic and static model listing for all LLM providers.
//!
//! This module centralises model discovery so that both the GUI (Tauri commands) and
//! the CLI can fetch / fall back to the same lists without duplicating HTTP logic.
//!
//! Each provider's listing is feature-gated to match the rest of `gestura-core-llm`.

use gestura_core_foundation::AppError;
use serde::{Deserialize, Serialize};

use crate::default_models::{DEFAULT_GEMINI_BASE_URL, DEFAULT_OLLAMA_BASE_URL};

/// Timeout for model-listing HTTP calls (shorter than inference calls).
const MODEL_LIST_TIMEOUT_SECS: u64 = 10;

/// A single model entry returned by listing endpoints.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelInfo {
    /// Provider-specific model identifier (e.g. `gpt-4o`, `claude-sonnet-4-20250514`).
    pub id: String,
    /// Human-readable display name.
    pub name: String,
    /// Provider key (e.g. `openai`, `anthropic`, `gemini`, `grok`, `ollama`).
    pub provider: String,
}

/// Create a lightweight HTTP client for model listing (shorter timeout than inference).
fn listing_client() -> reqwest::Client {
    reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(MODEL_LIST_TIMEOUT_SECS))
        .connect_timeout(std::time::Duration::from_secs(5))
        .build()
        .unwrap_or_else(|_| reqwest::Client::new())
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// List available models for the given provider.
///
/// Tries a live API call; returns an empty list when no API key is provided
/// or the API is unreachable.
///
/// # Arguments
/// * `provider` – one of `openai`, `anthropic`, `grok`, `gemini`, `ollama`
/// * `api_key` – required for cloud providers; ignored for `ollama`
/// * `base_url` – optional override; uses provider default when `None`
pub async fn list_models_for_provider(
    provider: &str,
    api_key: Option<&str>,
    base_url: Option<&str>,
) -> Result<Vec<ModelInfo>, AppError> {
    match provider.to_lowercase().as_str() {
        #[cfg(feature = "openai")]
        "openai" => list_openai(api_key, base_url).await,
        #[cfg(feature = "anthropic")]
        "anthropic" => list_anthropic(api_key).await,
        #[cfg(feature = "grok")]
        "grok" => list_grok(api_key).await,
        #[cfg(feature = "gemini")]
        "gemini" => list_gemini(api_key, base_url).await,
        #[cfg(feature = "ollama")]
        "ollama" => list_ollama(base_url).await,
        other => Err(AppError::Config(format!(
            "Unknown or disabled provider: {other}"
        ))),
    }
}

/// Return the static / fallback model list for a provider (no network).
///
/// Returns an empty list — static fallback lists have been removed.
/// Kept for API compatibility.
pub fn static_models_for_provider(_provider: &str) -> Vec<ModelInfo> {
    Vec::new()
}

// ---------------------------------------------------------------------------
// Provider-specific implementations
// ---------------------------------------------------------------------------

#[cfg(feature = "openai")]
async fn list_openai(
    api_key: Option<&str>,
    base_url: Option<&str>,
) -> Result<Vec<ModelInfo>, AppError> {
    let key = match api_key.filter(|k| !k.is_empty()) {
        Some(k) => k,
        None => return Ok(Vec::new()),
    };
    let base = base_url
        .filter(|u| !u.is_empty())
        .unwrap_or("https://api.openai.com");

    let url = format!("{}/v1/models", base.trim_end_matches('/'));
    let resp = listing_client()
        .get(&url)
        .bearer_auth(key)
        .send()
        .await
        .map_err(|e| AppError::Llm(format!("openai model list failed: {e}")))?;

    if !resp.status().is_success() {
        tracing::warn!("OpenAI /v1/models returned {}", resp.status());
        return Ok(Vec::new());
    }

    let data: serde_json::Value = resp.json().await?;
    let mut models: Vec<ModelInfo> = data
        .get("data")
        .and_then(|d| d.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|m| {
                    let id = m.get("id")?.as_str()?;
                    let is_chat = (id.starts_with("gpt-") && !id.contains("instruct"))
                        || id.starts_with("o1-")
                        || id.starts_with("o3-")
                        || id.starts_with("o4-")
                        || id.starts_with("o5-");
                    if !is_chat {
                        return None;
                    }
                    Some(ModelInfo {
                        id: id.to_string(),
                        name: gestura_core_foundation::model_display::format_model_name(
                            "openai", id,
                        ),
                        provider: "openai".to_string(),
                    })
                })
                .collect()
        })
        .unwrap_or_default();

    models.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(models)
}

#[cfg(feature = "anthropic")]
async fn list_anthropic(api_key: Option<&str>) -> Result<Vec<ModelInfo>, AppError> {
    let key = match api_key.filter(|k| !k.is_empty()) {
        Some(k) => k,
        None => return Ok(Vec::new()),
    };

    let url = "https://api.anthropic.com/v1/models";
    let resp = listing_client()
        .get(url)
        .header("x-api-key", key)
        .header("anthropic-version", "2023-06-01")
        .send()
        .await
        .map_err(|e| AppError::Llm(format!("anthropic model list failed: {e}")))?;

    if !resp.status().is_success() {
        tracing::warn!("Anthropic /v1/models returned {}", resp.status());
        return Ok(Vec::new());
    }

    let data: serde_json::Value = resp.json().await?;
    let mut models: Vec<ModelInfo> = data
        .get("data")
        .and_then(|d| d.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|m| {
                    let id = m.get("id")?.as_str()?;
                    if !id.starts_with("claude-") {
                        return None;
                    }
                    Some(ModelInfo {
                        id: id.to_string(),
                        name: gestura_core_foundation::model_display::format_model_name(
                            "anthropic",
                            id,
                        ),
                        provider: "anthropic".to_string(),
                    })
                })
                .collect()
        })
        .unwrap_or_default();

    models.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(models)
}

#[cfg(feature = "grok")]
async fn list_grok(api_key: Option<&str>) -> Result<Vec<ModelInfo>, AppError> {
    let key = match api_key.filter(|k| !k.is_empty()) {
        Some(k) => k,
        None => return Ok(Vec::new()),
    };

    let url = "https://api.x.ai/v1/models";
    let resp = listing_client()
        .get(url)
        .bearer_auth(key)
        .send()
        .await
        .map_err(|e| AppError::Llm(format!("grok model list failed: {e}")))?;

    if !resp.status().is_success() {
        tracing::warn!("Grok /v1/models returned {}", resp.status());
        return Ok(Vec::new());
    }

    let data: serde_json::Value = resp.json().await?;
    let mut models: Vec<ModelInfo> = data
        .get("data")
        .and_then(|d| d.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|m| {
                    let id = m.get("id")?.as_str()?;
                    if id.contains("image") {
                        return None;
                    }
                    Some(ModelInfo {
                        id: id.to_string(),
                        name: gestura_core_foundation::model_display::format_model_name("grok", id),
                        provider: "grok".to_string(),
                    })
                })
                .collect()
        })
        .unwrap_or_default();

    models.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(models)
}

#[cfg(feature = "gemini")]
async fn list_gemini(
    api_key: Option<&str>,
    base_url: Option<&str>,
) -> Result<Vec<ModelInfo>, AppError> {
    let key = match api_key.filter(|k| !k.is_empty()) {
        Some(k) => k,
        None => return Ok(Vec::new()),
    };
    let base = base_url
        .filter(|u| !u.is_empty())
        .unwrap_or(DEFAULT_GEMINI_BASE_URL);

    // Gemini uses key as query param, not bearer auth.
    let url = format!("{}/v1beta/models?key={}", base.trim_end_matches('/'), key);
    let resp = listing_client()
        .get(&url)
        .send()
        .await
        .map_err(|e| AppError::Llm(format!("gemini model list failed: {e}")))?;

    if !resp.status().is_success() {
        tracing::warn!("Gemini /v1beta/models returned {}", resp.status());
        return Ok(Vec::new());
    }

    let data: serde_json::Value = resp.json().await?;
    let mut models: Vec<ModelInfo> = data
        .get("models")
        .and_then(|d| d.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|m| {
                    // Gemini returns "name": "models/gemini-2.0-flash" — strip the prefix.
                    let raw_name = m.get("name")?.as_str()?;
                    let id = raw_name.strip_prefix("models/").unwrap_or(raw_name);
                    // Only include generative models (skip embedding, AQA, etc.).
                    let methods = m
                        .get("supportedGenerationMethods")
                        .and_then(|v| v.as_array());
                    let is_generative = methods
                        .map(|ms| ms.iter().any(|v| v.as_str() == Some("generateContent")))
                        .unwrap_or(false);
                    if !is_generative {
                        return None;
                    }
                    let display = m.get("displayName").and_then(|d| d.as_str()).unwrap_or(id);
                    Some(ModelInfo {
                        id: id.to_string(),
                        name: display.to_string(),
                        provider: "gemini".to_string(),
                    })
                })
                .collect()
        })
        .unwrap_or_default();

    models.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(models)
}

#[cfg(feature = "ollama")]
async fn list_ollama(base_url: Option<&str>) -> Result<Vec<ModelInfo>, AppError> {
    let base = base_url
        .filter(|u| !u.is_empty())
        .unwrap_or(DEFAULT_OLLAMA_BASE_URL);

    let url = format!("{}/api/tags", base.trim_end_matches('/'));
    let resp = listing_client()
        .get(&url)
        .send()
        .await
        .map_err(|e| AppError::Llm(format!("ollama model list failed: {e}")))?;

    if !resp.status().is_success() {
        return Err(AppError::Llm(format!(
            "Ollama at {} returned status {}",
            base,
            resp.status()
        )));
    }

    let data: serde_json::Value = resp.json().await?;
    let models: Vec<ModelInfo> = data
        .get("models")
        .and_then(|m| m.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|m| {
                    let name = m.get("name").and_then(|n| n.as_str())?;
                    Some(ModelInfo {
                        id: name.to_string(),
                        name: gestura_core_foundation::model_display::format_model_name(
                            "ollama", name,
                        ),
                        provider: "ollama".to_string(),
                    })
                })
                .collect()
        })
        .unwrap_or_default();

    Ok(models)
}

// ---------------------------------------------------------------------------
// Ollama connectivity check
// ---------------------------------------------------------------------------

/// Timeout for the lightweight Ollama connectivity ping.
const OLLAMA_PING_TIMEOUT_SECS: u64 = 3;

/// Ping the Ollama endpoint to verify it is reachable.
///
/// Issues a lightweight `GET /api/tags` with a short timeout.
/// Returns `true` only on a successful HTTP response.
///
/// # Arguments
/// * `base_url` – Ollama base URL (e.g. `http://localhost:11434`).
///   Falls back to [`DEFAULT_OLLAMA_BASE_URL`] when empty.
#[cfg(feature = "ollama")]
pub async fn check_ollama_connectivity(base_url: &str) -> bool {
    let base = if base_url.is_empty() {
        DEFAULT_OLLAMA_BASE_URL
    } else {
        base_url
    };
    let url = format!("{}/api/tags", base.trim_end_matches('/'));
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(OLLAMA_PING_TIMEOUT_SECS))
        .connect_timeout(std::time::Duration::from_secs(OLLAMA_PING_TIMEOUT_SECS))
        .build()
        .unwrap_or_else(|_| reqwest::Client::new());
    match client.get(&url).send().await {
        Ok(resp) => resp.status().is_success(),
        Err(_) => false,
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn static_openai_returns_empty() {
        let models = static_models_for_provider("openai");
        assert!(models.is_empty());
    }

    #[test]
    fn static_anthropic_returns_empty() {
        let models = static_models_for_provider("anthropic");
        assert!(models.is_empty());
    }

    #[test]
    fn static_grok_returns_empty() {
        let models = static_models_for_provider("grok");
        assert!(models.is_empty());
    }

    #[test]
    fn static_gemini_returns_empty() {
        let models = static_models_for_provider("gemini");
        assert!(models.is_empty());
    }

    #[test]
    fn static_unknown_returns_empty() {
        let models = static_models_for_provider("unknown_provider");
        assert!(models.is_empty());
    }

    #[tokio::test]
    async fn list_without_key_returns_empty() {
        // Cloud providers without an API key should return an empty list.
        let models = list_models_for_provider("openai", None, None)
            .await
            .unwrap();
        assert!(models.is_empty());
    }
}
