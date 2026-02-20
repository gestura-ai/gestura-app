//! Smart context management for efficient LLM interactions
//!
//! This module provides tools for analyzing user requests, determining what
//! context is needed, and efficiently managing context through caching.
//!
//! # Architecture
//!
//! The context system uses a three-tier approach:
//!
//! 1. **Request Analysis** - Parse user requests to determine intent without LLM
//! 2. **Context Resolution** - Load only the context needed for the request
//! 3. **Smart Caching** - Cache frequently accessed context with TTL
//!
//! # Example
//!
//! ```rust,ignore
//! use gestura_core::context::ContextManager;
//!
//! let manager = ContextManager::new();
//!
//! // Analyze a request
//! let analysis = manager.analyze("Read the file src/main.rs");
//! println!("Needs tools: {}", analysis.needs_tools);
//! println!("Categories: {:?}", analysis.categories);
//!
//! // Resolve context (includes caching)
//! let context = manager.resolve_context("Show git status");
//! println!("Tools available: {}", context.tools.len());
//! ```

mod analyzer;
mod cache;
mod manager;

pub use analyzer::RequestAnalyzer;
pub use cache::{CacheStats, ContextCache};
pub use gestura_core_foundation::context::*;
pub use manager::{ContextManager, ContextManagerStats, ToolProviderFn, estimate_tokens};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_full_context_flow() {
        let manager = ContextManager::new();

        // Test file-related request
        let ctx = manager.resolve_simple("Read src/lib.rs and show its contents", None);
        assert!(ctx.categories.contains(&ContextCategory::FileSystem));

        // Test git request
        let ctx = manager.resolve_simple("Show me the git log", None);
        assert!(ctx.categories.contains(&ContextCategory::Git));

        // Test general conversation
        let ctx = manager.resolve_simple("Hello, how are you?", None);
        assert!(ctx.categories.contains(&ContextCategory::General));
        assert!(ctx.tools.is_empty());
    }

    #[test]
    fn test_caching_behavior() {
        let manager = ContextManager::new();

        // First call
        let ctx1 = manager.resolve_simple("Run a shell command", None);

        // Second call should be cached
        let ctx2 = manager.resolve_simple("Execute a terminal command", None);

        // Both should have shell category
        assert!(ctx1.categories.contains(&ContextCategory::Shell));
        assert!(ctx2.categories.contains(&ContextCategory::Shell));

        // Check cache has entries
        let stats = manager.cache_stats();
        assert!(stats.context_cache.size > 0);
    }

    #[test]
    fn test_entity_extraction() {
        let analyzer = RequestAnalyzer::new();

        let analysis = analyzer.analyze("Fetch https://api.example.com/data");
        assert!(
            analysis
                .entities
                .iter()
                .any(|e| e.entity_type == EntityType::Url)
        );
        assert!(analysis.categories.contains(&ContextCategory::Web));
    }

    #[test]
    fn test_resolve_context_with_tool_provider() {
        // Create a manager with a mock tool provider
        let manager = ContextManager::new().with_tool_provider(Box::new(|| {
            vec![
                ("file".to_string(), "Read/write files".to_string()),
                ("shell".to_string(), "Run shell commands".to_string()),
                ("git".to_string(), "Git operations".to_string()),
            ]
        }));
        let context = manager.resolve_simple("List files in the current directory", None);

        assert!(context.categories.contains(&ContextCategory::FileSystem));
        assert!(!context.tools.is_empty());
    }
}
