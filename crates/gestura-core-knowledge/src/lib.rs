//! Knowledge system for agent expertise and context
//!
//! This crate provides a progressive disclosure knowledge system that loads
//! specialized expertise on-demand based on the user's query. Inspired by
//! the claude-skills pattern.
//!
//! # Architecture
//!
//! ```text
//! knowledge/
//! ├── rust-expert/
//! │   ├── KNOWLEDGE.md          # Lean core (~80 lines)
//! │   └── references/           # Loaded on-demand
//! │       ├── async.md
//! │       ├── errors.md
//! │       └── testing.md
//! └── tauri-expert/
//!     ├── KNOWLEDGE.md
//!     └── references/
//!         ├── commands.md
//!         └── plugins.md
//! ```
//!
//! # Usage
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
