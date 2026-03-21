//! Web fetching and search tool
//!
//! Provides web operations with structured output including:
//! - Smart content extraction from web pages
//! - Multiple search provider support (Local, SerpAPI, DuckDuckGo, Brave)
//! - Configurable fallback chains
//!
//! # Default Behavior
//! By default, uses local HTTP-based search via DuckDuckGo HTML scraping (no API key required).
//! This provides a "batteries included" experience while allowing users to upgrade to
//! API-based providers for better results.

use crate::config::{WebSearchConfig, WebSearchProvider};
use crate::error::{AppError, Result};
use scraper::{ElementRef, Html, Selector};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::time::Duration;

// ============================================================================
// Core Data Types
// ============================================================================

/// Web page fetch result
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FetchResult {
    pub url: String,
    pub status_code: u16,
    pub content_type: Option<String>,
    pub content: String,
    pub headers: HashMap<String, String>,
}

/// Extracted content from a web page
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExtractedContent {
    /// Page title
    pub title: Option<String>,
    /// Meta description
    pub description: Option<String>,
    /// Main content text (cleaned)
    pub main_content: String,
    /// All links found on the page
    pub links: Vec<PageLink>,
    /// Headings structure
    pub headings: Vec<Heading>,
    /// Code blocks found
    pub code_blocks: Vec<CodeBlock>,
    /// Source URL
    pub url: String,
}

/// A link found on a page
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PageLink {
    pub text: String,
    pub href: String,
}

/// A heading from the document
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Heading {
    pub level: u8, // 1-6 for h1-h6
    pub text: String,
}

/// A code block from the document
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CodeBlock {
    pub language: Option<String>,
    pub code: String,
}

/// Web search result
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchResult {
    pub query: String,
    pub results: Vec<SearchItem>,
    /// Which provider returned these results
    pub provider: String,
}

/// A single search result item
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SearchItem {
    pub title: String,
    pub url: String,
    pub snippet: String,
    /// Optional extracted content (if extract_content is enabled)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<ExtractedContent>,
}

// ============================================================================
// Content Extractor
// ============================================================================

/// Smart content extraction from HTML pages
#[derive(Debug, Clone)]
pub struct ContentExtractor {
    /// Selectors for main content areas (priority order)
    main_content_selectors: Vec<&'static str>,
    /// Elements to filter out during extraction (ads, navigation, footers, etc.)
    noise_selectors: Vec<&'static str>,
    /// Maximum content length
    max_content_length: usize,
}

impl Default for ContentExtractor {
    fn default() -> Self {
        Self::new(10_000)
    }
}

impl ContentExtractor {
    pub fn new(max_content_length: usize) -> Self {
        Self {
            // Priority order: most specific content containers first
            main_content_selectors: vec![
                "article",
                "main",
                "[role=\"main\"]",
                ".post-content",
                ".article-content",
                ".entry-content",
                ".content",
                ".markdown-body",
                ".prose",
                "#content",
                "#main",
                "body",
            ],
            // Noise elements to remove
            noise_selectors: vec![
                "script",
                "style",
                "nav",
                "header",
                "footer",
                "aside",
                ".sidebar",
                ".navigation",
                ".menu",
                ".ads",
                ".advertisement",
                ".comments",
                ".social-share",
                "[role=\"navigation\"]",
                "[role=\"banner\"]",
                "[role=\"complementary\"]",
            ],
            max_content_length,
        }
    }

    /// Extract structured content from HTML
    pub fn extract(&self, html: &str, url: &str) -> ExtractedContent {
        let document = Html::parse_document(html);

        // Extract title
        let title = self.extract_title(&document);

        // Extract meta description
        let description = self.extract_meta_description(&document);

        // Extract headings
        let headings = self.extract_headings(&document);

        // Extract code blocks
        let code_blocks = self.extract_code_blocks(&document);

        // Extract links
        let links = self.extract_links(&document, url);

        // Extract main content
        let main_content = self.extract_main_content(&document);

        ExtractedContent {
            title,
            description,
            main_content,
            links,
            headings,
            code_blocks,
            url: url.to_string(),
        }
    }

    fn extract_title(&self, doc: &Html) -> Option<String> {
        let selector = Selector::parse("title").ok()?;
        doc.select(&selector)
            .next()
            .map(|el| self.get_text(&el).trim().to_string())
            .filter(|s| !s.is_empty())
    }

    fn extract_meta_description(&self, doc: &Html) -> Option<String> {
        let selector = Selector::parse("meta[name=\"description\"]").ok()?;
        doc.select(&selector)
            .next()
            .and_then(|el| el.value().attr("content"))
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
    }

    fn extract_headings(&self, doc: &Html) -> Vec<Heading> {
        let mut headings = Vec::new();
        for level in 1..=6 {
            if let Ok(selector) = Selector::parse(&format!("h{}", level)) {
                for el in doc.select(&selector) {
                    let text = self.get_text(&el).trim().to_string();
                    if !text.is_empty() && text.len() < 200 {
                        headings.push(Heading {
                            level: level as u8,
                            text,
                        });
                    }
                }
            }
        }
        headings
    }

    fn extract_code_blocks(&self, doc: &Html) -> Vec<CodeBlock> {
        let mut blocks = Vec::new();

        // Helper to extract language from class attribute
        let extract_lang = |class: Option<&str>| -> Option<String> {
            class.and_then(|c| {
                c.split_whitespace()
                    .find(|cls| {
                        cls.starts_with("language-")
                            || cls.starts_with("lang-")
                            || cls.starts_with("hljs-")
                    })
                    .map(|cls| {
                        cls.trim_start_matches("language-")
                            .trim_start_matches("lang-")
                            .trim_start_matches("hljs-")
                            .to_string()
                    })
            })
        };

        // First, look for <pre><code> blocks (most common pattern)
        if let Ok(selector) = Selector::parse("pre code") {
            for el in doc.select(&selector) {
                let code = self.get_text(&el);
                if code.len() > 10 && code.len() < 10_000 {
                    // Try to detect language from the code element's class
                    let language = extract_lang(el.value().attr("class"));
                    blocks.push(CodeBlock { language, code });
                }
            }
        }

        // Then look for standalone <pre> blocks (without nested <code>)
        if let Ok(selector) = Selector::parse("pre") {
            for el in doc.select(&selector) {
                // Skip if this <pre> contains a <code> element (already handled above)
                if let Ok(code_sel) = Selector::parse("code")
                    && el.select(&code_sel).next().is_some()
                {
                    continue;
                }
                let code = self.get_text(&el);
                if code.len() > 10 && code.len() < 10_000 {
                    let language = extract_lang(el.value().attr("class"));
                    blocks.push(CodeBlock { language, code });
                }
            }
        }

        blocks
    }

    fn extract_links(&self, doc: &Html, base_url: &str) -> Vec<PageLink> {
        let mut links = Vec::new();
        if let Ok(selector) = Selector::parse("a[href]") {
            for el in doc.select(&selector) {
                if let Some(href) = el.value().attr("href") {
                    let text = self.get_text(&el).trim().to_string();
                    if !text.is_empty() && text.len() < 200 {
                        // Resolve relative URLs
                        let resolved_href = if href.starts_with("http") {
                            href.to_string()
                        } else if href.starts_with('/') {
                            // Absolute path
                            if let Ok(base) = url::Url::parse(base_url) {
                                format!(
                                    "{}://{}{}",
                                    base.scheme(),
                                    base.host_str().unwrap_or(""),
                                    href
                                )
                            } else {
                                href.to_string()
                            }
                        } else {
                            href.to_string()
                        };
                        links.push(PageLink {
                            text,
                            href: resolved_href,
                        });
                    }
                }
            }
        }
        // Deduplicate by URL
        links.sort_by(|a, b| a.href.cmp(&b.href));
        links.dedup_by(|a, b| a.href == b.href);
        links.truncate(50); // Limit links
        links
    }

    fn extract_main_content(&self, doc: &Html) -> String {
        // Try each main content selector in priority order
        for selector_str in &self.main_content_selectors {
            if let Ok(selector) = Selector::parse(selector_str)
                && let Some(el) = doc.select(&selector).next()
            {
                // Use noise-filtered extraction for cleaner content
                let text = self.get_clean_text_without_noise(&el);
                if text.len() > 100 {
                    return self.truncate_content(&text);
                }
            }
        }

        // Fallback: get all text from body, filtering noise
        if let Ok(selector) = Selector::parse("body")
            && let Some(el) = doc.select(&selector).next()
        {
            return self.truncate_content(&self.get_clean_text_without_noise(&el));
        }

        String::new()
    }

    fn get_text(&self, el: &ElementRef) -> String {
        el.text().collect::<Vec<_>>().join(" ")
    }

    /// Check if an element matches any noise selector.
    /// Used to filter out navigation, ads, footers, etc.
    fn is_noise_element(&self, el: &ElementRef) -> bool {
        for selector_str in &self.noise_selectors {
            if let Ok(selector) = Selector::parse(selector_str) {
                // Check if this element matches the noise selector
                if selector.matches(el) {
                    return true;
                }
            }
        }
        false
    }

    /// Get text content from an element, excluding noise elements.
    /// Recursively extracts text while filtering out ads, navigation, etc.
    fn get_text_without_noise(&self, el: &ElementRef) -> String {
        let mut text_parts = Vec::new();

        for child in el.children() {
            if let Some(text) = child.value().as_text() {
                text_parts.push(text.to_string());
            } else if let Some(child_el) = ElementRef::wrap(child) {
                // Skip noise elements
                if !self.is_noise_element(&child_el) {
                    text_parts.push(self.get_text_without_noise(&child_el));
                }
            }
        }

        text_parts.join(" ")
    }

    /// Get clean text content, excluding noise elements.
    fn get_clean_text_without_noise(&self, el: &ElementRef) -> String {
        let raw = self.get_text_without_noise(el);
        // Collapse whitespace and clean up
        let ws_re = regex::Regex::new(r"\s+").unwrap();
        ws_re.replace_all(&raw, " ").trim().to_string()
    }

    fn truncate_content(&self, content: &str) -> String {
        if content.len() <= self.max_content_length {
            content.to_string()
        } else {
            // Try to truncate at a sentence boundary
            let truncated = &content[..self.max_content_length];
            if let Some(last_period) = truncated.rfind(". ") {
                format!("{}...", &truncated[..=last_period])
            } else {
                format!("{}...", truncated)
            }
        }
    }
}

// ============================================================================
// Search Providers
// ============================================================================

/// Trait for search providers
#[async_trait::async_trait]
pub trait SearchProvider: Send + Sync {
    /// Provider name for logging
    fn name(&self) -> &str;

    /// Execute a search query
    async fn search(&self, query: &str, max_results: usize) -> Result<Vec<SearchItem>>;
}

/// Local HTTP-based search using DuckDuckGo HTML scraping
/// No API key required - default provider
pub struct LocalSearchProvider {
    client: reqwest::Client,
    /// Content extractor for fetching and parsing search result pages
    extractor: ContentExtractor,
}

impl Default for LocalSearchProvider {
    fn default() -> Self {
        Self::new(Duration::from_secs(30), 10_000)
    }
}

impl LocalSearchProvider {
    /// Realistic Chrome user agent to avoid bot detection by DuckDuckGo.
    /// A truncated UA string is immediately flagged as automated traffic.
    const USER_AGENT: &str = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) \
        AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36";

    pub fn new(timeout: Duration, max_content_length: usize) -> Self {
        use reqwest::header::{self, HeaderMap, HeaderValue};

        let mut default_headers = HeaderMap::new();
        default_headers.insert(
            header::ACCEPT,
            HeaderValue::from_static(
                "text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8",
            ),
        );
        default_headers.insert(
            header::ACCEPT_LANGUAGE,
            HeaderValue::from_static("en-US,en;q=0.9"),
        );
        default_headers.insert(
            header::REFERER,
            HeaderValue::from_static("https://html.duckduckgo.com/"),
        );

        Self {
            client: reqwest::Client::builder()
                .timeout(timeout)
                .user_agent(Self::USER_AGENT)
                .default_headers(default_headers)
                .build()
                .unwrap_or_default(),
            extractor: ContentExtractor::new(max_content_length),
        }
    }

    /// Returns `true` when DuckDuckGo has responded with a CAPTCHA /
    /// bot-challenge page instead of real search results.
    fn is_captcha_page(html: &str) -> bool {
        html.contains("anomaly-modal") || html.contains("cc=botnet")
    }

    /// Parse DuckDuckGo HTML search results
    fn parse_ddg_html(&self, html: &str, query: &str) -> Vec<SearchItem> {
        let document = Html::parse_document(html);
        let mut results = Vec::new();

        // DuckDuckGo HTML structure: .result class contains search results
        // Each result has .result__a (link) and .result__snippet (description)
        if let Ok(result_selector) = Selector::parse(".result, .web-result") {
            let result_count = document.select(&result_selector).count();
            tracing::debug!("Found {} result containers in DDG HTML", result_count);

            for result_el in document.select(&result_selector) {
                // Get title and URL from the link
                let (title, url) = if let Ok(link_sel) =
                    Selector::parse(".result__a, .result-link, a.result__url")
                {
                    if let Some(link) = result_el.select(&link_sel).next() {
                        let title = link.text().collect::<String>().trim().to_string();
                        let url = link.value().attr("href").unwrap_or("").to_string();
                        (title, url)
                    } else {
                        tracing::debug!("No link found in result container");
                        continue;
                    }
                } else {
                    continue;
                };

                // Get snippet
                let snippet =
                    if let Ok(snippet_sel) = Selector::parse(".result__snippet, .result-snippet") {
                        result_el
                            .select(&snippet_sel)
                            .next()
                            .map(|el| el.text().collect::<String>().trim().to_string())
                            .unwrap_or_default()
                    } else {
                        String::new()
                    };

                // Resolve DuckDuckGo redirect URLs
                let resolved_url = self.resolve_ddg_url(&url);

                if !title.is_empty() && !resolved_url.is_empty() {
                    tracing::debug!("Parsed result: {} -> {}", title, resolved_url);
                    results.push(SearchItem {
                        title,
                        url: resolved_url,
                        snippet,
                        content: None,
                    });
                }
            }
        }

        // Fallback: try to extract from simpler structure
        if results.is_empty() {
            tracing::warn!("Primary DDG parser returned no results, trying fallback parser");

            if let Ok(link_sel) = Selector::parse("a.result__a, .results a[href*=\"http\"]") {
                let link_count = document.select(&link_sel).count();
                tracing::debug!("Fallback parser found {} links", link_count);

                for link in document.select(&link_sel).take(10) {
                    let title = link.text().collect::<String>().trim().to_string();
                    let url = link.value().attr("href").unwrap_or("").to_string();
                    if !title.is_empty() && url.contains("http") {
                        let resolved_url = self.resolve_ddg_url(&url);
                        tracing::debug!("Fallback result: {} -> {}", title, resolved_url);
                        results.push(SearchItem {
                            title,
                            url: resolved_url,
                            snippet: format!("Search result for: {}", query),
                            content: None,
                        });
                    }
                }
            }
        }

        if results.is_empty() {
            tracing::error!(
                "DDG HTML parsing found no results. HTML length: {} bytes. \
                 DuckDuckGo may have changed their HTML structure, or the \
                 response is an unrecognised challenge page. \
                 Consider configuring a Brave or SerpAPI key for reliable search.",
                html.len()
            );
        } else {
            tracing::info!(
                "Successfully parsed {} results from DDG HTML",
                results.len()
            );
        }

        results
    }

    /// Resolve DuckDuckGo redirect URL to actual URL
    fn resolve_ddg_url(&self, url: &str) -> String {
        // DDG sometimes uses redirect URLs like //duckduckgo.com/l/?uddg=...
        if url.contains("uddg=")
            && let Some(encoded) = url.split("uddg=").nth(1)
        {
            if let Some(end) = encoded.find('&') {
                return urlencoding::decode(&encoded[..end])
                    .unwrap_or_default()
                    .to_string();
            }
            return urlencoding::decode(encoded).unwrap_or_default().to_string();
        }
        // Handle //hostname/path format
        if url.starts_with("//") {
            return format!("https:{}", url);
        }
        url.to_string()
    }

    /// Fetch and extract content from a URL.
    /// Returns extracted content including title, description, and main text.
    pub async fn fetch_content(&self, url: &str) -> Result<ExtractedContent> {
        Self::fetch_content_with(self.client.clone(), self.extractor.clone(), url.to_string()).await
    }

    async fn fetch_content_with(
        client: reqwest::Client,
        extractor: ContentExtractor,
        url: String,
    ) -> Result<ExtractedContent> {
        let response = client.get(&url).send().await.map_err(|e| {
            AppError::Io(std::io::Error::other(format!(
                "Failed to fetch URL {}: {}",
                url, e
            )))
        })?;

        let html = response.text().await.map_err(|e| {
            AppError::Io(std::io::Error::other(format!(
                "Failed to read response from {}: {}",
                url, e
            )))
        })?;

        let url_for_extract = url.clone();
        tokio::task::spawn_blocking(move || extractor.extract(&html, &url_for_extract))
            .await
            .map_err(|error| {
                AppError::Io(std::io::Error::other(format!(
                    "Content extraction task failed for {}: {}",
                    url, error
                )))
            })
    }

    /// Enrich a search result by fetching and extracting content from its URL.
    /// Returns the search item with the `content` field populated.
    pub async fn enrich_result(&self, mut item: SearchItem) -> SearchItem {
        if let Ok(mut extracted) = self.fetch_content(&item.url).await {
            // Truncate main content for summary if too long
            if extracted.main_content.len() > 500 {
                extracted.main_content = format!("{}...", &extracted.main_content[..500]);
            }
            item.content = Some(extracted);
        }
        item
    }

    /// Search with content extraction - fetches and extracts content from top results.
    /// This is slower but provides richer results with actual page content.
    pub async fn search_with_content(
        &self,
        query: &str,
        max_results: usize,
        fetch_content_count: usize,
    ) -> Result<Vec<SearchItem>> {
        let mut results = self.search_basic(query, max_results).await?;

        // Fetch content for top N results
        let fetch_count = fetch_content_count.min(results.len());
        if fetch_count == 0 {
            return Ok(results);
        }

        let mut tasks = tokio::task::JoinSet::new();
        for index in 0..fetch_count {
            let url = results[index].url.clone();
            let client = self.client.clone();
            let extractor = self.extractor.clone();
            tasks.spawn(async move {
                let content = Self::fetch_content_with(client, extractor, url).await.ok();
                (index, content)
            });
        }

        while let Some(joined) = tasks.join_next().await {
            match joined {
                Ok((index, Some(mut extracted))) => {
                    if extracted.main_content.len() > 500 {
                        extracted.main_content = format!("{}...", &extracted.main_content[..500]);
                    }
                    if let Some(item) = results.get_mut(index) {
                        item.content = Some(extracted);
                    }
                }
                Ok((_index, None)) => {}
                Err(error) => {
                    tracing::warn!(error = %error, "Search content enrichment task failed");
                }
            }
        }

        Ok(results)
    }

    /// Basic search without content extraction (faster).
    ///
    /// Uses HTTP **POST** with form-encoded data, matching DuckDuckGo's own
    /// HTML search form. GET requests to this endpoint are aggressively
    /// blocked with CAPTCHA challenges.
    async fn search_basic(&self, query: &str, max_results: usize) -> Result<Vec<SearchItem>> {
        let search_url = "https://html.duckduckgo.com/html/";

        let response = self
            .client
            .post(search_url)
            .form(&[("q", query)])
            .send()
            .await
            .map_err(|e| {
                AppError::Io(std::io::Error::other(format!("Search request failed: {e}")))
            })?;

        let html = response.text().await.map_err(|e| {
            AppError::Io(std::io::Error::other(format!(
                "Failed to read response: {e}"
            )))
        })?;

        // Detect CAPTCHA / bot-challenge page before attempting to parse.
        // Returning an error (instead of an empty Vec) ensures the fallback
        // provider chain in WebSearchService::search() is triggered.
        if Self::is_captcha_page(&html) {
            tracing::warn!(
                "DuckDuckGo returned a CAPTCHA bot-challenge page ({} bytes). \
                 Automated requests are being rate-limited. \
                 Consider configuring a Brave or SerpAPI key for reliable search.",
                html.len()
            );
            return Err(AppError::Io(std::io::Error::other(
                "DuckDuckGo returned a CAPTCHA challenge — automated requests are being blocked. \
                 Configure a Brave Search or SerpAPI key in Settings → Web Search for reliable results.",
            )));
        }

        let mut results = self.parse_ddg_html(&html, query);
        results.truncate(max_results);

        Ok(results)
    }
}

#[async_trait::async_trait]
impl SearchProvider for LocalSearchProvider {
    fn name(&self) -> &str {
        "local"
    }

    async fn search(&self, query: &str, max_results: usize) -> Result<Vec<SearchItem>> {
        // Use basic search for the trait implementation (fast, no content fetching)
        self.search_basic(query, max_results).await
    }
}

/// SerpAPI provider for Google search results
pub struct SerpApiProvider {
    client: reqwest::Client,
    api_key: String,
}

impl SerpApiProvider {
    pub fn new(api_key: String, timeout: Duration) -> Self {
        Self {
            client: reqwest::Client::builder()
                .timeout(timeout)
                .build()
                .unwrap_or_default(),
            api_key,
        }
    }
}

#[async_trait::async_trait]
impl SearchProvider for SerpApiProvider {
    fn name(&self) -> &str {
        "serpapi"
    }

    async fn search(&self, query: &str, max_results: usize) -> Result<Vec<SearchItem>> {
        let encoded_query = urlencoding::encode(query);
        let search_url = format!(
            "https://serpapi.com/search.json?q={}&api_key={}&num={}",
            encoded_query, self.api_key, max_results
        );

        let response = self.client.get(&search_url).send().await.map_err(|e| {
            AppError::Io(std::io::Error::other(format!(
                "SerpAPI request failed: {e}"
            )))
        })?;

        let json: serde_json::Value = response.json().await.map_err(|e| {
            AppError::Io(std::io::Error::other(format!(
                "Failed to parse SerpAPI response: {e}"
            )))
        })?;

        let mut results = Vec::new();
        if let Some(organic) = json.get("organic_results").and_then(|v| v.as_array()) {
            for item in organic.iter().take(max_results) {
                let title = item
                    .get("title")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let url = item
                    .get("link")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let snippet = item
                    .get("snippet")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();

                if !title.is_empty() && !url.is_empty() {
                    results.push(SearchItem {
                        title,
                        url,
                        snippet,
                        content: None,
                    });
                }
            }
        }

        Ok(results)
    }
}

/// DuckDuckGo Instant Answer API provider
pub struct DuckDuckGoApiProvider {
    client: reqwest::Client,
}

impl Default for DuckDuckGoApiProvider {
    fn default() -> Self {
        Self::new(Duration::from_secs(30))
    }
}

impl DuckDuckGoApiProvider {
    pub fn new(timeout: Duration) -> Self {
        Self {
            client: reqwest::Client::builder()
                .timeout(timeout)
                .build()
                .unwrap_or_default(),
        }
    }
}

#[async_trait::async_trait]
impl SearchProvider for DuckDuckGoApiProvider {
    fn name(&self) -> &str {
        "duckduckgo"
    }

    async fn search(&self, query: &str, max_results: usize) -> Result<Vec<SearchItem>> {
        let encoded_query = urlencoding::encode(query);
        // DuckDuckGo Instant Answer API
        let search_url = format!(
            "https://api.duckduckgo.com/?q={}&format=json&no_html=1&skip_disambig=1",
            encoded_query
        );

        let response = self.client.get(&search_url).send().await.map_err(|e| {
            AppError::Io(std::io::Error::other(format!(
                "DDG API request failed: {e}"
            )))
        })?;

        let json: serde_json::Value = response.json().await.map_err(|e| {
            AppError::Io(std::io::Error::other(format!(
                "Failed to parse DDG response: {e}"
            )))
        })?;

        let mut results = Vec::new();

        // Check for abstract (main answer)
        if let Some(abstract_text) = json.get("Abstract").and_then(|v| v.as_str())
            && !abstract_text.is_empty()
        {
            let url = json
                .get("AbstractURL")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let source = json
                .get("AbstractSource")
                .and_then(|v| v.as_str())
                .unwrap_or("DuckDuckGo");
            results.push(SearchItem {
                title: format!("{} - {}", query, source),
                url,
                snippet: abstract_text.to_string(),
                content: None,
            });
        }

        // Add related topics
        if let Some(topics) = json.get("RelatedTopics").and_then(|v| v.as_array()) {
            for topic in topics
                .iter()
                .take(max_results.saturating_sub(results.len()))
            {
                if let (Some(text), Some(url)) = (
                    topic.get("Text").and_then(|v| v.as_str()),
                    topic.get("FirstURL").and_then(|v| v.as_str()),
                ) && !text.is_empty()
                    && !url.is_empty()
                {
                    // Extract title from text (usually format: "Title - Description")
                    let (title, snippet) = if let Some(idx) = text.find(" - ") {
                        (text[..idx].to_string(), text[idx + 3..].to_string())
                    } else {
                        (query.to_string(), text.to_string())
                    };
                    results.push(SearchItem {
                        title,
                        url: url.to_string(),
                        snippet,
                        content: None,
                    });
                }
            }
        }

        Ok(results)
    }
}

/// Brave Search API provider
pub struct BraveSearchProvider {
    client: reqwest::Client,
    api_key: String,
}

impl BraveSearchProvider {
    pub fn new(api_key: String, timeout: Duration) -> Self {
        Self {
            client: reqwest::Client::builder()
                .timeout(timeout)
                .build()
                .unwrap_or_default(),
            api_key,
        }
    }
}

#[async_trait::async_trait]
impl SearchProvider for BraveSearchProvider {
    fn name(&self) -> &str {
        "brave"
    }

    async fn search(&self, query: &str, max_results: usize) -> Result<Vec<SearchItem>> {
        let encoded_query = urlencoding::encode(query);
        let search_url = format!(
            "https://api.search.brave.com/res/v1/web/search?q={}&count={}",
            encoded_query, max_results
        );

        let response = self
            .client
            .get(&search_url)
            .header("X-Subscription-Token", &self.api_key)
            .header("Accept", "application/json")
            .send()
            .await
            .map_err(|e| {
                AppError::Io(std::io::Error::other(format!(
                    "Brave API request failed: {e}"
                )))
            })?;

        let json: serde_json::Value = response.json().await.map_err(|e| {
            AppError::Io(std::io::Error::other(format!(
                "Failed to parse Brave response: {e}"
            )))
        })?;

        let mut results = Vec::new();
        if let Some(web) = json
            .get("web")
            .and_then(|v| v.get("results"))
            .and_then(|v| v.as_array())
        {
            for item in web.iter().take(max_results) {
                let title = item
                    .get("title")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let url = item
                    .get("url")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let snippet = item
                    .get("description")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();

                if !title.is_empty() && !url.is_empty() {
                    results.push(SearchItem {
                        title,
                        url,
                        snippet,
                        content: None,
                    });
                }
            }
        }

        Ok(results)
    }
}

// ============================================================================
// Web Search Service (Unified Facade)
// ============================================================================

/// Unified web search service with configurable providers and fallback chains
pub struct WebSearchService {
    config: WebSearchConfig,
    client: reqwest::Client,
    extractor: ContentExtractor,
}

impl Default for WebSearchService {
    fn default() -> Self {
        Self::new(WebSearchConfig::default())
    }
}

impl WebSearchService {
    pub fn new(config: WebSearchConfig) -> Self {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(config.timeout_secs))
            .user_agent(&config.user_agent)
            .build()
            .unwrap_or_default();

        Self {
            extractor: ContentExtractor::new(config.max_content_length),
            config,
            client,
        }
    }

    /// Create provider based on configuration
    fn create_provider(&self, provider_type: &WebSearchProvider) -> Box<dyn SearchProvider> {
        let timeout = Duration::from_secs(self.config.timeout_secs);

        match provider_type {
            WebSearchProvider::Local => Box::new(LocalSearchProvider::new(
                timeout,
                self.config.max_content_length,
            )),
            WebSearchProvider::SerpApi => {
                if let Some(ref key) = self.config.serpapi_key {
                    Box::new(SerpApiProvider::new(key.clone(), timeout))
                } else {
                    tracing::warn!("SerpAPI key not configured, falling back to local");
                    Box::new(LocalSearchProvider::new(
                        timeout,
                        self.config.max_content_length,
                    ))
                }
            }
            WebSearchProvider::DuckDuckGo => Box::new(DuckDuckGoApiProvider::new(timeout)),
            WebSearchProvider::Brave => {
                if let Some(ref key) = self.config.brave_key {
                    Box::new(BraveSearchProvider::new(key.clone(), timeout))
                } else {
                    tracing::warn!("Brave API key not configured, falling back to local");
                    Box::new(LocalSearchProvider::new(
                        timeout,
                        self.config.max_content_length,
                    ))
                }
            }
        }
    }

    /// Search using configured provider with fallback chain
    pub async fn search(&self, query: &str) -> Result<SearchResult> {
        let max_results = self.config.max_results;

        // Build provider chain: primary first, then fallbacks
        let mut providers = vec![self.config.provider.clone()];
        providers.extend(self.config.fallback_providers.clone());

        let mut last_error: Option<AppError> = None;

        for provider_type in &providers {
            let provider = self.create_provider(provider_type);
            tracing::info!(
                "Trying search provider '{}' for query: {}",
                provider.name(),
                query
            );

            match provider.search(query, max_results).await {
                Ok(mut results) => {
                    if results.is_empty() {
                        tracing::warn!(
                            "Provider '{}' returned 0 results for query: {}. This may indicate HTML structure changes or API issues.",
                            provider.name(),
                            query
                        );
                        // Don't treat empty results as an error - continue to next provider
                        last_error = Some(AppError::Io(std::io::Error::other(format!(
                            "Provider '{}' returned no results",
                            provider.name()
                        ))));
                        continue;
                    }

                    tracing::info!(
                        "Provider '{}' returned {} results",
                        provider.name(),
                        results.len()
                    );

                    // Optionally extract content from result pages
                    if self.config.extract_content {
                        results = self.enrich_with_content(results).await;
                    }

                    return Ok(SearchResult {
                        query: query.to_string(),
                        results,
                        provider: provider.name().to_string(),
                    });
                }
                Err(e) => {
                    tracing::warn!("Provider '{}' failed with error: {}", provider.name(), e);
                    last_error = Some(e);
                }
            }
        }

        // All providers failed or returned empty results
        Err(last_error.unwrap_or_else(|| {
            AppError::Io(std::io::Error::other(
                "All search providers failed or returned no results. \
                Consider configuring a search API (Brave or SerpAPI) in your config file. \
                The default Local provider uses DuckDuckGo HTML scraping which may be unreliable.",
            ))
        }))
    }

    /// Enrich search results with extracted content from top results
    async fn enrich_with_content(&self, mut results: Vec<SearchItem>) -> Vec<SearchItem> {
        // Only fetch content for first 3 results to avoid overwhelming requests
        let fetch_count = 3.min(results.len());

        for item in results.iter_mut().take(fetch_count) {
            if let Ok(content) = self.fetch_and_extract(&item.url).await {
                item.content = Some(content);
            }
        }

        results
    }

    /// Fetch a URL and extract structured content
    pub async fn fetch_and_extract(&self, url: &str) -> Result<ExtractedContent> {
        let response = self
            .client
            .get(url)
            .send()
            .await
            .map_err(|e| AppError::Io(std::io::Error::other(format!("Fetch failed: {e}"))))?;

        let html = response
            .text()
            .await
            .map_err(|e| AppError::Io(std::io::Error::other(format!("Read failed: {e}"))))?;

        let extractor = self.extractor.clone();
        let url = url.to_string();
        let error_url = url.clone();
        tokio::task::spawn_blocking(move || extractor.extract(&html, &url))
            .await
            .map_err(|error| {
                AppError::Io(std::io::Error::other(format!(
                    "Content extraction task failed for {}: {}",
                    error_url, error
                )))
            })
    }

    /// Fetch a web page (raw)
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

        let mut headers = HashMap::new();
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
}

// ============================================================================
// Legacy WebTools (for backward compatibility)
// ============================================================================

/// Web operations service (legacy interface)
/// Prefer using WebSearchService for new code
pub struct WebTools {
    service: WebSearchService,
}

impl Default for WebTools {
    fn default() -> Self {
        Self::new()
    }
}

impl WebTools {
    pub fn new() -> Self {
        Self {
            service: WebSearchService::default(),
        }
    }

    pub fn with_config(config: WebSearchConfig) -> Self {
        Self {
            service: WebSearchService::new(config),
        }
    }

    /// Fetch a web page
    pub async fn fetch(&self, url: &str) -> Result<FetchResult> {
        self.service.fetch(url).await
    }

    /// Search the web
    pub async fn search(&self, query: &str, num_results: Option<usize>) -> Result<SearchResult> {
        // Create a temporary config with custom result count if provided
        if let Some(count) = num_results {
            let mut config = self.service.config.clone();
            config.max_results = count;
            let temp_service = WebSearchService::new(config);
            temp_service.search(query).await
        } else {
            self.service.search(query).await
        }
    }

    /// Convert HTML to plain text
    pub fn html_to_text(&self, html: &str) -> String {
        let content = self.service.extractor.extract(html, "");
        content.main_content
    }

    /// Extract structured content from HTML
    pub fn extract_content(&self, html: &str, url: &str) -> ExtractedContent {
        self.service.extractor.extract(html, url)
    }

    /// Fetch a web page and extract structured content
    /// This is more efficient than fetch() for LLM consumption as it returns
    /// only the extracted content instead of raw HTML
    pub async fn fetch_and_extract(&self, url: &str) -> Result<ExtractedContent> {
        self.service.fetch_and_extract(url).await
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_content_extractor_basic() {
        let extractor = ContentExtractor::new(1000);
        let html = r#"
            <!DOCTYPE html>
            <html>
            <head><title>Test Page</title></head>
            <body>
                <h1>Hello World</h1>
                <p>This is a test paragraph.</p>
                <a href="https://example.com">Example Link</a>
            </body>
            </html>
        "#;

        let content = extractor.extract(html, "https://test.com");
        assert_eq!(content.title, Some("Test Page".to_string()));
        assert!(content.main_content.contains("Hello World"));
        assert!(content.main_content.contains("test paragraph"));
        assert!(!content.links.is_empty());
        assert!(!content.headings.is_empty());
    }

    #[test]
    fn test_content_extractor_code_blocks() {
        let extractor = ContentExtractor::new(1000);
        let html = r#"
            <html>
            <body>
                <pre><code class="language-rust">fn main() { println!("Hello"); }</code></pre>
            </body>
            </html>
        "#;

        let content = extractor.extract(html, "https://test.com");
        assert!(!content.code_blocks.is_empty());
        assert_eq!(content.code_blocks[0].language, Some("rust".to_string()));
    }

    #[test]
    fn test_ddg_url_resolution() {
        let provider = LocalSearchProvider::default();

        // Test redirect URL
        let redirect = "//duckduckgo.com/l/?uddg=https%3A%2F%2Fexample.com%2Fpath";
        assert_eq!(
            provider.resolve_ddg_url(redirect),
            "https://example.com/path"
        );

        // Test protocol-relative URL
        let relative = "//example.com/path";
        assert_eq!(
            provider.resolve_ddg_url(relative),
            "https://example.com/path"
        );

        // Test normal URL
        let normal = "https://example.com";
        assert_eq!(provider.resolve_ddg_url(normal), "https://example.com");
    }

    #[test]
    fn test_web_search_config_default() {
        let config = WebSearchConfig::default();
        assert!(matches!(config.provider, WebSearchProvider::Local));
        assert_eq!(config.max_results, 5);
        assert!(config.extract_content);
    }
}
