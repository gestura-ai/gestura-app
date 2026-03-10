//! Knowledge system for agent expertise and contextual guidance.
//!
//! `gestura-core-knowledge` provides a progressive-disclosure knowledge base
//! that lets the runtime expose specialized expertise only when it is relevant
//! to the current request. This keeps default prompts lean while still allowing
//! rich built-in guidance for areas such as Rust, Tauri, CLI workflows, MCP,
//! voice, and other expert domains.
//!
//! ## Design role
//!
//! This crate sits beside, not inside, the request-context system:
//!
//! - `gestura-core-context` decides *what categories of context* are relevant
//! - `gestura-core-knowledge` provides curated expert content that can be
//!   enabled, matched, and loaded when those requests benefit from it
//!
//! Knowledge items can come from built-in expert documents or user-managed
//! additions persisted on disk.
//!
//! ## Main concepts
//!
//! - `KnowledgeStore`: registry and persistence layer for knowledge items
//! - `KnowledgeItem`: a single expert document with metadata, triggers, and
//!   optional reference material
//! - `KnowledgeQuery`: query-time filter and ranking input
//! - `KnowledgeSettingsManager`: per-session/default enablement for knowledge
//!   items so users and sessions can opt into specific expertise
//!
//! ## Built-in knowledge structure
//!
//! Built-in experts follow a compact core-plus-references pattern:
//!
//! ```text
//! knowledge/
//! ├── rust-expert/
//! │   ├── KNOWLEDGE.md
//! │   └── references/
//! └── tauri-expert/
//!     ├── KNOWLEDGE.md
//!     └── references/
//! ```
//!
//! The goal is to keep the top-level expert doc concise and load reference
//! material only when that extra depth is needed.
//!
//! ## Usage
//!
//! ```rust,ignore
//! use gestura_core::knowledge::{KnowledgeStore, KnowledgeQuery};
//!
//! let store = KnowledgeStore::with_default_dir();
//! register_builtin_knowledge(&store);
//!
//! let query = KnowledgeQuery {
//!     query: "Help me with async Rust".to_string(),
//!     ..Default::default()
//! };
//!
//! let matches = store.find(&query);
//! for m in matches {
//!     println!("Matched: {} (score: {})", m.item.name, m.score);
//! }
//! ```
//!
//! ## Stable import paths
//!
//! Most code should import through `gestura_core::knowledge::*`.

pub mod session_settings;
mod store;
mod types;

pub use session_settings::DEFAULT_KNOWLEDGE_SETTINGS_SESSION_ID;
pub use session_settings::{KnowledgeSettingsManager, SessionKnowledgeSettings};
pub use store::{KnowledgeError, KnowledgeStore, register_builtin_knowledge};
pub use types::{KnowledgeItem, KnowledgeMatch, KnowledgeQuery, KnowledgeReference, LoadCondition};

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_store_creation() {
        let store = KnowledgeStore::with_default_dir();
        assert_eq!(store.count(), 0);
    }

    #[test]
    fn test_register_and_find() {
        let tmp = tempdir().unwrap();
        let store = KnowledgeStore::new(tmp.path());

        let item = KnowledgeItem::new("test-item", "Test Item", "A test knowledge item")
            .with_triggers(["test", "example"]);

        store.register(item);
        assert_eq!(store.count(), 1);

        let query = KnowledgeQuery {
            query: "help me with a test".to_string(),
            ..Default::default()
        };

        let matches = store.find(&query);
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].item.id, "test-item");
    }

    #[test]
    fn test_builtin_knowledge() {
        let tmp = tempdir().unwrap();
        let store = KnowledgeStore::new(tmp.path());
        register_builtin_knowledge(&store);

        // Should have builtin items
        assert!(store.count() > 0);

        // Should find rust expert
        let query = KnowledgeQuery {
            query: "help with rust ownership".to_string(),
            ..Default::default()
        };
        let matches = store.find(&query);
        assert!(!matches.is_empty());
    }

    #[test]
    fn test_categories() {
        let tmp = tempdir().unwrap();
        let store = KnowledgeStore::new(tmp.path());
        register_builtin_knowledge(&store);

        let cats = store.categories();
        assert!(cats.contains(&"language".to_string()));
        assert!(cats.contains(&"framework".to_string()));
    }

    #[test]
    fn persist_and_reload_user_item() {
        let tmp = tempdir().unwrap();

        let store = KnowledgeStore::new(tmp.path());
        register_builtin_knowledge(&store);

        let user_item = KnowledgeItem::new(
            "my-custom",
            "My Custom",
            "Custom knowledge for testing persistence",
        )
        .with_triggers(["custom", "persist"])
        .with_category("user")
        .with_content("Hello from disk");

        store.upsert_user_item(user_item).unwrap();

        // New store should be able to load it.
        let store2 = KnowledgeStore::new(tmp.path());
        register_builtin_knowledge(&store2);
        let loaded = store2.load_user_items().unwrap();
        assert_eq!(loaded, 1);

        let fetched = store2.get("my-custom").unwrap();
        assert_eq!(fetched.name, "My Custom");
        assert!(fetched.core_content.contains("Hello from disk"));
        assert_eq!(
            fetched.metadata.get("origin").map(|s| s.as_str()),
            Some("user")
        );
    }
}
