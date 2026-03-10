//! Knowledge store for loading and managing knowledge items
//!
//! The store handles loading knowledge from the filesystem, caching,
//! and matching queries to relevant knowledge items.

use super::types::{
    KnowledgeItem, KnowledgeMatch, KnowledgeQuery, KnowledgeReference, LoadCondition,
};
use std::collections::HashMap;
use std::fs;
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
    #[error("Invalid knowledge id: {0}")]
    InvalidId(String),
    #[error("Forbidden operation: {0}")]
    Forbidden(String),
    #[error("IO error: {0}")]
    IoError(#[from] std::io::Error),
}

const ORIGIN_KEY: &str = "origin";
const ORIGIN_BUILTIN: &str = "builtin";
const ORIGIN_USER: &str = "user";

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

    fn validate_id(id: &str) -> Result<(), KnowledgeError> {
        let trimmed = id.trim();
        if trimmed.is_empty() {
            return Err(KnowledgeError::InvalidId("id cannot be empty".to_string()));
        }
        if trimmed.contains('/') || trimmed.contains('\\') || trimmed.contains("..") {
            return Err(KnowledgeError::InvalidId(format!(
                "id contains illegal path characters: {trimmed}"
            )));
        }
        Ok(())
    }

    fn item_dir(&self, id: &str) -> PathBuf {
        self.base_dir.join(id)
    }

    fn item_json_path(&self, id: &str) -> PathBuf {
        self.item_dir(id).join("item.json")
    }

    /// Load user-persisted knowledge items from disk.
    ///
    /// This is best-effort: invalid entries are skipped (with a warning) so the
    /// app can still start.
    pub fn load_user_items(&self) -> Result<usize, KnowledgeError> {
        fs::create_dir_all(&self.base_dir)?;

        let mut loaded = 0usize;
        let entries = match fs::read_dir(&self.base_dir) {
            Ok(e) => e,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(0),
            Err(e) => return Err(KnowledgeError::IoError(e)),
        };

        for entry in entries {
            let entry = match entry {
                Ok(e) => e,
                Err(e) => {
                    tracing::warn!(error = %e, "Skipping unreadable knowledge dir entry");
                    continue;
                }
            };

            let path = entry.path();
            if !path.is_dir() {
                continue;
            }

            let item_path = path.join("item.json");
            if !item_path.exists() {
                continue;
            }

            let raw = match fs::read_to_string(&item_path) {
                Ok(s) => s,
                Err(e) => {
                    tracing::warn!(path = %item_path.display(), error = %e, "Skipping unreadable knowledge item");
                    continue;
                }
            };

            let mut item: KnowledgeItem = match serde_json::from_str(&raw) {
                Ok(v) => v,
                Err(e) => {
                    tracing::warn!(path = %item_path.display(), error = %e, "Skipping unparseable knowledge item");
                    continue;
                }
            };

            if let Err(e) = Self::validate_id(&item.id) {
                tracing::warn!(path = %item_path.display(), error = %e, "Skipping knowledge item with invalid id");
                continue;
            }

            // Ensure user origin.
            item.metadata
                .insert(ORIGIN_KEY.to_string(), ORIGIN_USER.to_string());

            // Do not allow a persisted user item to override a builtin.
            if let Some(existing) = self.get(&item.id)
                && existing
                    .metadata
                    .get(ORIGIN_KEY)
                    .is_some_and(|v| v == ORIGIN_BUILTIN)
            {
                tracing::warn!(id = %item.id, "Skipping user item that collides with a builtin id");
                continue;
            }

            self.register(item);
            loaded += 1;
        }

        Ok(loaded)
    }

    /// Create or update a user knowledge item on disk, and register it in-memory.
    pub fn upsert_user_item(&self, mut item: KnowledgeItem) -> Result<(), KnowledgeError> {
        Self::validate_id(&item.id)?;

        // Prevent overwriting builtin items.
        if let Some(existing) = self.get(&item.id)
            && existing
                .metadata
                .get(ORIGIN_KEY)
                .is_some_and(|v| v == ORIGIN_BUILTIN)
        {
            return Err(KnowledgeError::Forbidden(format!(
                "cannot modify builtin knowledge item: {}",
                item.id
            )));
        }

        item.metadata
            .insert(ORIGIN_KEY.to_string(), ORIGIN_USER.to_string());

        let dir = self.item_dir(&item.id);
        fs::create_dir_all(&dir)?;

        let path = self.item_json_path(&item.id);
        let tmp_path = path.with_extension("json.tmp");
        let raw = serde_json::to_string_pretty(&item)
            .map_err(|e| KnowledgeError::ParseError(e.to_string()))?;

        fs::write(&tmp_path, raw)?;
        fs::rename(&tmp_path, &path)?;

        self.register(item);
        Ok(())
    }

    /// Delete a user knowledge item from disk and remove it from the store.
    pub fn delete_user_item(&self, id: &str) -> Result<(), KnowledgeError> {
        Self::validate_id(id)?;

        let existing = self
            .get(id)
            .ok_or_else(|| KnowledgeError::NotFound(id.to_string()))?;
        let origin = existing.metadata.get(ORIGIN_KEY).map(|s| s.as_str());
        if origin == Some(ORIGIN_BUILTIN) {
            return Err(KnowledgeError::Forbidden(format!(
                "cannot delete builtin knowledge item: {id}"
            )));
        }

        let dir = self.item_dir(id);
        if dir.exists() {
            fs::remove_dir_all(&dir)?;
        }

        {
            let mut items = self.items.write().unwrap();
            items.remove(id);
        }
        self.clear_cache();
        Ok(())
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
    ///
    /// Searches across all registered items regardless of their `enabled` field.
    /// Per-session enablement is managed by [`crate::KnowledgeSettingsManager`].
    pub fn find(&self, query: &KnowledgeQuery) -> Vec<KnowledgeMatch> {
        let items = self.items.read().unwrap();
        let mut matches: Vec<KnowledgeMatch> = items
            .values()
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
    let mut items = vec![
        // Rust Expert
        KnowledgeItem::new(
            "rust-expert",
            "Rust Expert",
            "Expert Rust programming with ownership, borrowing, and async",
        )
        .with_metadata(ORIGIN_KEY, ORIGIN_BUILTIN)
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
        .with_metadata(ORIGIN_KEY, ORIGIN_BUILTIN)
        .with_triggers(["tauri", "desktop app", "tauri v2", "tauri command", "ipc"])
        .with_category("framework")
        .with_content(include_str!("builtin/tauri_expert.md")),
        // CLI Development
        KnowledgeItem::new(
            "cli-expert",
            "CLI Expert",
            "Expert command-line interface development with clap and ratatui",
        )
        .with_metadata(ORIGIN_KEY, ORIGIN_BUILTIN)
        .with_triggers(["cli", "clap", "ratatui", "terminal", "tui", "command line"])
        .with_category("tool")
        .with_content(include_str!("builtin/cli_expert.md")),
        // Voice & Audio
        KnowledgeItem::new(
            "voice-expert",
            "Voice Expert",
            "Expert voice processing with Whisper and audio capture",
        )
        .with_metadata(ORIGIN_KEY, ORIGIN_BUILTIN)
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
        .with_metadata(ORIGIN_KEY, ORIGIN_BUILTIN)
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
        .with_metadata(ORIGIN_KEY, ORIGIN_BUILTIN)
        .with_triggers(["a2a", "agent to agent", "agent card", "agent protocol"])
        .with_category("protocol")
        .with_content(include_str!("builtin/a2a_expert.md")),
        // Python Expert
        KnowledgeItem::new(
            "python-expert",
            "Python Expert",
            "Expert Python programming with modern Python 3.10+ patterns",
        )
        .with_metadata(ORIGIN_KEY, ORIGIN_BUILTIN)
        .with_triggers([
            "python", "pip", "asyncio", "pydantic", "fastapi", "django", "flask", "pytest",
        ])
        .with_category("language")
        .with_content(include_str!("builtin/python_expert.md")),
        // JavaScript Expert
        KnowledgeItem::new(
            "javascript-expert",
            "JavaScript Expert",
            "Expert JavaScript programming with modern ES2023+ patterns",
        )
        .with_metadata(ORIGIN_KEY, ORIGIN_BUILTIN)
        .with_triggers([
            "javascript",
            "js",
            "node",
            "nodejs",
            "npm",
            "ecmascript",
            "promise",
            "event loop",
        ])
        .with_category("language")
        .with_content(include_str!("builtin/javascript_expert.md")),
        // TypeScript Expert
        KnowledgeItem::new(
            "typescript-expert",
            "TypeScript Expert",
            "Expert TypeScript programming with strict type system patterns",
        )
        .with_metadata(ORIGIN_KEY, ORIGIN_BUILTIN)
        .with_triggers([
            "typescript",
            "ts",
            "tsconfig",
            "type inference",
            "generics typescript",
            "zod",
        ])
        .with_category("language")
        .with_content(include_str!("builtin/typescript_expert.md")),
        // Go Expert
        KnowledgeItem::new(
            "go-expert",
            "Go Expert",
            "Expert Go programming with idiomatic patterns and concurrency",
        )
        .with_metadata(ORIGIN_KEY, ORIGIN_BUILTIN)
        .with_triggers([
            "go",
            "golang",
            "goroutine",
            "channel",
            "go module",
            "defer",
            "interface go",
        ])
        .with_category("language")
        .with_content(include_str!("builtin/go_expert.md")),
        // Java Expert
        KnowledgeItem::new(
            "java-expert",
            "Java Expert",
            "Expert Java programming with modern Java 21+ and the JVM ecosystem",
        )
        .with_metadata(ORIGIN_KEY, ORIGIN_BUILTIN)
        .with_triggers([
            "java",
            "jvm",
            "spring",
            "maven",
            "gradle",
            "record java",
            "stream api",
            "virtual threads",
        ])
        .with_category("language")
        .with_content(include_str!("builtin/java_expert.md")),
        // C++ Expert
        KnowledgeItem::new(
            "cpp-expert",
            "C++ Expert",
            "Expert C++ programming with modern C++20/23 and RAII patterns",
        )
        .with_metadata(ORIGIN_KEY, ORIGIN_BUILTIN)
        .with_triggers([
            "c++",
            "cpp",
            "cmake",
            "raii",
            "smart pointer",
            "template",
            "stl",
            "concepts",
        ])
        .with_category("language")
        .with_content(include_str!("builtin/cpp_expert.md")),
        // C# Expert
        KnowledgeItem::new(
            "csharp-expert",
            "C# Expert",
            "Expert C# programming with modern C# 12+ and .NET 8+",
        )
        .with_metadata(ORIGIN_KEY, ORIGIN_BUILTIN)
        .with_triggers([
            "c#", "csharp", "dotnet", ".net", "asp.net", "linq", "nuget", "blazor",
        ])
        .with_category("language")
        .with_content(include_str!("builtin/csharp_expert.md")),
        // Swift Expert
        KnowledgeItem::new(
            "swift-expert",
            "Swift Expert",
            "Expert Swift programming with Swift 6, concurrency, and Apple platform development",
        )
        .with_metadata(ORIGIN_KEY, ORIGIN_BUILTIN)
        .with_triggers([
            "swift",
            "swiftui",
            "ios",
            "macos development",
            "xcode",
            "actor swift",
            "combine",
            "swiftpm",
        ])
        .with_category("language")
        .with_content(include_str!("builtin/swift_expert.md")),
        // Kotlin Expert
        KnowledgeItem::new(
            "kotlin-expert",
            "Kotlin Expert",
            "Expert Kotlin programming with coroutines and multiplatform development",
        )
        .with_metadata(ORIGIN_KEY, ORIGIN_BUILTIN)
        .with_triggers([
            "kotlin",
            "coroutines",
            "ktor",
            "android kotlin",
            "kotlin multiplatform",
            "flow kotlin",
        ])
        .with_category("language")
        .with_content(include_str!("builtin/kotlin_expert.md")),
        // Ruby Expert
        KnowledgeItem::new(
            "ruby-expert",
            "Ruby Expert",
            "Expert Ruby programming with Ruby 3.x, Rails, and idiomatic patterns",
        )
        .with_metadata(ORIGIN_KEY, ORIGIN_BUILTIN)
        .with_triggers([
            "ruby",
            "rails",
            "gem",
            "bundler",
            "rspec",
            "active record",
            "rake",
        ])
        .with_category("language")
        .with_content(include_str!("builtin/ruby_expert.md")),
        // PHP Expert
        KnowledgeItem::new(
            "php-expert",
            "PHP Expert",
            "Expert PHP programming with modern PHP 8.3+ and Composer ecosystem",
        )
        .with_metadata(ORIGIN_KEY, ORIGIN_BUILTIN)
        .with_triggers([
            "php", "composer", "laravel", "symfony", "phpunit", "psr", "doctrine",
        ])
        .with_category("language")
        .with_content(include_str!("builtin/php_expert.md")),
    ];

    items.extend(anthropic_knowledge_work_plugins_sales_items());
    items.extend(anthropic_skills_items());
    items
}

fn with_apache_source(
    item: KnowledgeItem,
    source_repo: &'static str,
    source_path: &'static str,
    source_url: &'static str,
) -> KnowledgeItem {
    item.with_metadata(ORIGIN_KEY, ORIGIN_BUILTIN)
        .with_metadata("license", "Apache-2.0")
        .with_metadata("source_repo", source_repo)
        .with_metadata("source_path", source_path)
        .with_metadata("source_url", source_url)
}

fn anthropic_knowledge_work_plugins_sales_items() -> Vec<KnowledgeItem> {
    const REPO: &str = "anthropics/knowledge-work-plugins";

    vec![
        // Commands (slash-command style playbooks)
        with_apache_source(
            KnowledgeItem::new(
                "sales-call-summary",
                "Sales: Call Summary",
                "Turn call notes/transcripts into action items + follow-up email",
            )
            .with_triggers([
                "sales",
                "call summary",
                "meeting notes",
                "transcript",
                "follow-up email",
                "action items",
            ])
            .with_category("sales")
            .with_priority(60)
            .with_content(include_str!("builtin/anthropics/sales_call_summary.md")),
            REPO,
            "sales/commands/call-summary.md",
            "https://raw.githubusercontent.com/anthropics/knowledge-work-plugins/main/sales/commands/call-summary.md",
        ),
        with_apache_source(
            KnowledgeItem::new(
                "sales-forecast",
                "Sales: Forecast",
                "Generate commit/upside forecast with best/likely/worst scenarios",
            )
            .with_triggers([
                "sales",
                "forecast",
                "quota",
                "pipeline",
                "commit",
                "upside",
                "coverage ratio",
            ])
            .with_category("sales")
            .with_priority(55)
            .with_content(include_str!("builtin/anthropics/sales_forecast.md")),
            REPO,
            "sales/commands/forecast.md",
            "https://raw.githubusercontent.com/anthropics/knowledge-work-plugins/main/sales/commands/forecast.md",
        ),
        with_apache_source(
            KnowledgeItem::new(
                "sales-pipeline-review",
                "Sales: Pipeline Review",
                "Pipeline health check + prioritization + weekly action plan",
            )
            .with_triggers([
                "sales",
                "pipeline review",
                "pipeline health",
                "stale deals",
                "close date",
                "next steps",
            ])
            .with_category("sales")
            .with_priority(55)
            .with_content(include_str!("builtin/anthropics/sales_pipeline_review.md")),
            REPO,
            "sales/commands/pipeline-review.md",
            "https://raw.githubusercontent.com/anthropics/knowledge-work-plugins/main/sales/commands/pipeline-review.md",
        ),
        // Skills (auto-triggered workflows)
        with_apache_source(
            KnowledgeItem::new(
                "sales-account-research",
                "Sales: Account Research",
                "Research a company/person for outreach and meeting prep",
            )
            .with_triggers([
                "account research",
                "prospect research",
                "company research",
                "firmographics",
                "linkedin",
                "sales",
            ])
            .with_category("sales")
            .with_priority(50)
            .with_content(include_str!("builtin/anthropics/sales_account_research.md")),
            REPO,
            "sales/skills/account-research/SKILL.md",
            "https://raw.githubusercontent.com/anthropics/knowledge-work-plugins/main/sales/skills/account-research/SKILL.md",
        ),
        with_apache_source(
            KnowledgeItem::new(
                "sales-call-prep",
                "Sales: Call Prep",
                "Generate a call brief (agenda, questions, objections) for a prospect meeting",
            )
            .with_triggers([
                "call prep",
                "meeting prep",
                "discovery call",
                "demo",
                "agenda",
                "sales",
            ])
            .with_category("sales")
            .with_priority(50)
            .with_content(include_str!("builtin/anthropics/sales_call_prep.md")),
            REPO,
            "sales/skills/call-prep/SKILL.md",
            "https://raw.githubusercontent.com/anthropics/knowledge-work-plugins/main/sales/skills/call-prep/SKILL.md",
        ),
        with_apache_source(
            KnowledgeItem::new(
                "sales-competitive-intelligence",
                "Sales: Competitive Intelligence",
                "Create a competitor battlecard + positioning talk-tracks",
            )
            .with_triggers([
                "competitive intelligence",
                "battlecard",
                "competitor",
                "positioning",
                "win loss",
                "sales",
            ])
            .with_category("sales")
            .with_priority(45)
            .with_content(include_str!(
                "builtin/anthropics/sales_competitive_intelligence.md"
            )),
            REPO,
            "sales/skills/competitive-intelligence/SKILL.md",
            "https://raw.githubusercontent.com/anthropics/knowledge-work-plugins/main/sales/skills/competitive-intelligence/SKILL.md",
        ),
        with_apache_source(
            KnowledgeItem::new(
                "sales-create-an-asset",
                "Sales: Create an Asset",
                "Build customer-ready sales assets (one-pagers, decks, landing pages, workflows)",
            )
            .with_triggers([
                "create an asset",
                "sales asset",
                "one pager",
                "deck",
                "landing page",
                "workflow demo",
            ])
            .with_category("sales")
            .with_priority(45)
            .with_content(include_str!("builtin/anthropics/sales_create_an_asset.md")),
            REPO,
            "sales/skills/create-an-asset/SKILL.md",
            "https://raw.githubusercontent.com/anthropics/knowledge-work-plugins/main/sales/skills/create-an-asset/SKILL.md",
        ),
        with_apache_source(
            KnowledgeItem::new(
                "sales-daily-briefing",
                "Sales: Daily Briefing",
                "Daily prioritized plan (meetings, pipeline alerts, email priorities)",
            )
            .with_triggers([
                "daily briefing",
                "morning brief",
                "what should i do today",
                "sales",
                "pipeline alerts",
            ])
            .with_category("sales")
            .with_priority(40)
            .with_content(include_str!("builtin/anthropics/sales_daily_briefing.md")),
            REPO,
            "sales/skills/daily-briefing/SKILL.md",
            "https://raw.githubusercontent.com/anthropics/knowledge-work-plugins/main/sales/skills/daily-briefing/SKILL.md",
        ),
        with_apache_source(
            KnowledgeItem::new(
                "sales-draft-outreach",
                "Sales: Draft Outreach",
                "Research-first cold outreach (email + LinkedIn) with clear CTA",
            )
            .with_triggers([
                "draft outreach",
                "cold email",
                "linkedin message",
                "personalized outreach",
                "sales",
            ])
            .with_category("sales")
            .with_priority(45)
            .with_content(include_str!("builtin/anthropics/sales_draft_outreach.md")),
            REPO,
            "sales/skills/draft-outreach/SKILL.md",
            "https://raw.githubusercontent.com/anthropics/knowledge-work-plugins/main/sales/skills/draft-outreach/SKILL.md",
        ),
    ]
}

fn anthropic_skills_items() -> Vec<KnowledgeItem> {
    const REPO: &str = "anthropics/skills";

    vec![
        with_apache_source(
            KnowledgeItem::new(
                "anthropic-mcp-builder",
                "MCP Builder (Anthropic Skill)",
                "Guide to building high-quality MCP servers and tools",
            )
            .with_triggers([
                "mcp",
                "model context protocol",
                "mcp server",
                "tool schema",
                "tool design",
                "inspector",
            ])
            .with_category("protocol")
            .with_priority(55)
            .with_content(include_str!("builtin/anthropics/skill_mcp_builder.md")),
            REPO,
            "skills/mcp-builder/SKILL.md",
            "https://raw.githubusercontent.com/anthropics/skills/main/skills/mcp-builder/SKILL.md",
        ),
        with_apache_source(
            KnowledgeItem::new(
                "anthropic-frontend-design",
                "Frontend Design (Anthropic Skill)",
                "Guidelines for distinctive, production-grade UI implementation",
            )
            .with_triggers([
                "frontend design",
                "ui design",
                "css",
                "react",
                "component",
                "visual design",
                "typography",
            ])
            .with_category("design")
            .with_priority(50)
            .with_content(include_str!("builtin/anthropics/skill_frontend_design.md")),
            REPO,
            "skills/frontend-design/SKILL.md",
            "https://raw.githubusercontent.com/anthropics/skills/main/skills/frontend-design/SKILL.md",
        ),
    ]
}
