//! MCP Client — connects to external MCP servers and invokes tools.
//!
//! Supports two transports:
//! - **stdio**: spawns a child process, communicates via JSON-RPC over stdin/stdout.
//! - **http**: sends JSON-RPC requests over HTTP POST (Streamable HTTP transport).
//!
//! The client performs the MCP initialize/initialized handshake, discovers tools
//! via `tools/list`, and invokes tools via `tools/call`.

use crate::config::McpServerEntry;
use crate::error::{AppError, Result};
use crate::mcp::types::{Tool, ToolsCallResult};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use tokio::sync::RwLock;

/// JSON-RPC request envelope used by the MCP client.
#[derive(Debug, Serialize)]
struct JsonRpcClientRequest {
    jsonrpc: &'static str,
    id: u64,
    method: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    params: Option<serde_json::Value>,
}

/// JSON-RPC response envelope received from MCP servers.
#[derive(Debug, Deserialize)]
struct JsonRpcClientResponse {
    #[allow(dead_code)]
    jsonrpc: String,
    #[allow(dead_code)]
    id: Option<serde_json::Value>,
    result: Option<serde_json::Value>,
    error: Option<JsonRpcClientError>,
}

/// JSON-RPC error object.
#[derive(Debug, Deserialize)]
struct JsonRpcClientError {
    code: i64,
    message: String,
    #[allow(dead_code)]
    data: Option<serde_json::Value>,
}

/// An active connection to a single MCP server.
#[derive(Debug)]
pub struct McpClient {
    /// Server name (from config).
    pub name: String,
    /// Transport backend.
    transport: McpTransport,
    /// Monotonically increasing request ID.
    next_id: AtomicU64,
    /// Tools discovered from this server.
    tools: RwLock<Vec<Tool>>,
}

/// Transport backend for an MCP client connection.
#[derive(Debug)]
enum McpTransport {
    /// HTTP transport — uses a shared reqwest client.
    Http {
        url: String,
        headers: HashMap<String, String>,
        client: reqwest::Client,
    },
    /// Stdio transport — owns a child process handle.
    Stdio {
        child: Arc<RwLock<tokio::process::Child>>,
    },
}

/// Global registry of active MCP client connections, keyed by server name.
pub struct McpClientRegistry {
    clients: RwLock<HashMap<String, Arc<McpClient>>>,
}

impl Default for McpClientRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl McpClientRegistry {
    /// Create an empty registry.
    pub fn new() -> Self {
        Self {
            clients: RwLock::new(HashMap::new()),
        }
    }

    /// Connect to an MCP server described by `entry`.
    ///
    /// On success the client is stored in the registry and its discovered tools
    /// are returned.
    pub async fn connect(&self, entry: &McpServerEntry) -> Result<Vec<Tool>> {
        if !entry.enabled {
            return Err(AppError::Io(std::io::Error::other(format!(
                "MCP server '{}' is disabled",
                entry.name
            ))));
        }

        let client = McpClient::connect(entry).await?;
        let tools = client.tools.read().await.clone();
        self.clients
            .write()
            .await
            .insert(entry.name.clone(), Arc::new(client));
        Ok(tools)
    }

    /// Get an active client by server name.
    pub async fn get(&self, name: &str) -> Option<Arc<McpClient>> {
        self.clients.read().await.get(name).cloned()
    }

    /// Remove and drop a client connection.
    pub async fn disconnect(&self, name: &str) {
        self.clients.write().await.remove(name);
    }

    /// List all connected server names.
    pub async fn connected_servers(&self) -> Vec<String> {
        self.clients.read().await.keys().cloned().collect()
    }

    /// Get all discovered tools across all connected servers.
    pub async fn all_tools(&self) -> Vec<(String, Vec<Tool>)> {
        let clients = self.clients.read().await;
        let mut out = Vec::with_capacity(clients.len());
        for (name, client) in clients.iter() {
            let tools = client.tools.read().await.clone();
            out.push((name.clone(), tools));
        }
        out
    }

    /// Call a tool on a specific server.
    pub async fn call_tool(
        &self,
        server_name: &str,
        tool_name: &str,
        arguments: serde_json::Value,
    ) -> Result<ToolsCallResult> {
        let client = self.get(server_name).await.ok_or_else(|| {
            AppError::Io(std::io::Error::other(format!(
                "MCP server '{}' is not connected",
                server_name
            )))
        })?;
        client.call_tool(tool_name, arguments).await
    }
}

// ============================================================================
// McpClient implementation
// ============================================================================

impl McpClient {
    /// Connect to an MCP server, perform the initialize handshake, and
    /// discover tools.
    pub async fn connect(entry: &McpServerEntry) -> Result<Self> {
        use crate::config::McpTransportType;

        let transport = match entry.transport {
            McpTransportType::Http | McpTransportType::Sse => {
                let url = entry.url.clone().ok_or_else(|| {
                    AppError::Io(std::io::Error::other(format!(
                        "MCP server '{}': HTTP transport requires a url",
                        entry.name
                    )))
                })?;
                McpTransport::Http {
                    url,
                    headers: entry.headers.clone(),
                    client: reqwest::Client::builder()
                        .timeout(std::time::Duration::from_secs(entry.timeout_secs))
                        .build()
                        .map_err(|e| {
                            AppError::Io(std::io::Error::other(format!(
                                "Failed to create HTTP client: {e}"
                            )))
                        })?,
                }
            }
            McpTransportType::Stdio => {
                let command = entry.command.as_deref().ok_or_else(|| {
                    AppError::Io(std::io::Error::other(format!(
                        "MCP server '{}': stdio transport requires a command",
                        entry.name
                    )))
                })?;

                let mut cmd = tokio::process::Command::new(command);
                cmd.args(&entry.args)
                    .envs(&entry.env)
                    .stdin(std::process::Stdio::piped())
                    .stdout(std::process::Stdio::piped())
                    .stderr(std::process::Stdio::null());

                let child = cmd.spawn().map_err(|e| {
                    AppError::Io(std::io::Error::other(format!(
                        "Failed to spawn MCP server '{}' ({}): {e}",
                        entry.name, command
                    )))
                })?;
                McpTransport::Stdio {
                    child: Arc::new(RwLock::new(child)),
                }
            }
        };

        let client = Self {
            name: entry.name.clone(),
            transport,
            next_id: AtomicU64::new(1),
            tools: RwLock::new(Vec::new()),
        };

        // Perform MCP initialize handshake
        client.initialize().await?;

        // Discover tools
        let tools = client.list_tools_rpc().await?;
        *client.tools.write().await = tools;

        tracing::info!(
            "MCP client '{}': connected, {} tools discovered",
            client.name,
            client.tools.read().await.len()
        );

        Ok(client)
    }

    /// Send a JSON-RPC request and return the result value.
    async fn rpc(
        &self,
        method: &str,
        params: Option<serde_json::Value>,
    ) -> Result<serde_json::Value> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let request = JsonRpcClientRequest {
            jsonrpc: "2.0",
            id,
            method: method.to_string(),
            params,
        };

        let response_value = match &self.transport {
            McpTransport::Http {
                url,
                headers,
                client,
            } => {
                let mut req = client.post(url).json(&request);
                for (k, v) in headers {
                    req = req.header(k, v);
                }
                let resp = req.send().await.map_err(|e| {
                    AppError::Io(std::io::Error::other(format!(
                        "MCP HTTP request to '{}' failed: {e}",
                        self.name
                    )))
                })?;
                if !resp.status().is_success() {
                    let status = resp.status();
                    let body = resp.text().await.unwrap_or_default();
                    return Err(AppError::Io(std::io::Error::other(format!(
                        "MCP server '{}' HTTP {}: {}",
                        self.name, status, body
                    ))));
                }
                resp.json::<JsonRpcClientResponse>().await.map_err(|e| {
                    AppError::Io(std::io::Error::other(format!(
                        "MCP server '{}': invalid JSON-RPC response: {e}",
                        self.name
                    )))
                })?
            }
            McpTransport::Stdio { child } => self.stdio_rpc(child, &request).await?,
        };

        if let Some(err) = response_value.error {
            return Err(AppError::Io(std::io::Error::other(format!(
                "MCP server '{}' RPC error {}: {}",
                self.name, err.code, err.message
            ))));
        }

        response_value.result.ok_or_else(|| {
            AppError::Io(std::io::Error::other(format!(
                "MCP server '{}': empty result for method '{}'",
                self.name, method
            )))
        })
    }

    /// Send/receive a single JSON-RPC message over a child process stdio.
    async fn stdio_rpc(
        &self,
        child: &Arc<RwLock<tokio::process::Child>>,
        request: &JsonRpcClientRequest,
    ) -> Result<JsonRpcClientResponse> {
        use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

        let mut guard = child.write().await;

        let stdin = guard.stdin.as_mut().ok_or_else(|| {
            AppError::Io(std::io::Error::other(format!(
                "MCP server '{}': stdin not available",
                self.name
            )))
        })?;
        let mut line = serde_json::to_string(request).map_err(|e| {
            AppError::Io(std::io::Error::other(format!(
                "Failed to serialize JSON-RPC request: {e}"
            )))
        })?;
        line.push('\n');
        stdin.write_all(line.as_bytes()).await?;
        stdin.flush().await?;

        let stdout = guard.stdout.as_mut().ok_or_else(|| {
            AppError::Io(std::io::Error::other(format!(
                "MCP server '{}': stdout not available",
                self.name
            )))
        })?;
        let mut reader = BufReader::new(stdout);
        let mut buf = String::new();
        reader.read_line(&mut buf).await?;

        serde_json::from_str(&buf).map_err(|e| {
            AppError::Io(std::io::Error::other(format!(
                "MCP server '{}': invalid JSON-RPC response on stdout: {e}",
                self.name
            )))
        })
    }

    /// Perform the MCP `initialize` / `notifications/initialized` handshake.
    async fn initialize(&self) -> Result<()> {
        let params = serde_json::json!({
            "protocolVersion": "2025-11-25",
            "capabilities": {},
            "clientInfo": {
                "name": "gestura",
                "version": env!("CARGO_PKG_VERSION")
            }
        });
        let _result = self.rpc("initialize", Some(params)).await?;

        // Send `notifications/initialized` (no id, no result expected).
        // For HTTP this is fire-and-forget; for stdio we write but don't read.
        let notif = serde_json::json!({
            "jsonrpc": "2.0",
            "method": "notifications/initialized"
        });

        match &self.transport {
            McpTransport::Http {
                url,
                headers,
                client,
            } => {
                let mut req = client.post(url).json(&notif);
                for (k, v) in headers {
                    req = req.header(k, v);
                }
                // Fire-and-forget — ignore errors.
                let _ = req.send().await;
            }
            McpTransport::Stdio { child } => {
                use tokio::io::AsyncWriteExt;
                let mut guard = child.write().await;
                if let Some(stdin) = guard.stdin.as_mut() {
                    let mut line = serde_json::to_string(&notif).unwrap_or_default();
                    line.push('\n');
                    let _ = stdin.write_all(line.as_bytes()).await;
                    let _ = stdin.flush().await;
                }
            }
        }

        tracing::debug!("MCP client '{}': initialized", self.name);
        Ok(())
    }

    /// Discover tools from the server via `tools/list`.
    async fn list_tools_rpc(&self) -> Result<Vec<Tool>> {
        let result = self.rpc("tools/list", None).await?;

        #[derive(Deserialize)]
        struct ToolsListResponse {
            tools: Vec<Tool>,
        }

        let parsed: ToolsListResponse = serde_json::from_value(result).map_err(|e| {
            AppError::Io(std::io::Error::other(format!(
                "MCP server '{}': failed to parse tools/list: {e}",
                self.name
            )))
        })?;
        Ok(parsed.tools)
    }

    /// Invoke a tool on this server via `tools/call`.
    pub async fn call_tool(
        &self,
        tool_name: &str,
        arguments: serde_json::Value,
    ) -> Result<ToolsCallResult> {
        let params = serde_json::json!({
            "name": tool_name,
            "arguments": arguments
        });
        let result = self.rpc("tools/call", Some(params)).await?;

        serde_json::from_value(result).map_err(|e| {
            AppError::Io(std::io::Error::other(format!(
                "MCP server '{}': failed to parse tools/call result: {e}",
                self.name
            )))
        })
    }

    /// Get the list of discovered tools (cached from the last `tools/list`).
    pub async fn get_tools(&self) -> Vec<Tool> {
        self.tools.read().await.clone()
    }

    /// Refresh the tool list from the server.
    pub async fn refresh_tools(&self) -> Result<Vec<Tool>> {
        let tools = self.list_tools_rpc().await?;
        *self.tools.write().await = tools.clone();
        Ok(tools)
    }
}

// ============================================================================
// Global singleton for the MCP client registry
// ============================================================================

static MCP_CLIENT_REGISTRY: std::sync::OnceLock<McpClientRegistry> = std::sync::OnceLock::new();

/// Get the global MCP client registry.
pub fn get_mcp_client_registry() -> &'static McpClientRegistry {
    MCP_CLIENT_REGISTRY.get_or_init(McpClientRegistry::new)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_starts_empty() {
        let registry = McpClientRegistry::new();
        let rt = tokio::runtime::Runtime::new().unwrap();
        let servers = rt.block_on(registry.connected_servers());
        assert!(servers.is_empty());
    }

    #[test]
    fn registry_disconnect_nonexistent_is_noop() {
        let registry = McpClientRegistry::new();
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(registry.disconnect("nonexistent"));
        assert!(rt.block_on(registry.connected_servers()).is_empty());
    }
}
