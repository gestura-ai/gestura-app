//! Context manager for smart context reduction
//!
//! Manages context loading, caching, and reduction based on request analysis.

use super::analyzer::RequestAnalyzer;
use super::cache::{CacheStats, ContextCache};
use super::types::{ContextCategory, FileContext, RequestAnalysis, ResolvedContext, ToolContext};
use crate::tools::registry;
use std::collections::{HashMap, HashSet};
use std::hash::{Hash, Hasher};
use std::sync::{Arc, RwLock};
use std::time::SystemTime;

/// File metadata for cache invalidation
#[derive(Debug, Clone)]
struct FileMeta {
    /// Last modification time
    mtime: SystemTime,
    /// File size
    size: u64,
}

/// Cached response for similar requests
#[derive(Debug, Clone)]
pub struct CachedResponse {
    /// The response content
    pub response: String,
    /// When this was cached
    pub cached_at: std::time::Instant,
    /// Request hash that generated this
    pub request_hash: u64,
}

/// Manager for handling context in a smart, efficient way
pub struct ContextManager {
    /// Request analyzer
    analyzer: RequestAnalyzer,
    /// Cache for resolved contexts
    context_cache: Arc<ContextCache<ResolvedContext>>,
    /// Cache for file contents
    file_cache: Arc<ContextCache<FileContext>>,
    /// Cache for file metadata (for invalidation)
    file_meta_cache: Arc<RwLock<HashMap<String, FileMeta>>>,
    /// Cache for summarized history
    history_cache: Arc<ContextCache<String>>,
    /// Cache for similar request responses
    response_cache: Arc<RwLock<Vec<CachedResponse>>>,
    /// Maximum tokens for context
    max_context_tokens: usize,
    /// Whether to include tool schemas
    include_tool_schemas: bool,
    /// History summarization threshold (number of messages)
    history_threshold: usize,
    /// Maximum cached responses
    max_cached_responses: usize,
}

impl ContextManager {
    /// Create a new context manager
    pub fn new() -> Self {
        Self {
            analyzer: RequestAnalyzer::new(),
            context_cache: Arc::new(ContextCache::with_ttl(600)), // 10 min TTL
            file_cache: Arc::new(ContextCache::with_ttl(300)),    // 5 min TTL (LOW-2)
            file_meta_cache: Arc::new(RwLock::new(HashMap::new())),
            history_cache: Arc::new(ContextCache::with_ttl(300)), // 5 min TTL
            response_cache: Arc::new(RwLock::new(Vec::new())),
            max_context_tokens: 8000, // Conservative default
            include_tool_schemas: true,
            history_threshold: 10, // Summarize after 10 messages (matches max_history_messages)
            max_cached_responses: 10, // Keep last 10 responses (LOW-3)
        }
    }

    /// Set maximum context tokens
    pub fn with_max_tokens(mut self, max_tokens: usize) -> Self {
        self.max_context_tokens = max_tokens;
        self
    }

    /// Set history summarization threshold
    pub fn with_history_threshold(mut self, threshold: usize) -> Self {
        self.history_threshold = threshold;
        self
    }

    /// Disable tool schema inclusion for simpler contexts
    pub fn without_tool_schemas(mut self) -> Self {
        self.include_tool_schemas = false;
        self
    }

    /// Analyze a request to determine what context is needed
    pub fn analyze(&self, request: &str) -> RequestAnalysis {
        self.analyzer.analyze(request)
    }

    /// Resolve context for a request
    pub fn resolve_context<M>(
        &self,
        _request: &str,
        analysis: &RequestAnalysis,
        history: &[M],
    ) -> ResolvedContext
    where
        M: AsRef<str>,
    {
        self.resolve_for_analysis_with_history(analysis, history)
    }

    /// Simple resolve without history
    pub fn resolve_simple(&self, request: &str) -> ResolvedContext {
        let analysis = self.analyze(request);
        self.resolve_for_analysis(&analysis)
    }

    /// Resolve context for a pre-analyzed request with history
    pub fn resolve_for_analysis_with_history<M>(
        &self,
        analysis: &RequestAnalysis,
        history: &[M],
    ) -> ResolvedContext
    where
        M: AsRef<str>,
    {
        // Check cache first
        let cache_key = self.cache_key_for(analysis);
        if let Some(cached) = self.context_cache.get(&cache_key) {
            return cached;
        }

        // Build new context
        let mut context = ResolvedContext {
            categories: analysis.categories.clone(),
            ..ResolvedContext::default()
        };

        // Add tools if needed
        if analysis.needs_tools {
            context.tools = self.get_tools_for_categories(&analysis.categories);
            context.estimated_tokens += self.estimate_tool_tokens(&context.tools);
        }

        // Load files if mentioned (with mtime-based cache invalidation - LOW-2)
        for entity in &analysis.entities {
            if entity.entity_type != super::types::EntityType::FilePath {
                continue;
            }
            if let Some(file_ctx) = self.load_file_context_with_validation(&entity.value) {
                context.estimated_tokens += estimate_tokens(&file_ctx.content);
                context.files.push(file_ctx);
            }
        }

        // Add history summary with threshold-based summarization (LOW-1)
        let summary = self.summarize_history(history);
        if !summary.is_empty() {
            context.history_summary = Some(summary.clone());
            context.estimated_tokens += estimate_tokens(&summary);
        }

        // Cache the result
        self.context_cache.insert(cache_key, context.clone());

        context
    }

    /// Summarize history with intelligent threshold (LOW-1)
    pub fn summarize_history<M>(&self, history: &[M]) -> String
    where
        M: AsRef<str>,
    {
        if history.is_empty() {
            return String::new();
        }

        // Generate cache key from history length and last message hash
        let cache_key = format!(
            "history:{}:{}",
            history.len(),
            history
                .last()
                .map(|m| {
                    let mut hasher = std::collections::hash_map::DefaultHasher::new();
                    m.as_ref().hash(&mut hasher);
                    hasher.finish()
                })
                .unwrap_or(0)
        );

        // Check cache
        if let Some(cached) = self.history_cache.get(&cache_key) {
            return cached;
        }

        let summary = if history.len() > self.history_threshold {
            // Summarize: keep first 3 messages (context), last 5 messages (recent)
            let first_msgs: Vec<_> = history.iter().take(3).map(|m| m.as_ref()).collect();
            let last_msgs: Vec<_> = history
                .iter()
                .rev()
                .take(5)
                .map(|m| m.as_ref())
                .collect::<Vec<_>>()
                .into_iter()
                .rev()
                .collect();

            let summarized_count = history.len() - 8;
            tracing::debug!(
                total_messages = history.len(),
                threshold = self.history_threshold,
                summarized_messages = summarized_count,
                "Context manager: applying history summarization"
            );

            format!(
                "[Conversation start]\n{}\n[...{} messages summarized...]\n[Recent]\n{}",
                first_msgs.join("\n---\n"),
                summarized_count,
                last_msgs.join("\n---\n")
            )
        } else if history.len() > 5 {
            // Medium history: take last 5 messages
            tracing::debug!(
                total_messages = history.len(),
                threshold = self.history_threshold,
                "Context manager: medium history, taking last 5 messages"
            );

            history
                .iter()
                .rev()
                .take(5)
                .map(|m| m.as_ref())
                .collect::<Vec<_>>()
                .into_iter()
                .rev()
                .collect::<Vec<_>>()
                .join("\n---\n")
        } else {
            // Short history: include all
            tracing::debug!(
                total_messages = history.len(),
                "Context manager: short history, including all messages"
            );

            history
                .iter()
                .map(|m| m.as_ref())
                .collect::<Vec<_>>()
                .join("\n---\n")
        };

        // Cache the summary
        self.history_cache.insert(cache_key, summary.clone());
        summary
    }

    /// Resolve context for a pre-analyzed request (no history)
    pub fn resolve_for_analysis(&self, analysis: &RequestAnalysis) -> ResolvedContext {
        let empty: Vec<String> = Vec::new();
        self.resolve_for_analysis_with_history(analysis, &empty)
    }

    /// Get tools relevant to the given categories
    fn get_tools_for_categories(&self, categories: &HashSet<ContextCategory>) -> Vec<ToolContext> {
        let all_tools = registry::all_tools();
        let mut result = Vec::new();

        for tool in all_tools {
            let tool_cat = self.tool_to_category(tool.name);
            if categories.contains(&tool_cat) || categories.contains(&ContextCategory::Tools) {
                result.push(ToolContext {
                    name: tool.name.to_string(),
                    description: tool.summary.to_string(),
                    has_full_schema: self.include_tool_schemas,
                });
            }
        }

        result
    }

    /// Map tool name to category
    fn tool_to_category(&self, name: &str) -> ContextCategory {
        match name {
            "file" => ContextCategory::FileSystem,
            "shell" => ContextCategory::Shell,
            "git" => ContextCategory::Git,
            "code" => ContextCategory::Code,
            "web" => ContextCategory::Web,
            "permissions" => ContextCategory::Config,
            _ => ContextCategory::General,
        }
    }

    /// Load file context with mtime-based cache invalidation (LOW-2)
    fn load_file_context_with_validation(&self, path: &str) -> Option<FileContext> {
        // Check if file exists and get metadata
        let metadata = std::fs::metadata(path).ok()?;
        let mtime = metadata.modified().ok()?;
        let size = metadata.len();

        // Check if cached version is still valid
        let cache_valid = {
            let meta_cache = self.file_meta_cache.read().ok()?;
            if let Some(cached_meta) = meta_cache.get(path) {
                cached_meta.mtime == mtime && cached_meta.size == size
            } else {
                false
            }
        };

        if cache_valid && let Some(cached) = self.file_cache.get(path) {
            return Some(cached);
        }

        // Cache miss or invalidated - reload file
        let ctx = self.load_file_context(path)?;

        // Update metadata cache
        if let Ok(mut meta_cache) = self.file_meta_cache.write() {
            meta_cache.insert(path.to_string(), FileMeta { mtime, size });
        }

        Some(ctx)
    }

    /// Load file context (with caching)
    fn load_file_context(&self, path: &str) -> Option<FileContext> {
        // Check cache
        if let Some(cached) = self.file_cache.get(path) {
            return Some(cached);
        }

        // Try to read file
        match std::fs::read_to_string(path) {
            Ok(content) => {
                let lines: Vec<&str> = content.lines().collect();
                let total_lines = lines.len();
                let (content, truncated) = if total_lines > 100 {
                    // Truncate to first 100 lines
                    (lines[..100].join("\n"), true)
                } else {
                    (content, false)
                };

                let file_ctx = FileContext {
                    path: path.to_string(),
                    content,
                    truncated,
                    total_lines,
                };
                self.file_cache.insert(path.to_string(), file_ctx.clone());
                Some(file_ctx)
            }
            Err(_) => None,
        }
    }

    /// Estimate tokens for tools
    fn estimate_tool_tokens(&self, tools: &[ToolContext]) -> usize {
        tools
            .iter()
            .map(|t| {
                let base = estimate_tokens(&t.name) + estimate_tokens(&t.description);
                if t.has_full_schema { base + 100 } else { base }
            })
            .sum()
    }

    /// Generate cache key for analysis
    fn cache_key_for(&self, analysis: &RequestAnalysis) -> String {
        let mut cats: Vec<_> = analysis
            .categories
            .iter()
            .map(|c| format!("{:?}", c))
            .collect();
        cats.sort();
        format!("ctx:{}:{}", cats.join(","), analysis.needs_tools)
    }

    /// Get cache statistics
    pub fn cache_stats(&self) -> ContextManagerStats {
        ContextManagerStats {
            context_cache: self.context_cache.stats(),
            file_cache: self.file_cache.stats(),
            history_cache: self.history_cache.stats(),
        }
    }

    /// Clear all caches
    pub fn clear_caches(&self) {
        self.context_cache.clear();
        self.file_cache.clear();
        self.history_cache.clear();
    }

    /// Evict expired entries from all caches
    pub fn cleanup(&self) {
        self.context_cache.evict_expired();
        self.file_cache.evict_expired();
        self.history_cache.evict_expired();
    }

    // =========================================================================
    // Request Similarity Detection (LOW-3)
    // =========================================================================

    /// Compute a hash for a request based on categories and key entities
    pub fn compute_request_hash(&self, analysis: &RequestAnalysis) -> u64 {
        use std::collections::hash_map::DefaultHasher;
        let mut hasher = DefaultHasher::new();

        // Hash categories (sorted for consistency)
        let mut cats: Vec<_> = analysis
            .categories
            .iter()
            .map(|c| format!("{:?}", c))
            .collect();
        cats.sort();
        for cat in cats {
            cat.hash(&mut hasher);
        }

        // Hash key entities (sorted)
        let mut entities: Vec<_> = analysis
            .entities
            .iter()
            .map(|e| format!("{}:{}", e.entity_type as u8, e.value))
            .collect();
        entities.sort();
        for entity in entities {
            entity.hash(&mut hasher);
        }

        // Hash tool requirement
        analysis.needs_tools.hash(&mut hasher);

        hasher.finish()
    }

    /// Check if we have a cached response for a similar request
    pub fn get_cached_response(&self, analysis: &RequestAnalysis) -> Option<CachedResponse> {
        let request_hash = self.compute_request_hash(analysis);
        let cache = self.response_cache.read().ok()?;

        // Find matching response (within 5 minutes)
        let max_age = std::time::Duration::from_secs(300);
        cache
            .iter()
            .find(|r| r.request_hash == request_hash && r.cached_at.elapsed() < max_age)
            .cloned()
    }

    /// Cache a response for potential reuse
    pub fn cache_response(&self, analysis: &RequestAnalysis, response: String) {
        let request_hash = self.compute_request_hash(analysis);

        if let Ok(mut cache) = self.response_cache.write() {
            // Remove old entry with same hash if exists
            cache.retain(|r| r.request_hash != request_hash);

            // Add new entry
            cache.push(CachedResponse {
                response,
                cached_at: std::time::Instant::now(),
                request_hash,
            });

            // Trim to max size
            while cache.len() > self.max_cached_responses {
                cache.remove(0);
            }
        }
    }

    /// Check if a request is similar to a recent one (for deduplication)
    pub fn is_similar_to_recent(&self, analysis: &RequestAnalysis) -> bool {
        self.get_cached_response(analysis).is_some()
    }
}

impl Default for ContextManager {
    fn default() -> Self {
        Self::new()
    }
}

/// Statistics for the context manager
#[derive(Debug, Clone)]
pub struct ContextManagerStats {
    /// Context cache stats
    pub context_cache: CacheStats,
    /// File cache stats
    pub file_cache: CacheStats,
    /// History cache stats
    pub history_cache: CacheStats,
}

/// Estimate token count for a string (rough approximation)
pub fn estimate_tokens(s: &str) -> usize {
    // Rough estimate: ~4 chars per token on average
    (s.len() / 4).max(1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_context_manager_new() {
        let manager = ContextManager::new();
        assert!(manager.max_context_tokens > 0);
    }

    #[test]
    fn test_analyze_file_request() {
        let manager = ContextManager::new();
        let analysis = manager.analyze("Read the file src/main.rs");

        assert!(analysis.categories.contains(&ContextCategory::FileSystem));
        assert!(analysis.needs_tools);
    }

    #[test]
    fn test_resolve_context_general() {
        let manager = ContextManager::new();
        let context = manager.resolve_simple("What is Rust?");

        assert!(context.categories.contains(&ContextCategory::General));
        assert!(context.tools.is_empty());
    }

    #[test]
    fn test_resolve_context_with_tools() {
        let manager = ContextManager::new();
        let context = manager.resolve_simple("List files in the current directory");

        assert!(context.categories.contains(&ContextCategory::FileSystem));
        assert!(!context.tools.is_empty());
    }

    #[test]
    fn test_cache_stats() {
        let manager = ContextManager::new();
        // Make some calls to populate cache
        let _ = manager.resolve_simple("Read a file");
        let _ = manager.resolve_simple("Git status");

        let stats = manager.cache_stats();
        assert!(stats.context_cache.size > 0);
    }
}
