//! Web fetching and search tool
//!
//! Provides web operations with structured output.

use crate::error::{AppError, Result};
use serde::{Deserialize, Serialize};

/// Web page fetch result
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FetchResult {
    pub url: String,
    pub status_code: u16,
    pub content_type: Option<String>,
    pub content: String,
    pub headers: std::collections::HashMap<String, String>,
}

/// Web search result
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchResult {
    pub query: String,
    pub results: Vec<SearchItem>,
}

/// A single search result item
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchItem {
    pub title: String,
    pub url: String,
    pub snippet: String,
}

/// Web operations service
pub struct WebTools {
    client: reqwest::Client,
}

impl Default for WebTools {
    fn default() -> Self {
        Self::new()
    }
}

impl WebTools {
    pub fn new() -> Self {
        Self {
            client: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(30))
                .user_agent("Gestura/0.2.0")
                .build()
                .unwrap_or_default(),
        }
    }

    /// Fetch a web page
    pub async fn fetch(&self, url: &str) -> Result<FetchResult> {
        let response = self
            .client
            .get(url)
            .send()
            .await
            .map_err(|e| AppError::Io(std::io::Error::other(format!("HTTP error: {e}"))))?;

        let status_code = response.status().as_u16();
        let content_type = response
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .map(|s| s.to_string());

        let mut headers = std::collections::HashMap::new();
        for (key, value) in response.headers() {
            if let Ok(v) = value.to_str() {
                headers.insert(key.to_string(), v.to_string());
            }
        }

        let content = response
            .text()
            .await
            .map_err(|e| AppError::Io(std::io::Error::other(format!("Read error: {e}"))))?;

        Ok(FetchResult {
            url: url.to_string(),
            status_code,
            content_type,
            content,
            headers,
        })
    }

    /// Search the web (placeholder - requires search API integration)
    pub async fn search(&self, query: &str, _num_results: Option<usize>) -> Result<SearchResult> {
        // This is a placeholder. Real implementation would use a search API
        // like Google Custom Search, Bing Search, or DuckDuckGo.
        Ok(SearchResult {
            query: query.to_string(),
            results: vec![SearchItem {
                title: format!("Search results for: {}", query),
                url: format!("https://duckduckgo.com/?q={}", urlencoding::encode(query)),
                snippet: "Web search requires API integration. Visit DuckDuckGo for results."
                    .to_string(),
            }],
        })
    }

    /// Convert HTML to plain text (basic implementation)
    pub fn html_to_text(&self, html: &str) -> String {
        // Basic HTML stripping - in production use a proper HTML parser
        let mut text = html.to_string();

        // Remove script and style tags with content
        let script_re = regex::Regex::new(r"(?is)<script[^>]*>.*?</script>").unwrap();
        text = script_re.replace_all(&text, "").to_string();

        let style_re = regex::Regex::new(r"(?is)<style[^>]*>.*?</style>").unwrap();
        text = style_re.replace_all(&text, "").to_string();

        // Remove all HTML tags
        let tag_re = regex::Regex::new(r"<[^>]+>").unwrap();
        text = tag_re.replace_all(&text, "").to_string();

        // Decode common HTML entities
        text = text
            .replace("&nbsp;", " ")
            .replace("&amp;", "&")
            .replace("&lt;", "<")
            .replace("&gt;", ">")
            .replace("&quot;", "\"");

        // Collapse whitespace
        let ws_re = regex::Regex::new(r"\s+").unwrap();
        text = ws_re.replace_all(&text, " ").trim().to_string();

        text
    }
}
