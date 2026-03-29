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
    fn test_builtin_queries_resolve_expected_experts() {
        let tmp = tempdir().unwrap();
        let store = KnowledgeStore::new(tmp.path());
        register_builtin_knowledge(&store);

        for (query, expected_id) in [
            (
                "help me fix cargo clippy warnings in async rust with tokio and serde",
                "rust-expert",
            ),
            (
                "build a tauri plugin command with an invoke handler and capabilities",
                "tauri-expert",
            ),
            (
                "add a clap subcommand with --json output and no_color-friendly terminal ux",
                "cli-expert",
            ),
            (
                "transcribe microphone audio with whisper-rs, cpal, vad, and speech-to-text",
                "voice-expert",
            ),
            (
                "implement an mcp server with tools/list, tools/call, resources/read, and notifications/initialized",
                "mcp-expert",
            ),
            (
                "publish an agent card for a remote agent and stream task delegation with sendSubscribe",
                "a2a-expert",
            ),
        ] {
            let matches = store.find(&KnowledgeQuery {
                query: query.to_string(),
                limit: Some(3),
                ..Default::default()
            });

            assert!(!matches.is_empty(), "expected matches for query: {query}");
            assert_eq!(matches[0].item.id, expected_id, "query: {query}");
            assert!(
                !matches[0].matched_triggers.is_empty(),
                "expected trigger matches for query: {query}"
            );
        }
    }

    #[test]
    fn test_specialty_queries_resolve_expected_experts() {
        let tmp = tempdir().unwrap();
        let store = KnowledgeStore::new(tmp.path());
        register_builtin_knowledge(&store);

        for (query, expected_id) in [
            (
                "design an analytics metric tree with instrumentation, retention cohorts, and experimentation readouts",
                "analytics-expert",
            ),
            (
                "review a scientific study for experimental design, evidence quality, reproducibility, and confounders",
                "science-expert",
            ),
            (
                "work through mathematical modeling, optimization, derivations, and estimation checks",
                "math-expert",
            ),
            (
                "develop marketing positioning, messaging, go to market planning, and growth campaigns for an ICP",
                "marketing-expert",
            ),
            (
                "review software system design with api contracts, observability, architecture, and production reliability",
                "software-systems-expert",
            ),
            (
                "design a robotics autonomy stack covering perception, localization, controls, and robot safety",
                "robotics-expert",
            ),
            (
                "evaluate a mechanical engineering concept for loads, materials, tolerances, manufacturability, and fatigue",
                "mechanical-engineering-expert",
            ),
            (
                "review an electrical engineering design for power electronics, pcb interfaces, signal integrity, and validation",
                "electrical-engineering-expert",
            ),
            (
                "plan a civil engineering site with grading, drainage, structures, constructability, and foundation constraints",
                "civil-engineering-expert",
            ),
            (
                "analyze a chemical engineering process with mass balance, thermodynamics, reaction kinetics, unit operations, and process safety",
                "chemical-engineering-expert",
            ),
            (
                "evaluate an aerospace mission architecture with flight dynamics, guidance, control, and verification",
                "aerospace-expert",
            ),
        ] {
            let matches = store.find(&KnowledgeQuery {
                query: query.to_string(),
                limit: Some(3),
                ..Default::default()
            });

            assert!(!matches.is_empty(), "expected matches for query: {query}");
            assert_eq!(matches[0].item.id, expected_id, "query: {query}");
            assert!(
                !matches[0].matched_triggers.is_empty(),
                "expected trigger matches for query: {query}"
            );
        }
    }

    #[test]
    fn test_categories() {
        let tmp = tempdir().unwrap();
        let store = KnowledgeStore::new(tmp.path());
        register_builtin_knowledge(&store);

        let cats = store.categories();
        assert!(cats.contains(&"language".to_string()));
        assert!(cats.contains(&"framework".to_string()));
        assert!(cats.contains(&"analytics".to_string()));
        assert!(cats.contains(&"robotics".to_string()));
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
