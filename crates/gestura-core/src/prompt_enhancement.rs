//! Prompt enhancement functionality for Gestura
//!
//! This module provides LLM-powered prompt enhancement to help users craft
//! more effective prompts for AI assistants.

use crate::config::AppConfig;
use crate::error::AppError;
use crate::llm_provider::{AgentContext, select_provider};
use std::collections::{HashMap, VecDeque};
use std::sync::Mutex;

/// LRU cache for prompt enhancements
/// Stores recently enhanced prompts to avoid redundant LLM calls
struct PromptCache {
    cache: HashMap<String, String>,
    lru_queue: VecDeque<String>,
    max_size: usize,
}

impl PromptCache {
    fn new(max_size: usize) -> Self {
        Self {
            cache: HashMap::new(),
            lru_queue: VecDeque::new(),
            max_size,
        }
    }

    fn get(&mut self, key: &str) -> Option<String> {
        if let Some(value) = self.cache.get(key) {
            // Move to front of LRU queue (most recently used)
            self.lru_queue.retain(|k| k != key);
            self.lru_queue.push_back(key.to_string());
            Some(value.clone())
        } else {
            None
        }
    }

    fn insert(&mut self, key: String, value: String) {
        // If cache is full, evict least recently used
        if self.cache.len() >= self.max_size
            && !self.cache.contains_key(&key)
            && let Some(lru_key) = self.lru_queue.pop_front()
        {
            self.cache.remove(&lru_key);
            tracing::debug!(evicted_key = %lru_key, "Evicted LRU cache entry");
        }

        // Insert new entry
        self.cache.insert(key.clone(), value);

        // Update LRU queue
        self.lru_queue.retain(|k| k != &key);
        self.lru_queue.push_back(key);
    }

    fn clear(&mut self) {
        self.cache.clear();
        self.lru_queue.clear();
    }
}

lazy_static::lazy_static! {
    static ref PROMPT_CACHE: Mutex<PromptCache> = Mutex::new(PromptCache::new(20));
}

/// Generate a cache key for prompt enhancement.
///
/// The cache key includes prompt, context, and the effective LLM provider/model.
/// This ensures switching a session's model/provider does not incorrectly reuse a
/// cached enhancement produced by a different backend.
fn generate_cache_key(prompt: &str, config: &AppConfig, context: &Option<PromptContext>) -> String {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};

    let mut hasher = DefaultHasher::new();

    // Include LLM selection
    config.llm.primary.hash(&mut hasher);
    match config.llm.primary.as_str() {
        "openai" => {
            if let Some(c) = &config.llm.openai {
                c.base_url.hash(&mut hasher);
                c.model.hash(&mut hasher);
            }
        }
        "anthropic" => {
            if let Some(c) = &config.llm.anthropic {
                c.base_url.hash(&mut hasher);
                c.model.hash(&mut hasher);
                c.thinking_budget_tokens.hash(&mut hasher);
            }
        }
        "grok" => {
            if let Some(c) = &config.llm.grok {
                c.base_url.hash(&mut hasher);
                c.model.hash(&mut hasher);
            }
        }
        "ollama" => {
            if let Some(c) = &config.llm.ollama {
                c.base_url.hash(&mut hasher);
                c.model.hash(&mut hasher);
            }
        }
        _ => {}
    }
    prompt.hash(&mut hasher);

    // Include context in hash if present
    if let Some(ctx) = context {
        if let Some(history) = &ctx.session_history {
            for (role, content) in history {
                role.hash(&mut hasher);
                content.hash(&mut hasher);
            }
        }
        if let Some((path, content)) = &ctx.active_file {
            path.hash(&mut hasher);
            content.hash(&mut hasher);
        }
        if let Some(info) = &ctx.project_info {
            info.hash(&mut hasher);
        }
        if let Some(entries) = &ctx.knowledge_entries {
            for entry in entries {
                entry.hash(&mut hasher);
            }
        }
    }

    format!("{:x}", hasher.finish())
}

/// Context information to include when enhancing prompts
#[derive(Debug, Clone, Default)]
pub struct PromptContext {
    /// Recent conversation history: (role, content) pairs
    /// Limited to last N messages to avoid token overflow
    pub session_history: Option<Vec<(String, String)>>,

    /// Active file being edited: (file_path, content)
    /// Useful for code-related prompts
    pub active_file: Option<(String, String)>,

    /// Project/codebase information
    /// Brief description of the project context
    pub project_info: Option<String>,

    /// Relevant knowledge base entries
    /// Pre-filtered knowledge that might be relevant
    pub knowledge_entries: Option<Vec<String>>,
}

impl PromptContext {
    /// Create a new empty context
    pub fn new() -> Self {
        Self::default()
    }

    /// Add session history (last N messages)
    pub fn with_session_history(mut self, history: Vec<(String, String)>) -> Self {
        self.session_history = Some(history);
        self
    }

    /// Add active file context
    pub fn with_active_file(mut self, path: String, content: String) -> Self {
        self.active_file = Some((path, content));
        self
    }

    /// Add project information
    pub fn with_project_info(mut self, info: String) -> Self {
        self.project_info = Some(info);
        self
    }

    /// Add knowledge entries
    pub fn with_knowledge(mut self, entries: Vec<String>) -> Self {
        self.knowledge_entries = Some(entries);
        self
    }

    /// Check if context is empty
    pub fn is_empty(&self) -> bool {
        self.session_history.is_none()
            && self.active_file.is_none()
            && self.project_info.is_none()
            && self.knowledge_entries.is_none()
    }
}

/// Generate system prompt based on enhancement style
fn get_enhancement_system_prompt(style: &str, max_length_multiplier: f64) -> String {
    let style_guidance = match style {
        "detailed" => {
            "Be thorough and comprehensive. Add detailed context, examples, and step-by-step breakdowns. Explain the reasoning behind requests."
        }
        "technical" => {
            "Use precise technical language. Include specific implementation details, edge cases, and technical constraints. Reference relevant technologies and best practices."
        }
        "concise" => {
            "Be brief and to the point. Add only essential context and clarity. Avoid unnecessary elaboration."
        }
        _ => {
            "Be brief and to the point. Add only essential context and clarity. Avoid unnecessary elaboration."
        }
    };

    format!(
        r#"You are a prompt enhancement assistant. Your task is to improve user prompts to be more effective for AI assistants.

Style: {}

Guidelines:
1. Preserve the user's intent and core request
2. Add relevant context and specificity where helpful
3. Structure complex requests into clear steps
4. Include success criteria when appropriate
5. Keep enhancements within {:.1}x original length
6. Maintain the user's tone and style
7. If the prompt is already clear and well-structured, make minimal changes
8. When context is provided (conversation history, files, project info), use it to make the prompt more specific and actionable
9. Reference relevant context naturally without being verbose

Respond with ONLY the enhanced prompt, no explanations or meta-commentary."#,
        style_guidance, max_length_multiplier
    )
}

/// Format context information into a string for the enhancement prompt
fn format_context(context: &PromptContext) -> String {
    let mut sections = Vec::new();

    // Add session history
    if let Some(history) = context
        .session_history
        .as_ref()
        .filter(|history| !history.is_empty())
    {
        let mut history_text = String::from("## Recent Conversation:\n");
        for (role, content) in history {
            // Truncate very long messages to avoid token overflow
            let truncated = if content.len() > 500 {
                format!("{}...", &content[..500])
            } else {
                content.clone()
            };
            history_text.push_str(&format!("{}: {}\n", role, truncated));
        }
        sections.push(history_text);
    }

    // Add active file context
    if let Some((path, content)) = &context.active_file {
        let mut file_text = format!("## Active File: {}\n", path);
        // Truncate file content to avoid token overflow (max 1000 chars)
        let truncated = if content.len() > 1000 {
            format!("{}...\n[truncated]", &content[..1000])
        } else {
            content.clone()
        };
        file_text.push_str(&truncated);
        sections.push(file_text);
    }

    // Add project information
    if let Some(info) = &context.project_info {
        sections.push(format!("## Project Context:\n{}\n", info));
    }

    // Add knowledge entries
    if let Some(entries) = context
        .knowledge_entries
        .as_ref()
        .filter(|entries| !entries.is_empty())
    {
        let mut knowledge_text = String::from("## Relevant Knowledge:\n");
        for entry in entries {
            // Truncate long knowledge entries
            let truncated = if entry.len() > 300 {
                format!("- {}...\n", &entry[..300])
            } else {
                format!("- {}\n", entry)
            };
            knowledge_text.push_str(&truncated);
        }
        sections.push(knowledge_text);
    }

    if sections.is_empty() {
        String::new()
    } else {
        format!("# Available Context:\n\n{}", sections.join("\n"))
    }
}

/// Enhance a user prompt using the configured LLM provider
///
/// This function takes a user's prompt and uses an LLM to improve it by:
/// - Adding relevant context and specificity
/// - Structuring complex requests into clear steps
/// - Including success criteria when appropriate
/// - Maintaining the user's original intent and tone
/// - Leveraging provided context (session history, files, project info)
///
/// # Arguments
///
/// * `prompt` - The original user prompt to enhance
/// * `config` - Application configuration (for LLM provider selection)
/// * `context` - Optional context information (session history, files, etc.)
///
/// # Returns
///
/// Returns the enhanced prompt as a String, or an error if enhancement fails.
///
/// # Example
///
/// ```no_run
/// use gestura_core::prompt_enhancement::{enhance_prompt_with_llm, PromptContext};
/// use gestura_core::config::AppConfig;
/// use gestura_core::AppConfigSecurityExt;
///
/// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
/// let config = AppConfig::load_async().await;
/// let original = "fix the bug";
///
/// // Without context
/// let enhanced = enhance_prompt_with_llm(original, &config, None).await?;
///
/// // With context
/// let context = PromptContext::new()
///     .with_session_history(vec![
///         ("user".to_string(), "I'm working on the login feature".to_string()),
///         ("assistant".to_string(), "I can help with that".to_string()),
///     ]);
/// let enhanced = enhance_prompt_with_llm(original, &config, Some(context)).await?;
///
/// println!("Enhanced: {}", enhanced);
/// # Ok(())
/// # }
/// ```
pub async fn enhance_prompt_with_llm(
    prompt: &str,
    config: &AppConfig,
    context: Option<PromptContext>,
) -> Result<String, AppError> {
    // Validate input
    let trimmed_prompt = prompt.trim();
    if trimmed_prompt.is_empty() {
        return Err(AppError::Llm("Prompt cannot be empty".to_string()));
    }

    // Check cache first
    let cache_key = generate_cache_key(trimmed_prompt, config, &context);
    {
        let mut cache = PROMPT_CACHE.lock().unwrap();
        if let Some(cached_result) = cache.get(&cache_key) {
            tracing::debug!(
                cache_key = %cache_key,
                "Returning cached prompt enhancement"
            );
            return Ok(cached_result);
        }
    }

    // Create agent context for prompt enhancement
    let agent_context = AgentContext {
        agent_id: "prompt_enhancer".to_string(),
    };

    // Select the configured LLM provider
    let provider = select_provider(config, &agent_context);

    // Get enhancement preferences from config
    let enhancement_settings = &config.prompt_enhancement;
    let system_prompt = get_enhancement_system_prompt(
        &enhancement_settings.style,
        enhancement_settings.max_length_multiplier(),
    );

    // Format context if provided
    let context_section = if let Some(ctx) = context.clone() {
        format_context(&ctx)
    } else {
        String::new()
    };

    // Construct the full prompt with system instructions and context
    let full_prompt = if context_section.is_empty() {
        format!(
            "{}\n\nUser prompt to enhance:\n{}\n\nEnhanced prompt:",
            system_prompt, trimmed_prompt
        )
    } else {
        format!(
            "{}\n\n{}\n\nUser prompt to enhance:\n{}\n\nEnhanced prompt:",
            system_prompt, context_section, trimmed_prompt
        )
    };

    tracing::debug!(
        original_length = trimmed_prompt.len(),
        has_context = !context_section.is_empty(),
        cache_key = %cache_key,
        "Enhancing prompt with LLM (cache miss)"
    );

    // Call the LLM
    let enhanced = provider.call(&full_prompt).await?;

    // Clean up the response
    // - Remove leading/trailing whitespace
    // - Remove surrounding quotes if present
    // - Ensure we have actual content
    let cleaned = enhanced.trim().trim_matches('"').trim();

    if cleaned.is_empty() {
        tracing::warn!("LLM returned empty enhancement, using original prompt");
        return Ok(trimmed_prompt.to_string());
    }

    tracing::debug!(
        original_length = trimmed_prompt.len(),
        enhanced_length = cleaned.len(),
        expansion_ratio = cleaned.len() as f64 / trimmed_prompt.len() as f64,
        "Prompt enhancement complete"
    );

    // Store in cache for future use
    {
        let mut cache = PROMPT_CACHE.lock().unwrap();
        cache.insert(cache_key, cleaned.to_string());
        tracing::debug!("Cached prompt enhancement");
    }

    Ok(cleaned.to_string())
}

/// Clear the prompt enhancement cache
/// Useful for testing or when you want to force fresh enhancements
pub fn clear_prompt_cache() {
    let mut cache = PROMPT_CACHE.lock().unwrap();
    cache.clear();
    tracing::info!("Cleared prompt enhancement cache");
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::PromptEnhancementSettings;

    #[tokio::test]
    async fn test_enhance_prompt_with_unconfigured_provider() {
        // Test that enhancement fails gracefully with unconfigured provider
        let config = AppConfig::default();

        let original = "fix the bug";
        let result = enhance_prompt_with_llm(original, &config, None).await;

        // Should fail because no provider is configured
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("not configured"));
    }

    #[test]
    fn test_empty_prompt_validation() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let config = AppConfig::default();
            let result = enhance_prompt_with_llm("", &config, None).await;
            assert!(result.is_err());
            assert!(
                result
                    .unwrap_err()
                    .to_string()
                    .contains("Prompt cannot be empty")
            );
        });
    }

    #[test]
    fn test_whitespace_only_prompt_validation() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let config = AppConfig::default();
            let result = enhance_prompt_with_llm("   \n\t  ", &config, None).await;
            assert!(result.is_err());
        });
    }

    #[test]
    fn test_enhance_with_session_context_unconfigured() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let config = AppConfig::default();

            let context = PromptContext::new().with_session_history(vec![
                (
                    "user".to_string(),
                    "I'm working on authentication".to_string(),
                ),
                ("assistant".to_string(), "I can help with that".to_string()),
            ]);

            // Should fail because no provider is configured
            let result = enhance_prompt_with_llm("add login", &config, Some(context)).await;
            assert!(result.is_err());
        });
    }

    #[test]
    fn test_context_formatting() {
        let context = PromptContext::new()
            .with_session_history(vec![
                ("user".to_string(), "Hello".to_string()),
                ("assistant".to_string(), "Hi there!".to_string()),
            ])
            .with_project_info("A Rust project".to_string());

        let formatted = format_context(&context);
        assert!(formatted.contains("Recent Conversation"));
        assert!(formatted.contains("Project Context"));
        assert!(formatted.contains("Hello"));
        assert!(formatted.contains("A Rust project"));
    }

    #[test]
    fn test_empty_context() {
        let context = PromptContext::new();
        let formatted = format_context(&context);
        assert!(formatted.is_empty());
    }

    #[test]
    fn test_cache_operations() {
        // Test that cache clear works without errors
        clear_prompt_cache();

        // Clear again to ensure idempotency
        clear_prompt_cache();
    }

    #[test]
    fn test_cache_key_generation() {
        let prompt1 = "test prompt";
        let prompt2 = "test prompt";
        let prompt3 = "different prompt";

        let config = AppConfig::default();

        // Same prompt should generate same key
        let key1 = generate_cache_key(prompt1, &config, &None);
        let key2 = generate_cache_key(prompt2, &config, &None);
        assert_eq!(key1, key2);

        // Different prompt should generate different key
        let key3 = generate_cache_key(prompt3, &config, &None);
        assert_ne!(key1, key3);

        // Same prompt with different model should generate different key
        let mut openai_cfg = AppConfig::default();
        openai_cfg.llm.primary = "openai".to_string();
        openai_cfg.llm.openai = Some(crate::config::OpenAiConfig {
            api_key: String::new(),
            base_url: None,
            model: "gpt-4o".to_string(),
        });
        let mut openai_cfg_2 = openai_cfg.clone();
        if let Some(c) = openai_cfg_2.llm.openai.as_mut() {
            c.model = "gpt-4o-mini".to_string();
        }
        let key_model_1 = generate_cache_key(prompt1, &openai_cfg, &None);
        let key_model_2 = generate_cache_key(prompt1, &openai_cfg_2, &None);
        assert_ne!(key_model_1, key_model_2);

        // Same prompt with different context should generate different key
        let context = Some(
            PromptContext::new()
                .with_session_history(vec![("user".to_string(), "context".to_string())]),
        );
        let key4 = generate_cache_key(prompt1, &config, &context);
        assert_ne!(key1, key4);
    }

    #[test]
    fn test_enhancement_style_system_prompts() {
        // Test concise style
        let concise_prompt = get_enhancement_system_prompt("concise", 3.0);
        assert!(concise_prompt.contains("Be brief and to the point"));
        assert!(concise_prompt.contains("3.0x"));

        // Test detailed style
        let detailed_prompt = get_enhancement_system_prompt("detailed", 4.0);
        assert!(detailed_prompt.contains("Be thorough and comprehensive"));
        assert!(detailed_prompt.contains("4.0x"));

        // Test technical style
        let technical_prompt = get_enhancement_system_prompt("technical", 2.5);
        assert!(technical_prompt.contains("Use precise technical language"));
        assert!(technical_prompt.contains("2.5x"));

        // Test unknown style defaults to concise
        let unknown_prompt = get_enhancement_system_prompt("unknown", 3.0);
        assert!(unknown_prompt.contains("Be brief and to the point"));
    }

    #[test]
    fn test_user_preferences_settings() {
        // Test that user preferences can be set without calling the LLM
        let mut config = AppConfig::default();

        // Test with detailed style
        config.prompt_enhancement.style = "detailed".to_string();
        config.prompt_enhancement.set_max_length_multiplier(4.0);
        assert_eq!(config.prompt_enhancement.style, "detailed");
        assert_eq!(config.prompt_enhancement.max_length_multiplier(), 4.0);

        // Test with technical style
        config.prompt_enhancement.style = "technical".to_string();
        config.prompt_enhancement.set_max_length_multiplier(2.0);
        assert_eq!(config.prompt_enhancement.style, "technical");
        assert_eq!(config.prompt_enhancement.max_length_multiplier(), 2.0);
    }

    #[test]
    fn test_max_length_multiplier_conversion() {
        let mut settings = PromptEnhancementSettings::default();

        // Test default value (3.0x)
        assert_eq!(settings.max_length_multiplier(), 3.0);

        // Test setting valid values
        settings.set_max_length_multiplier(2.5);
        assert_eq!(settings.max_length_multiplier(), 2.5);

        settings.set_max_length_multiplier(4.0);
        assert_eq!(settings.max_length_multiplier(), 4.0);

        // Test clamping to valid range (1.0 - 5.0)
        settings.set_max_length_multiplier(0.5);
        assert_eq!(settings.max_length_multiplier(), 1.0);

        settings.set_max_length_multiplier(10.0);
        assert_eq!(settings.max_length_multiplier(), 5.0);
    }
}
