//! Knowledge types for the agent knowledge system
//!
//! This module defines the core types for the knowledge system, which provides
//! specialized context and expertise to the agent based on the task at hand.
//! Inspired by the claude-skills progressive disclosure pattern.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// A knowledge item represents a specialized area of expertise
/// that can be loaded on-demand to provide context to the agent.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KnowledgeItem {
    /// Unique identifier for this knowledge item
    pub id: String,
    /// Human-readable name
    pub name: String,
    /// Brief description with trigger keywords
    pub description: String,
    /// Keywords that trigger this knowledge item
    pub triggers: Vec<String>,
    /// Category for organization (e.g., "language", "framework", "tool")
    pub category: String,
    /// Core content (lean, ~80 lines max)
    pub core_content: String,
    /// Reference files that can be loaded on-demand
    pub references: Vec<KnowledgeReference>,
    /// Metadata for filtering and search
    pub metadata: HashMap<String, String>,
    /// Whether this knowledge is enabled
    pub enabled: bool,
    /// Priority for conflict resolution (higher = more important)
    pub priority: u32,
}

/// A reference file that can be loaded on-demand
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KnowledgeReference {
    /// Reference identifier
    pub id: String,
    /// Topic name
    pub topic: String,
    /// Relative path to the reference file
    pub path: String,
    /// When to load this reference
    pub load_when: LoadCondition,
    /// Cached content (loaded on-demand)
    #[serde(skip)]
    pub content: Option<String>,
}

/// Conditions for when to load a reference
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum LoadCondition {
    /// Always load with the knowledge item
    Always,
    /// Load when specific keywords are detected
    Keywords(Vec<String>),
    /// Load when explicitly requested
    OnDemand,
    /// Load based on context analysis
    Context,
}

/// Result of matching knowledge items to a query
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KnowledgeMatch {
    /// The matched knowledge item
    pub item: KnowledgeItem,
    /// Match score (0.0 - 1.0)
    pub score: f32,
    /// Which triggers matched
    pub matched_triggers: Vec<String>,
    /// References that should be loaded based on context
    pub suggested_references: Vec<String>,
}

/// Knowledge query for finding relevant knowledge items
#[derive(Debug, Clone, Default)]
pub struct KnowledgeQuery {
    /// The user's query or task description
    pub query: String,
    /// Specific categories to search
    pub categories: Option<Vec<String>>,
    /// Maximum number of results
    pub limit: Option<usize>,
    /// Minimum match score threshold
    pub min_score: Option<f32>,
}

impl KnowledgeItem {
    /// Create a new knowledge item with minimal required fields
    pub fn new(
        id: impl Into<String>,
        name: impl Into<String>,
        description: impl Into<String>,
    ) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            description: description.into(),
            triggers: Vec::new(),
            category: "general".to_string(),
            core_content: String::new(),
            references: Vec::new(),
            metadata: HashMap::new(),
            enabled: true,
            priority: 50,
        }
    }

    /// Add a trigger keyword
    pub fn with_trigger(mut self, trigger: impl Into<String>) -> Self {
        self.triggers.push(trigger.into());
        self
    }

    /// Add multiple trigger keywords
    pub fn with_triggers(mut self, triggers: impl IntoIterator<Item = impl Into<String>>) -> Self {
        self.triggers.extend(triggers.into_iter().map(|t| t.into()));
        self
    }

    /// Set the category
    pub fn with_category(mut self, category: impl Into<String>) -> Self {
        self.category = category.into();
        self
    }

    /// Set the core content
    pub fn with_content(mut self, content: impl Into<String>) -> Self {
        self.core_content = content.into();
        self
    }

    /// Add a reference
    pub fn with_reference(mut self, reference: KnowledgeReference) -> Self {
        self.references.push(reference);
        self
    }

    /// Check if this knowledge item matches a query
    pub fn matches(&self, query: &str) -> Option<KnowledgeMatch> {
        let query_lower = query.to_lowercase();
        let mut matched_triggers = Vec::new();
        let mut score = 0.0f32;

        // Check triggers
        for trigger in &self.triggers {
            if query_lower.contains(&trigger.to_lowercase()) {
                matched_triggers.push(trigger.clone());
                score += 0.3;
            }
        }

        // Check name and description
        if query_lower.contains(&self.name.to_lowercase()) {
            score += 0.4;
        }
        if query_lower.contains(&self.description.to_lowercase()) {
            score += 0.2;
        }

        // Check for partial word matches in description
        let desc_words: Vec<&str> = self.description.split_whitespace().collect();
        for word in desc_words {
            if word.len() > 3 && query_lower.contains(&word.to_lowercase()) {
                score += 0.1;
            }
        }

        score = score.min(1.0);

        if score > 0.0 || !matched_triggers.is_empty() {
            Some(KnowledgeMatch {
                item: self.clone(),
                score,
                matched_triggers,
                suggested_references: Vec::new(),
            })
        } else {
            None
        }
    }
}

impl KnowledgeReference {
    /// Create a new reference
    pub fn new(id: impl Into<String>, topic: impl Into<String>, path: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            topic: topic.into(),
            path: path.into(),
            load_when: LoadCondition::OnDemand,
            content: None,
        }
    }

    /// Set the load condition
    pub fn with_load_condition(mut self, condition: LoadCondition) -> Self {
        self.load_when = condition;
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_knowledge_item_creation() {
        let item = KnowledgeItem::new(
            "rust-expert",
            "Rust Expert",
            "Expert Rust programming knowledge",
        )
        .with_triggers(["rust", "cargo", "ownership", "borrowing"])
        .with_category("language");

        assert_eq!(item.id, "rust-expert");
        assert_eq!(item.triggers.len(), 4);
        assert_eq!(item.category, "language");
    }

    #[test]
    fn test_knowledge_matching() {
        let item = KnowledgeItem::new("rust-expert", "Rust Expert", "Expert Rust programming")
            .with_triggers(["rust", "cargo", "ownership"]);

        let match_result = item.matches("Help me with Rust ownership");
        assert!(match_result.is_some());
        let m = match_result.unwrap();
        assert!(m.score > 0.0);
        assert!(m.matched_triggers.contains(&"rust".to_string()));
        assert!(m.matched_triggers.contains(&"ownership".to_string()));
    }

    #[test]
    fn test_no_match() {
        let item = KnowledgeItem::new("rust-expert", "Rust Expert", "Expert Rust programming")
            .with_triggers(["rust", "cargo"]);

        let match_result = item.matches("Help me with Python");
        assert!(match_result.is_none());
    }
}
