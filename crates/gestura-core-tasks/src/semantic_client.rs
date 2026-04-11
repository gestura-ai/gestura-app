#![cfg(feature = "advanced-primitives")]

//! Optional lightweight semantic client for live intent enrichment.

use reqwest::StatusCode;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::time::Duration;

/// Runtime configuration for the optional semantic client.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SemanticClientConfig {
    /// Whether semantic lookups are enabled.
    pub enabled: bool,
    /// Optional endpoint for live semantic queries.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub endpoint: Option<String>,
    /// Optional bearer token for authenticated semantic backends.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_key: Option<String>,
    /// Optional semantic domain such as finance, health, or code.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub domain: Option<String>,
    /// Maximum number of hits to keep.
    pub max_results: usize,
    /// Request timeout in milliseconds.
    pub timeout_ms: u64,
}

/// Query envelope for semantic lookups.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SemanticQueryRequest {
    /// Natural-language query to resolve.
    pub query: String,
    /// Optional semantic domain selector.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub domain: Option<String>,
    /// Session identifier for correlation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    /// Task identifier for correlation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task_id: Option<String>,
    /// Human-readable source such as voice/chat/gesture/orchestrator.
    pub source: String,
    /// Additional request hints.
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub hints: HashMap<String, String>,
}

/// One semantic hit returned by the client.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SemanticQueryHit {
    /// Display title.
    pub title: String,
    /// Short snippet or rationale.
    pub snippet: String,
    /// Source identifier if supplied by the backend.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    /// Optional relevance score.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub score: Option<f32>,
}

/// Result returned by the semantic client.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SemanticQueryResult {
    /// Domain the backend resolved against.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub domain: Option<String>,
    /// Condensed summary suitable for system-prompt enrichment.
    pub summary: String,
    /// Supporting hits from the backend.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub hits: Vec<SemanticQueryHit>,
}

/// Semantic client errors are non-fatal to the surrounding pipeline.
#[derive(Debug, thiserror::Error)]
pub enum SemanticClientError {
    /// The configuration does not permit a live semantic request.
    #[error("semantic client is missing a valid endpoint")]
    InvalidConfiguration,
    /// Transport failure while calling the semantic backend.
    #[error("semantic request failed: {0}")]
    Transport(#[from] reqwest::Error),
    /// Backend returned a non-success status code.
    #[error("semantic backend returned status {status}: {body}")]
    Status {
        /// HTTP status.
        status: StatusCode,
        /// Response body excerpt.
        body: String,
    },
}

/// Thin HTTP client for optional live semantic lookups.
#[derive(Debug, Clone)]
pub struct SemanticClient {
    config: SemanticClientConfig,
    client: reqwest::Client,
}

impl SemanticClient {
    /// Construct a semantic client from runtime configuration.
    pub fn new(config: SemanticClientConfig) -> Result<Self, SemanticClientError> {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_millis(config.timeout_ms.max(100)))
            .build()?;
        Ok(Self { config, client })
    }

    /// Execute an optional semantic lookup.
    pub async fn query(
        &self,
        request: &SemanticQueryRequest,
    ) -> Result<Option<SemanticQueryResult>, SemanticClientError> {
        if !self.config.enabled || request.query.trim().is_empty() {
            return Ok(None);
        }
        let Some(endpoint) = self.config.endpoint.as_deref() else {
            return Err(SemanticClientError::InvalidConfiguration);
        };

        let mut builder = self.client.post(endpoint).json(&serde_json::json!({
            "query": request.query.clone(),
            "domain": request.domain.as_ref().or(self.config.domain.as_ref()),
            "session_id": request.session_id.clone(),
            "task_id": request.task_id.clone(),
            "source": request.source.clone(),
            "limit": self.config.max_results.max(1),
            "hints": request.hints.clone(),
        }));
        if let Some(api_key) = self.config.api_key.as_deref() {
            builder = builder.bearer_auth(api_key);
        }

        let response = builder.send().await?;
        let status = response.status();
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            return Err(SemanticClientError::Status { status, body });
        }

        let body: Value = response.json().await?;
        Ok(parse_semantic_response(
            &body,
            request
                .domain
                .clone()
                .or_else(|| self.config.domain.clone()),
            self.config.max_results.max(1),
        ))
    }
}

fn parse_semantic_response(
    body: &Value,
    fallback_domain: Option<String>,
    max_results: usize,
) -> Option<SemanticQueryResult> {
    let summary = body
        .get("summary")
        .and_then(Value::as_str)
        .or_else(|| body.get("content").and_then(Value::as_str))
        .or_else(|| body.pointer("/answer/summary").and_then(Value::as_str))
        .unwrap_or_default()
        .trim()
        .to_string();

    let hits = body
        .get("results")
        .or_else(|| body.get("hits"))
        .or_else(|| body.pointer("/data/results"))
        .and_then(Value::as_array)
        .map(|values| {
            values
                .iter()
                .take(max_results)
                .map(|value| SemanticQueryHit {
                    title: value
                        .get("title")
                        .and_then(Value::as_str)
                        .unwrap_or("semantic-result")
                        .trim()
                        .to_string(),
                    snippet: value
                        .get("snippet")
                        .or_else(|| value.get("summary"))
                        .or_else(|| value.get("text"))
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .trim()
                        .to_string(),
                    source: value
                        .get("source")
                        .or_else(|| value.get("uri"))
                        .or_else(|| value.get("url"))
                        .and_then(Value::as_str)
                        .map(ToOwned::to_owned),
                    score: value
                        .get("score")
                        .and_then(Value::as_f64)
                        .map(|score| score as f32),
                })
                .filter(|hit| !hit.snippet.is_empty() || !hit.title.is_empty())
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    if summary.is_empty() && hits.is_empty() {
        return None;
    }

    Some(SemanticQueryResult {
        domain: body
            .get("domain")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned)
            .or(fallback_domain),
        summary,
        hits,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_semantic_response_accepts_generic_payloads() {
        let body = serde_json::json!({
            "summary": "Cross-check the known protocol constraints before execution.",
            "results": [
                {
                    "title": "Protocol reference",
                    "snippet": "Ring transport expects BOS1921 success waveform IDs.",
                    "source": "docs://ring"
                }
            ]
        });

        let result = parse_semantic_response(&body, Some("haptics".to_string()), 3)
            .expect("result should parse");
        assert_eq!(result.domain.as_deref(), Some("haptics"));
        assert_eq!(result.hits.len(), 1);
        assert!(result.summary.contains("protocol constraints"));
    }

    #[test]
    fn parse_semantic_response_returns_none_for_empty_payloads() {
        let body = serde_json::json!({"results": []});
        assert!(parse_semantic_response(&body, None, 3).is_none());
    }
}
