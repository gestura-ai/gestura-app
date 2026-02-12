//! MCP Tool Discovery and Registry
//!
//! Provides unified tool discovery from external MCP servers, capability negotiation,
//! and tool metadata caching for performance.

#[cfg(test)]
use super::types::ToolAnnotations;
use super::types::{Tool, ToolsCapability};
use crate::execution_mode::ToolCategory;
use crate::tool_inspection::ToolMetadata;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::RwLock;
use std::time::{Duration, Instant};

/// Configuration for MCP server connection
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpServerConfig {
    /// Server name/identifier
    pub name: String,
    /// Server URI (e.g., "stdio://path/to/server" or "http://localhost:3000")
    pub uri: String,
    /// Whether this server is enabled
    pub enabled: bool,
    /// Connection timeout in seconds
    pub timeout_secs: u64,
    /// Auto-reconnect on failure
    pub auto_reconnect: bool,
}

impl Default for McpServerConfig {
    fn default() -> Self {
        Self {
            name: "default".to_string(),
            uri: String::new(),
            enabled: true,
            timeout_secs: 30,
            auto_reconnect: true,
        }
    }
}

/// Cached tool information from an MCP server
#[derive(Debug, Clone)]
pub struct CachedTool {
    /// The MCP tool definition
    pub tool: Tool,
    /// Derived metadata for permission checking
    pub metadata: ToolMetadata,
    /// Source server name
    pub server_name: String,
    /// When this was cached
    pub cached_at: Instant,
}

/// MCP server connection state
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ServerState {
    /// Not connected
    Disconnected,
    /// Connecting
    Connecting,
    /// Connected and ready
    Connected,
    /// Connection failed
    Failed,
}

/// Information about a connected MCP server
#[derive(Debug, Clone)]
pub struct ServerInfo {
    /// Server configuration
    pub config: McpServerConfig,
    /// Current connection state
    pub state: ServerState,
    /// Server's advertised tools capability
    pub tools_capability: Option<ToolsCapability>,
    /// Number of tools available
    pub tool_count: usize,
    /// Last successful connection time
    pub last_connected: Option<Instant>,
    /// Last error message
    pub last_error: Option<String>,
}

/// MCP Tool Discovery Manager
///
/// Manages connections to external MCP servers, discovers available tools,
/// and caches tool metadata for performance.
pub struct McpDiscoveryManager {
    /// Registered MCP servers
    servers: RwLock<HashMap<String, ServerInfo>>,
    /// Cached tools from all servers
    tool_cache: RwLock<HashMap<String, CachedTool>>,
    /// Cache TTL
    cache_ttl: Duration,
}

impl Default for McpDiscoveryManager {
    fn default() -> Self {
        Self::new()
    }
}

impl McpDiscoveryManager {
    /// Create a new discovery manager
    pub fn new() -> Self {
        Self {
            servers: RwLock::new(HashMap::new()),
            tool_cache: RwLock::new(HashMap::new()),
            cache_ttl: Duration::from_secs(300), // 5 minutes default
        }
    }

    /// Create with custom cache TTL
    pub fn with_cache_ttl(cache_ttl: Duration) -> Self {
        Self {
            servers: RwLock::new(HashMap::new()),
            tool_cache: RwLock::new(HashMap::new()),
            cache_ttl,
        }
    }

    /// Register an MCP server
    pub fn register_server(&self, config: McpServerConfig) {
        if let Ok(mut servers) = self.servers.write() {
            let info = ServerInfo {
                config: config.clone(),
                state: ServerState::Disconnected,
                tools_capability: None,
                tool_count: 0,
                last_connected: None,
                last_error: None,
            };
            servers.insert(config.name.clone(), info);
            tracing::info!("Registered MCP server: {}", config.name);
        }
    }

    /// Unregister an MCP server
    pub fn unregister_server(&self, name: &str) {
        if let Ok(mut servers) = self.servers.write() {
            servers.remove(name);
            tracing::info!("Unregistered MCP server: {}", name);
        }
        // Also remove cached tools from this server
        if let Ok(mut cache) = self.tool_cache.write() {
            cache.retain(|_, v| v.server_name != name);
        }
    }

    /// Get all registered servers
    pub fn list_servers(&self) -> Vec<ServerInfo> {
        self.servers
            .read()
            .map(|s| s.values().cloned().collect())
            .unwrap_or_default()
    }

    /// Get server info by name
    pub fn get_server(&self, name: &str) -> Option<ServerInfo> {
        self.servers.read().ok().and_then(|s| s.get(name).cloned())
    }

    /// Update server state
    pub fn update_server_state(&self, name: &str, state: ServerState, error: Option<String>) {
        if let Ok(mut servers) = self.servers.write()
            && let Some(info) = servers.get_mut(name)
        {
            info.state = state;
            if state == ServerState::Connected {
                info.last_connected = Some(Instant::now());
                info.last_error = None;
            } else if let Some(err) = error {
                info.last_error = Some(err);
            }
        }
    }

    /// Cache tools from a server
    pub fn cache_tools(&self, server_name: &str, tools: Vec<Tool>) {
        let now = Instant::now();
        if let Ok(mut cache) = self.tool_cache.write() {
            for tool in tools {
                let metadata = self.derive_metadata(&tool, server_name);
                let key = format!("{}:{}", server_name, tool.name);
                cache.insert(
                    key,
                    CachedTool {
                        tool,
                        metadata,
                        server_name: server_name.to_string(),
                        cached_at: now,
                    },
                );
            }
        }
        // Update server tool count
        if let Ok(mut servers) = self.servers.write()
            && let Some(info) = servers.get_mut(server_name)
        {
            info.tool_count = self
                .tool_cache
                .read()
                .map(|c| c.values().filter(|t| t.server_name == server_name).count())
                .unwrap_or(0);
        }
    }

    /// Derive ToolMetadata from MCP Tool definition
    fn derive_metadata(&self, tool: &Tool, server_name: &str) -> ToolMetadata {
        let category = self.infer_category(tool);
        let risk_level = self.infer_risk_level(tool, category);
        let has_side_effects = tool
            .annotations
            .as_ref()
            .map(|a| a.destructive_hint || a.open_world_hint)
            .unwrap_or(category != ToolCategory::ReadOnly);

        ToolMetadata {
            name: format!("{}:{}", server_name, tool.name),
            description: tool
                .description
                .clone()
                .unwrap_or_else(|| format!("MCP tool from {}", server_name)),
            category,
            has_side_effects,
            risk_level,
            required_capabilities: vec!["mcp".to_string(), server_name.to_string()],
        }
    }

    /// Infer tool category from MCP tool definition
    fn infer_category(&self, tool: &Tool) -> ToolCategory {
        let name = tool.name.to_lowercase();
        let desc = tool
            .description
            .as_ref()
            .map(|s| s.to_lowercase())
            .unwrap_or_default();

        // Check annotations first
        if let Some(annotations) = &tool.annotations
            && annotations.destructive_hint
        {
            return ToolCategory::Write;
        }

        // Infer from name/description
        if name.contains("read") || name.contains("get") || name.contains("list") {
            ToolCategory::ReadOnly
        } else if name.contains("write")
            || name.contains("create")
            || name.contains("delete")
            || name.contains("update")
        {
            ToolCategory::Write
        } else if name.contains("shell")
            || name.contains("exec")
            || name.contains("run")
            || name.contains("command")
        {
            ToolCategory::Shell
        } else if name.contains("git") || desc.contains("git") {
            ToolCategory::Git
        } else if name.contains("http")
            || name.contains("fetch")
            || name.contains("request")
            || desc.contains("network")
        {
            ToolCategory::Network
        } else {
            // Default to Shell (most restrictive) for unknown tools
            ToolCategory::Shell
        }
    }

    /// Infer risk level from tool definition
    fn infer_risk_level(&self, tool: &Tool, category: ToolCategory) -> u8 {
        let base_risk = match category {
            ToolCategory::ReadOnly => 0,
            ToolCategory::Network => 2,
            ToolCategory::Write => 4,
            ToolCategory::Git => 5,
            ToolCategory::Shell => 7,
            ToolCategory::System => 9,
        };

        // Adjust based on annotations
        if let Some(annotations) = &tool.annotations {
            if annotations.destructive_hint {
                return (base_risk + 2).min(10);
            }
            if annotations.idempotent_hint {
                return base_risk.saturating_sub(1);
            }
        }

        base_risk
    }

    /// Get all cached tools
    pub fn list_tools(&self) -> Vec<CachedTool> {
        let now = Instant::now();
        self.tool_cache
            .read()
            .map(|c| {
                c.values()
                    .filter(|t| now.duration_since(t.cached_at) < self.cache_ttl)
                    .cloned()
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Get a specific tool by server:name
    pub fn get_tool(&self, server_name: &str, tool_name: &str) -> Option<CachedTool> {
        let key = format!("{}:{}", server_name, tool_name);
        self.tool_cache
            .read()
            .ok()
            .and_then(|c| c.get(&key).cloned())
    }

    /// Get all tools from a specific server
    pub fn tools_from_server(&self, server_name: &str) -> Vec<CachedTool> {
        self.tool_cache
            .read()
            .map(|c| {
                c.values()
                    .filter(|t| t.server_name == server_name)
                    .cloned()
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Clear expired cache entries
    pub fn clear_expired(&self) {
        let now = Instant::now();
        if let Ok(mut cache) = self.tool_cache.write() {
            cache.retain(|_, v| now.duration_since(v.cached_at) < self.cache_ttl);
        }
    }

    /// Clear all cached tools
    pub fn clear_cache(&self) {
        if let Ok(mut cache) = self.tool_cache.write() {
            cache.clear();
        }
    }

    /// Get cache statistics
    pub fn cache_stats(&self) -> CacheStats {
        let now = Instant::now();
        let (total, expired) = self
            .tool_cache
            .read()
            .map(|c| {
                let total = c.len();
                let expired = c
                    .values()
                    .filter(|t| now.duration_since(t.cached_at) >= self.cache_ttl)
                    .count();
                (total, expired)
            })
            .unwrap_or((0, 0));

        CacheStats {
            total_tools: total,
            expired_tools: expired,
            server_count: self.servers.read().map(|s| s.len()).unwrap_or(0),
        }
    }
}

/// Cache statistics
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CacheStats {
    /// Total cached tools
    pub total_tools: usize,
    /// Expired tools (not yet cleaned)
    pub expired_tools: usize,
    /// Number of registered servers
    pub server_count: usize,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_server_registration() {
        let manager = McpDiscoveryManager::new();

        let config = McpServerConfig {
            name: "test-server".to_string(),
            uri: "stdio://test".to_string(),
            ..Default::default()
        };

        manager.register_server(config);

        let servers = manager.list_servers();
        assert_eq!(servers.len(), 1);
        assert_eq!(servers[0].config.name, "test-server");
        assert_eq!(servers[0].state, ServerState::Disconnected);
    }

    #[test]
    fn test_tool_caching() {
        let manager = McpDiscoveryManager::new();

        let config = McpServerConfig {
            name: "test-server".to_string(),
            uri: "stdio://test".to_string(),
            ..Default::default()
        };
        manager.register_server(config);

        let tools = vec![
            Tool {
                name: "read_file".to_string(),
                description: Some("Read a file".to_string()),
                input_schema: serde_json::json!({}),
                annotations: None,
            },
            Tool {
                name: "write_file".to_string(),
                description: Some("Write to a file".to_string()),
                input_schema: serde_json::json!({}),
                annotations: Some(ToolAnnotations {
                    destructive_hint: true,
                    ..Default::default()
                }),
            },
        ];

        manager.cache_tools("test-server", tools);

        let cached = manager.list_tools();
        assert_eq!(cached.len(), 2);

        // Check category inference
        let read_tool = manager.get_tool("test-server", "read_file").unwrap();
        assert_eq!(read_tool.metadata.category, ToolCategory::ReadOnly);

        let write_tool = manager.get_tool("test-server", "write_file").unwrap();
        assert_eq!(write_tool.metadata.category, ToolCategory::Write);
        assert!(write_tool.metadata.has_side_effects);
    }

    #[test]
    fn test_category_inference() {
        let manager = McpDiscoveryManager::new();

        let shell_tool = Tool {
            name: "run_command".to_string(),
            description: Some("Execute a shell command".to_string()),
            input_schema: serde_json::json!({}),
            annotations: None,
        };
        assert_eq!(manager.infer_category(&shell_tool), ToolCategory::Shell);

        let git_tool = Tool {
            name: "git_status".to_string(),
            description: Some("Get git status".to_string()),
            input_schema: serde_json::json!({}),
            annotations: None,
        };
        assert_eq!(manager.infer_category(&git_tool), ToolCategory::Git);
    }

    #[test]
    fn test_cache_stats() {
        let manager = McpDiscoveryManager::new();

        let config = McpServerConfig {
            name: "test".to_string(),
            uri: "stdio://test".to_string(),
            ..Default::default()
        };
        manager.register_server(config);

        let stats = manager.cache_stats();
        assert_eq!(stats.server_count, 1);
        assert_eq!(stats.total_tools, 0);
    }
}
