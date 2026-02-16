//! Tool-related configuration types.
//!
//! These types are owned by the tools domain because they primarily configure
//! built-in tools (e.g. web search). The `gestura-core` facade re-exports them
//! from its `config` module for backwards compatibility.

use serde::{Deserialize, Serialize};

// ============================================================================
// Web Search Configuration
// ============================================================================

/// Web search provider selection.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum WebSearchProvider {
    /// Local HTTP-based search (no API key required) - DEFAULT.
    ///
    /// Uses DuckDuckGo HTML scraping with smart content extraction.
    #[default]
    Local,
    /// SerpAPI provider (requires API key).
    SerpApi,
    /// DuckDuckGo Instant Answer API (no API key, limited results).
    DuckDuckGo,
    /// Brave Search API (requires API key).
    Brave,
}

/// Web search configuration.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct WebSearchConfig {
    /// Primary search provider.
    pub provider: WebSearchProvider,
    /// SerpAPI API key (optional).
    pub serpapi_key: Option<String>,
    /// Brave Search API key (optional).
    pub brave_key: Option<String>,
    /// Maximum number of search results to return.
    pub max_results: usize,
    /// Request timeout in seconds.
    pub timeout_secs: u64,
    /// User agent string for HTTP requests.
    pub user_agent: String,
    /// Enable content extraction from search result pages.
    pub extract_content: bool,
    /// Maximum content length per page (in characters).
    pub max_content_length: usize,
    /// Fallback providers if primary fails (in order).
    pub fallback_providers: Vec<WebSearchProvider>,
}

impl Default for WebSearchConfig {
    fn default() -> Self {
        Self {
            provider: WebSearchProvider::Local,
            serpapi_key: None,
            brave_key: None,
            max_results: 5,
            timeout_secs: 30,
            user_agent: format!(
                "Gestura/{} (+https://gestura.ai)",
                env!("CARGO_PKG_VERSION")
            ),
            extract_content: true,
            max_content_length: 10_000,
            fallback_providers: vec![WebSearchProvider::DuckDuckGo],
        }
    }
}
