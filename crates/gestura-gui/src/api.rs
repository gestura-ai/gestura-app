//! Tauri command handlers for configuration, MCP tools, MDH pointers, and tests.
use crate::{
    AppConfig,
    llm_provider::{AgentContext, select_provider},
};
use tauri::{Emitter, Manager};

/// Try to get an API key from the keychain (synchronous, for use in config creation).
/// Returns empty string if not found or keychain unavailable.
fn try_get_api_key_from_keychain(provider: &str) -> String {
    try_get_api_key_from_keychain_sync(provider)
}

/// Apply session-scoped LLM provider/model overrides to an in-memory `AppConfig`.
///
/// This helper keeps all per-session LLM behavior consistent across features (chat,
/// prompt enhancement, etc.) by applying the same override rules:
///
/// - If the session overrides the provider, set `cfg.llm.primary`.
/// - If the session overrides the model, apply it to the active provider's config.
///
/// This does **not** persist any changes to disk; it only affects the current request.
fn apply_session_llm_config_overrides(cfg: &mut AppConfig, session_id: &str) {
    let session_llm = crate::window_manager::get_session_llm_config(session_id);
    tracing::debug!(
        session_id = %session_id,
        session_llm_config = ?session_llm,
        "Retrieved session LLM config for overrides"
    );

    let Some(session_llm) = session_llm else {
        return;
    };

    if let Some(provider) = session_llm.provider {
        tracing::info!(
            session_id = %session_id,
            provider = %provider,
            "Applying session-scoped LLM provider override"
        );
        cfg.llm.primary = provider;
    }

    if let Some(model) = session_llm.model {
        // Defensive validation: if the persisted session override is inconsistent
        // (e.g. provider=openai + model=grok-2), ignore the model override.
        if !crate::llm_validation::is_model_compatible_with_provider(&cfg.llm.primary, &model) {
            tracing::warn!(
                session_id = %session_id,
                provider = %cfg.llm.primary,
                model = %model,
                "Ignoring incompatible session-scoped LLM model override"
            );
            return;
        }

        tracing::info!(
            session_id = %session_id,
            model = %model,
            provider = %cfg.llm.primary,
            "Applying session-scoped LLM model override"
        );

        // Apply model to the active provider's config
        // Create provider config if it doesn't exist, trying keychain for API keys.
        match cfg.llm.primary.as_str() {
            "openai" => {
                let openai = cfg.llm.openai.get_or_insert_with(|| {
                    let api_key = try_get_api_key_from_keychain("openai");
                    if api_key.is_empty() {
                        tracing::warn!("OpenAI provider selected but no API key found");
                    }
                    gestura_core::config::OpenAiConfig {
                        api_key,
                        model: model.clone(),
                        base_url: None,
                    }
                });
                openai.model = model;
            }
            "anthropic" => {
                let anthropic = cfg.llm.anthropic.get_or_insert_with(|| {
                    let api_key = try_get_api_key_from_keychain("anthropic");
                    if api_key.is_empty() {
                        tracing::warn!("Anthropic provider selected but no API key found");
                    }
                    gestura_core::config::AnthropicConfig {
                        api_key,
                        model: model.clone(),
                        base_url: None,
                        thinking_budget_tokens: None,
                    }
                });
                anthropic.model = model;
            }
            "grok" => {
                let grok = cfg.llm.grok.get_or_insert_with(|| {
                    let api_key = try_get_api_key_from_keychain("grok");
                    if api_key.is_empty() {
                        tracing::warn!("Grok provider selected but no API key found");
                    }
                    gestura_core::config::GrokConfig {
                        api_key,
                        model: model.clone(),
                        base_url: None,
                    }
                });
                grok.model = model;
            }
            "ollama" => {
                // Ollama doesn't require API key, so create default config if missing.
                let ollama =
                    cfg.llm
                        .ollama
                        .get_or_insert_with(|| gestura_core::config::OllamaConfig {
                            base_url: "http://localhost:11434".into(),
                            model: model.clone(),
                        });
                ollama.model = model;
            }
            "echo" => {
                // Echo provider doesn't need config; model is ignored.
                tracing::debug!("Echo provider selected - model override not applicable");
            }
            other => {
                tracing::warn!(
                    provider = other,
                    "Unknown provider - model override ignored"
                );
            }
        }
    }
}

/// Public synchronous keychain API key retrieval.
///
/// This is used by other modules (e.g., speech.rs) to retrieve API keys from the
/// system keychain with a fallback to empty string if not found.
///
/// Provider names are case-insensitive and match the keychain key format:
/// - `"openai"` → `gestura_api_key_openai`
/// - `"voice_openai"` → `gestura_api_key_voice_openai`
/// - `"anthropic"` → `gestura_api_key_anthropic`
pub fn try_get_api_key_from_keychain_sync(provider: &str) -> String {
    let key = format!("gestura_api_key_{}", provider.to_lowercase());
    let storage = crate::security::create_secure_storage();

    // Use a blocking runtime to call the async method
    // This is safe because we're in a sync context and the keychain operation is fast
    match std::thread::spawn(move || {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .ok()?;
        rt.block_on(async { storage.get_secret(&key).await.ok().flatten() })
    })
    .join()
    {
        Ok(Some(key)) => key,
        Ok(None) => String::new(),
        Err(_) => String::new(),
    }
}

/// Get the current application configuration.
///
/// JS↔Rust interop: This command is exposed to the frontend via Tauri.
#[tauri::command(rename_all = "snake_case")]
pub async fn get_config() -> Result<AppConfig, String> {
    Ok(AppConfig::load_async().await)
}

/// Persist a new application configuration.
///
/// JS↔Rust interop: The frontend invokes this command with `{ cfg: AppConfig }`.
#[tauri::command(rename_all = "snake_case")]
pub async fn save_config(cfg: AppConfig) -> Result<(), String> {
    cfg.save_async().await.map_err(|e| e.to_string())
}

/// Check if this is the first run of the application (no config file exists yet).
#[tauri::command]
pub fn is_first_run() -> bool {
    AppConfig::is_first_run()
}

/// Get the path to the configuration file.
#[tauri::command]
pub fn get_config_path() -> String {
    AppConfig::default_path().to_string_lossy().to_string()
}

/// Tool information for the frontend
#[derive(serde::Serialize)]
pub struct ToolInfo {
    pub name: String,
    pub summary: String,
    pub inputs: Vec<String>,
    pub side_effects: Vec<String>,
    pub examples: Vec<String>,
}

/// List all built-in tools
#[tauri::command]
pub fn list_builtin_tools() -> Vec<ToolInfo> {
    gestura_core::tools::all_tools()
        .iter()
        .map(|t| ToolInfo {
            name: t.name.to_string(),
            summary: t.summary.to_string(),
            inputs: t.inputs.iter().map(|s| s.to_string()).collect(),
            side_effects: t.side_effects.iter().map(|s| s.to_string()).collect(),
            examples: t.examples.iter().map(|s| s.to_string()).collect(),
        })
        .collect()
}

#[tauri::command]
pub async fn list_mcp_tools() -> Result<Vec<crate::config::McpTool>, String> {
    Ok(AppConfig::load_async().await.mcp_tools)
}

#[tauri::command]
pub async fn add_mcp_tool(tool: crate::config::McpTool) -> Result<(), String> {
    let mut cfg = AppConfig::load_async().await;
    if !cfg.mcp_tools.iter().any(|t| t.name == tool.name) {
        cfg.mcp_tools.push(tool);
    }
    cfg.save_async().await.map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn remove_mcp_tool(name: String) -> Result<(), String> {
    let mut cfg = AppConfig::load_async().await;
    cfg.mcp_tools.retain(|t| t.name != name);
    cfg.save_async().await.map_err(|e| e.to_string())
}

// ============================================================================
// MCP Discovery Manager - Dynamic Tool Provisioning
// ============================================================================

use gestura_core::{McpDiscoveryManager, McpServerConfig};

/// Global MCP discovery manager instance
static MCP_DISCOVERY_MANAGER: std::sync::OnceLock<McpDiscoveryManager> = std::sync::OnceLock::new();

/// Get or initialize the global MCP discovery manager
fn get_mcp_discovery_manager() -> &'static McpDiscoveryManager {
    MCP_DISCOVERY_MANAGER.get_or_init(McpDiscoveryManager::new)
}

/// MCP tool information for the frontend (matches ToolInfo pattern)
#[derive(serde::Serialize)]
pub struct McpToolInfo {
    /// Full tool name in format "server:tool_name"
    pub name: String,
    /// Tool description from MCP server
    pub summary: String,
    /// Source MCP server name
    pub server_name: String,
    /// Tool category (read, write, execute, etc.)
    pub category: String,
    /// Whether this tool has side effects
    pub has_side_effects: bool,
    /// Risk level (low, medium, high)
    pub risk_level: String,
}

/// Initialize MCP servers from config
/// This registers all configured MCP servers with the discovery manager
#[tauri::command]
pub async fn init_mcp_servers() -> Result<usize, String> {
    let config = AppConfig::load_async().await;
    let manager = get_mcp_discovery_manager();

    let mut registered = 0;
    for mcp_tool in &config.mcp_tools {
        let server_config = McpServerConfig {
            name: mcp_tool.name.clone(),
            uri: mcp_tool.endpoint.clone(),
            enabled: true,
            timeout_secs: 30,
            auto_reconnect: true,
        };
        manager.register_server(server_config);
        registered += 1;
        tracing::info!("Registered MCP server: {}", mcp_tool.name);
    }

    Ok(registered)
}

/// List all discovered tools from MCP servers
/// Returns tools that have been cached from connected MCP servers
#[tauri::command]
pub fn list_discovered_mcp_tools() -> Vec<McpToolInfo> {
    use gestura_core::execution_mode::ToolCategory;

    let manager = get_mcp_discovery_manager();
    let cached_tools = manager.list_tools();

    cached_tools
        .into_iter()
        .map(|ct| {
            let category_str = match ct.metadata.category {
                ToolCategory::ReadOnly => "read",
                ToolCategory::Write => "write",
                ToolCategory::Shell => "shell",
                ToolCategory::Network => "network",
                ToolCategory::System => "system",
                ToolCategory::Git => "git",
            };
            // Convert numeric risk_level (0-10) to string category
            let risk_str = if ct.metadata.risk_level <= 2 {
                "low"
            } else if ct.metadata.risk_level <= 5 {
                "medium"
            } else {
                "high"
            };
            McpToolInfo {
                name: ct.metadata.name.clone(),
                summary: ct.metadata.description.clone(),
                server_name: ct.server_name.clone(),
                category: category_str.to_string(),
                has_side_effects: ct.metadata.has_side_effects,
                risk_level: risk_str.to_string(),
            }
        })
        .collect()
}

/// Get MCP server status information
#[tauri::command]
pub fn get_mcp_server_status() -> Vec<McpServerStatus> {
    let manager = get_mcp_discovery_manager();
    manager
        .list_servers()
        .into_iter()
        .map(|s| McpServerStatus {
            name: s.config.name,
            uri: s.config.uri,
            state: format!("{:?}", s.state),
            tool_count: s.tool_count,
            last_error: s.last_error,
        })
        .collect()
}

/// MCP server status for frontend display
#[derive(serde::Serialize)]
pub struct McpServerStatus {
    pub name: String,
    pub uri: String,
    pub state: String,
    pub tool_count: usize,
    pub last_error: Option<String>,
}

/// Register a new MCP server and initialize discovery
#[tauri::command]
pub async fn register_mcp_server(name: String, endpoint: String) -> Result<(), String> {
    // Add to config
    let tool = crate::config::McpTool {
        name: name.clone(),
        endpoint: endpoint.clone(),
    };
    add_mcp_tool(tool).await?;

    // Register with discovery manager
    let manager = get_mcp_discovery_manager();
    let server_config = McpServerConfig {
        name: name.clone(),
        uri: endpoint,
        enabled: true,
        timeout_secs: 30,
        auto_reconnect: true,
    };
    manager.register_server(server_config);

    tracing::info!("Registered new MCP server: {}", name);
    Ok(())
}

/// Unregister an MCP server
#[tauri::command]
pub async fn unregister_mcp_server(name: String) -> Result<(), String> {
    // Remove from config
    remove_mcp_tool(name.clone()).await?;

    // Unregister from discovery manager
    let manager = get_mcp_discovery_manager();
    manager.unregister_server(&name);

    tracing::info!("Unregistered MCP server: {}", name);
    Ok(())
}

#[tauri::command]
pub async fn get_mdh_pointers() -> Result<std::collections::HashMap<String, String>, String> {
    Ok(AppConfig::load_async().await.mdh_pointers)
}

#[tauri::command]
pub async fn set_mdh_pointer(key: String, value: String) -> Result<(), String> {
    let mut cfg = AppConfig::load_async().await;
    cfg.mdh_pointers.insert(key, value);
    cfg.save_async().await.map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn remove_mdh_pointer(key: String) -> Result<(), String> {
    let mut cfg = AppConfig::load_async().await;
    cfg.mdh_pointers.remove(&key);
    cfg.save_async().await.map_err(|e| e.to_string())
}

// Knowledge Management Commands

/// Add a knowledge entry from chat (saved responses)
#[tauri::command]
pub fn add_knowledge_entry(
    content: String,
    category: String,
    tags: Vec<String>,
) -> Result<String, String> {
    use gestura_core::knowledge::{KnowledgeItem, KnowledgeStore};
    use std::time::{SystemTime, UNIX_EPOCH};

    let store = KnowledgeStore::with_default_dir();

    // Generate a unique ID based on timestamp
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    let id = format!("saved-{}", timestamp);

    // Create a knowledge item from the saved content
    let item = KnowledgeItem::new(
        &id,
        format!("Saved: {}", &content[..content.len().min(50)]),
        &content,
    )
    .with_category(&category)
    .with_triggers(tags)
    .with_content(&content);

    store.register(item);

    tracing::info!("Added knowledge entry: {}", id);
    Ok(id)
}

/// List knowledge entries
#[tauri::command]
pub fn list_knowledge_entries(category: Option<String>) -> Result<Vec<serde_json::Value>, String> {
    use gestura_core::knowledge::{KnowledgeQuery, KnowledgeStore, register_builtin_knowledge};

    let store = KnowledgeStore::with_default_dir();
    register_builtin_knowledge(&store);

    let query = KnowledgeQuery {
        query: String::new(),
        categories: category.map(|c| vec![c]),
        limit: Some(100),
        min_score: None,
    };

    let matches = store.find(&query);
    let entries: Vec<serde_json::Value> = matches
        .into_iter()
        .map(|m| {
            serde_json::json!({
                "id": m.item.id,
                "name": m.item.name,
                "category": m.item.category,
                "description": m.item.description,
                "score": m.score,
            })
        })
        .collect();

    Ok(entries)
}

/// Search knowledge base
#[tauri::command]
pub fn search_knowledge(
    query: String,
    limit: Option<usize>,
) -> Result<Vec<serde_json::Value>, String> {
    use gestura_core::knowledge::{KnowledgeQuery, KnowledgeStore, register_builtin_knowledge};

    let store = KnowledgeStore::with_default_dir();
    register_builtin_knowledge(&store);

    let kquery = KnowledgeQuery {
        query,
        categories: None,
        limit: limit.or(Some(10)),
        min_score: Some(0.1),
    };

    let matches = store.find(&kquery);
    let entries: Vec<serde_json::Value> = matches
        .into_iter()
        .map(|m| {
            serde_json::json!({
                "id": m.item.id,
                "name": m.item.name,
                "category": m.item.category,
                "description": m.item.description,
                "content": m.item.core_content,
                "score": m.score,
                "matched_triggers": m.matched_triggers,
            })
        })
        .collect();

    Ok(entries)
}

#[tauri::command]
pub async fn test_llm(prompt: String) -> Result<String, String> {
    let cfg = AppConfig::load_async().await;
    let provider = select_provider(
        &cfg,
        &AgentContext {
            agent_id: "test".into(),
        },
    );
    provider.call(&prompt).await.map_err(|e| format!("{e}"))
}

/// Enhance a user prompt using LLM to make it more effective
///
/// This command takes a user's prompt and uses the configured LLM provider
/// to improve it by adding context, structure, and clarity while preserving
/// the original intent.
///
/// # Arguments
///
/// * `prompt` - The original user prompt to enhance
/// * `session_id` - Optional session ID to include conversation history as context
///
/// # Returns
///
/// Returns the enhanced prompt as a String, or an error message if enhancement fails.
///
/// Note: This command uses `snake_case` argument names for JS↔Rust interop.
#[tauri::command(rename_all = "snake_case")]
pub async fn enhance_prompt(prompt: String, session_id: Option<String>) -> Result<String, String> {
    use gestura_core::prompt_enhancement::{PromptContext, enhance_prompt_with_llm};

    // Validate input
    if prompt.trim().is_empty() {
        return Err("Prompt cannot be empty".to_string());
    }

    // Load config
    let mut cfg = AppConfig::load_async().await;

    // Build context from session history if session_id provided
    let context = if let Some(ref sid) = session_id {
        // Get session state using public API
        if let Some(state) = crate::window_manager::get_session_state(sid) {
            // Get last 5 messages for context (to avoid token overflow)
            let history: Vec<(String, String)> = state
                .messages
                .iter()
                .rev()
                .take(5)
                .rev()
                .map(|msg| (msg.role.clone(), msg.content.clone()))
                .collect();

            if !history.is_empty() {
                tracing::debug!(
                    session_id = %sid,
                    history_count = history.len(),
                    "Including session history in prompt enhancement"
                );
                Some(PromptContext::new().with_session_history(history))
            } else {
                None
            }
        } else {
            tracing::warn!(session_id = %sid, "Session not found for prompt enhancement");
            None
        }
    } else {
        None
    };

    // Apply session-specific LLM config overrides so the prompt enhancer uses the same
    // effective provider/model as chat for this session.
    if let Some(ref sid) = session_id {
        apply_session_llm_config_overrides(&mut cfg, sid);
    }

    tracing::info!(
        prompt_length = prompt.len(),
        has_context = context.is_some(),
        "Enhancing user prompt"
    );

    let enhanced = enhance_prompt_with_llm(&prompt, &cfg, context)
        .await
        .map_err(|e| format!("Enhancement failed: {}", e))?;

    tracing::info!(
        original_length = prompt.len(),
        enhanced_length = enhanced.len(),
        "Prompt enhancement successful"
    );

    Ok(enhanced)
}

#[tauri::command]
pub async fn test_voice() -> Result<String, String> {
    let cfg = AppConfig::load_async().await;
    let engine = crate::voice_select::select_voice(&cfg);
    let name = engine.engine_name();
    let sample = engine.process_command(&cfg, None).await.unwrap_or_default();
    Ok(format!("engine={name} sample={sample}"))
}

/// Test Ollama connection and return server info
#[tauri::command]
pub async fn test_ollama_connection(endpoint: String) -> Result<serde_json::Value, String> {
    let client = reqwest::Client::new();
    let url = format!("{}/api/version", endpoint.trim_end_matches('/'));

    let resp = client
        .get(&url)
        .timeout(std::time::Duration::from_secs(5))
        .send()
        .await
        .map_err(|e| format!("Connection failed: {}", e))?;

    if !resp.status().is_success() {
        return Err(format!("Server returned error: {}", resp.status()));
    }

    let version: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| format!("Invalid response: {}", e))?;

    Ok(serde_json::json!({
        "connected": true,
        "version": version.get("version").and_then(|v| v.as_str()).unwrap_or("unknown")
    }))
}

/// List available Ollama models.
///
/// Note: This command uses `snake_case` argument names for JS↔Rust interop.
#[tauri::command(rename_all = "snake_case")]
pub async fn list_ollama_models(endpoint: String) -> Result<Vec<serde_json::Value>, String> {
    let client = reqwest::Client::new();
    let url = format!("{}/api/tags", endpoint.trim_end_matches('/'));

    let resp = client
        .get(&url)
        .timeout(std::time::Duration::from_secs(10))
        .send()
        .await
        .map_err(|e| format!("Failed to list models: {}", e))?;

    if !resp.status().is_success() {
        return Err(format!("Server returned error: {}", resp.status()));
    }

    let data: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| format!("Invalid response: {}", e))?;

    let models = data
        .get("models")
        .and_then(|m| m.as_array())
        .map(|arr| {
            arr.iter()
                .map(|m| {
                    serde_json::json!({
                        "name": m.get("name").and_then(|n| n.as_str()).unwrap_or("unknown"),
                        "size": m.get("size").and_then(|s| s.as_u64()).unwrap_or(0),
                        "modified_at": m.get("modified_at").and_then(|d| d.as_str()).unwrap_or("")
                    })
                })
                .collect()
        })
        .unwrap_or_default();

    Ok(models)
}

/// List available OpenAI models.
///
/// Note: This command uses `snake_case` argument names for JS↔Rust interop.
#[tauri::command(rename_all = "snake_case")]
pub async fn list_openai_models(api_key: String) -> Result<Vec<serde_json::Value>, String> {
    let client = reqwest::Client::new();
    let url = "https://api.openai.com/v1/models";

    let resp = client
        .get(url)
        .bearer_auth(&api_key)
        .timeout(std::time::Duration::from_secs(10))
        .send()
        .await
        .map_err(|e| format!("Failed to list OpenAI models: {}", e))?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(format!("OpenAI API error {}: {}", status, body));
    }

    let data: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| format!("Invalid response: {}", e))?;

    // Filter to only chat models (gpt-*) and sort by name
    let models: Vec<serde_json::Value> = data
        .get("data")
        .and_then(|d| d.as_array())
        .map(|arr| {
            let mut models: Vec<serde_json::Value> = arr
                .iter()
                .filter_map(|m| {
                    let id = m.get("id")?.as_str()?;
                    // Only include GPT chat models, exclude embeddings, whisper, tts, dall-e, etc.
                    if id.starts_with("gpt-") && !id.contains("instruct") {
                        Some(serde_json::json!({
                            "id": id,
                            "name": format_openai_model_name(id),
                            "created": m.get("created").and_then(|c| c.as_i64()).unwrap_or(0)
                        }))
                    } else {
                        None
                    }
                })
                .collect();
            // Sort by created date descending (newest first)
            models.sort_by(|a, b| {
                let a_created = a.get("created").and_then(|c| c.as_i64()).unwrap_or(0);
                let b_created = b.get("created").and_then(|c| c.as_i64()).unwrap_or(0);
                b_created.cmp(&a_created)
            });
            models
        })
        .unwrap_or_default();

    Ok(models)
}

/// List available OpenAI STT (Speech-to-Text) models
/// Fetches from /v1/models and filters for transcription-capable models
#[tauri::command(rename_all = "snake_case")]
pub async fn list_openai_stt_models(api_key: String) -> Result<Vec<serde_json::Value>, String> {
    if api_key.is_empty() {
        // Return static list with sensible defaults when no API key
        return Ok(get_static_openai_stt_models());
    }

    let client = reqwest::Client::new();
    let url = "https://api.openai.com/v1/models";

    let resp = client
        .get(url)
        .bearer_auth(&api_key)
        .timeout(std::time::Duration::from_secs(10))
        .send()
        .await
        .map_err(|e| format!("Failed to list OpenAI models: {}", e))?;

    if !resp.status().is_success() {
        tracing::warn!(
            "OpenAI API returned status {}, falling back to static STT model list",
            resp.status()
        );
        return Ok(get_static_openai_stt_models());
    }

    let data: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| format!("Invalid response: {}", e))?;

    // Filter to only STT/transcription models
    let mut models: Vec<serde_json::Value> = data
        .get("data")
        .and_then(|d| d.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|m| {
                    let id = m.get("id")?.as_str()?;
                    // Include whisper and transcribe models
                    if id.contains("whisper") || id.contains("transcribe") {
                        Some(serde_json::json!({
                            "id": id,
                            "name": format_openai_stt_model_name(id),
                            "created": m.get("created").and_then(|c| c.as_i64()).unwrap_or(0)
                        }))
                    } else {
                        None
                    }
                })
                .collect()
        })
        .unwrap_or_default();

    // Sort: prefer newer models (gpt-4o-transcribe) first, then whisper-1
    models.sort_by(|a, b| {
        let a_id = a.get("id").and_then(|i| i.as_str()).unwrap_or("");
        let b_id = b.get("id").and_then(|i| i.as_str()).unwrap_or("");
        // Prioritize gpt-4o-transcribe models over whisper
        let a_priority = if a_id.contains("gpt-4o") { 0 } else { 1 };
        let b_priority = if b_id.contains("gpt-4o") { 0 } else { 1 };
        a_priority.cmp(&b_priority).then_with(|| a_id.cmp(b_id))
    });

    // If no models found from API, return static list
    if models.is_empty() {
        return Ok(get_static_openai_stt_models());
    }

    Ok(models)
}

/// Static list of OpenAI STT models (fallback when API unavailable)
fn get_static_openai_stt_models() -> Vec<serde_json::Value> {
    vec![
        serde_json::json!({
            "id": "gpt-4o-transcribe",
            "name": "GPT-4o Transcribe (Best Quality)",
            "description": "Highest accuracy, lower WER than Whisper"
        }),
        serde_json::json!({
            "id": "gpt-4o-mini-transcribe",
            "name": "GPT-4o Mini Transcribe (Balanced)",
            "description": "Good balance of cost and quality"
        }),
        serde_json::json!({
            "id": "whisper-1",
            "name": "Whisper V2 (Classic)",
            "description": "Original Whisper model, cost-effective"
        }),
    ]
}

/// Format OpenAI STT model ID to a human-readable name
fn format_openai_stt_model_name(id: &str) -> String {
    match id {
        "whisper-1" => "Whisper V2 (Classic)".to_string(),
        "gpt-4o-transcribe" => "GPT-4o Transcribe (Best Quality)".to_string(),
        "gpt-4o-transcribe-latest" => "GPT-4o Transcribe (Latest)".to_string(),
        "gpt-4o-mini-transcribe" => "GPT-4o Mini Transcribe (Balanced)".to_string(),
        "gpt-4o-transcribe-diarize" => "GPT-4o Transcribe + Diarization".to_string(),
        _ => {
            // Convert kebab-case to Title Case
            id.split('-')
                .map(|part| {
                    let mut chars = part.chars();
                    match chars.next() {
                        None => String::new(),
                        Some(first) => first.to_uppercase().chain(chars).collect(),
                    }
                })
                .collect::<Vec<_>>()
                .join(" ")
        }
    }
}

/// Format OpenAI model ID to a human-readable name
fn format_openai_model_name(id: &str) -> String {
    match id {
        "gpt-4o" => "GPT-4o".to_string(),
        "gpt-4o-mini" => "GPT-4o Mini".to_string(),
        "gpt-4-turbo" => "GPT-4 Turbo".to_string(),
        "gpt-4-turbo-preview" => "GPT-4 Turbo Preview".to_string(),
        "gpt-4" => "GPT-4".to_string(),
        "gpt-3.5-turbo" => "GPT-3.5 Turbo".to_string(),
        _ => {
            // Convert kebab-case to Title Case
            id.split('-')
                .map(|part| {
                    let mut chars = part.chars();
                    match chars.next() {
                        None => String::new(),
                        Some(first) => first.to_uppercase().chain(chars).collect(),
                    }
                })
                .collect::<Vec<_>>()
                .join(" ")
        }
    }
}

/// List available Anthropic models.
///
/// Note: This command uses `snake_case` argument names for JS↔Rust interop.
#[tauri::command(rename_all = "snake_case")]
pub async fn list_anthropic_models(api_key: String) -> Result<Vec<serde_json::Value>, String> {
    let client = reqwest::Client::new();
    let url = "https://api.anthropic.com/v1/models";

    let resp = client
        .get(url)
        .header("x-api-key", &api_key)
        .header("anthropic-version", "2023-06-01")
        .timeout(std::time::Duration::from_secs(10))
        .send()
        .await
        .map_err(|e| format!("Failed to list Anthropic models: {}", e))?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(format!("Anthropic API error {}: {}", status, body));
    }

    let data: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| format!("Invalid response: {}", e))?;

    // Parse the models list
    let models: Vec<serde_json::Value> = data
        .get("data")
        .and_then(|d| d.as_array())
        .map(|arr| {
            let mut models: Vec<serde_json::Value> = arr
                .iter()
                .filter_map(|m| {
                    let id = m.get("id")?.as_str()?;
                    // Only include Claude chat models
                    if id.starts_with("claude-") {
                        Some(serde_json::json!({
                            "id": id,
                            "name": format_anthropic_model_name(id),
                            "created": m.get("created_at").and_then(|c| c.as_str()).unwrap_or("")
                        }))
                    } else {
                        None
                    }
                })
                .collect();
            // Sort alphabetically by name for consistency
            models.sort_by(|a, b| {
                let a_name = a.get("name").and_then(|n| n.as_str()).unwrap_or("");
                let b_name = b.get("name").and_then(|n| n.as_str()).unwrap_or("");
                a_name.cmp(b_name)
            });
            models
        })
        .unwrap_or_default();

    Ok(models)
}

/// Format Anthropic model ID to a human-readable name
fn format_anthropic_model_name(id: &str) -> String {
    match id {
        "claude-sonnet-4-20250514" => "Claude Sonnet 4".to_string(),
        "claude-3-5-sonnet-20241022" => "Claude 3.5 Sonnet".to_string(),
        "claude-3-opus-20240229" => "Claude 3 Opus".to_string(),
        "claude-3-sonnet-20240229" => "Claude 3 Sonnet".to_string(),
        "claude-3-haiku-20240307" => "Claude 3 Haiku".to_string(),
        _ => {
            // Try to parse the model name from the ID
            // Format: claude-{version}-{variant}-{date}
            let parts: Vec<&str> = id.split('-').collect();
            if parts.len() >= 3 {
                let version = parts[1];
                let variant = parts[2];
                format!(
                    "Claude {} {}",
                    version,
                    variant
                        .chars()
                        .next()
                        .map(|c| c.to_uppercase().to_string() + &variant[1..])
                        .unwrap_or_else(|| variant.to_string())
                )
            } else {
                id.to_string()
            }
        }
    }
}

/// Fetch available Grok models from xAI API
/// API Reference: https://docs.x.ai/docs/api-reference#list-models
///
/// Note: This command uses `snake_case` argument names for JS↔Rust interop.
#[tauri::command(rename_all = "snake_case")]
pub async fn list_grok_models(api_key: String) -> Result<Vec<serde_json::Value>, String> {
    if api_key.is_empty() {
        return Ok(get_static_grok_models());
    }

    let client = reqwest::Client::new();
    let response = client
        .get("https://api.x.ai/v1/models")
        .header("Authorization", format!("Bearer {}", api_key))
        .header("Content-Type", "application/json")
        .send()
        .await
        .map_err(|e| format!("Failed to fetch Grok models: {}", e))?;

    if !response.status().is_success() {
        tracing::warn!(
            "Grok API returned status {}, falling back to static list",
            response.status()
        );
        return Ok(get_static_grok_models());
    }

    let data: serde_json::Value = response
        .json()
        .await
        .map_err(|e| format!("Failed to parse Grok models response: {}", e))?;

    let models: Vec<serde_json::Value> = data["data"]
        .as_array()
        .unwrap_or(&vec![])
        .iter()
        .filter_map(|model| {
            let id = model["id"].as_str()?;
            // Filter to chat-capable models (exclude image-only models)
            if id.contains("image") {
                return None;
            }
            Some(serde_json::json!({
                "id": id,
                "name": format_grok_model_name(id)
            }))
        })
        .collect();

    if models.is_empty() {
        Ok(get_static_grok_models())
    } else {
        Ok(models)
    }
}

/// Format Grok model ID to human-readable name
fn format_grok_model_name(id: &str) -> String {
    // grok-4-0709 -> Grok 4 (0709)
    // grok-3-mini -> Grok 3 Mini
    let parts: Vec<&str> = id.split('-').collect();
    let mut name = String::new();

    for (i, part) in parts.iter().enumerate() {
        if i == 0 {
            name.push_str(&part.to_uppercase().replace("GROK", "Grok"));
        } else if part.chars().all(|c| c.is_numeric()) {
            if i == 1 {
                name.push_str(&format!(" {}", part));
            } else {
                name.push_str(&format!(" ({})", part));
            }
        } else {
            let formatted = match *part {
                "mini" => "Mini",
                "fast" => "Fast",
                "vision" => "Vision",
                "code" => "Code",
                _ => part,
            };
            name.push_str(&format!(" {}", formatted));
        }
    }

    name.trim().to_string()
}

/// Fallback static list of Grok models
fn get_static_grok_models() -> Vec<serde_json::Value> {
    vec![
        serde_json::json!({ "id": "grok-3", "name": "Grok 3" }),
        serde_json::json!({ "id": "grok-3-mini", "name": "Grok 3 Mini" }),
        serde_json::json!({ "id": "grok-2-vision-1212", "name": "Grok 2 Vision" }),
    ]
}

/// Test local Whisper model with detailed validation
#[tauri::command(rename_all = "snake_case")]
pub async fn test_local_whisper(model_path: String) -> Result<String, String> {
    use crate::voice::validate_whisper_model;
    use std::path::Path;

    let path = Path::new(&model_path);
    let validation = validate_whisper_model(path);

    if !validation.is_valid {
        return Err(validation
            .error
            .unwrap_or_else(|| "Unknown error".to_string()));
    }

    Ok(format!(
        "Model valid: {} ({:.1} MB, GGML format)",
        path.file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("unknown"),
        validation.file_size_mb
    ))
}

/// Validate a Whisper model file and return structured validation info
#[tauri::command]
pub async fn validate_whisper_model(
    path: String,
) -> Result<crate::voice::WhisperModelValidation, String> {
    use std::path::Path;

    let path = Path::new(&path);
    Ok(crate::voice::validate_whisper_model(path))
}

/// Get available Whisper models for download
#[tauri::command]
pub fn get_whisper_models() -> Vec<crate::config::WhisperModelInfo> {
    crate::config::WhisperModelInfo::available_models()
}

/// Check if a specific Whisper model file is already downloaded
#[tauri::command(rename_all = "snake_case")]
pub fn is_whisper_model_downloaded(model_filename: String) -> Result<serde_json::Value, String> {
    let models_dir = crate::config::AppConfig::whisper_models_dir();
    let model_path = models_dir.join(&model_filename);
    let exists = model_path.exists();

    let validation = if exists {
        Some(crate::voice::validate_whisper_model(&model_path))
    } else {
        None
    };

    Ok(serde_json::json!({
        "exists": exists,
        "path": model_path.to_string_lossy().to_string(),
        "is_valid": validation.as_ref().map(|v| v.is_valid).unwrap_or(false),
        "validation": validation
    }))
}

/// Get the default Whisper model path and status
#[tauri::command]
pub fn get_whisper_model_status() -> Result<serde_json::Value, String> {
    let (exists, path) = crate::voice::get_default_model_status();
    let path_str = path.to_string_lossy().to_string();

    let validation = if exists {
        Some(crate::voice::validate_whisper_model(&path))
    } else {
        None
    };

    Ok(serde_json::json!({
        "default_path": path_str,
        "exists": exists,
        "validation": validation,
        "models_dir": crate::config::AppConfig::whisper_models_dir().to_string_lossy()
    }))
}

/// Download a Whisper model from HuggingFace
/// Returns progress updates via Tauri events
#[tauri::command(rename_all = "snake_case")]
pub async fn download_whisper_model(
    app: tauri::AppHandle,
    model_filename: String,
) -> Result<String, String> {
    use std::io::Write;
    use tauri::Emitter;

    tracing::info!(
        "Whisper download command invoked for model filename: {}",
        model_filename
    );

    // Find the model info
    let model_info = crate::config::WhisperModelInfo::find_by_filename(&model_filename)
        .ok_or_else(|| format!("Unknown model: {}", model_filename))?;

    // Create the models directory
    let models_dir = crate::config::AppConfig::whisper_models_dir();
    std::fs::create_dir_all(&models_dir)
        .map_err(|e| format!("Failed to create models directory: {}", e))?;

    let output_path = models_dir.join(&model_filename);

    tracing::info!(
        "Preparing to download Whisper model '{}' (filename='{}') to {:?}",
        model_info.name,
        model_info.filename,
        output_path
    );

    // Check if already downloaded
    if output_path.exists() {
        let validation = crate::voice::validate_whisper_model(&output_path);
        if validation.is_valid {
            return Ok(format!(
                "Model already downloaded: {}",
                output_path.to_string_lossy()
            ));
        }
        // Remove invalid file
        std::fs::remove_file(&output_path).ok();
    }

    tracing::info!(
        "Downloading Whisper model: {} from {}",
        model_filename,
        model_info.url
    );

    // Emit start event
    let _ = app.emit(
        "whisper-download-progress",
        serde_json::json!({
            "status": "starting",
            "filename": model_filename,
            "total_mb": model_info.size_mb,
            "downloaded_mb": 0,
            "percent": 0
        }),
    );

    // Download the model with proper timeout and User-Agent
    // Hugging Face CDN requires a User-Agent header and may need time for large files
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(1800)) // 30 minute timeout for large models
        .user_agent("Gestura/0.2.0 (https://gestura.ai)")
        .build()
        .map_err(|e| format!("Failed to create HTTP client: {}", e))?;

    tracing::info!("Starting HTTP request to: {}", model_info.url);

    let response = client.get(&model_info.url).send().await.map_err(|e| {
        tracing::error!("HTTP request failed: {}", e);
        format!("Failed to start download: {}", e)
    })?;

    tracing::info!(
        "HTTP response received: status={}, content_length={:?}",
        response.status(),
        response.content_length()
    );

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        tracing::error!("Download failed with HTTP {}: {}", status, body);
        let _ = app.emit(
            "whisper-download-progress",
            serde_json::json!({
                "status": "error",
                "filename": model_filename,
                "error": format!("HTTP {}", status)
            }),
        );
        return Err(format!("Download failed: HTTP {} - {}", status, body));
    }

    let total_size = response
        .content_length()
        .unwrap_or(model_info.size_mb * 1024 * 1024);

    tracing::info!(
        "Starting streaming download: total_size={} bytes ({:.1} MB)",
        total_size,
        total_size as f64 / (1024.0 * 1024.0)
    );

    // Create temp file for download
    let temp_path = output_path.with_extension("tmp");
    let mut file = std::fs::File::create(&temp_path).map_err(|e| {
        tracing::error!("Failed to create temp file {:?}: {}", temp_path, e);
        format!("Failed to create temp file: {}", e)
    })?;

    let mut downloaded: u64 = 0;
    let mut last_percent: u64 = 0;

    // Stream the response
    let mut stream = response.bytes_stream();
    use futures_util::StreamExt;

    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| {
            tracing::error!("Download stream error after {} bytes: {}", downloaded, e);
            let _ = app.emit(
                "whisper-download-progress",
                serde_json::json!({
                    "status": "error",
                    "filename": model_filename,
                    "error": format!("Stream error: {}", e)
                }),
            );
            format!("Download error: {}", e)
        })?;
        file.write_all(&chunk).map_err(|e| {
            tracing::error!("Failed to write chunk to file: {}", e);
            format!("Failed to write file: {}", e)
        })?;

        downloaded += chunk.len() as u64;
        let percent = (downloaded * 100) / total_size;

        // Emit progress every 1%
        if percent > last_percent {
            last_percent = percent;
            let downloaded_mb = downloaded as f64 / (1024.0 * 1024.0);
            let _ = app.emit(
                "whisper-download-progress",
                serde_json::json!({
                    "status": "downloading",
                    "filename": model_filename,
                    "total_mb": model_info.size_mb,
                    "downloaded_mb": downloaded_mb,
                    "percent": percent
                }),
            );
        }
    }

    tracing::info!(
        "Download complete: {} bytes written to {:?}",
        downloaded,
        temp_path
    );

    // Rename temp file to final path
    std::fs::rename(&temp_path, &output_path)
        .map_err(|e| format!("Failed to save model file: {}", e))?;

    // Validate the downloaded model
    let validation = crate::voice::validate_whisper_model(&output_path);
    if !validation.is_valid {
        std::fs::remove_file(&output_path).ok();
        return Err(format!(
            "Downloaded file is invalid: {}",
            validation
                .error
                .unwrap_or_else(|| "Unknown error".to_string())
        ));
    }

    // Emit completion event
    let _ = app.emit(
        "whisper-download-progress",
        serde_json::json!({
            "status": "complete",
            "filename": model_filename,
            "path": output_path.to_string_lossy(),
            "percent": 100
        }),
    );

    // Update config with the new model path
    let mut config = AppConfig::load_async().await;
    config.voice.local_model_path = Some(output_path.to_string_lossy().to_string());
    config
        .save_async()
        .await
        .map_err(|e| format!("Failed to save config: {}", e))?;

    tracing::info!(
        "Whisper model downloaded successfully: {}",
        output_path.to_string_lossy()
    );

    Ok(output_path.to_string_lossy().to_string())
}

#[tauri::command]
pub async fn get_ui_prefs() -> Result<crate::config::UiSettings, String> {
    Ok(AppConfig::load_async().await.ui)
}

#[tauri::command]
pub async fn set_ui_prefs(ui: crate::config::UiSettings) -> Result<(), String> {
    let mut cfg = AppConfig::load_async().await;
    cfg.ui = ui;
    cfg.save_async().await.map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn run_voice_once() -> Result<String, String> {
    let cfg = AppConfig::load_async().await;
    let engine = crate::voice_select::select_voice(&cfg);
    crate::voice_select::validate_voice_config_for_run(&cfg, engine.as_ref())
        .map_err(|e| e.to_string())?;
    let text = engine
        .process_command(&cfg, None)
        .await
        .map_err(|e| e.to_string())?;
    Ok(text)
}

/// Scan for available Haptic Harmony rings
#[tauri::command]
pub async fn scan_for_rings() -> Result<Vec<String>, String> {
    let ring_manager = crate::ble::create_ring_manager();
    ring_manager
        .scan_for_rings()
        .await
        .map_err(|e| e.to_string())
}

/// Get ring status by device ID
#[tauri::command(rename_all = "snake_case")]
pub async fn get_ring_status(device_id: String) -> Result<Option<crate::ble::RingStatus>, String> {
    let ring_manager = crate::ble::create_ring_manager();
    ring_manager
        .get_ring_status(&device_id)
        .await
        .map_err(|e| e.to_string())
}

/// Pair with a ring
#[tauri::command(rename_all = "snake_case")]
pub async fn pair_ring(device_id: String) -> Result<(), String> {
    let ring_manager = crate::ble::create_ring_manager();
    ring_manager
        .pair_ring(&device_id)
        .await
        .map_err(|e| e.to_string())
}

/// Send haptic feedback to ring
#[tauri::command(rename_all = "snake_case")]
pub async fn send_haptic_feedback(
    device_id: String,
    pattern: String,
    intensity: f32,
    duration_ms: u32,
) -> Result<(), String> {
    let haptic_pattern = match pattern.as_str() {
        "click" => crate::haptics::HapticPattern::Click,
        "pulse" => crate::haptics::HapticPattern::Pulse,
        "ramp" => crate::haptics::HapticPattern::Ramp,
        _ => return Err("Invalid haptic pattern".to_string()),
    };

    let request = crate::haptics::HapticRequest {
        pattern: haptic_pattern,
        intensity,
        duration_ms,
        repeat_count: 0,
        repeat_delay_ms: 0,
    };

    let ring_manager = crate::ble::create_ring_manager();
    ring_manager
        .send_haptic(&device_id, request)
        .await
        .map_err(|e| e.to_string())
}

/// Start gesture monitoring for a ring
#[tauri::command(rename_all = "snake_case")]
pub async fn start_gesture_monitoring(device_id: String) -> Result<(), String> {
    let ring_manager = crate::ble::create_ring_manager();
    let (event_tx, _) = tokio::sync::broadcast::channel(100);
    ring_manager
        .start_gesture_monitoring(&device_id, event_tx)
        .await
        .map_err(|e| e.to_string())
}

/// Stop gesture monitoring for a ring
#[tauri::command(rename_all = "snake_case")]
pub async fn stop_gesture_monitoring(device_id: String) -> Result<(), String> {
    let ring_manager = crate::ble::create_ring_manager();
    ring_manager
        .stop_gesture_monitoring(&device_id)
        .await
        .map_err(|e| e.to_string())
}

/// Get current NATS connection status.
///
/// Returns `true` if the app has an active NATS connection, otherwise `false`.
#[tauri::command]
pub fn get_nats_status(state: tauri::State<'_, crate::AppState>) -> Result<bool, String> {
    Ok(state.nats.is_some())
}

/// Get system health status
#[tauri::command]
pub async fn get_system_health() -> Result<serde_json::Value, String> {
    let telemetry = crate::telemetry::get_telemetry_manager().await;
    let health = telemetry.get_system_health().await;
    serde_json::to_value(health).map_err(|e| e.to_string())
}

/// Get telemetry metrics summary
#[tauri::command]
pub async fn get_metrics_summary() -> Result<serde_json::Value, String> {
    let telemetry = crate::telemetry::get_telemetry_manager().await;
    Ok(telemetry.get_metrics_summary().await)
}

/// Get recent telemetry metrics
#[tauri::command]
pub async fn get_recent_metrics(
    limit: Option<usize>,
) -> Result<Vec<crate::telemetry::Metric>, String> {
    let telemetry = crate::telemetry::get_telemetry_manager().await;
    Ok(telemetry.get_recent_metrics(limit.unwrap_or(100)).await)
}

/// Clear telemetry metrics
#[tauri::command]
pub async fn clear_metrics() -> Result<(), String> {
    let telemetry = crate::telemetry::get_telemetry_manager().await;
    telemetry.clear_metrics().await;
    Ok(())
}

/// Export user data (GDPR compliance)
#[tauri::command]
pub async fn export_user_data(user_id: String) -> Result<serde_json::Value, String> {
    let gdpr = crate::gdpr::get_gdpr_manager().await;
    gdpr.export_user_data(&user_id)
        .await
        .map_err(|e| e.to_string())
}

/// Delete user data (GDPR compliance)
#[tauri::command]
pub async fn delete_user_data(
    user_id: String,
    verify: Option<bool>,
) -> Result<Vec<String>, String> {
    let gdpr = crate::gdpr::get_gdpr_manager().await;
    gdpr.delete_user_data(&user_id, verify.unwrap_or(false))
        .await
        .map_err(|e| e.to_string())
}

/// Get user consent status
#[tauri::command]
pub async fn get_user_consents(user_id: String) -> Result<Vec<crate::gdpr::ConsentRecord>, String> {
    let gdpr = crate::gdpr::get_gdpr_manager().await;
    Ok(gdpr.get_user_consents(&user_id).await)
}

/// Register user consent
#[tauri::command]
pub async fn register_consent(
    user_id: String,
    category: String,
    purpose: String,
) -> Result<(), String> {
    let gdpr = crate::gdpr::get_gdpr_manager().await;
    let data_category = match category.as_str() {
        "voice" => crate::gdpr::DataCategory::VoiceRecordings,
        "biometric" => crate::gdpr::DataCategory::BiometricData,
        "device" => crate::gdpr::DataCategory::DeviceData,
        "usage" => crate::gdpr::DataCategory::UsageAnalytics,
        "config" => crate::gdpr::DataCategory::ConfigurationData,
        _ => return Err("Invalid data category".to_string()),
    };

    gdpr.register_consent(user_id, data_category, purpose, "User consent".to_string())
        .await
        .map_err(|e| e.to_string())
}

// Chat and Agent Commands

/// Process a chat message through the configured LLM provider
#[tauri::command]
pub async fn process_chat_message(
    app: tauri::AppHandle,
    message: String,
) -> Result<String, String> {
    use crate::notifications::{NotificationType, get_notification_manager};

    let cfg = AppConfig::load_async().await;
    let provider = select_provider(
        &cfg,
        &AgentContext {
            agent_id: "chat".into(),
        },
    );

    tracing::info!(
        "Processing chat message through LLM provider: {}",
        cfg.llm.primary
    );

    // Call the LLM provider with the user message
    let response = provider.call(&message).await;

    match &response {
        Ok(resp) => {
            tracing::info!("LLM response received: {} chars", resp.len());
            // Send completion notification
            get_notification_manager()
                .notify(NotificationType::ResponseComplete, Some(&app))
                .await;
        }
        Err(e) => {
            tracing::error!("LLM error: {}", e);
            // Send error notification
            get_notification_manager()
                .notify(NotificationType::Error, Some(&app))
                .await;
        }
    }

    response.map_err(|e| format!("LLM error: {}", e))
}

/// Cancellation token key used when a chat stream is not associated with a session.
///
/// This primarily supports the legacy/non-session chat surfaces (e.g., single-window UI).
const GLOBAL_STREAM_CANCEL_KEY: &str = "__global_stream__";

/// Per-session cancellation tokens for streaming requests.
///
/// Why: a single global token can cancel the wrong stream when multiple sessions
/// are running concurrently.
static STREAMING_CANCEL_TOKENS: std::sync::OnceLock<
    std::sync::Mutex<std::collections::HashMap<String, gestura_core::CancellationToken>>,
> = std::sync::OnceLock::new();

/// Get the global store of active streaming cancellation tokens.
///
/// The map is keyed by a per-window cancel key derived from the calling webview's
/// window label (e.g., `window:chat-<uuid>`).
///
/// Why: keying by session alone can cause cross-window cancellation when multiple
/// windows exist or when session inference falls back incorrectly.
fn get_cancel_token_store()
-> &'static std::sync::Mutex<std::collections::HashMap<String, gestura_core::CancellationToken>> {
    STREAMING_CANCEL_TOKENS.get_or_init(|| std::sync::Mutex::new(std::collections::HashMap::new()))
}

/// Build the cancellation token key for a particular window label.
///
/// This intentionally scopes cancellation to a single window so concurrent streams
/// in different chat windows do not cancel each other.
fn cancel_key_for_window_label(window_label: &str) -> String {
    format!("window:{window_label}")
}

/// Process a chat message with streaming response
///
/// Emits `chat-stream-chunk` events with partial content and `chat-stream-done` when complete.
///
/// The optional `source` argument can be used to hint how the message was produced:
/// - `"voice"` for transcribed speech
/// - `"text"` for typed input (default)
///
/// Note: This command uses `snake_case` argument names for JS↔Rust interop.
#[tauri::command(rename_all = "snake_case")]
pub async fn process_chat_message_streaming(
    webview_window: tauri::WebviewWindow,
    app: tauri::AppHandle,
    message: String,
    session_id: Option<String>,
    source: Option<String>,
) -> Result<(), String> {
    use gestura_core::{CancellationToken, StreamChunk};
    use gestura_core::{render_capabilities, render_tool_detail, render_tools_overview};
    use tokio::sync::mpsc;

    let mut cfg = AppConfig::load_async().await;

    // Log initial config state
    tracing::debug!(
        global_provider = %cfg.llm.primary,
        session_id = ?session_id,
        "Starting chat message processing"
    );

    // Apply session-specific LLM config overrides (doesn't modify persisted global config)
    if let Some(ref sid) = session_id {
        apply_session_llm_config_overrides(&mut cfg, sid);
    }

    tracing::info!(
        final_provider = %cfg.llm.primary,
        "Processing chat message with LLM provider"
    );

    let message_source = match source.as_deref().map(|s| s.trim().to_ascii_lowercase()) {
        Some(s) if s == "voice" => crate::window_manager::MessageSource::Voice,
        _ => crate::window_manager::MessageSource::Text,
    };
    let request_source = match message_source {
        crate::window_manager::MessageSource::Voice => gestura_core::RequestSource::GuiVoice,
        _ => gestura_core::RequestSource::GuiText,
    };

    // --- Secure window/session isolation ---
    //
    // We always emit streaming events to a single target window. We never broadcast
    // (`app.emit`) because that can leak content across chat windows.
    let calling_window_label = webview_window.label().to_string();
    let calling_session_id =
        crate::window_manager::get_session_id_for_window_label(&calling_window_label);

    // Defense-in-depth: if the caller is a chat window with a known session, do not
    // allow it to stream into a different session by passing a mismatched session id.
    match (&calling_session_id, &session_id) {
        (Some(calling_sid), Some(request_sid)) if calling_sid != request_sid => {
            return Err(format!(
                "Session mismatch for window '{}': caller session '{}' != requested session '{}'",
                calling_window_label, calling_sid, request_sid
            ));
        }
        _ => {}
    }

    // Resolve session id (typed input: use the calling window; voice: optionally route to active chat).
    let resolved_session_id = session_id
        .or_else(|| calling_session_id.clone())
        .or_else(|| {
            if matches!(message_source, crate::window_manager::MessageSource::Voice) {
                crate::window_manager::get_active_chat_for_voice()
            } else {
                None
            }
        });

    // Choose the window to receive stream events.
    // - Text: the calling window.
    // - Voice: the resolved active chat window if available; otherwise the calling window.
    let target_window_label =
        if matches!(message_source, crate::window_manager::MessageSource::Voice) {
            resolved_session_id
                .as_deref()
                .and_then(crate::window_manager::get_session_window_label)
                .unwrap_or_else(|| calling_window_label.clone())
        } else {
            calling_window_label.clone()
        };

    // Centralized, window-scoped emission (never broadcast): emits via `emit_to` and
    // attaches `session_id` for frontend filtering.
    let emit = |event: &str, payload: serde_json::Value| {
        let payload =
            crate::chat_events::attach_session_id(payload, resolved_session_id.as_deref());
        if let Err(err) = crate::chat_events::emit_chat_event_to_window(
            &app,
            &target_window_label,
            &calling_window_label,
            event,
            &payload,
            resolved_session_id.as_deref(),
        ) {
            tracing::error!(
                event = %event,
                target_window_label = %target_window_label,
                calling_window_label = %calling_window_label,
                error = %err,
                "Failed to emit chat event"
            );
        }
    };

    // Check if this is a tools/capabilities/summarize/memory command (explicit slash command) and handle it locally without LLM
    // Natural language questions like "what tools do you have?" should go through the LLM for dynamic, session-aware responses
    let trimmed = message.trim();
    const LOCAL_STREAM_CHUNK_CHARS: usize = 64;
    let is_tools_cmd = trimmed.starts_with("/tools");
    let is_capabilities_cmd = trimmed.starts_with("/capabilities");
    let is_summarize_cmd = trimmed.starts_with("/summarize");
    let is_memory_cmd = trimmed.starts_with("/memory");

    // Only handle explicit /tools command, not natural language questions
    if is_tools_cmd {
        let thinking_note =
            Some("Using local tool catalog (no LLM call) and streaming the result...".to_string());
        let response = if is_tools_cmd {
            // Parse /tools <name> command
            let mut parts = trimmed.split_whitespace();
            let _ = parts.next(); // skip /tools
            if let Some(name) = parts.next() {
                render_tool_detail(name).unwrap_or_else(|| {
                    format!(
                        "Unknown tool '{}'. Try `/tools` to list all available tools.",
                        name
                    )
                })
            } else {
                render_tools_overview()
            }
        } else {
            render_tools_overview()
        };

        // Persist to session (if any)
        if let Some(ref sid) = resolved_session_id {
            crate::window_manager::add_user_message(sid, &message, message_source);
            crate::window_manager::add_assistant_message(sid, &response, thinking_note.clone());
        }

        if let Some(note) = thinking_note {
            emit("chat-stream-thinking", serde_json::json!(note));
            tokio::task::yield_now().await;
        }

        // Stream the response in chunks so the UX stays consistently "live".
        let mut rest = response.as_str();
        while !rest.is_empty() {
            let split_at = rest
                .char_indices()
                .nth(LOCAL_STREAM_CHUNK_CHARS)
                .map(|(i, _)| i)
                .unwrap_or(rest.len());

            let (chunk, next) = rest.split_at(split_at);
            rest = next;

            if !chunk.is_empty() {
                emit("chat-stream-chunk", serde_json::json!(chunk));
                tokio::task::yield_now().await;
            }
        }
        emit("chat-stream-done", serde_json::json!(null));
        return Ok(());
    }

    // Handle capabilities command (explicit slash command only)
    // Natural language questions should go through the LLM for dynamic responses
    if is_capabilities_cmd {
        let thinking_note = Some(
            "Reading local capabilities (no LLM call) and streaming the result...".to_string(),
        );
        let response = render_capabilities(&cfg);

        // Persist to session (if any)
        if let Some(ref sid) = resolved_session_id {
            crate::window_manager::add_user_message(sid, &message, message_source);
            crate::window_manager::add_assistant_message(sid, &response, thinking_note.clone());
        }

        if let Some(note) = thinking_note {
            emit("chat-stream-thinking", serde_json::json!(note));
            tokio::task::yield_now().await;
        }

        let mut rest = response.as_str();
        while !rest.is_empty() {
            let split_at = rest
                .char_indices()
                .nth(LOCAL_STREAM_CHUNK_CHARS)
                .map(|(i, _)| i)
                .unwrap_or(rest.len());

            let (chunk, next) = rest.split_at(split_at);
            rest = next;

            if !chunk.is_empty() {
                emit("chat-stream-chunk", serde_json::json!(chunk));
                tokio::task::yield_now().await;
            }
        }
        emit("chat-stream-done", serde_json::json!(null));
        return Ok(());
    }

    // Handle /summarize command - summarize conversation history without calling LLM
    if is_summarize_cmd {
        let thinking_note = Some("Summarizing conversation history (no LLM call)...".to_string());

        // Get conversation history from session
        let history = if let Some(ref sid) = resolved_session_id {
            crate::window_manager::get_session_state(sid)
                .map(|state| {
                    state
                        .messages
                        .into_iter()
                        .map(|msg| msg.content)
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default()
        } else {
            Vec::new()
        };

        // Use context manager to summarize
        use gestura_core::context::ContextManager;
        let context_manager = ContextManager::new();
        let summary = if history.is_empty() {
            "No conversation history to summarize.".to_string()
        } else {
            let summary_text = context_manager.summarize_history(&history);
            format!(
                "## Conversation Summary\n\n{}\n\n---\n\n*Summarized {} messages*",
                summary_text,
                history.len()
            )
        };

        // Persist to session (if any)
        if let Some(ref sid) = resolved_session_id {
            crate::window_manager::add_user_message(sid, &message, message_source);
            crate::window_manager::add_assistant_message(sid, &summary, thinking_note.clone());
        }

        if let Some(note) = thinking_note {
            emit("chat-stream-thinking", serde_json::json!(note));
            tokio::task::yield_now().await;
        }

        let mut rest = summary.as_str();
        while !rest.is_empty() {
            let split_at = rest
                .char_indices()
                .nth(LOCAL_STREAM_CHUNK_CHARS)
                .map(|(i, _)| i)
                .unwrap_or(rest.len());

            let (chunk, next) = rest.split_at(split_at);
            rest = next;

            if !chunk.is_empty() {
                emit("chat-stream-chunk", serde_json::json!(chunk));
                tokio::task::yield_now().await;
            }
        }
        emit("chat-stream-done", serde_json::json!(null));
        return Ok(());
    }

    // Handle /memory command - manage memory bank without calling LLM
    if is_memory_cmd {
        let thinking_note = Some("Managing memory bank (no LLM call)...".to_string());

        // Parse subcommand: /memory list|save|clear
        let mut parts = trimmed.split_whitespace();
        let _ = parts.next(); // skip /memory
        let subcommand = parts.next().unwrap_or("list");

        let response = match subcommand {
            "list" => {
                // List all memory bank entries
                // Retrieve workspace from session state
                let workspace_dir = if let Some(ref sid) = resolved_session_id {
                    crate::window_manager::get_session_state(sid).and_then(|s| s.workspace_dir)
                } else {
                    crate::window_manager::get_active_session_workspace()
                };

                if let Some(workspace_dir) = workspace_dir.as_ref() {
                    match gestura_core::memory_bank::list_memory_bank(workspace_dir).await {
                        Ok(entries) if !entries.is_empty() => {
                            let mut output =
                                format!("## Memory Bank Entries ({} total)\n\n", entries.len());
                            for entry in entries {
                                output.push_str(&format!(
                                    "### {} (Session: {})\n",
                                    entry.timestamp.format("%Y-%m-%d %H:%M UTC"),
                                    entry.session_id
                                ));
                                output.push_str(&format!("**Summary**: {}\n\n", entry.summary));
                                if let Some(path) = entry.file_path {
                                    output.push_str(&format!("**File**: `{}`\n\n", path.display()));
                                }
                                output.push_str("---\n\n");
                            }
                            output
                        }
                        Ok(_) => "No memory bank entries found.".to_string(),
                        Err(e) => format!("Error listing memory bank: {}", e),
                    }
                } else {
                    "No workspace directory configured. Cannot access memory bank.".to_string()
                }
            }
            "save" => {
                // Save current context to memory bank
                // Retrieve workspace from session state
                let workspace_dir = if let Some(ref sid) = resolved_session_id {
                    crate::window_manager::get_session_state(sid).and_then(|s| s.workspace_dir)
                } else {
                    crate::window_manager::get_active_session_workspace()
                };

                if let Some(workspace_dir) = workspace_dir.as_ref() {
                    let history = if let Some(ref sid) = resolved_session_id {
                        crate::window_manager::get_session_state(sid)
                            .map(|state| {
                                state
                                    .messages
                                    .into_iter()
                                    .map(|msg| msg.content)
                                    .collect::<Vec<_>>()
                            })
                            .unwrap_or_default()
                    } else {
                        Vec::new()
                    };

                    if history.is_empty() {
                        "No conversation history to save.".to_string()
                    } else {
                        use gestura_core::context::ContextManager;
                        let context_manager = ContextManager::new();
                        let summary = context_manager.summarize_history(&history);
                        let content = history.join("\n\n");

                        let entry = gestura_core::memory_bank::MemoryBankEntry {
                            timestamp: chrono::Utc::now(),
                            session_id: resolved_session_id
                                .clone()
                                .unwrap_or_else(|| "unknown".to_string()),
                            summary: summary.clone(),
                            content,
                            file_path: None,
                        };

                        match gestura_core::memory_bank::save_to_memory_bank(workspace_dir, &entry)
                            .await
                        {
                            Ok(path) => format!(
                                "✅ Saved {} messages to memory bank\n\n**File**: `{}`\n\n**Summary**: {}",
                                history.len(),
                                path.display(),
                                summary
                            ),
                            Err(e) => format!("Error saving to memory bank: {}", e),
                        }
                    }
                } else {
                    "No workspace directory configured. Cannot save to memory bank.".to_string()
                }
            }
            "clear" => {
                // Clear all memory bank entries
                // Retrieve workspace from session state
                let workspace_dir = if let Some(ref sid) = resolved_session_id {
                    crate::window_manager::get_session_state(sid).and_then(|s| s.workspace_dir)
                } else {
                    crate::window_manager::get_active_session_workspace()
                };

                if let Some(workspace_dir) = workspace_dir.as_ref() {
                    let memory_dir = workspace_dir.join(".gestura").join("memory");
                    match std::fs::remove_dir_all(&memory_dir) {
                        Ok(_) => {
                            // Recreate the directory
                            let _ = std::fs::create_dir_all(&memory_dir);
                            "✅ Cleared all memory bank entries.".to_string()
                        }
                        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                            "Memory bank is already empty.".to_string()
                        }
                        Err(e) => format!("Error clearing memory bank: {}", e),
                    }
                } else {
                    "No workspace directory configured. Cannot clear memory bank.".to_string()
                }
            }
            _ => {
                format!(
                    "Unknown /memory subcommand: '{}'\n\nUsage:\n- `/memory list` - Show all memory bank entries\n- `/memory save` - Save current conversation to memory bank\n- `/memory clear` - Delete all memory bank entries",
                    subcommand
                )
            }
        };

        // Persist to session (if any)
        if let Some(ref sid) = resolved_session_id {
            crate::window_manager::add_user_message(sid, &message, message_source);
            crate::window_manager::add_assistant_message(sid, &response, thinking_note.clone());
        }

        if let Some(note) = thinking_note {
            emit("chat-stream-thinking", serde_json::json!(note));
            tokio::task::yield_now().await;
        }

        let mut rest = response.as_str();
        while !rest.is_empty() {
            let split_at = rest
                .char_indices()
                .nth(LOCAL_STREAM_CHUNK_CHARS)
                .map(|(i, _)| i)
                .unwrap_or(rest.len());

            let (chunk, next) = rest.split_at(split_at);
            rest = next;

            if !chunk.is_empty() {
                emit("chat-stream-chunk", serde_json::json!(chunk));
                tokio::task::yield_now().await;
            }
        }
        emit("chat-stream-done", serde_json::json!(null));
        return Ok(());
    }

    tracing::info!(
        "Starting streaming chat through AgentPipeline with LLM provider: {}",
        cfg.llm.primary
    );

    // Create an agent task for this chat processing (if we have a session)
    // This makes agent work visible in the task panel
    let agent_task_id: Option<String> = if let Some(ref sid) = resolved_session_id {
        let task_name = {
            let preview: String = message.chars().take(50).collect();
            if message.len() > 50 {
                format!("{}...", preview)
            } else {
                preview
            }
        };

        match crate::task_integration::create_agent_task(
            &app, sid, &task_name, &message,
            None, // agent_id - we could use the provider name here
            None, // parent_id
        ) {
            Ok(task) => {
                tracing::debug!(
                    task_id = %task.id,
                    session_id = %sid,
                    "Created agent task for chat processing"
                );
                // Mark as in progress immediately
                let _ = crate::task_integration::mark_task_in_progress(&app, sid, &task.id);
                Some(task.id)
            }
            Err(e) => {
                tracing::warn!(
                    error = %e,
                    "Failed to create agent task for chat processing"
                );
                None
            }
        }
    } else {
        None
    };

    // Create channel for streaming chunks
    let (tx, mut rx) = mpsc::channel::<StreamChunk>(100);

    // Create cancellation token and store it (window-scoped)
    let cancel_token = CancellationToken::new();
    let cancel_key = if !target_window_label.is_empty() {
        cancel_key_for_window_label(&target_window_label)
    } else {
        GLOBAL_STREAM_CANCEL_KEY.to_string()
    };
    {
        let mut store = get_cancel_token_store().lock().unwrap();
        // If an old stream is still recorded for this key, cancel it to avoid overlap.
        if let Some(prev) = store.insert(cancel_key.clone(), cancel_token.clone()) {
            prev.cancel();
        }
    }

    // Build the agent request with workspace sandboxing
    use gestura_core::{AgentPipeline, AgentRequest};

    // Resolve the effective provider/model for this session (session override or global fallback).
    // IMPORTANT: We must use this for BOTH agent awareness metadata and for the actual pipeline config,
    // otherwise the agent may report one provider/model while the pipeline uses another.
    let (effective_provider, effective_model) = resolved_session_id
        .as_deref()
        .and_then(|sid| crate::window_manager::get_session_llm_config(sid).map(|c| (sid, c)))
        .map(|(_sid, session_llm)| {
            let provider = session_llm
                .provider
                .unwrap_or_else(|| cfg.llm.primary.clone());

            let fallback_model = || get_model_for_provider(&cfg, &provider).unwrap_or_default();
            let model = match session_llm.model {
                Some(m)
                    if crate::llm_validation::is_model_compatible_with_provider(&provider, &m) =>
                {
                    m
                }
                Some(m) => {
                    tracing::warn!(
                        provider = %provider,
                        model = %m,
                        "Ignoring incompatible session-scoped LLM model override (falling back)"
                    );
                    fallback_model()
                }
                None => fallback_model(),
            };
            (provider, model)
        })
        .unwrap_or_else(|| {
            let provider = cfg.llm.primary.clone();
            let model = get_model_for_provider(&cfg, &provider).unwrap_or_default();
            (provider, model)
        });

    // Use a conservative default history limit to prevent token explosion.
    // This matches the PipelineConfig default and prevents excessive context buildup.
    // Users can adjust this via the pipeline configuration if needed.
    let max_history = 10; // Conservative default to prevent token explosion

    // Build conversation history for the pipeline (session-scoped) BEFORE adding this new user message.
    // This mirrors the CLI TUI behavior.
    let history: Vec<gestura_core::Message> = resolved_session_id
        .as_deref()
        .map(|sid| {
            let msgs = crate::window_manager::get_pipeline_messages(sid);
            let total_msgs = msgs.len();
            let result: Vec<_> = msgs
                .into_iter()
                .rev()
                .take(max_history)
                .collect::<Vec<_>>()
                .into_iter()
                .rev()
                .collect();
            tracing::debug!(
                total_messages = total_msgs,
                included = result.len(),
                max_history = max_history,
                "Pre-filtered conversation history for token efficiency"
            );
            result
        })
        .unwrap_or_default();

    // Persist user message to session state now that we've captured the prior history.
    if let Some(ref sid) = resolved_session_id {
        crate::window_manager::add_user_message(sid, &message, message_source);
    }

    let mut request = AgentRequest::new(&message)
        .with_streaming(true)
        .with_source(request_source)
        .with_history(history);

    if let Some(ref sid) = resolved_session_id {
        request = request.with_session(sid);
    }

    // Set workspace directory for sandboxed operations
    if let Some(ref sid) = resolved_session_id {
        if let Some(workspace) =
            crate::window_manager::get_session_state(sid).and_then(|s| s.workspace_dir)
        {
            request = request.with_workspace(workspace);
        }
    } else if let Some(workspace) = crate::window_manager::get_active_session_workspace() {
        // Backwards-compatible fallback
        request = request.with_workspace(workspace);
    }

    // Set session LLM config and permission level for agent awareness.
    // The agent can use this info to report its current configuration.
    request = request.with_session_llm_config(&effective_provider, &effective_model);

    if let Some(ref sid) = resolved_session_id
        && let Some(state) = crate::window_manager::get_session_state(sid)
        && let Some(ref tool_settings) = state.tool_settings
    {
        use gestura_core::pipeline::PermissionLevel;
        let perm_level = match tool_settings.permission_level {
            crate::window_manager::SessionPermissionLevel::Sandbox => PermissionLevel::Sandbox,
            crate::window_manager::SessionPermissionLevel::Restricted => {
                PermissionLevel::Restricted
            }
            crate::window_manager::SessionPermissionLevel::Full => PermissionLevel::Full,
        };
        request = request.with_permission_level(perm_level);

        // Pass enabled tools to the pipeline so the agent knows what tools are available.
        // Only include tools that are explicitly enabled (value == true).
        let enabled_tools: Vec<String> = tool_settings
            .enabled_tools
            .iter()
            .filter_map(|(name, enabled)| if *enabled { Some(name.clone()) } else { None })
            .collect();

        // Log all tool settings for debugging
        tracing::debug!(
            session_id = sid,
            all_tool_settings = ?tool_settings.enabled_tools,
            enabled_tools = ?enabled_tools,
            "Session tool configuration"
        );

        if !enabled_tools.is_empty() {
            request = request.with_allowed_tools(enabled_tools);
        } else {
            tracing::warn!(
                session_id = sid,
                "No tools enabled in session - LLM will receive category-based tool list"
            );
        }
    }

    // Create the pipeline with provider-optimized configuration and spawn the streaming task.
    // We must apply the effective (session-scoped) provider/model to the pipeline config.
    let mut cfg_clone = cfg.clone();
    cfg_clone.llm.primary = effective_provider.clone();
    match effective_provider.as_str() {
        "openai" => {
            if let Some(ref mut c) = cfg_clone.llm.openai {
                c.model = effective_model.clone();
            }
        }
        "anthropic" => {
            if let Some(ref mut c) = cfg_clone.llm.anthropic {
                c.model = effective_model.clone();
            }
        }
        "grok" => {
            if let Some(ref mut c) = cfg_clone.llm.grok {
                c.model = effective_model.clone();
            }
        }
        "ollama" => {
            if let Some(ref mut c) = cfg_clone.llm.ollama {
                c.model = effective_model.clone();
            }
        }
        _ => {}
    }
    let cancel_token_clone = cancel_token.clone();
    let pipeline_handle = tokio::spawn(async move {
        // Use provider-optimized config for better token management
        // and integrate with knowledge system
        let pipeline = AgentPipeline::with_provider_optimized_config(cfg_clone)
            .with_knowledge(get_knowledge_store(), get_knowledge_settings());
        if let Err(e) = pipeline
            .process_streaming(request, tx.clone(), cancel_token_clone)
            .await
        {
            tracing::error!("AgentPipeline streaming error: {}", e);
            // Ensure the GUI always receives a terminal event.
            let _ = tx.send(StreamChunk::Error(e.to_string())).await;
            let _ = tx.send(StreamChunk::Done(None)).await;
        }
    });

    use tokio::time::{Duration, Instant};

    // Forward chunks to frontend via Tauri events
    let mut assistant_text = String::new();
    let mut assistant_thinking: Option<String> = None;
    let mut saw_terminal = false;
    // Normal idle timeout detects backend hangs.
    // When we are waiting for *user* tool confirmation, we extend this to avoid
    // cancelling a healthy stream that is intentionally paused.
    let idle_timeout_normal = Duration::from_secs(90);
    let idle_timeout_waiting_for_user = Duration::from_secs(10 * 60);
    let mut idle_timeout = idle_timeout_normal;
    let idle_timer = tokio::time::sleep(idle_timeout);
    tokio::pin!(idle_timer);

    loop {
        tokio::select! {
            maybe_chunk = rx.recv() => {
                let Some(chunk) = maybe_chunk else {
                    break;
                };
                // Update idle timeout based on what we just received.
                idle_timeout = match &chunk {
                    StreamChunk::ToolConfirmationRequired { .. } => idle_timeout_waiting_for_user,
                    _ => idle_timeout_normal,
                };
                idle_timer.as_mut().reset(Instant::now() + idle_timeout);
                match chunk {
            StreamChunk::Thinking(text) => {
                tracing::debug!("[Stream] Thinking chunk: {}", &text.chars().take(100).collect::<String>());
                assistant_thinking
                    .get_or_insert_with(String::new)
                    .push_str(&text);
                emit("chat-stream-thinking", serde_json::json!(text));
            }
            StreamChunk::Text(text) => {
                tracing::debug!("[Stream] Text chunk: {}", &text.chars().take(100).collect::<String>());
                assistant_text.push_str(&text);
                emit("chat-stream-chunk", serde_json::json!(text));
            }
            StreamChunk::ToolCallStart { id, name } => {
                let payload = serde_json::json!({ "id": id, "name": name });
                emit("chat-stream-tool-start", payload);
            }
            StreamChunk::ToolCallArgs(args) => {
                emit("chat-stream-tool-args", serde_json::json!(args));
            }
            StreamChunk::ToolCallEnd => {
                emit("chat-stream-tool-end", serde_json::json!(null));
            }
            StreamChunk::ToolCallResult {
                name,
                success,
                output,
                duration_ms,
            } => {
                let payload = serde_json::json!({
                    "name": name,
                    "success": success,
                    "output": output,
                    "duration_ms": duration_ms
                });
                emit("chat-stream-tool-result", payload);
            }
            StreamChunk::RetryAttempt {
                attempt,
                max_attempts,
                delay_ms,
                error_message,
            } => {
                let payload = serde_json::json!({
                    "attempt": attempt,
                    "max_attempts": max_attempts,
                    "delay_ms": delay_ms,
                    "error_message": error_message
                });
                emit("chat-stream-retry", payload);
            }
            StreamChunk::ContextCompacted {
                messages_before,
                messages_after,
                tokens_saved,
                summary,
            } => {
                let payload = serde_json::json!({
                    "messages_before": messages_before,
                    "messages_after": messages_after,
                    "tokens_saved": tokens_saved,
                    "summary": summary
                });
                emit("chat-context-compacted", payload);
            }
            StreamChunk::MemoryBankSaved {
                file_path,
                session_id,
                summary,
                messages_saved,
            } => {
                let payload = serde_json::json!({
                    "file_path": file_path,
                    "session_id": session_id,
                    "summary": summary,
                    "messages_saved": messages_saved
                });
                emit("chat-memory-bank-saved", payload);
            }
            StreamChunk::TokenUsageUpdate {
                estimated,
                limit,
                percentage,
                status,
                estimated_cost,
            } => {
                let status_str = match status {
                    gestura_core::streaming::TokenUsageStatus::Green => "green",
                    gestura_core::streaming::TokenUsageStatus::Yellow => "yellow",
                    gestura_core::streaming::TokenUsageStatus::Red => "red",
                };
                let payload = serde_json::json!({
                    "estimated": estimated,
                    "limit": limit,
                    "percentage": percentage,
                    "status": status_str,
                    "estimated_cost": estimated_cost
                });
                emit("chat-token-usage", payload);
            }
            StreamChunk::ConfigRequest {
                operation,
                key,
                value,
                requires_confirmation,
            } => {
                let payload = serde_json::json!({
                    "operation": operation,
                    "key": key,
                    "value": value,
                    "requires_confirmation": requires_confirmation,
                    "session_id": resolved_session_id
                });
                emit("chat-config-request", payload);
            }
            StreamChunk::ToolConfirmationRequired {
                confirmation_id,
                tool_name,
                tool_args,
                description,
                risk_level,
                category,
            } => {
                let payload = serde_json::json!({
                    "confirmation_id": confirmation_id,
                    "tool_name": tool_name,
                    "tool_args": tool_args,
                    "description": description,
                    "risk_level": risk_level,
                    "category": category,
                    "session_id": resolved_session_id
                });
                emit("chat-stream-tool-confirmation", payload);
            }
            StreamChunk::ToolBlocked { tool_name, reason } => {
                let payload = serde_json::json!({
                    "tool_name": tool_name,
                    "reason": reason,
                    "session_id": resolved_session_id
                });
                emit("chat-stream-tool-blocked", payload);
            }
            StreamChunk::Done(usage) => {
                saw_terminal = true;
                // Emit token usage if available
                if let Some(ref usage) = usage {
                    emit(
                        "chat-token-usage",
                    serde_json::to_value(usage).unwrap_or(serde_json::Value::Null),
                    );

                    if let Some(ref sid) = resolved_session_id {
                        let total = u64::from(usage.input_tokens)
                            .saturating_add(u64::from(usage.output_tokens));
                        crate::window_manager::update_token_count(sid, total);
                    }
                }

                    // Persist assistant message to session state
                    if let Some(ref sid) = resolved_session_id
                        && (!assistant_text.trim().is_empty()
                            || assistant_thinking
                                .as_ref()
                                .is_some_and(|t| !t.trim().is_empty()))
                    {
                        crate::window_manager::add_assistant_message(
                            sid,
                            &assistant_text,
                            assistant_thinking.clone(),
                        );
                    }

                // Mark agent task as completed
                if let (Some(sid), Some(task_id)) = (&resolved_session_id, &agent_task_id) {
                    let _ = crate::task_integration::mark_task_completed(&app, sid, task_id);
                }

                emit("chat-stream-done", serde_json::json!(null));
                break;
            }
            StreamChunk::Cancelled => {
                saw_terminal = true;
                    // Persist any partial assistant output so context isn't lost.
                    if let Some(ref sid) = resolved_session_id
                        && (!assistant_text.trim().is_empty()
                            || assistant_thinking
                                .as_ref()
                                .is_some_and(|t| !t.trim().is_empty()))
                    {
                        crate::window_manager::add_assistant_message(
                            sid,
                            &assistant_text,
                            assistant_thinking.clone(),
                        );
                    }

                // Mark agent task as cancelled
                if let (Some(sid), Some(task_id)) = (&resolved_session_id, &agent_task_id) {
                    let _ = crate::task_integration::mark_task_cancelled(&app, sid, task_id);
                }

                emit("chat-stream-cancelled", serde_json::json!(null));
                break;
            }
            StreamChunk::Error(err) => {
                saw_terminal = true;
                    // Persist any partial assistant output so context isn't lost.
                    if let Some(ref sid) = resolved_session_id
                        && (!assistant_text.trim().is_empty()
                            || assistant_thinking
                                .as_ref()
                                .is_some_and(|t| !t.trim().is_empty()))
                    {
                        crate::window_manager::add_assistant_message(
                            sid,
                            &assistant_text,
                            assistant_thinking.clone(),
                        );
                    }

                // Mark agent task as cancelled (error case)
                if let (Some(sid), Some(task_id)) = (&resolved_session_id, &agent_task_id) {
                    let _ = crate::task_integration::mark_task_cancelled(&app, sid, task_id);
                }

                emit("chat-stream-error", serde_json::json!(err));
                break;
            }
                }
            }
            _ = &mut idle_timer => {
                // If we haven't received any stream events in a while, treat this as a backend hang.
                // Ensure the frontend gets an explicit terminal event.
                saw_terminal = true;
                tracing::error!("Streaming chat timed out (no events for {:?})", idle_timeout);
                cancel_token.cancel();
                emit(
                    "chat-stream-error",
                    serde_json::json!(format!(
                        "Timed out waiting for agent response (no events for {:?}).",
                        idle_timeout
                    )),
                );
                break;
            }
        }
    }

    // If the channel closed without any terminal event, surface that as an error.
    if !saw_terminal {
        emit(
            "chat-stream-error",
            serde_json::json!("Streaming ended unexpectedly (no terminal event received)"),
        );
    }

    // Ensure we observe pipeline task failures (panic/abort) and don't silently swallow them.
    let mut pipeline_handle = pipeline_handle;
    tokio::select! {
        res = &mut pipeline_handle => {
            if let Err(join_err) = res {
                tracing::error!("AgentPipeline task join error: {}", join_err);
                if !saw_terminal {
                    emit("chat-stream-error", serde_json::json!(format!("Agent task failed: {join_err}")));
                }
            }
        }
        _ = tokio::time::sleep(Duration::from_secs(2)) => {
            // Pipeline task didn't finish promptly after we stopped listening.
            // Abort to avoid leaked work.
            tracing::warn!("AgentPipeline task did not finish after terminal event; aborting");
            pipeline_handle.abort();
        }
    }

    // Clear the cancellation token for this stream
    {
        let mut store = get_cancel_token_store().lock().unwrap();
        store.remove(&cancel_key);
    }

    Ok(())
}

/// Cancel an ongoing streaming chat request.
///
/// Cancellation is scoped to a single webview window.
///
/// - If `session_id` is provided, we resolve the session's current chat window label and
///   cancel that window's stream.
/// - If `session_id` is omitted, we cancel the stream for the **calling window**.
///
/// This prevents a cancel action in one chat window from cancelling another window's
/// in-flight stream.
///
/// Note: This command uses `snake_case` argument names for JS↔Rust interop.
#[tauri::command(rename_all = "snake_case")]
pub fn cancel_chat_streaming(
    webview_window: tauri::WebviewWindow,
    session_id: Option<String>,
) -> Result<(), String> {
    let calling_window_label = webview_window.label().to_string();
    cancel_chat_streaming_internal(Some(calling_window_label), session_id)
}

/// Approve a pending tool confirmation request.
///
/// JS↔Rust interop: The frontend calls this when the user clicks "Approve" on a
/// `chat-stream-tool-confirmation` dialog.
///
/// Note: This command uses `snake_case` argument names for JS↔Rust interop.
#[tauri::command(rename_all = "snake_case")]
pub fn approve_tool_confirmation(
    confirmation_id: String,
    session_id: Option<String>,
) -> Result<(), String> {
    gestura_core::tool_confirmation::TOOL_CONFIRMATIONS.resolve(
        &confirmation_id,
        session_id.as_deref(),
        true,
    )
}

/// Deny a pending tool confirmation request.
///
/// JS↔Rust interop: The frontend calls this when the user clicks "Deny" (or
/// dismisses) a `chat-stream-tool-confirmation` dialog.
///
/// Note: This command uses `snake_case` argument names for JS↔Rust interop.
#[tauri::command(rename_all = "snake_case")]
pub fn deny_tool_confirmation(
    confirmation_id: String,
    session_id: Option<String>,
) -> Result<(), String> {
    gestura_core::tool_confirmation::TOOL_CONFIRMATIONS.resolve(
        &confirmation_id,
        session_id.as_deref(),
        false,
    )
}

/// Returns recent chat event emission trace entries.
///
/// This is a diagnostics-only command used to debug cross-window event leakage.
/// The trace is an in-memory ring buffer recorded by `crate::chat_events`.
#[tauri::command]
pub fn get_chat_event_trace(max: Option<usize>) -> Vec<crate::chat_events::ChatEventTraceEntry> {
    crate::chat_events::get_chat_event_trace(max)
}

/// Clears the in-memory chat event emission trace.
#[tauri::command]
pub fn clear_chat_event_trace() -> Result<(), String> {
    crate::chat_events::clear_chat_event_trace();
    Ok(())
}

/// Records a frontend "receipt" payload into an in-memory trace.
///
/// This is diagnostics-only and best-effort.
///
/// The frontend should send a JSON string containing at least:
/// - `eventName`
/// - `windowLabel` (optional)
/// - `sessionId` (optional)
/// - `incomingSessionId` (optional)
/// - `accept` + `reason` (optional)
#[tauri::command]
pub fn record_chat_receipt(payload: String) -> Result<(), String> {
    crate::chat_receipts::record_chat_receipt_payload(&payload);
    Ok(())
}

/// Returns recent chat receipt trace entries.
///
/// This is a diagnostics-only command used to debug cross-window event leakage.
#[tauri::command]
pub fn get_chat_receipt_trace(
    max: Option<usize>,
) -> Vec<crate::chat_receipts::ChatReceiptTraceEntry> {
    crate::chat_receipts::get_chat_receipt_trace(max)
}

/// Clears the in-memory chat receipt trace.
#[tauri::command]
pub fn clear_chat_receipt_trace() -> Result<(), String> {
    crate::chat_receipts::clear_chat_receipt_trace();
    Ok(())
}

/// Run a deterministic cross-window isolation probe.
///
/// This does not call any external LLM providers. It emits a `chat-probe` event to
/// two open chat windows and returns an analysis based on backend traces.
#[tauri::command]
pub async fn run_chat_isolation_probe(
    app: tauri::AppHandle,
) -> Result<crate::chat_probe::ChatIsolationProbeReport, String> {
    crate::chat_probe::run_chat_isolation_probe(app).await
}

/// Internal cancellation implementation shared by `cancel_chat_streaming`.
///
/// This helper keeps the key-resolution logic testable without requiring an actual
/// Tauri [`tauri::WebviewWindow`] instance.
fn cancel_chat_streaming_internal(
    calling_window_label: Option<String>,
    session_id: Option<String>,
) -> Result<(), String> {
    let mut store = get_cancel_token_store().lock().unwrap();

    let cancel_key = if let Some(sid) = session_id {
        let label = crate::window_manager::get_session_window_label(&sid).ok_or_else(|| {
            format!(
                "Cannot cancel stream: no window label found for session {}",
                sid
            )
        })?;
        cancel_key_for_window_label(&label)
    } else if let Some(label) = calling_window_label {
        cancel_key_for_window_label(&label)
    } else {
        return Err(
            "Cannot cancel stream: no session_id provided and no calling window context"
                .to_string(),
        );
    };

    if let Some(token) = store.remove(&cancel_key) {
        token.cancel();
        tracing::info!(cancel_key = %cancel_key, "Streaming chat cancelled");
        Ok(())
    } else {
        Err(format!(
            "No active streaming request to cancel for key {}",
            cancel_key
        ))
    }
}

#[cfg(test)]
mod streaming_cancellation_tests {
    use super::*;

    #[test]
    fn cancel_key_is_window_scoped() {
        assert_eq!(cancel_key_for_window_label("chat-abc"), "window:chat-abc");
    }

    #[test]
    fn cancel_internal_cancels_calling_window_when_no_session_id() {
        let label = "chat-test-cancel-internal";
        let key = cancel_key_for_window_label(label);

        let token = gestura_core::CancellationToken::new();
        {
            let mut store = get_cancel_token_store().lock().unwrap();
            store.remove(&key);
            store.insert(key.clone(), token.clone());
        }

        cancel_chat_streaming_internal(Some(label.to_string()), None)
            .expect("expected cancellation to succeed");

        assert!(token.is_cancelled(), "token should be cancelled");
        let store = get_cancel_token_store().lock().unwrap();
        assert!(
            !store.contains_key(&key),
            "token entry should be removed after cancellation"
        );
    }

    #[test]
    fn cancel_internal_requires_context() {
        let err = cancel_chat_streaming_internal(None, None).expect_err("expected error");
        assert!(err.contains("no session_id") || err.contains("no calling window"));
    }
}

#[tauri::command]
pub async fn send_agent_message(
    agent_id: String,
    message: String,
    _state: tauri::State<'_, crate::AppState>,
) -> Result<String, String> {
    // Use the configured LLM provider for agent communication
    let cfg = AppConfig::load_async().await;
    let provider = select_provider(
        &cfg,
        &AgentContext {
            agent_id: agent_id.clone(),
        },
    );

    tracing::info!("Agent {} sending message through LLM", agent_id);

    // Format the prompt with agent context
    let prompt = format!(
        "You are agent '{}'. Respond to the following message:\n\n{}",
        agent_id, message
    );

    let response = provider
        .call(&prompt)
        .await
        .map_err(|e| format!("Agent LLM error: {}", e))?;

    tracing::info!("Agent {} response received", agent_id);
    Ok(response)
}

#[tauri::command]
pub async fn get_agent_status(
    agent_id: String,
    state: tauri::State<'_, crate::AppState>,
) -> Result<serde_json::Value, String> {
    let agents = &state.agents;

    // Check if agent exists in the manager
    if let Some(info) = agents.get_agent_status(&agent_id).await {
        return Ok(serde_json::json!({
            "id": info.id,
            "name": info.name,
            "status": info.status,
            "last_activity": info.last_activity.to_rfc3339()
        }));
    }

    // Agent not found - return inactive status
    Ok(serde_json::json!({
        "id": agent_id,
        "name": "Unknown Agent",
        "status": "inactive",
        "last_activity": null
    }))
}

/// List all active agents
#[tauri::command]
pub async fn list_agents(
    state: tauri::State<'_, crate::AppState>,
) -> Result<serde_json::Value, String> {
    let agents = &state.agents;
    let agent_list = agents.list_agents().await;

    Ok(serde_json::json!({
        "agents": agent_list,
        "count": agent_list.len()
    }))
}

// Orchestrator Commands

/// Delegate a task to a subagent
#[tauri::command]
pub async fn delegate_task(
    task: crate::orchestrator::DelegatedTask,
    state: tauri::State<'_, crate::AppState>,
) -> Result<String, String> {
    let orchestrator =
        crate::orchestrator::AgentOrchestrator::new(state.agents.clone(), state.config.clone());
    orchestrator.delegate_task(task).await
}

/// Spawn a new subagent
#[tauri::command]
pub async fn spawn_subagent(
    agent_id: String,
    name: String,
    state: tauri::State<'_, crate::AppState>,
) -> Result<(), String> {
    let orchestrator =
        crate::orchestrator::AgentOrchestrator::new(state.agents.clone(), state.config.clone());
    orchestrator.spawn_subagent(&agent_id, &name).await
}

/// List all active tasks
#[tauri::command]
pub async fn list_active_tasks(
    state: tauri::State<'_, crate::AppState>,
) -> Result<Vec<crate::orchestrator::DelegatedTask>, String> {
    let orchestrator =
        crate::orchestrator::AgentOrchestrator::new(state.agents.clone(), state.config.clone());
    Ok(orchestrator.list_active_tasks().await)
}

/// Cancel a running task
#[tauri::command]
pub async fn cancel_task(
    task_id: String,
    state: tauri::State<'_, crate::AppState>,
) -> Result<(), String> {
    let orchestrator =
        crate::orchestrator::AgentOrchestrator::new(state.agents.clone(), state.config.clone());
    orchestrator.cancel_task(&task_id).await
}

// Audio Device Management Commands

/// List all available audio input devices
#[tauri::command]
pub fn list_audio_devices() -> Result<Vec<crate::audio_capture::AudioDeviceInfo>, String> {
    Ok(crate::audio_capture::list_audio_input_devices())
}

/// Check if microphone is available
#[tauri::command]
pub fn check_microphone_available() -> bool {
    crate::audio_capture::is_microphone_available()
}

// Permission Management Commands

#[tauri::command]
pub async fn check_permission(permission: String) -> Result<String, String> {
    use crate::permissions::{
        check_accessibility_permission, check_bluetooth_permission, check_microphone_permission,
        check_screen_recording_permission,
    };

    match permission.as_str() {
        "microphone" => {
            let status = check_microphone_permission();
            tracing::info!("Permission check: microphone -> {}", status);
            Ok(status.to_string())
        }
        "accessibility" => {
            let status = check_accessibility_permission();
            tracing::info!("Permission check: accessibility -> {}", status);
            Ok(status.to_string())
        }
        "bluetooth" => {
            let status = check_bluetooth_permission();
            tracing::info!("Permission check: bluetooth -> {}", status);
            Ok(status.to_string())
        }
        "screen_recording" => {
            let status = check_screen_recording_permission();
            tracing::info!("Permission check: screen_recording -> {}", status);
            Ok(status.to_string())
        }
        _ => Err(format!("Unknown permission: {}", permission)),
    }
}

#[tauri::command]
pub async fn request_permission(permission: String) -> Result<(), String> {
    use crate::permissions::{
        SystemPermissionStatus, check_accessibility_permission, check_bluetooth_permission,
        check_microphone_permission, check_screen_recording_permission, open_system_preferences,
        request_bluetooth_permission, request_microphone_permission,
        request_screen_recording_permission,
    };

    tracing::info!("🔐 Permission request received: {}", permission);

    match permission.as_str() {
        "microphone" => {
            let status = check_microphone_permission();
            tracing::info!("🎤 Microphone permission status before request: {}", status);

            match status {
                SystemPermissionStatus::Granted => {
                    tracing::info!("🎤 Microphone permission already granted; nothing to request");
                    Ok(())
                }
                SystemPermissionStatus::Denied | SystemPermissionStatus::Restricted => {
                    tracing::info!(
                        "🎤 Microphone permission denied or restricted; opening System Settings",
                    );
                    if open_system_preferences("microphone") {
                        tracing::info!("✅ Opened System Preferences for Microphone");
                        Ok(())
                    } else {
                        Err("Failed to open System Preferences for Microphone".to_string())
                    }
                }
                SystemPermissionStatus::NotDetermined | SystemPermissionStatus::Unknown => {
                    tracing::info!(
                        "🎤 Microphone permission not determined/unknown; attempting to trigger system dialog",
                    );
                    if request_microphone_permission() {
                        tracing::info!("✅ Microphone permission request initiated");
                        Ok(())
                    } else {
                        tracing::warn!(
                            "⚠️ Microphone permission request script failed; attempting to open System Preferences",
                        );
                        if open_system_preferences("microphone") {
                            tracing::info!(
                                "✅ Opened System Preferences for Microphone (fallback)",
                            );
                            Ok(())
                        } else {
                            Err(
                                "Failed to request microphone permission or open System Preferences"
                                    .to_string(),
                            )
                        }
                    }
                }
            }
        }
        "accessibility" => {
            let status = check_accessibility_permission();
            tracing::info!(
                "♿ Accessibility permission status before request: {}",
                status
            );

            if status == SystemPermissionStatus::Granted {
                tracing::info!("♿ Accessibility permission already granted; nothing to request",);
                return Ok(());
            }

            // Accessibility CANNOT be requested programmatically on macOS
            // It always requires manual grant in System Settings
            if open_system_preferences("accessibility") {
                tracing::info!("✅ Opened System Preferences for Accessibility");
                Ok(())
            } else {
                Err("Failed to open System Preferences for Accessibility".to_string())
            }
        }
        "bluetooth" => {
            let status = check_bluetooth_permission();
            tracing::info!("🔵 Bluetooth permission status before request: {}", status);

            match status {
                SystemPermissionStatus::Granted => {
                    tracing::info!("🔵 Bluetooth permission already granted; nothing to request",);
                    Ok(())
                }
                SystemPermissionStatus::Denied | SystemPermissionStatus::Restricted => {
                    tracing::info!(
                        "🔵 Bluetooth permission denied or restricted; opening System Settings",
                    );
                    if open_system_preferences("bluetooth") {
                        tracing::info!("✅ Opened System Preferences for Bluetooth");
                        Ok(())
                    } else {
                        Err("Failed to open System Preferences for Bluetooth".to_string())
                    }
                }
                SystemPermissionStatus::NotDetermined | SystemPermissionStatus::Unknown => {
                    tracing::info!(
                        "🔵 Bluetooth permission not determined/unknown; attempting to trigger system dialog",
                    );
                    if request_bluetooth_permission() {
                        tracing::info!("✅ Bluetooth permission request initiated");
                        Ok(())
                    } else {
                        tracing::warn!(
                            "⚠️ Bluetooth permission request script failed; attempting to open System Preferences",
                        );
                        if open_system_preferences("bluetooth") {
                            tracing::info!("✅ Opened System Preferences for Bluetooth (fallback)",);
                            Ok(())
                        } else {
                            Err(
                                "Failed to request Bluetooth permission or open System Preferences"
                                    .to_string(),
                            )
                        }
                    }
                }
            }
        }
        "screen_recording" => {
            let status = check_screen_recording_permission();
            tracing::info!(
                "🖥️ Screen Recording permission status before request: {}",
                status
            );

            match status {
                SystemPermissionStatus::Granted => {
                    tracing::info!(
                        "🖥️ Screen Recording permission already granted; nothing to request",
                    );
                    Ok(())
                }
                SystemPermissionStatus::Denied | SystemPermissionStatus::Restricted => {
                    tracing::info!(
                        "🖥️ Screen Recording permission denied/restricted; opening System Settings",
                    );
                    if open_system_preferences("screen_recording") {
                        Ok(())
                    } else {
                        Err("Failed to open System Preferences for Screen Recording".to_string())
                    }
                }
                SystemPermissionStatus::NotDetermined | SystemPermissionStatus::Unknown => {
                    tracing::info!(
                        "🖥️ Screen Recording permission not determined/unknown; attempting to trigger system prompt",
                    );

                    if request_screen_recording_permission()
                        || open_system_preferences("screen_recording")
                    {
                        Ok(())
                    } else {
                        Err(
                            "Failed to request Screen Recording permission or open System Preferences"
                                .to_string(),
                        )
                    }
                }
            }
        }
        _ => Err(format!("Cannot request unknown permission: {}", permission)),
    }
}

/// Open the configuration/settings window
#[tauri::command]
pub fn open_config_window() -> Result<(), String> {
    crate::window_manager::open_config_window().map_err(|e| e.to_string())
}

// UI Testing and Validation Commands

#[tauri::command]
pub async fn test_open_window(
    window_type: String,
    _app: tauri::AppHandle,
) -> Result<String, String> {
    tracing::info!("Testing window open: {}", window_type);

    match window_type.as_str() {
        "config" => {
            crate::window_manager::open_config_window().map_err(|e| e.to_string())?;
            Ok("Config window opened".to_string())
        }
        "chat" => {
            let session_id =
                crate::window_manager::create_new_chat_session().map_err(|e| e.to_string())?;
            Ok(format!("Chat session created: {}", session_id))
        }
        _ => Err(format!("Unknown window type: {}", window_type)),
    }
}

#[tauri::command]
pub async fn capture_window_screenshot(
    window_label: String,
    app: tauri::AppHandle,
) -> Result<String, String> {
    if let Some(window) = app.get_webview_window(&window_label) {
        // Take screenshot using Tauri's screenshot API
        // Note: This requires the window to be visible and focused
        let _ = window.show();
        let _ = window.set_focus();

        // Wait a moment for the window to render
        tokio::time::sleep(tokio::time::Duration::from_millis(1000)).await;

        tracing::info!("Screenshot captured for window: {}", window_label);
        Ok(format!("Screenshot captured for {}", window_label))
    } else {
        Err(format!("Window not found: {}", window_label))
    }
}

#[tauri::command]
pub async fn validate_window_content(
    window_label: String,
    app: tauri::AppHandle,
) -> Result<serde_json::Value, String> {
    if let Some(window) = app.get_webview_window(&window_label) {
        let _ = window.show();
        let _ = window.set_focus();

        // Wait for content to load
        tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;

        // Return validation results
        Ok(serde_json::json!({
            "window": window_label,
            "visible": true,
            "focused": true,
            "timestamp": chrono::Utc::now().to_rfc3339(),
            "status": "validated"
        }))
    } else {
        Err(format!("Window not found: {}", window_label))
    }
}

#[tauri::command]
pub async fn get_window_list(app: tauri::AppHandle) -> Result<Vec<String>, String> {
    let windows: Vec<String> = app.webview_windows().keys().cloned().collect();

    Ok(windows)
}

#[tauri::command]
pub async fn close_test_windows(app: tauri::AppHandle) -> Result<String, String> {
    let test_windows = ["permissions", "config", "chat", "status", "about"];
    let mut closed_count = 0;

    for window_label in test_windows.iter() {
        if let Some(window) = app.get_webview_window(window_label) {
            let _ = window.close();
            closed_count += 1;
        }
    }

    Ok(format!("Closed {} test windows", closed_count))
}

/// Get current listening state
#[tauri::command]
pub fn get_listening_state() -> Result<(bool, Option<u64>), String> {
    let (is_listening, remaining) = crate::tray::get_listening_state();
    let remaining_secs = remaining.map(|d| d.as_secs());
    Ok((is_listening, remaining_secs))
}

/// Set listening timeout duration
#[tauri::command]
pub fn set_listening_timeout(seconds: u64) -> Result<(), String> {
    let duration = std::time::Duration::from_secs(seconds);
    crate::tray::set_listening_timeout(duration);
    Ok(())
}

/// Toggle listening mode
#[tauri::command]
pub fn toggle_listening(_app: tauri::AppHandle) -> Result<String, String> {
    // This would trigger the same logic as the tray menu
    // For now, we'll just return the current state
    let (is_listening, _) = crate::tray::get_listening_state();
    if is_listening {
        Ok("Listening stopped".to_string())
    } else {
        Ok("Listening started".to_string())
    }
}

/// Update speech processing configuration
#[tauri::command]
pub fn update_speech_config(config: crate::speech::SpeechConfig) -> Result<(), String> {
    crate::speech::update_speech_config(config);
    Ok(())
}

/// Get current speech processing status
#[tauri::command]
pub fn get_speech_status() -> Result<bool, String> {
    Ok(crate::speech::is_speech_recording())
}

/// Get tray diagnostic information
#[tauri::command]
pub fn get_tray_diagnostic_info() -> Result<serde_json::Value, String> {
    Ok(crate::tray::get_tray_diagnostic_info())
}

/// Check system permissions status
#[tauri::command]
pub fn check_system_permissions() -> Result<serde_json::Value, String> {
    use crate::permissions::{
        SystemPermissionStatus, check_accessibility_permission, check_bluetooth_permission,
        check_microphone_permission, check_screen_recording_permission,
    };

    let mic_status = check_microphone_permission();
    let accessibility_status = check_accessibility_permission();
    let bluetooth_status = check_bluetooth_permission();
    let screen_recording_status = check_screen_recording_permission();

    tracing::info!(
        "System permission snapshot: microphone={}, accessibility={}, bluetooth={}, screen_recording={}",
        mic_status,
        accessibility_status,
        bluetooth_status,
        screen_recording_status
    );

    let mic_instructions = match mic_status {
        SystemPermissionStatus::Granted => "Microphone access is working properly",
        SystemPermissionStatus::Denied => {
            "Please enable microphone access in System Preferences > Privacy & Security > Microphone"
        }
        SystemPermissionStatus::NotDetermined => {
            "Microphone access will be requested when you start listening"
        }
        _ => "Check System Preferences for microphone access",
    };

    let accessibility_instructions = match accessibility_status {
        SystemPermissionStatus::Granted => "Accessibility access is working properly",
        SystemPermissionStatus::Denied => {
            "Please enable accessibility in System Preferences > Privacy & Security > Accessibility"
        }
        _ => "Accessibility access is required for hotkey functionality",
    };

    let bluetooth_instructions = match bluetooth_status {
        SystemPermissionStatus::Granted => "Bluetooth access is working properly",
        SystemPermissionStatus::Denied => {
            "Please enable Bluetooth in System Preferences > Privacy & Security > Bluetooth"
        }
        SystemPermissionStatus::NotDetermined => {
            "Bluetooth access will be requested when connecting to a ring"
        }
        _ => "Check System Preferences for Bluetooth access",
    };

    let screen_recording_instructions = match screen_recording_status {
        SystemPermissionStatus::Granted => "Screen Recording access is working properly",
        SystemPermissionStatus::Denied => {
            "Please enable Screen Recording in System Preferences > Privacy & Security > Screen Recording"
        }
        SystemPermissionStatus::NotDetermined => {
            "Screen Recording access will be requested when screen capture is needed"
        }
        _ => "Check System Preferences for Screen Recording access",
    };

    // Only include real OS-level permissions here.
    let permissions = vec![
        serde_json::json!({
            "id": "microphone",
            "name": "Microphone",
            "description": "Required for voice commands and speech recognition",
            "status": mic_status.to_string(),
            "required": true,
            "instructions": mic_instructions
        }),
        serde_json::json!({
            "id": "accessibility",
            "name": "Accessibility",
            "description": "Required for global hotkeys and gesture shortcuts",
            "status": accessibility_status.to_string(),
            "required": true,
            "instructions": accessibility_instructions
        }),
        serde_json::json!({
            "id": "bluetooth",
            "name": "Bluetooth",
            "description": "Required for connecting to Haptic Harmony ring",
            "status": bluetooth_status.to_string(),
            "required": false,
            "instructions": bluetooth_instructions
        }),
        serde_json::json!({
            "id": "screen_recording",
            "name": "Screen Recording",
            "description": "Optional: required for screen capture features",
            "status": screen_recording_status.to_string(),
            "required": false,
            "instructions": screen_recording_instructions
        }),
    ];

    let total_count = permissions.len();

    let granted_count = permissions
        .iter()
        .filter(|p| p.get("status").and_then(|v| v.as_str()) == Some("granted"))
        .count();

    let required_count = permissions
        .iter()
        .filter(|p| p.get("required").and_then(|v| v.as_bool()) == Some(true))
        .count();

    let required_granted_count = permissions
        .iter()
        .filter(|p| {
            p.get("required").and_then(|v| v.as_bool()) == Some(true)
                && p.get("status").and_then(|v| v.as_str()) == Some("granted")
        })
        .count();

    let missing_required_count = required_count.saturating_sub(required_granted_count);

    Ok(serde_json::json!({
        "permissions": permissions,
        "total_count": total_count,
        "granted_count": granted_count,
        "required_count": required_count,
        "required_granted_count": required_granted_count,
        "missing_required_count": missing_required_count,
        "summary": {
            // Back-compat keys used by current UI.
            "total": total_count,
            "granted": granted_count,
            "required": required_count,
            // New explicit keys.
            "required_granted": required_granted_count,
            "missing_required": missing_required_count
        }
    }))
}

// ============================================================================
// Session History Commands
// ============================================================================

/// Get all chat sessions (both open and closed)
#[tauri::command]
pub fn get_chat_sessions() -> Result<Vec<crate::window_manager::ChatSession>, String> {
    Ok(crate::window_manager::get_all_sessions())
}

/// Restore a closed chat session
#[tauri::command]
pub fn restore_chat_session(session_id: String) -> Result<(), String> {
    crate::window_manager::restore_chat_session(&session_id)
        .map_err(|e| format!("Failed to restore session: {}", e))
}

/// Create a new chat session
#[tauri::command]
pub fn create_chat_session() -> Result<String, String> {
    crate::window_manager::create_new_chat_session()
        .map_err(|e| format!("Failed to create session: {}", e))
}

/// Get session counts (active, closed)
#[tauri::command]
pub fn get_session_counts() -> Result<(usize, usize), String> {
    Ok(crate::window_manager::get_session_counts())
}

/// Get the conversation history for a session
///
/// Note: This command uses `snake_case` argument names for JS↔Rust interop.
#[tauri::command(rename_all = "snake_case")]
pub fn get_session_history(
    session_id: String,
) -> Result<Vec<crate::window_manager::ConversationMessage>, String> {
    crate::window_manager::get_session_state(&session_id)
        .map(|state| state.messages)
        .ok_or_else(|| format!("Session not found: {}", session_id))
}

/// Get the workspace directory for the current active session
#[tauri::command]
pub fn get_session_workspace() -> Option<String> {
    crate::window_manager::get_active_session_workspace().map(|p| p.display().to_string())
}

/// Get the workspace directory for a specific session by ID.
///
/// Note: This command uses `snake_case` argument names for JS↔Rust interop.
#[tauri::command(rename_all = "snake_case")]
pub fn get_session_workspace_by_id(session_id: String) -> Option<String> {
    crate::window_manager::get_session_state(&session_id)
        .and_then(|s| s.workspace_dir)
        .map(|p| p.display().to_string())
}

/// Set the workspace directory for a session
#[tauri::command]
pub fn set_session_workspace(session_id: String, workspace_path: String) -> Result<(), String> {
    let path = std::path::PathBuf::from(&workspace_path);
    if !path.exists() {
        return Err(format!("Directory does not exist: {}", workspace_path));
    }
    if !path.is_dir() {
        return Err(format!("Path is not a directory: {}", workspace_path));
    }
    crate::window_manager::set_session_workspace(&session_id, path);
    tracing::info!(
        session_id = %session_id,
        workspace = %workspace_path,
        "Workspace directory updated for session"
    );
    Ok(())
}

/// Open a directory picker dialog and set it as the workspace for a session
/// If session_id is provided, sets workspace for that session.
/// Otherwise, sets workspace for the current active session.
///
/// Note: This command uses `snake_case` argument names for JS↔Rust interop.
#[tauri::command(rename_all = "snake_case")]
pub async fn pick_workspace_directory(
    app: tauri::AppHandle,
    session_id: Option<String>,
) -> Result<Option<String>, String> {
    use tauri_plugin_dialog::DialogExt;
    use tokio::sync::oneshot;

    let (tx, rx) = oneshot::channel();

    app.dialog()
        .file()
        .set_title("Select Workspace Directory")
        .pick_folder(move |result| {
            let _ = tx.send(result);
        });

    match rx.await {
        Ok(Some(path)) => {
            let path_str = path.to_string();
            // Use provided session_id or fall back to active session
            let target_session =
                session_id.or_else(crate::window_manager::get_active_chat_for_voice);
            if let Some(sid) = target_session {
                let path_buf = std::path::PathBuf::from(&path_str);
                crate::window_manager::set_session_workspace(&sid, path_buf);
                tracing::info!(
                    session_id = %sid,
                    workspace = %path_str,
                    "Workspace picked and set for session"
                );
            }
            Ok(Some(path_str))
        }
        Ok(None) => Ok(None),
        Err(_) => Err("Dialog was cancelled".to_string()),
    }
}

// ============================================================================
// Session LLM Config Commands (session-scoped, doesn't modify global config)
// ============================================================================

/// Get the session-scoped LLM config for a chat session
/// Returns None if no session-specific override is set (uses global config)
///
/// Note: This command uses `snake_case` argument names for JS↔Rust interop.
#[tauri::command(rename_all = "snake_case")]
pub fn get_session_llm_config(
    session_id: String,
) -> Option<crate::window_manager::SessionLlmConfig> {
    crate::window_manager::get_session_llm_config(&session_id)
}

/// Set the LLM provider for a specific session (doesn't modify global config)
/// This allows users to switch providers mid-conversation without affecting defaults
///
/// Note: This command uses `snake_case` argument names for JS↔Rust interop.
#[tauri::command(rename_all = "snake_case")]
pub fn set_session_llm_provider(session_id: String, provider: String) -> Result<(), String> {
    crate::window_manager::set_session_llm_provider(&session_id, provider.clone());
    tracing::info!(
        session_id = %session_id,
        provider = %provider,
        "Session LLM provider updated (session-scoped)"
    );
    Ok(())
}

/// Set the LLM model for a specific session (doesn't modify global config)
///
/// Note: This command uses `snake_case` argument names for JS↔Rust interop.
#[tauri::command(rename_all = "snake_case")]
pub fn set_session_llm_model(session_id: String, model: String) -> Result<(), String> {
    crate::window_manager::set_session_llm_model(&session_id, model.clone())?;
    tracing::info!(
        session_id = %session_id,
        model = %model,
        "Session LLM model updated (session-scoped)"
    );
    Ok(())
}

/// Clear session LLM config (revert to global config for this session)
///
/// Note: This command uses `snake_case` argument names for JS↔Rust interop.
#[tauri::command(rename_all = "snake_case")]
pub fn clear_session_llm_config(session_id: String) -> Result<(), String> {
    crate::window_manager::clear_session_llm_config(&session_id);
    tracing::info!(session_id = %session_id, "Session LLM config cleared (using global config)");
    Ok(())
}

// =========================================================================
// Session Voice/STT Config Commands (session-scoped, doesn't modify globals)
// =========================================================================

/// Get the session-scoped voice/STT config for a chat session.
///
/// Returns `None` if no session-specific override is set (uses global config).
///
/// Note: This command uses `snake_case` argument names for JS↔Rust interop.
#[tauri::command(rename_all = "snake_case")]
pub fn get_session_voice_config(
    session_id: String,
) -> Option<crate::window_manager::SessionVoiceConfig> {
    crate::window_manager::get_session_voice_config(&session_id)
}

/// Set the STT provider for a specific session (doesn't modify global config).
///
/// Note: This command uses `snake_case` argument names for JS↔Rust interop.
#[tauri::command(rename_all = "snake_case")]
pub fn set_session_voice_provider(session_id: String, provider: String) -> Result<(), String> {
    crate::window_manager::set_session_voice_provider(&session_id, provider.clone());
    tracing::info!(
        session_id = %session_id,
        provider = %provider,
        "Session voice provider updated (session-scoped)"
    );
    Ok(())
}

/// Set the STT model for a specific session (doesn't modify global config).
///
/// Note: This command uses `snake_case` argument names for JS↔Rust interop.
#[tauri::command(rename_all = "snake_case")]
pub fn set_session_voice_model(session_id: String, model: String) -> Result<(), String> {
    crate::window_manager::set_session_voice_model(&session_id, model.clone());
    tracing::info!(
        session_id = %session_id,
        model = %model,
        "Session voice model updated (session-scoped)"
    );
    Ok(())
}

/// Clear session voice config (revert to global config for this session).
///
/// Note: This command uses `snake_case` argument names for JS↔Rust interop.
#[tauri::command(rename_all = "snake_case")]
pub fn clear_session_voice_config(session_id: String) -> Result<(), String> {
    crate::window_manager::clear_session_voice_config(&session_id);
    tracing::info!(session_id = %session_id, "Session voice config cleared (using global config)");
    Ok(())
}

/// Get the effective LLM config for a session (session override or global fallback)
/// Returns (provider, model) tuple
///
/// Note: This command uses `snake_case` argument names for JS↔Rust interop.
#[tauri::command(rename_all = "snake_case")]
pub async fn get_effective_llm_config(session_id: String) -> Result<(String, String), String> {
    let global_cfg = AppConfig::load_async().await;

    // Check for session-specific override first
    if let Some(session_llm) = crate::window_manager::get_session_llm_config(&session_id) {
        let provider = session_llm
            .provider
            .unwrap_or_else(|| global_cfg.llm.primary.clone());

        let fallback_model = || get_model_for_provider(&global_cfg, &provider).unwrap_or_default();
        let model = match session_llm.model {
            Some(m) if crate::llm_validation::is_model_compatible_with_provider(&provider, &m) => m,
            Some(m) => {
                tracing::warn!(
                    session_id = %session_id,
                    provider = %provider,
                    model = %m,
                    "Ignoring incompatible session-scoped model override in get_effective_llm_config"
                );
                fallback_model()
            }
            None => fallback_model(),
        };

        return Ok((provider, model));
    }

    // Fall back to global config
    let provider = global_cfg.llm.primary.clone();
    let model = get_model_for_provider(&global_cfg, &provider).unwrap_or_default();
    Ok((provider, model))
}

/// Helper to get the configured model for a provider from global config
fn get_model_for_provider(cfg: &AppConfig, provider: &str) -> Option<String> {
    match provider {
        "openai" => cfg.llm.openai.as_ref().map(|c| c.model.clone()),
        "anthropic" => cfg.llm.anthropic.as_ref().map(|c| c.model.clone()),
        "grok" => cfg.llm.grok.as_ref().map(|c| c.model.clone()),
        "ollama" => cfg.llm.ollama.as_ref().map(|c| c.model.clone()),
        _ => None,
    }
}

// ============================================================================
// Session Tool and Permission Settings Commands
// ============================================================================

/// Get the tool settings for a session (permission level and enabled tools).
///
/// Note: This command uses `snake_case` argument names for JS↔Rust interop.
#[tauri::command(rename_all = "snake_case")]
pub fn get_session_tool_settings(session_id: String) -> crate::window_manager::SessionToolSettings {
    crate::window_manager::get_session_tool_settings(&session_id)
}

/// Set the permission level for a session
/// Valid levels: "sandbox", "restricted", "full"
///
/// Note: This command uses `snake_case` argument names for JS↔Rust interop.
#[tauri::command(rename_all = "snake_case")]
pub fn set_session_permission_level(session_id: String, level: String) -> Result<(), String> {
    let permission_level = match level.to_lowercase().as_str() {
        "sandbox" => crate::window_manager::SessionPermissionLevel::Sandbox,
        "restricted" => crate::window_manager::SessionPermissionLevel::Restricted,
        "full" => crate::window_manager::SessionPermissionLevel::Full,
        _ => {
            return Err(format!(
                "Invalid permission level: {}. Use 'sandbox', 'restricted', or 'full'",
                level
            ));
        }
    };
    crate::window_manager::set_session_permission_level(&session_id, permission_level);
    tracing::info!(
        session_id = %session_id,
        level = %level,
        "Session permission level updated"
    );
    Ok(())
}

/// Enable or disable a tool for a session.
///
/// Note: This command uses `snake_case` argument names for JS↔Rust interop.
#[tauri::command(rename_all = "snake_case")]
pub fn set_session_tool_enabled(
    session_id: String,
    tool_name: String,
    enabled: bool,
) -> Result<(), String> {
    crate::window_manager::set_session_tool_enabled(&session_id, &tool_name, enabled);
    tracing::info!(
        session_id = %session_id,
        tool = %tool_name,
        enabled = %enabled,
        "Session tool availability updated"
    );
    Ok(())
}

/// Check if a tool is enabled for a session
#[tauri::command]
pub fn is_session_tool_enabled(session_id: String, tool_name: String) -> bool {
    crate::window_manager::is_session_tool_enabled(&session_id, &tool_name)
}

/// Check if an action is allowed based on session permission level
#[tauri::command]
pub fn is_session_action_allowed(session_id: String, is_write_operation: bool) -> bool {
    crate::window_manager::is_action_allowed(&session_id, is_write_operation)
}

/// Check if confirmation is required for an action based on session permission level
#[tauri::command]
pub fn session_requires_confirmation(session_id: String, is_write_operation: bool) -> bool {
    crate::window_manager::requires_confirmation(&session_id, is_write_operation)
}

// ============================================================================
// Task Management Commands
// ============================================================================

use gestura_core::{Task, TaskManager, TaskStatus};
use std::sync::OnceLock;

/// Global task manager instance
static TASK_MANAGER: OnceLock<TaskManager> = OnceLock::new();

/// Get or initialize the global task manager
fn get_task_manager() -> &'static TaskManager {
    TASK_MANAGER.get_or_init(|| {
        // Use the user's home directory for task storage
        let base_dir = dirs::home_dir().unwrap_or_else(|| std::path::PathBuf::from("."));
        TaskManager::new(base_dir)
    })
}

/// Create a new task.
///
/// Note: This command uses `snake_case` argument names for JS↔Rust interop.
#[tauri::command(rename_all = "snake_case")]
pub fn create_task(
    app: tauri::AppHandle,
    session_id: String,
    name: String,
    description: String,
    parent_id: Option<String>,
) -> Result<Task, String> {
    let manager = get_task_manager();
    let task = manager
        .create_task(&session_id, name, description, parent_id)
        .map_err(|e| e.to_string())?;

    // Emit task-created event for frontend reactivity
    let _ = app.emit(
        "task-created",
        serde_json::json!({
            "session_id": session_id,
            "task": &task
        }),
    );

    Ok(task)
}

/// Update a task's status
///
/// Note: This command uses `snake_case` argument names for JS↔Rust interop.
#[tauri::command(rename_all = "snake_case")]
pub fn update_task_status(
    app: tauri::AppHandle,
    session_id: String,
    task_id: String,
    status: String,
) -> Result<(), String> {
    let manager = get_task_manager();
    let task_status = match status.to_lowercase().as_str() {
        "notstarted" | "not_started" => TaskStatus::NotStarted,
        "inprogress" | "in_progress" => TaskStatus::InProgress,
        "completed" => TaskStatus::Completed,
        "cancelled" => TaskStatus::Cancelled,
        _ => {
            return Err(format!(
                "Invalid task status: {}. Use 'notstarted', 'inprogress', 'completed', or 'cancelled'",
                status
            ));
        }
    };
    manager
        .update_task_status(&session_id, &task_id, task_status)
        .map_err(|e| e.to_string())?;

    // Emit task-updated event for frontend reactivity
    let _ = app.emit(
        "task-updated",
        serde_json::json!({
            "session_id": session_id,
            "task_id": task_id,
            "status": status
        }),
    );

    Ok(())
}

/// Update a task's name and/or description
///
/// Note: This command uses `snake_case` argument names for JS↔Rust interop.
#[tauri::command(rename_all = "snake_case")]
pub fn update_task(
    app: tauri::AppHandle,
    session_id: String,
    task_id: String,
    name: Option<String>,
    description: Option<String>,
) -> Result<(), String> {
    let manager = get_task_manager();
    manager
        .update_task(&session_id, &task_id, name.clone(), description.clone())
        .map_err(|e| e.to_string())?;

    // Emit task-updated event for frontend reactivity
    let _ = app.emit(
        "task-updated",
        serde_json::json!({
            "session_id": session_id,
            "task_id": task_id,
            "name": name,
            "description": description
        }),
    );

    Ok(())
}

/// Delete a task.
///
/// Note: This command uses `snake_case` argument names for JS↔Rust interop.
#[tauri::command(rename_all = "snake_case")]
pub fn delete_task(
    app: tauri::AppHandle,
    session_id: String,
    task_id: String,
) -> Result<Task, String> {
    let manager = get_task_manager();
    let task = manager
        .delete_task(&session_id, &task_id)
        .map_err(|e| e.to_string())?;

    // Emit task-deleted event for frontend reactivity
    let _ = app.emit(
        "task-deleted",
        serde_json::json!({
            "session_id": session_id,
            "task_id": task_id
        }),
    );

    Ok(task)
}

/// List all tasks for a session
///
/// Note: This command uses `snake_case` argument names for JS↔Rust interop.
#[tauri::command(rename_all = "snake_case")]
pub fn list_tasks(session_id: String) -> Result<Vec<Task>, String> {
    let manager = get_task_manager();
    manager.list_tasks(&session_id).map_err(|e| e.to_string())
}

/// Get task hierarchy for a session (root tasks with their subtasks).
///
/// Note: This command uses `snake_case` argument names for JS↔Rust interop.
#[tauri::command(rename_all = "snake_case")]
pub fn get_task_hierarchy(session_id: String) -> Result<Vec<(Task, Vec<Task>)>, String> {
    let manager = get_task_manager();
    manager
        .get_hierarchy(&session_id)
        .map_err(|e| e.to_string())
}

/// Break down requirements into a task hierarchy using the LLM.
///
/// This command analyzes the provided requirements text and generates
/// a prioritized task hierarchy with dependencies identified.
///
/// Note: This command uses `snake_case` argument names for JS↔Rust interop.
#[tauri::command(rename_all = "snake_case")]
pub async fn break_down_requirements(
    app: tauri::AppHandle,
    session_id: String,
    requirements: String,
) -> Result<Vec<String>, String> {
    use gestura_core::{AgentContext, AppConfig, llm_provider::select_provider};

    let cfg = AppConfig::load_async().await;
    let provider = select_provider(
        &cfg,
        &AgentContext {
            agent_id: "task_breakdown".into(),
        },
    );

    // Construct a prompt that instructs the LLM to break down requirements
    let prompt = format!(
        r#"You are a project planning assistant. Analyze the following requirements and break them down into a structured task list.

Requirements:
{}

Please respond with a JSON array of tasks. Each task should have:
- "name": A concise task name (max 60 chars)
- "description": A detailed description of what needs to be done
- "priority": "high", "medium", or "low"
- "is_blocking": true if other tasks depend on this, false otherwise
- "parent_name": null for root tasks, or the exact name of the parent task for subtasks

Order tasks by priority and logical execution order. Group related tasks under parent tasks.

Example format:
[
  {{"name": "Setup project structure", "description": "Initialize the project...", "priority": "high", "is_blocking": true, "parent_name": null}},
  {{"name": "Configure build system", "description": "Set up the build...", "priority": "high", "is_blocking": false, "parent_name": "Setup project structure"}}
]

Respond ONLY with the JSON array, no additional text."#,
        requirements
    );

    let response = provider
        .call(&prompt)
        .await
        .map_err(|e| format!("LLM error: {}", e))?;

    // Parse the LLM response as JSON
    let tasks_json: serde_json::Value = serde_json::from_str(&response).map_err(|e| {
        format!(
            "Failed to parse LLM response: {}. Response was: {}",
            e, response
        )
    })?;

    let tasks_array = tasks_json
        .as_array()
        .ok_or_else(|| "LLM response is not a JSON array".to_string())?;

    let manager = get_task_manager();
    let mut created_task_ids = Vec::new();
    let mut name_to_id: std::collections::HashMap<String, String> =
        std::collections::HashMap::new();

    // First pass: create root tasks (no parent)
    for task_json in tasks_array {
        let parent_name = task_json.get("parent_name").and_then(|v| v.as_str());
        if parent_name.is_some() && !parent_name.unwrap().is_empty() {
            continue; // Skip subtasks in first pass
        }

        let name = task_json
            .get("name")
            .and_then(|v| v.as_str())
            .unwrap_or("Untitled Task")
            .to_string();

        let priority = task_json
            .get("priority")
            .and_then(|v| v.as_str())
            .unwrap_or("medium");
        let is_blocking = task_json
            .get("is_blocking")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        let description = format!(
            "{}\n\n[Priority: {} | Blocking: {}]",
            task_json
                .get("description")
                .and_then(|v| v.as_str())
                .unwrap_or(""),
            priority,
            if is_blocking { "Yes" } else { "No" }
        );

        let task = manager
            .create_task(&session_id, name.clone(), description, None)
            .map_err(|e| e.to_string())?;

        name_to_id.insert(name, task.id.clone());
        created_task_ids.push(task.id.clone());

        // Emit task-created event
        let _ = app.emit(
            "task-created",
            serde_json::json!({
                "session_id": &session_id,
                "task": &task
            }),
        );
    }

    // Second pass: create subtasks
    for task_json in tasks_array {
        let parent_name = match task_json.get("parent_name").and_then(|v| v.as_str()) {
            Some(name) if !name.is_empty() => name,
            _ => continue, // Skip root tasks
        };

        let parent_id = name_to_id.get(parent_name).cloned();

        let name = task_json
            .get("name")
            .and_then(|v| v.as_str())
            .unwrap_or("Untitled Subtask")
            .to_string();

        let priority = task_json
            .get("priority")
            .and_then(|v| v.as_str())
            .unwrap_or("medium");
        let is_blocking = task_json
            .get("is_blocking")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        let description = format!(
            "{}\n\n[Priority: {} | Blocking: {}]",
            task_json
                .get("description")
                .and_then(|v| v.as_str())
                .unwrap_or(""),
            priority,
            if is_blocking { "Yes" } else { "No" }
        );

        let task = manager
            .create_task(&session_id, name.clone(), description, parent_id)
            .map_err(|e| e.to_string())?;

        name_to_id.insert(name, task.id.clone());
        created_task_ids.push(task.id.clone());

        // Emit task-created event
        let _ = app.emit(
            "task-created",
            serde_json::json!({
                "session_id": &session_id,
                "task": &task
            }),
        );
    }

    Ok(created_task_ids)
}

// ============================================================================
// Knowledge Management Commands
// ============================================================================

use gestura_core::{
    KnowledgeItem, KnowledgeSettingsManager, KnowledgeStore, register_builtin_knowledge,
};

/// Global knowledge store instance
static KNOWLEDGE_STORE: OnceLock<KnowledgeStore> = OnceLock::new();

/// Global knowledge settings manager instance
static KNOWLEDGE_SETTINGS: OnceLock<KnowledgeSettingsManager> = OnceLock::new();

/// Get or initialize the global knowledge store
fn get_knowledge_store() -> &'static KnowledgeStore {
    KNOWLEDGE_STORE.get_or_init(|| {
        let store = KnowledgeStore::with_default_dir();
        register_builtin_knowledge(&store);
        store
    })
}

/// Get or initialize the global knowledge settings manager
fn get_knowledge_settings() -> &'static KnowledgeSettingsManager {
    KNOWLEDGE_SETTINGS.get_or_init(|| {
        let base_dir = dirs::home_dir().unwrap_or_else(|| std::path::PathBuf::from("."));
        KnowledgeSettingsManager::new(base_dir)
    })
}

/// List all available knowledge items
#[tauri::command]
pub fn list_knowledge_items() -> Result<Vec<KnowledgeItem>, String> {
    let store = get_knowledge_store();
    Ok(store.list())
}

/// Get a specific knowledge item by ID
#[tauri::command]
pub fn get_knowledge_item(knowledge_id: String) -> Result<Option<KnowledgeItem>, String> {
    let store = get_knowledge_store();
    Ok(store.get(&knowledge_id))
}

/// Set knowledge enabled/disabled for a session.
///
/// Note: This command uses `snake_case` argument names for JS↔Rust interop.
#[tauri::command(rename_all = "snake_case")]
pub fn set_knowledge_enabled(
    session_id: String,
    knowledge_id: String,
    enabled: bool,
) -> Result<(), String> {
    let settings = get_knowledge_settings();
    settings
        .set_knowledge_enabled(&session_id, &knowledge_id, enabled)
        .map_err(|e| e.to_string())
}

/// Get list of enabled knowledge IDs for a session.
///
/// Note: This command uses `snake_case` argument names for JS↔Rust interop.
#[tauri::command(rename_all = "snake_case")]
pub fn get_enabled_knowledge(session_id: String) -> Result<Vec<String>, String> {
    let settings = get_knowledge_settings();
    settings
        .get_enabled_knowledge(&session_id)
        .map_err(|e| e.to_string())
}

// ============================================================================
// Voice Listener Control Commands
// ============================================================================

/// Validation result for voice configuration
#[derive(Debug, Clone, serde::Serialize)]
pub struct VoiceConfigValidation {
    pub is_valid: bool,
    pub provider: String,
    pub error_code: Option<String>,
    pub error_message: Option<String>,
    pub suggestion: Option<String>,
}

/// Validate voice/STT configuration before starting listener (sync version for internal use)
pub fn validate_voice_config_sync() -> VoiceConfigValidation {
    let config = crate::AppConfig::load();
    validate_voice_config_with_config(&config)
}

/// Validate voice/STT configuration before starting listener
#[tauri::command]
pub async fn validate_voice_config() -> VoiceConfigValidation {
    let config = crate::AppConfig::load_async().await;
    validate_voice_config_with_config(&config)
}

/// Internal helper to validate voice config with a given config
fn validate_voice_config_with_config(config: &crate::AppConfig) -> VoiceConfigValidation {
    let provider = config.voice.provider.as_str();

    match provider {
        "local" => {
            // Check if local model is configured
            let model_path = config
                .voice
                .local_model_path
                .as_ref()
                .map(std::path::PathBuf::from)
                .unwrap_or_else(crate::AppConfig::default_whisper_model_path);

            if !model_path.exists() {
                return VoiceConfigValidation {
                    is_valid: false,
                    provider: "local".to_string(),
                    error_code: Some("LOCAL_MODEL_NOT_FOUND".to_string()),
                    error_message: Some("Local Whisper model not found.".to_string()),
                    suggestion: Some(
                        "Download a Whisper model in Settings → Voice & Audio → Local Whisper."
                            .to_string(),
                    ),
                };
            }

            // Validate the model file
            let validation = crate::voice::validate_whisper_model(&model_path);
            if !validation.is_valid {
                return VoiceConfigValidation {
                    is_valid: false,
                    provider: "local".to_string(),
                    error_code: Some("LOCAL_MODEL_INVALID".to_string()),
                    error_message: Some(
                        validation
                            .error
                            .unwrap_or_else(|| "Invalid Whisper model file.".to_string()),
                    ),
                    suggestion: Some(
                        "The model file may be corrupted. Try downloading it again.".to_string(),
                    ),
                };
            }

            VoiceConfigValidation {
                is_valid: true,
                provider: "local".to_string(),
                error_code: None,
                error_message: None,
                suggestion: None,
            }
        }
        "openai" => {
            // Check if OpenAI API key is configured (config file, keychain, or LLM fallback)
            let config_key = config.voice.openai_api_key.as_deref().unwrap_or("");

            // Try keychain fallback if config key is empty
            let has_api_key = if !config_key.is_empty() {
                true
            } else {
                // Check keychain: voice-specific key first, then general OpenAI key
                let voice_key = try_get_api_key_from_keychain_sync("voice_openai");
                if !voice_key.is_empty() {
                    true
                } else {
                    let general_key = try_get_api_key_from_keychain_sync("openai");
                    if !general_key.is_empty() {
                        true
                    } else {
                        // Fallback to LLM OpenAI config
                        config
                            .llm
                            .openai
                            .as_ref()
                            .is_some_and(|c| !c.api_key.is_empty())
                    }
                }
            };

            if !has_api_key {
                return VoiceConfigValidation {
                    is_valid: false,
                    provider: "openai".to_string(),
                    error_code: Some("OPENAI_API_KEY_MISSING".to_string()),
                    error_message: Some("OpenAI API key not configured.".to_string()),
                    suggestion: Some(
                        "Add your OpenAI API key in Settings → AI Providers → OpenAI.".to_string(),
                    ),
                };
            }

            VoiceConfigValidation {
                is_valid: true,
                provider: "openai".to_string(),
                error_code: None,
                error_message: None,
                suggestion: None,
            }
        }
        "none" | "" => VoiceConfigValidation {
            is_valid: false,
            provider: provider.to_string(),
            error_code: Some("NO_PROVIDER_CONFIGURED".to_string()),
            error_message: Some("No speech-to-text provider configured.".to_string()),
            suggestion: Some(
                "Configure a speech-to-text provider in Settings → Voice & Audio.".to_string(),
            ),
        },
        _ => VoiceConfigValidation {
            is_valid: false,
            provider: provider.to_string(),
            error_code: Some("UNKNOWN_PROVIDER".to_string()),
            error_message: Some(format!("Unknown speech-to-text provider: {}", provider)),
            suggestion: Some(
                "Select a valid provider (Local Whisper or OpenAI) in Settings.".to_string(),
            ),
        },
    }
}

/// Extended validation that also ensures a usable LLM provider is configured so
/// the full voice → STT → LLM agent loop can run (sync version for internal use).
pub fn validate_voice_and_llm_config_sync() -> VoiceConfigValidation {
    let config = crate::AppConfig::load();
    let stt_validation = validate_voice_config_with_config(&config);
    if !stt_validation.is_valid {
        return stt_validation;
    }
    validate_llm_config_with_config(&config, stt_validation)
}

/// Extended validation that also ensures a usable LLM provider is configured so
/// the full voice → STT → LLM agent loop can run.
pub async fn validate_voice_and_llm_config() -> VoiceConfigValidation {
    let config = crate::AppConfig::load_async().await;
    let stt_validation = validate_voice_config_with_config(&config);
    if !stt_validation.is_valid {
        return stt_validation;
    }
    validate_llm_config_with_config(&config, stt_validation)
}

/// Internal helper to validate LLM config with a given config
fn validate_llm_config_with_config(
    config: &crate::AppConfig,
    stt_validation: VoiceConfigValidation,
) -> VoiceConfigValidation {
    let llm_primary_raw = config.llm.primary.trim();
    let llm_primary = llm_primary_raw.to_lowercase();

    // Helper to construct LLM-related validation errors
    let llm_error = |code: &str, message: &str, suggestion: &str| VoiceConfigValidation {
        is_valid: false,
        provider: format!("llm:{}", llm_primary),
        error_code: Some(code.to_string()),
        error_message: Some(message.to_string()),
        suggestion: Some(suggestion.to_string()),
    };

    if llm_primary.is_empty() {
        return llm_error(
            "LLM_PROVIDER_MISSING",
            "No LLM provider configured.",
            "Select and configure an LLM provider in Settings → AI Providers.",
        );
    }

    // Echo does not require any external configuration and is always valid
    if llm_primary == "echo" {
        return stt_validation;
    }

    match llm_primary.as_str() {
        "openai" => {
            if let Some(c) = &config.llm.openai {
                if c.api_key.trim().is_empty() {
                    return llm_error(
                        "LLM_CONFIG_INCOMPLETE",
                        "OpenAI LLM provider is selected but API key is missing.",
                        "Add your OpenAI API key in Settings → AI Providers → OpenAI.",
                    );
                }
                if c.model.trim().is_empty() {
                    return llm_error(
                        "LLM_CONFIG_INCOMPLETE",
                        "OpenAI LLM provider is selected but no model is configured.",
                        "Choose a chat model for OpenAI in Settings → AI Providers.",
                    );
                }
            } else {
                return llm_error(
                    "LLM_CONFIG_MISSING",
                    "OpenAI LLM provider is selected but not configured.",
                    "Fill in OpenAI LLM settings under Settings → AI Providers → OpenAI.",
                );
            }
        }
        "anthropic" => {
            if let Some(c) = &config.llm.anthropic {
                if c.api_key.trim().is_empty() {
                    return llm_error(
                        "LLM_CONFIG_INCOMPLETE",
                        "Anthropic LLM provider is selected but API key is missing.",
                        "Add your Anthropic API key in Settings → AI Providers → Anthropic.",
                    );
                }
                if c.model.trim().is_empty() {
                    return llm_error(
                        "LLM_CONFIG_INCOMPLETE",
                        "Anthropic LLM provider is selected but no model is configured.",
                        "Choose a Claude model in Settings → AI Providers → Anthropic.",
                    );
                }
            } else {
                return llm_error(
                    "LLM_CONFIG_MISSING",
                    "Anthropic LLM provider is selected but not configured.",
                    "Fill in Anthropic LLM settings under Settings → AI Providers → Anthropic.",
                );
            }
        }
        "grok" => {
            if let Some(c) = &config.llm.grok {
                if c.api_key.trim().is_empty() {
                    return llm_error(
                        "LLM_CONFIG_INCOMPLETE",
                        "Grok LLM provider is selected but API key is missing.",
                        "Add your Grok API key in Settings → AI Providers → Grok.",
                    );
                }
                if c.model.trim().is_empty() {
                    return llm_error(
                        "LLM_CONFIG_INCOMPLETE",
                        "Grok LLM provider is selected but no model is configured.",
                        "Choose a Grok model in Settings → AI Providers → Grok.",
                    );
                }
            } else {
                return llm_error(
                    "LLM_CONFIG_MISSING",
                    "Grok LLM provider is selected but not configured.",
                    "Fill in Grok LLM settings under Settings → AI Providers → Grok.",
                );
            }
        }
        "ollama" => {
            if let Some(c) = &config.llm.ollama {
                if c.base_url.trim().is_empty() {
                    return llm_error(
                        "LLM_CONFIG_INCOMPLETE",
                        "Ollama LLM provider is selected but server URL is missing.",
                        "Set the Ollama server URL (for example http://localhost:11434) in Settings → AI Providers → Ollama.",
                    );
                }
                if c.model.trim().is_empty() {
                    return llm_error(
                        "LLM_CONFIG_INCOMPLETE",
                        "Ollama LLM provider is selected but no model is configured.",
                        "Select an Ollama model in Settings → AI Providers → Ollama.",
                    );
                }
            } else {
                return llm_error(
                    "LLM_CONFIG_MISSING",
                    "Ollama LLM provider is selected but not configured.",
                    "Fill in Ollama settings under Settings → AI Providers → Ollama.",
                );
            }
        }
        _ => {
            return llm_error(
                "LLM_PROVIDER_UNKNOWN",
                &format!("Unknown LLM provider: {}", llm_primary_raw),
                "Select a valid LLM provider (OpenAI, Anthropic, Grok, Ollama, or Echo) in Settings → AI Providers.",
            );
        }
    }

    // Both STT and LLM configuration look good; report overall success.
    stt_validation
}

/// Start voice listening with validation shared with the tray logic.
///
/// This command is typically triggered from the chat UI. It delegates to the
/// tray module so that both chat and tray use the exact same validation and
/// speech start pipeline.
#[tauri::command]
pub async fn start_voice_listening(app: tauri::AppHandle) -> Result<String, String> {
    crate::tray::start_listening_with_validation(&app)?;
    let provider = crate::AppConfig::load_async().await.voice.provider;
    Ok(format!("Voice listening started (provider: {})", provider))
}

/// Stop voice listening
#[tauri::command]
pub fn stop_voice_listening(app: tauri::AppHandle) -> Result<String, String> {
    // Stop the speech processing (audio recording)
    if let Err(e) = crate::speech::stop_speech_listening() {
        tracing::warn!("Failed to stop speech processing: {}", e);
    }
    // Update the listening state
    crate::tray::stop_listening();

    // Emit event to notify frontend that listening has stopped
    if let Err(e) = app.emit(
        "listening-state-changed",
        serde_json::json!({
            "is_listening": false
        }),
    ) {
        tracing::warn!("Failed to emit listening-state-changed: {}", e);
    }
    tracing::info!("Emitted listening-state-changed event (stopped via API)");

    Ok("Voice listening stopped".to_string())
}

/// Complete the onboarding process and mark it as done
#[tauri::command]
pub async fn complete_onboarding() -> Result<(), String> {
    tracing::info!("Onboarding completed by user");
    // Save a default config to mark first run as complete
    let config = AppConfig::load_async().await;
    config.save_async().await.map_err(|e| e.to_string())?;
    Ok(())
}

/// Close the onboarding window
#[tauri::command]
pub fn close_onboarding_window() -> Result<(), String> {
    crate::window_manager::close_onboarding().map_err(|e| e.to_string())
}

/// Open system preferences to a specific pane
#[tauri::command]
pub fn open_system_preferences(pane: String) -> Result<(), String> {
    use crate::permissions::open_system_preferences as open_prefs;
    if open_prefs(&pane) {
        Ok(())
    } else {
        Err(format!("Failed to open System Preferences for {}", pane))
    }
}

/// Update voice provider setting
#[tauri::command]
pub async fn update_voice_provider(provider: String) -> Result<(), String> {
    let mut cfg = AppConfig::load_async().await;
    cfg.voice.provider = provider.clone();
    cfg.save_async().await.map_err(|e| e.to_string())?;
    tracing::info!("Voice provider updated to: {}", provider);
    Ok(())
}

/// Update whisper model setting
#[tauri::command(rename_all = "snake_case")]
pub async fn update_whisper_model(model_filename: String) -> Result<(), String> {
    let mut cfg = AppConfig::load_async().await;
    let models_dir = AppConfig::whisper_models_dir();
    let model_path = models_dir.join(&model_filename);
    cfg.voice.local_model_path = Some(model_path.to_string_lossy().to_string());
    cfg.save_async().await.map_err(|e| e.to_string())?;
    tracing::info!("Whisper model updated to: {}", model_filename);
    Ok(())
}

/// Update LLM provider setting
#[tauri::command]
pub async fn update_llm_provider(provider: String) -> Result<(), String> {
    let mut cfg = AppConfig::load_async().await;
    cfg.llm.primary = provider.clone();
    cfg.save_async().await.map_err(|e| e.to_string())?;
    tracing::info!("LLM provider updated to: {}", provider);
    Ok(())
}

/// Update selected audio input device
#[tauri::command]
pub async fn update_audio_device(device_name: Option<String>) -> Result<(), String> {
    let mut cfg = AppConfig::load_async().await;
    cfg.voice.audio_device = device_name.clone();
    cfg.save_async().await.map_err(|e| e.to_string())?;
    tracing::info!("Audio device updated to: {:?}", device_name);
    Ok(())
}

/// Update Ollama configuration (URL and model)
#[tauri::command]
pub async fn update_ollama_config(base_url: String, model: String) -> Result<(), String> {
    let mut cfg = AppConfig::load_async().await;
    cfg.llm.ollama = Some(crate::config::OllamaConfig {
        base_url: base_url.clone(),
        model: model.clone(),
    });
    cfg.save_async().await.map_err(|e| e.to_string())?;
    tracing::info!("Ollama config updated: url={}, model={}", base_url, model);
    Ok(())
}

/// Get notification settings
#[tauri::command]
pub async fn get_notification_settings()
-> Result<gestura_core::config::NotificationSettings, String> {
    let cfg = AppConfig::load_async().await;
    Ok(cfg.notifications)
}

/// Update notification settings
#[tauri::command]
#[allow(clippy::too_many_arguments)] // Tauri command parameters expanded for fine-grained frontend control
pub async fn update_notification_settings(
    sound_enabled: Option<bool>,
    haptic_enabled: Option<bool>,
    sound_volume: Option<u8>,
    haptic_intensity: Option<u8>,
    notification_sound: Option<String>,
    command_confirm_sound: Option<String>,
    mcp_feedback_enabled: Option<bool>,
    auto_listen_on_feedback: Option<bool>,
) -> Result<(), String> {
    let mut cfg = AppConfig::load_async().await;

    if let Some(v) = sound_enabled {
        cfg.notifications.sound_enabled = v;
    }
    if let Some(v) = haptic_enabled {
        cfg.notifications.haptic_enabled = v;
    }
    if let Some(v) = sound_volume {
        cfg.notifications.sound_volume = v.min(100);
    }
    if let Some(v) = haptic_intensity {
        cfg.notifications.haptic_intensity = v.min(100);
    }
    if let Some(v) = notification_sound {
        cfg.notifications.notification_sound = normalize_notification_sound_choice(&v);
    }
    if let Some(v) = command_confirm_sound {
        cfg.notifications.command_confirm_sound = normalize_command_confirm_sound_choice(&v);
    }
    if let Some(v) = mcp_feedback_enabled {
        cfg.notifications.mcp_feedback_enabled = v;
    }
    if let Some(v) = auto_listen_on_feedback {
        cfg.notifications.auto_listen_on_feedback = v;
    }

    cfg.save_async().await.map_err(|e| e.to_string())?;
    tracing::info!("Notification settings updated");
    Ok(())
}

/// Preview a user-selected notification sound at the given volume.
///
/// This is used by the config UI to provide immediate auditory feedback when
/// a sound option is selected.
#[tauri::command]
pub async fn preview_notification_sound(sound: String, volume: Option<u8>) -> Result<(), String> {
    use crate::notifications::get_notification_manager;
    get_notification_manager()
        .preview_sound(&sound, volume)
        .await;
    Ok(())
}

fn normalize_notification_sound_choice(value: &str) -> String {
    match value {
        "default" | "chime" | "ping" | "pop" | "subtle" | "none" => value.to_string(),
        _ => "default".to_string(),
    }
}

fn normalize_command_confirm_sound_choice(value: &str) -> String {
    match value {
        "default" | "success" | "click" | "beep" | "none" => value.to_string(),
        _ => "default".to_string(),
    }
}

/// Set the connected ring device for haptic notifications
#[tauri::command]
pub fn set_notification_ring(device_id: Option<String>) -> Result<(), String> {
    use crate::notifications::get_notification_manager;
    get_notification_manager().set_connected_ring(device_id.clone());
    tracing::info!("Notification ring set to: {:?}", device_id);
    Ok(())
}

/// Test notification (for settings UI)
#[tauri::command]
pub async fn test_notification(
    app: tauri::AppHandle,
    notification_type: String,
) -> Result<(), String> {
    use crate::notifications::{NotificationType, get_notification_manager};

    let ntype = match notification_type.as_str() {
        "response_complete" => NotificationType::ResponseComplete,
        "mcp_feedback" => NotificationType::McpFeedbackRequest,
        "error" => NotificationType::Error,
        "listening_started" => NotificationType::ListeningStarted,
        "listening_stopped" => NotificationType::ListeningStopped,
        _ => return Err(format!("Unknown notification type: {}", notification_type)),
    };

    get_notification_manager().notify(ntype, Some(&app)).await;

    Ok(())
}

// ============================================================================
// Secure Secret Management Commands
// ============================================================================

/// Store a secret in secure storage (keychain on macOS, credential store on Windows/Linux)
/// Falls back to mock storage if security feature is disabled.
#[tauri::command]
pub async fn store_secret(key: String, value: String) -> Result<(), String> {
    let storage = crate::security::create_secure_storage();
    storage
        .store_secret(&key, &value)
        .await
        .map_err(|e| e.to_string())?;
    tracing::info!("Secret stored: {}", key);
    Ok(())
}

/// Retrieve a secret from secure storage.
/// Returns None if the secret doesn't exist.
#[tauri::command]
pub async fn get_secret(key: String) -> Result<Option<String>, String> {
    let storage = crate::security::create_secure_storage();
    storage.get_secret(&key).await.map_err(|e| e.to_string())
}

/// Delete a secret from secure storage.
#[tauri::command]
pub async fn delete_secret(key: String) -> Result<(), String> {
    let storage = crate::security::create_secure_storage();
    storage
        .delete_secret(&key)
        .await
        .map_err(|e| e.to_string())?;
    tracing::info!("Secret deleted: {}", key);
    Ok(())
}

/// Check if the security feature (keychain integration) is available.
#[tauri::command]
pub fn is_keychain_available() -> bool {
    #[cfg(feature = "security")]
    {
        true
    }
    #[cfg(not(feature = "security"))]
    {
        false
    }
}

/// Store an API key securely.
///
/// Convenience wrapper that uses provider-specific key names.
/// Provider can be: "openai", "anthropic", "grok", "serpapi", "brave".
///
/// JS↔Rust interop: This command is exposed to the frontend via Tauri.
#[tauri::command(rename_all = "snake_case")]
pub async fn store_api_key(provider: String, api_key: String) -> Result<(), String> {
    let key = format!("gestura_api_key_{}", provider.to_lowercase());
    store_secret(key, api_key).await
}

/// Retrieve an API key from secure storage.
///
/// JS↔Rust interop: This command is exposed to the frontend via Tauri.
#[tauri::command(rename_all = "snake_case")]
pub async fn get_api_key(provider: String) -> Result<Option<String>, String> {
    let key = format!("gestura_api_key_{}", provider.to_lowercase());
    get_secret(key).await
}

/// Delete an API key from secure storage.
///
/// JS↔Rust interop: This command is exposed to the frontend via Tauri.
#[tauri::command(rename_all = "snake_case")]
pub async fn delete_api_key(provider: String) -> Result<(), String> {
    let key = format!("gestura_api_key_{}", provider.to_lowercase());
    delete_secret(key).await
}

/// Migrate API keys from config file to secure storage.
/// This is a one-time operation for existing users.
#[tauri::command]
pub async fn migrate_api_keys_to_keychain() -> Result<serde_json::Value, String> {
    let cfg = AppConfig::load_async().await;
    let storage = crate::security::create_secure_storage();
    let mut migrated: Vec<String> = Vec::new();

    // Migrate OpenAI key
    if let Some(ref openai) = cfg.llm.openai
        && !openai.api_key.is_empty()
    {
        storage
            .store_secret("gestura_api_key_openai", &openai.api_key)
            .await
            .map_err(|e| e.to_string())?;
        migrated.push("openai".to_string());
    }

    // Migrate Anthropic key
    if let Some(ref anthropic) = cfg.llm.anthropic
        && !anthropic.api_key.is_empty()
    {
        storage
            .store_secret("gestura_api_key_anthropic", &anthropic.api_key)
            .await
            .map_err(|e| e.to_string())?;
        migrated.push("anthropic".to_string());
    }

    // Migrate Grok key
    if let Some(ref grok) = cfg.llm.grok
        && !grok.api_key.is_empty()
    {
        storage
            .store_secret("gestura_api_key_grok", &grok.api_key)
            .await
            .map_err(|e| e.to_string())?;
        migrated.push("grok".to_string());
    }

    // Migrate SerpAPI key
    if let Some(ref key) = cfg.web_search.serpapi_key
        && !key.is_empty()
    {
        storage
            .store_secret("gestura_api_key_serpapi", key)
            .await
            .map_err(|e| e.to_string())?;
        migrated.push("serpapi".to_string());
    }

    // Migrate Brave key
    if let Some(ref key) = cfg.web_search.brave_key
        && !key.is_empty()
    {
        storage
            .store_secret("gestura_api_key_brave", key)
            .await
            .map_err(|e| e.to_string())?;
        migrated.push("brave".to_string());
    }

    // Migrate Voice/STT OpenAI key (separate from LLM OpenAI key)
    if let Some(ref key) = cfg.voice.openai_api_key
        && !key.is_empty()
    {
        storage
            .store_secret("gestura_api_key_voice_openai", key)
            .await
            .map_err(|e| e.to_string())?;
        migrated.push("voice_openai".to_string());
    }

    tracing::info!("Migrated {} API keys to secure storage", migrated.len());

    Ok(serde_json::json!({
        "migrated": migrated,
        "count": migrated.len()
    }))
}

/// Get the current system theme (light or dark).
/// Returns "light" or "dark" based on the system's appearance settings.
#[tauri::command]
pub fn get_system_theme() -> String {
    if is_system_dark_mode() {
        "dark".to_string()
    } else {
        "light".to_string()
    }
}

/// Detect if the system is using dark mode (macOS-specific).
#[cfg(target_os = "macos")]
fn is_system_dark_mode() -> bool {
    use std::process::Command;

    // Query macOS for the current appearance setting
    let output = Command::new("defaults")
        .args(["read", "-g", "AppleInterfaceStyle"])
        .output();

    match output {
        Ok(output) => {
            let result = String::from_utf8_lossy(&output.stdout);
            result.trim().eq_ignore_ascii_case("dark")
        }
        Err(_) => {
            // If the command fails or the key doesn't exist, assume light mode
            false
        }
    }
}

/// Detect if the system is using dark mode (non-macOS platforms).
#[cfg(not(target_os = "macos"))]
fn is_system_dark_mode() -> bool {
    // Default to light mode on non-macOS platforms
    // TODO: Add Windows/Linux dark mode detection
    false
}
