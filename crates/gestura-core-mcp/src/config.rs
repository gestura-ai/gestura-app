//! MCP server configuration types.
//!
//! These types represent the “Claude Desktop / Claude Code” compatible MCP
//! configuration schema and are used by the MCP client registry.

use crate::discovery::McpServerConfig;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

fn default_true() -> bool {
    true
}

fn default_mcp_timeout() -> u64 {
    30
}

/// MCP tool entry (basic) — **DEPRECATED**: use [`McpServerEntry`] instead.
///
/// Kept only for backward-compatible deserialization of older config files.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct McpTool {
    pub name: String,
    pub endpoint: String,
}

impl McpTool {
    /// Convert a legacy `McpTool` into the full `McpServerEntry`.
    ///
    /// Heuristic: if `endpoint` looks like a URL (starts with `http://` or
    /// `https://`), assume HTTP transport; otherwise treat as stdio command.
    pub fn to_server_entry(&self) -> McpServerEntry {
        let endpoint_lower = self.endpoint.to_lowercase();
        if endpoint_lower.starts_with("http://") || endpoint_lower.starts_with("https://") {
            McpServerEntry {
                name: self.name.clone(),
                transport: McpTransportType::Http,
                enabled: true,
                url: Some(self.endpoint.clone()),
                ..McpServerEntry::default()
            }
        } else {
            // Treat as a stdio command (e.g. "npx -y @some/server")
            let parts: Vec<&str> = self.endpoint.split_whitespace().collect();
            let (command, args) = if let Some((cmd, rest)) = parts.split_first() {
                (
                    Some((*cmd).to_string()),
                    rest.iter().map(|s| (*s).to_string()).collect(),
                )
            } else {
                (Some(self.endpoint.clone()), vec![])
            };
            McpServerEntry {
                name: self.name.clone(),
                transport: McpTransportType::Stdio,
                enabled: true,
                command,
                args,
                ..McpServerEntry::default()
            }
        }
    }
}

// ============================================================================
// MCP Server Configuration (full spec — Claude Code compatible)
// ============================================================================

/// Transport type for MCP server connections.
///
/// Matches the `type` field in `.mcp.json` configuration files.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum McpTransportType {
    /// Standard I/O transport — server runs as a local child process.
    #[default]
    Stdio,
    /// Streamable HTTP transport (recommended for remote servers).
    Http,
    /// Server-Sent Events transport (deprecated, but still functional).
    Sse,
}

impl std::fmt::Display for McpTransportType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Stdio => write!(f, "stdio"),
            Self::Http => write!(f, "http"),
            Self::Sse => write!(f, "sse"),
        }
    }
}

impl std::str::FromStr for McpTransportType {
    type Err = String;
    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "stdio" => Ok(Self::Stdio),
            "http" | "streamable-http" => Ok(Self::Http),
            "sse" => Ok(Self::Sse),
            other => Err(format!(
                "Unknown MCP transport type '{}'. Expected: stdio, http, sse",
                other
            )),
        }
    }
}

/// Configuration scope for MCP servers.
///
/// Determines where the server configuration is stored and its precedence.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum McpScope {
    /// User-level configuration (`~/.gestura/config.yaml`).
    #[default]
    User,
    /// Project-level configuration (`.mcp.json` in repo root).
    Project,
    /// Local-only configuration (`.gestura.json` in project dir).
    Local,
}

impl std::fmt::Display for McpScope {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::User => write!(f, "user"),
            Self::Project => write!(f, "project"),
            Self::Local => write!(f, "local"),
        }
    }
}

impl std::str::FromStr for McpScope {
    type Err = String;
    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "user" => Ok(Self::User),
            "project" => Ok(Self::Project),
            "local" => Ok(Self::Local),
            other => Err(format!(
                "Unknown MCP scope '{}'. Expected: user, project, local",
                other
            )),
        }
    }
}

/// Full MCP server entry compatible with `.mcp.json`.
///
/// # Examples
///
/// ```
/// use gestura_core_mcp::config::{McpServerEntry, McpTransportType};
///
/// let _stdio_server = McpServerEntry {
///     name: "postgres".to_string(),
///     transport: McpTransportType::Stdio,
///     command: Some("npx".to_string()),
///     args: vec!["-y".to_string(), "@anthropic-ai/mcp-server-postgres".to_string()],
///     ..Default::default()
/// };
///
/// let _http_server = McpServerEntry {
///     name: "github".to_string(),
///     transport: McpTransportType::Http,
///     url: Some("https://example.invalid/mcp".to_string()),
///     ..Default::default()
/// };
/// ```
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct McpServerEntry {
    /// Unique server name/identifier.
    pub name: String,

    /// Transport type (`stdio`, `http`, `sse`).
    #[serde(rename = "type", default)]
    pub transport: McpTransportType,

    /// Whether this server is enabled.
    #[serde(default = "default_true")]
    pub enabled: bool,

    // -- stdio-specific fields --
    /// Command to execute (stdio transport).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
    /// Arguments to pass to `command`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub args: Vec<String>,
    /// Environment variables.
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub env: HashMap<String, String>,

    // -- http/sse-specific fields --
    /// HTTP/SSE URL.
    #[serde(default, skip_serializing_if = "Option::is_none", alias = "endpoint")]
    pub url: Option<String>,
    /// HTTP headers to send.
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub headers: HashMap<String, String>,

    // -- common fields --
    /// Configuration scope (user/project/local).
    #[serde(default)]
    pub scope: McpScope,
    /// Connection timeout in seconds.
    #[serde(default = "default_mcp_timeout")]
    pub timeout_secs: u64,
    /// Auto-reconnect on failure.
    #[serde(default = "default_true")]
    pub auto_reconnect: bool,
}

impl Default for McpServerEntry {
    fn default() -> Self {
        Self {
            name: String::new(),
            transport: McpTransportType::default(),
            enabled: true,
            command: None,
            args: Vec::new(),
            env: HashMap::new(),
            url: None,
            headers: HashMap::new(),
            scope: McpScope::default(),
            timeout_secs: 30,
            auto_reconnect: true,
        }
    }
}

impl McpServerEntry {
    /// Return the effective URI for this server.
    ///
    /// For HTTP/SSE this is the `url` field. For stdio, a synthetic
    /// `stdio://<command>` URI is returned for display/logging purposes.
    pub fn effective_uri(&self) -> String {
        match self.transport {
            McpTransportType::Http | McpTransportType::Sse => self.url.clone().unwrap_or_default(),
            McpTransportType::Stdio => {
                let cmd = self.command.as_deref().unwrap_or("unknown");
                format!("stdio://{}", cmd)
            }
        }
    }

    /// Convert to the discovery-layer `McpServerConfig`.
    pub fn to_discovery_config(&self) -> McpServerConfig {
        McpServerConfig {
            name: self.name.clone(),
            uri: self.effective_uri(),
            enabled: self.enabled,
            timeout_secs: self.timeout_secs,
            auto_reconnect: self.auto_reconnect,
        }
    }
}

/// Represents an `.mcp.json` configuration file.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct McpJsonFile {
    #[serde(rename = "mcpServers", default)]
    pub mcp_servers: HashMap<String, McpServerEntry>,
}

impl McpJsonFile {
    /// Read a `.mcp.json` file from `path`.
    pub fn load(path: &std::path::Path) -> std::result::Result<Self, String> {
        let content = std::fs::read_to_string(path)
            .map_err(|e| format!("Failed to read {}: {}", path.display(), e))?;
        let mut parsed: Self = serde_json::from_str(&content)
            .map_err(|e| format!("Failed to parse {}: {}", path.display(), e))?;
        // Ensure each entry's `name` matches the map key.
        for (key, entry) in parsed.mcp_servers.iter_mut() {
            if entry.name.is_empty() {
                entry.name = key.clone();
            }
        }
        Ok(parsed)
    }

    /// Write the file to `path` as pretty-printed JSON.
    pub fn save(&self, path: &std::path::Path) -> std::result::Result<(), String> {
        let json = serde_json::to_string_pretty(self)
            .map_err(|e| format!("Failed to serialize .mcp.json: {}", e))?;
        std::fs::write(path, json).map_err(|e| format!("Failed to write {}: {}", path.display(), e))
    }

    /// Flatten the map into a `Vec<McpServerEntry>` with a given scope.
    pub fn into_entries(self, scope: McpScope) -> Vec<McpServerEntry> {
        self.mcp_servers
            .into_iter()
            .map(|(key, mut entry)| {
                if entry.name.is_empty() {
                    entry.name = key;
                }
                entry.scope = scope;
                entry
            })
            .collect()
    }
}

/// Import MCP servers from Claude Desktop config.
pub fn import_claude_desktop_servers() -> std::result::Result<Vec<McpServerEntry>, String> {
    let config_path = claude_desktop_config_path()
        .ok_or_else(|| "Could not determine Claude Desktop config path".to_string())?;

    if !config_path.exists() {
        return Err(format!(
            "Claude Desktop config not found at {}",
            config_path.display()
        ));
    }

    let mcp_file = McpJsonFile::load(&config_path)?;
    Ok(mcp_file.into_entries(McpScope::User))
}

fn claude_desktop_config_path() -> Option<std::path::PathBuf> {
    #[cfg(target_os = "macos")]
    {
        dirs::home_dir()
            .map(|h| h.join("Library/Application Support/Claude/claude_desktop_config.json"))
    }
    #[cfg(target_os = "linux")]
    {
        dirs::config_dir().map(|c| c.join("Claude/claude_desktop_config.json"))
    }
    #[cfg(target_os = "windows")]
    {
        dirs::config_dir().map(|c| c.join("Claude/claude_desktop_config.json"))
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
    {
        None
    }
}
