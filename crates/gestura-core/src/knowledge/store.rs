//! Knowledge store for loading and managing knowledge items
//!
//! The store handles loading knowledge from the filesystem, caching,
//! and matching queries to relevant knowledge items.

use super::types::{
    KnowledgeItem, KnowledgeMatch, KnowledgeQuery, KnowledgeReference, LoadCondition,
};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::RwLock;

/// Error type for knowledge operations
#[derive(Debug, thiserror::Error)]
pub enum KnowledgeError {
    #[error("Knowledge item not found: {0}")]
    NotFound(String),
    #[error("Failed to load knowledge: {0}")]
    LoadError(String),
    #[error("Failed to parse knowledge file: {0}")]
    ParseError(String),
    #[error("IO error: {0}")]
    IoError(#[from] std::io::Error),
}

/// Knowledge store for managing knowledge items
pub struct KnowledgeStore {
    /// All loaded knowledge items
    items: RwLock<HashMap<String, KnowledgeItem>>,
    /// Base directory for knowledge files
    base_dir: PathBuf,
    /// Cache of loaded reference content
    reference_cache: RwLock<HashMap<String, String>>,
}

impl KnowledgeStore {
    /// Create a new knowledge store with the given base directory
    pub fn new(base_dir: impl Into<PathBuf>) -> Self {
        Self {
            items: RwLock::new(HashMap::new()),
            base_dir: base_dir.into(),
            reference_cache: RwLock::new(HashMap::new()),
        }
    }

    /// Create a store with default knowledge directory
    pub fn with_default_dir() -> Self {
        let base_dir = dirs::config_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("gestura")
            .join("knowledge");
        Self::new(base_dir)
    }

    /// Get the base directory
    pub fn base_dir(&self) -> &Path {
        &self.base_dir
    }

    /// Register a knowledge item
    pub fn register(&self, item: KnowledgeItem) {
        let mut items = self.items.write().unwrap();
        items.insert(item.id.clone(), item);
    }

    /// Register multiple knowledge items
    pub fn register_all(&self, items: impl IntoIterator<Item = KnowledgeItem>) {
        let mut store = self.items.write().unwrap();
        for item in items {
            store.insert(item.id.clone(), item);
        }
    }

    /// Get a knowledge item by ID
    pub fn get(&self, id: &str) -> Option<KnowledgeItem> {
        let items = self.items.read().unwrap();
        items.get(id).cloned()
    }

    /// List all knowledge items
    pub fn list(&self) -> Vec<KnowledgeItem> {
        let items = self.items.read().unwrap();
        items.values().cloned().collect()
    }

    /// List knowledge items by category
    pub fn list_by_category(&self, category: &str) -> Vec<KnowledgeItem> {
        let items = self.items.read().unwrap();
        items
            .values()
            .filter(|item| item.category == category)
            .cloned()
            .collect()
    }

    /// Find knowledge items matching a query
    pub fn find(&self, query: &KnowledgeQuery) -> Vec<KnowledgeMatch> {
        let items = self.items.read().unwrap();
        let mut matches: Vec<KnowledgeMatch> = items
            .values()
            .filter(|item| item.enabled)
            .filter(|item| {
                if let Some(ref cats) = query.categories {
                    cats.contains(&item.category)
                } else {
                    true
                }
            })
            .filter_map(|item| item.matches(&query.query))
            .filter(|m| m.score >= query.min_score.unwrap_or(0.1))
            .collect();

        // Sort by score descending, then by priority
        matches.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| b.item.priority.cmp(&a.item.priority))
        });

        if let Some(limit) = query.limit {
            matches.truncate(limit);
        }

        matches
    }

    /// Load a reference file content
    pub fn load_reference(&self, item_id: &str, ref_id: &str) -> Result<String, KnowledgeError> {
        let cache_key = format!("{}:{}", item_id, ref_id);

        // Check cache first
        {
            let cache = self.reference_cache.read().unwrap();
            if let Some(content) = cache.get(&cache_key) {
                return Ok(content.clone());
            }
        }

        // Find the reference
        let items = self.items.read().unwrap();
        let item = items
            .get(item_id)
            .ok_or_else(|| KnowledgeError::NotFound(item_id.to_string()))?;

        let reference = item
            .references
            .iter()
            .find(|r| r.id == ref_id)
            .ok_or_else(|| KnowledgeError::NotFound(format!("{}:{}", item_id, ref_id)))?;

        // Load from filesystem
        let ref_path = self.base_dir.join(&item.id).join(&reference.path);
        let content = std::fs::read_to_string(&ref_path).map_err(|e| {
            KnowledgeError::LoadError(format!("Failed to load {}: {}", ref_path.display(), e))
        })?;

        // Cache the content
        {
            let mut cache = self.reference_cache.write().unwrap();
            cache.insert(cache_key, content.clone());
        }

        Ok(content)
    }

    /// Get all categories
    pub fn categories(&self) -> Vec<String> {
        let items = self.items.read().unwrap();
        let mut cats: Vec<String> = items.values().map(|i| i.category.clone()).collect();
        cats.sort();
        cats.dedup();
        cats
    }

    /// Count of knowledge items
    pub fn count(&self) -> usize {
        self.items.read().unwrap().len()
    }

    /// Enable or disable a knowledge item
    pub fn set_enabled(&self, id: &str, enabled: bool) -> Result<(), KnowledgeError> {
        let mut items = self.items.write().unwrap();
        let item = items
            .get_mut(id)
            .ok_or_else(|| KnowledgeError::NotFound(id.to_string()))?;
        item.enabled = enabled;
        Ok(())
    }

    /// Clear the reference cache
    pub fn clear_cache(&self) {
        let mut cache = self.reference_cache.write().unwrap();
        cache.clear();
    }
}

/// Register built-in knowledge items
pub fn register_builtin_knowledge(store: &KnowledgeStore) {
    store.register_all(builtin_knowledge_items());
}

/// Get built-in knowledge items
fn builtin_knowledge_items() -> Vec<KnowledgeItem> {
    vec![
        // Rust Expert
        KnowledgeItem::new(
            "rust-expert",
            "Rust Expert",
            "Expert Rust programming with ownership, borrowing, and async",
        )
        .with_triggers([
            "rust",
            "cargo",
            "ownership",
            "borrowing",
            "lifetime",
            "async rust",
            "tokio",
        ])
        .with_category("language")
        .with_content(include_str!("builtin/rust_expert.md"))
        .with_reference(
            KnowledgeReference::new("async", "Async Rust", "references/async.md")
                .with_load_condition(LoadCondition::Keywords(vec![
                    "async".into(),
                    "tokio".into(),
                    "future".into(),
                ])),
        )
        .with_reference(
            KnowledgeReference::new("error-handling", "Error Handling", "references/errors.md")
                .with_load_condition(LoadCondition::Keywords(vec![
                    "error".into(),
                    "result".into(),
                    "anyhow".into(),
                ])),
        ),
        // Tauri Expert
        KnowledgeItem::new(
            "tauri-expert",
            "Tauri Expert",
            "Expert Tauri v2 desktop application development",
        )
        .with_triggers(["tauri", "desktop app", "tauri v2", "tauri command", "ipc"])
        .with_category("framework")
        .with_content(include_str!("builtin/tauri_expert.md")),
        // CLI Development
        KnowledgeItem::new(
            "cli-expert",
            "CLI Expert",
            "Expert command-line interface development with clap and ratatui",
        )
        .with_triggers(["cli", "clap", "ratatui", "terminal", "tui", "command line"])
        .with_category("tool")
        .with_content(include_str!("builtin/cli_expert.md")),
        // Voice & Audio
        KnowledgeItem::new(
            "voice-expert",
            "Voice Expert",
            "Expert voice processing with Whisper and audio capture",
        )
        .with_triggers([
            "voice",
            "whisper",
            "speech",
            "audio",
            "transcription",
            "stt",
        ])
        .with_category("domain")
        .with_content(include_str!("builtin/voice_expert.md")),
        // MCP Protocol
        KnowledgeItem::new(
            "mcp-expert",
            "MCP Expert",
            "Expert Model Context Protocol implementation",
        )
        .with_triggers([
            "mcp",
            "model context protocol",
            "tool calling",
            "mcp server",
        ])
        .with_category("protocol")
        .with_content(include_str!("builtin/mcp_expert.md")),
        // A2A Protocol
        KnowledgeItem::new(
            "a2a-expert",
            "A2A Expert",
            "Expert Agent-to-Agent protocol implementation",
        )
        .with_triggers(["a2a", "agent to agent", "agent card", "agent protocol"])
        .with_category("protocol")
        .with_content(include_str!("builtin/a2a_expert.md")),
    ]
}
