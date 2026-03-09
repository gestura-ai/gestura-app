//! LLM Pre-flight Tool Router
//!
//! Implements semantic tool selection via a cheap pre-flight LLM call.
//! Reduces silent tool-selection failures on ambiguous or novel user requests.

use std::collections::HashSet;
use std::sync::Arc;

use async_trait::async_trait;
use dashmap::DashMap;
use serde_json::Value;

use crate::config::AppConfig;
use crate::llm_provider::{AgentContext, select_provider};
use crate::tools::registry::ToolDefinition;
use gestura_core_pipeline::types::ToolRoutingStrategy;

// ---------------------------------------------------------------------------
// Result type
// ---------------------------------------------------------------------------

/// Result of a pre-flight tool routing operation.
#[derive(Debug, Clone)]
pub struct RoutingResult {
    /// Tool names selected by the router.
    ///
    /// An **empty** list is the signal to fall through to keyword/category
    /// routing — the router made no decision.
    pub suggested_tools: Vec<String>,
    /// Confidence in this routing decision (0.0–1.0).
    ///
    /// LLM-sourced decisions carry `1.0`; a fallthrough carries `0.0`.
    pub confidence: f32,
}

impl RoutingResult {
    /// Pass-through sentinel — tells the pipeline to use keyword routing instead.
    pub fn fallthrough() -> Self {
        Self {
            suggested_tools: Vec::new(),
            confidence: 0.0,
        }
    }

    /// Returns `true` if this result contains an explicit tool selection.
    pub fn has_selection(&self) -> bool {
        !self.suggested_tools.is_empty()
    }
}

// ---------------------------------------------------------------------------
// Trait
// ---------------------------------------------------------------------------

/// Async trait for tool selection strategies.
///
/// Implementations decide which built-in tools to expose to the LLM for a
/// given user request.  Return [`RoutingResult::fallthrough()`] to defer to
/// the existing keyword/category routing path.
#[async_trait]
pub trait ToolRouter: Send + Sync {
    /// Select the most relevant tools for a request.
    ///
    /// # Parameters
    /// - `request`: the raw user input string
    /// - `tools`: full list of available [`ToolDefinition`]s
    /// - `keyword_confidence`: confidence from the keyword [`RequestAnalyzer`]
    ///
    /// [`RequestAnalyzer`]: crate::context::RequestAnalyzer
    async fn route(
        &self,
        request: &str,
        tools: &[&'static ToolDefinition],
        keyword_confidence: f32,
    ) -> RoutingResult;
}

// ---------------------------------------------------------------------------
// LlmToolRouter
// ---------------------------------------------------------------------------

/// Cache key: blake3 hash of normalised request text.
type CacheKey = [u8; 32];

fn cache_key(request: &str) -> CacheKey {
    *blake3::hash(request.trim().to_lowercase().as_bytes()).as_bytes()
}

/// LLM-based tool router.
///
/// Fires a single, cheap pre-flight LLM call whose sole job is to pick
/// the minimal set of tool names relevant to the request.  Results are
/// cached in-process by request hash to avoid repeated calls for identical
/// inputs within a session.
pub struct LlmToolRouter {
    config: Arc<AppConfig>,
    cache: DashMap<CacheKey, Arc<RoutingResult>>,
}

impl LlmToolRouter {
    /// Create a new router backed by `config`.
    pub fn new(config: Arc<AppConfig>) -> Self {
        Self {
            config,
            cache: DashMap::new(),
        }
    }

    /// Build the routing prompt.
    fn build_routing_prompt(request: &str, tools: &[&'static ToolDefinition]) -> String {
        let mut prompt = String::from(
            "You are a tool selector. Given a user request and a list of tools, \
             respond with ONLY a JSON array of tool names needed to fulfill the request.\n\
             Choose the minimal set (1-4 tools). If no tool is needed respond with [].\n\
             Do not include any explanation — only the JSON array.\n\n\
             Available tools:\n",
        );
        for tool in tools {
            prompt.push_str(&format!("- {}: {}\n", tool.name, tool.description));
        }
        prompt.push_str(&format!(
            "\nUser request: \"{}\"\n\nJSON array:",
            request.trim()
        ));
        prompt
    }

    /// Parse a JSON array of tool names from the LLM response, validating
    /// each name against the known tool set.
    fn parse_tool_names(response: &str, tools: &[&'static ToolDefinition]) -> Vec<String> {
        let start = response.find('[');
        let end = response.rfind(']');
        let (Some(s), Some(e)) = (start, end) else {
            return Vec::new();
        };
        let Ok(Value::Array(arr)) = serde_json::from_str::<Value>(&response[s..=e]) else {
            return Vec::new();
        };
        let valid: HashSet<&str> = tools.iter().map(|t| t.name).collect();
        arr.into_iter()
            .filter_map(|v| v.as_str().map(str::to_lowercase))
            .filter(|name| valid.contains(name.as_str()))
            .collect()
    }
}

#[async_trait]
impl ToolRouter for LlmToolRouter {
    async fn route(
        &self,
        request: &str,
        tools: &[&'static ToolDefinition],
        _keyword_confidence: f32,
    ) -> RoutingResult {
        let key = cache_key(request);

        // Cache hit — avoid redundant LLM calls for identical requests.
        if let Some(cached) = self.cache.get(&key) {
            tracing::debug!("LlmToolRouter: cache hit");
            return (**cached).clone();
        }

        let prompt = Self::build_routing_prompt(request, tools);
        let ctx = AgentContext::default();
        let provider = select_provider(self.config.as_ref(), &ctx);

        let result = match provider.call(&prompt).await {
            Ok(response) => {
                let suggested = Self::parse_tool_names(&response, tools);
                tracing::debug!(tools = ?suggested, "LlmToolRouter: routed request to tools");
                RoutingResult {
                    suggested_tools: suggested,
                    confidence: 1.0,
                }
            }
            Err(e) => {
                tracing::warn!(
                    error = %e,
                    "LlmToolRouter: LLM call failed, falling through to keyword routing"
                );
                RoutingResult::fallthrough()
            }
        };

        let shared = Arc::new(result.clone());
        self.cache.insert(key, shared);
        result
    }
}

// ---------------------------------------------------------------------------
// HybridToolRouter
// ---------------------------------------------------------------------------

/// Hybrid tool router.
///
/// Uses the keyword analyzer's confidence score to decide when to invoke the
/// LLM router.  Requests where keyword analysis is confident enough bypass the
/// extra round-trip entirely.
pub struct HybridToolRouter {
    llm_router: LlmToolRouter,
    confidence_threshold: f32,
}

impl HybridToolRouter {
    /// Create a new hybrid router.
    ///
    /// `confidence_threshold` is the minimum keyword-analysis confidence above
    /// which the LLM call is skipped.  Values in `[0.2, 0.5]` are recommended.
    pub fn new(config: Arc<AppConfig>, confidence_threshold: f32) -> Self {
        Self {
            llm_router: LlmToolRouter::new(config),
            confidence_threshold,
        }
    }
}

#[async_trait]
impl ToolRouter for HybridToolRouter {
    async fn route(
        &self,
        request: &str,
        tools: &[&'static ToolDefinition],
        keyword_confidence: f32,
    ) -> RoutingResult {
        if keyword_confidence >= self.confidence_threshold {
            tracing::debug!(
                keyword_confidence,
                threshold = self.confidence_threshold,
                "HybridToolRouter: above threshold, using keyword routing"
            );
            return RoutingResult::fallthrough();
        }
        tracing::debug!(
            keyword_confidence,
            threshold = self.confidence_threshold,
            "HybridToolRouter: below threshold, invoking LLM router"
        );
        self.llm_router
            .route(request, tools, keyword_confidence)
            .await
    }
}

// ---------------------------------------------------------------------------
// Factory
// ---------------------------------------------------------------------------

/// Build a [`ToolRouter`] from a [`ToolRoutingStrategy`] and app config.
///
/// Returns `None` for [`ToolRoutingStrategy::Keyword`] — no extra router
/// object is needed; the pipeline's existing keyword path runs as-is.
pub fn build_tool_router(
    strategy: &ToolRoutingStrategy,
    config: Arc<AppConfig>,
) -> Option<Box<dyn ToolRouter>> {
    match strategy {
        ToolRoutingStrategy::Keyword => None,
        ToolRoutingStrategy::Llm => Some(Box::new(LlmToolRouter::new(config))),
        ToolRoutingStrategy::Hybrid {
            confidence_threshold,
        } => Some(Box::new(HybridToolRouter::new(
            config,
            *confidence_threshold,
        ))),
    }
}

// ---------------------------------------------------------------------------
// Tests (private helpers)
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::registry::all_tools;

    // ------------------------------------------------------------------
    // parse_tool_names
    // ------------------------------------------------------------------

    #[test]
    fn parse_tool_names_valid_json_filters_invalid_names() {
        let tools: Vec<&'static ToolDefinition> = all_tools().iter().collect();
        let response = r#"["file", "web", "not_a_real_tool", "web_search"]"#;
        let result = LlmToolRouter::parse_tool_names(response, &tools);
        assert_eq!(result, vec!["file", "web", "web_search"]);
    }

    #[test]
    fn parse_tool_names_no_brackets_returns_empty() {
        let tools: Vec<&'static ToolDefinition> = all_tools().iter().collect();
        let response = "file, web, web_search";
        let result = LlmToolRouter::parse_tool_names(response, &tools);
        assert!(result.is_empty(), "expected empty without JSON brackets");
    }

    #[test]
    fn parse_tool_names_invalid_json_returns_empty() {
        let tools: Vec<&'static ToolDefinition> = all_tools().iter().collect();
        let response = "[file, web]"; // not valid JSON
        let result = LlmToolRouter::parse_tool_names(response, &tools);
        assert!(result.is_empty(), "expected empty for invalid JSON");
    }

    #[test]
    fn parse_tool_names_empty_array_returns_empty() {
        let tools: Vec<&'static ToolDefinition> = all_tools().iter().collect();
        let result = LlmToolRouter::parse_tool_names("[]", &tools);
        assert!(result.is_empty());
    }

    #[test]
    fn parse_tool_names_normalises_case() {
        let tools: Vec<&'static ToolDefinition> = all_tools().iter().collect();
        // LLM might return mixed case — we lowercase before matching.
        let response = r#"["FILE", "Web", "WEB_SEARCH"]"#;
        let result = LlmToolRouter::parse_tool_names(response, &tools);
        assert_eq!(result, vec!["file", "web", "web_search"]);
    }

    #[test]
    fn parse_tool_names_extracts_from_prose_with_brackets() {
        let tools: Vec<&'static ToolDefinition> = all_tools().iter().collect();
        // LLM sometimes wraps the array in prose — we find the first/last bracket.
        let response = r#"The tools you need are: ["shell", "git"]."#;
        let result = LlmToolRouter::parse_tool_names(response, &tools);
        assert_eq!(result, vec!["shell", "git"]);
    }

    // ------------------------------------------------------------------
    // cache_key
    // ------------------------------------------------------------------

    #[test]
    fn cache_key_is_deterministic() {
        assert_eq!(cache_key("fetch gestura.ai"), cache_key("fetch gestura.ai"));
    }

    #[test]
    fn cache_key_normalises_whitespace_and_case() {
        // Leading/trailing whitespace and case differences should produce the
        // same key so near-duplicate requests hit the cache.
        assert_eq!(
            cache_key("  Fetch Gestura.ai  "),
            cache_key("fetch gestura.ai")
        );
    }

    #[test]
    fn cache_key_differs_for_different_requests() {
        assert_ne!(
            cache_key("take a screenshot"),
            cache_key("fetch gestura.ai")
        );
    }
}
